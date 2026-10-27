use std::borrow::Cow;
use std::collections::HashMap;

use redmine_api::api::issues::{CreateIssue, GetIssue, Issue, IssueWrapper, ListIssues, UpdateIssue, DeleteIssue, IssueInclude, IssueStatusFilter};
use redmine_api::api::issue_statuses::{ListIssueStatuses, IssueStatusesWrapper, IssueStatus};
use redmine_api::api::projects::{GetProject, ListProjects, Project, ProjectWrapper};
use redmine_api::api::{Endpoint, Redmine};
use redmine_api::reqwest::Method;
use serde::Serialize;

use gittask::{Task, Comment, Label, Subtask};

use crate::connectors::{status_names_match, RemoteConnector, RemoteTaskState};

#[derive(Debug, Clone, Serialize)]
struct UpdateJournalInner {
    notes: String,
}

#[derive(Debug, Clone, Serialize)]
struct UpdateJournalWrapper {
    journal: UpdateJournalInner,
}

struct UpdateJournal {
    journal_id: u64,
    notes: String,
}

impl Endpoint for UpdateJournal {
    fn method(&self) -> Method {
        Method::PUT
    }

    fn endpoint(&self) -> Cow<'static, str> {
        format!("journals/{}.json", self.journal_id).into()
    }

    fn body(&self) -> Result<Option<(&'static str, Vec<u8>)>, redmine_api::Error> {
        let wrapper = UpdateJournalWrapper {
            journal: UpdateJournalInner {
                notes: self.notes.clone(),
            },
        };
        Ok(Some(("application/json", serde_json::to_vec(&wrapper)?)))
    }
}

pub struct RedmineRemoteConnector;

impl RemoteConnector for RedmineRemoteConnector {
    fn type_name(&self) -> &str {
        "redmine"
    }

    fn get_config_options(&self) -> Option<Vec<String>> {
        Some(vec![
            "task.redmine.url".to_string(),
            "task.redmine.api.key".to_string(),
            "task.redmine.project.id".to_string(),
        ])
    }

    fn supports_remote(&self, _url: &str) -> Option<(String, String)> {
        match gittask::get_config_value("task.redmine.url") {
            Ok(_) => match gittask::get_config_value("task.redmine.project.id") {
                Ok(project_id) => Some((String::new(), project_id)),
                _ => None,
            }
            _ => None,
        }
    }

    fn remote_for_project(&self, _url: &str, project: &str) -> Option<(String, String)> {
        gittask::get_config_value("task.redmine.url").ok().map(|_| (String::new(), project.to_string()))
    }

    fn list_remote_tasks(
        &self,
        domain: &String,
        project: &String,
        with_comments: bool,
        _with_labels: bool,
        limit: Option<usize>,
        state: RemoteTaskState,
        _task_statuses: &Vec<String>
    ) -> Result<Vec<Task>, String> {
        let redmine = get_redmine_instance(domain)?;
        let project_id = resolve_project_id(&redmine, project)?;
        let status_filter = match &state {
            RemoteTaskState::Open(status, _) | RemoteTaskState::Closed(status, _) => {
                if let Ok(Some(status_id)) = get_status_id_by_name(&redmine, status) {
                    Some(IssueStatusFilter::TheseStatuses(vec![status_id]))
                } else {
                    match &state {
                        RemoteTaskState::Open(_, _) => Some(IssueStatusFilter::Open),
                        RemoteTaskState::Closed(_, _) => Some(IssueStatusFilter::Closed),
                        _ => None,
                    }
                }
            },
            RemoteTaskState::All => Some(IssueStatusFilter::All),
        };
        let mut builder = ListIssues::builder();
        builder.project_id(vec![project_id]);
        if let Some(filter) = status_filter {
            builder.status_id(filter);
        }
        let endpoint = builder.build().map_err(|e| e.to_string())?;
        let issues = match limit {
            Some(limit) => collect_pages(limit, |offset, page_size| {
                let response = redmine
                    .json_response_body_page::<_, Issue>(&endpoint, offset, page_size)
                    .map_err(|e| e.to_string())?;
                Ok((response.values, response.total_count))
            })?,
            None => redmine
                .json_response_body_all_pages::<_, Issue>(&endpoint)
                .map_err(|e| e.to_string())?
        };

        let mut tasks = Vec::new();
        for issue in issues {
            let mut task = issue_to_task(&issue)?;

            if with_comments {
                let mut builder = GetIssue::builder();
                builder.id(issue.id);
                builder.include(vec![IssueInclude::Journals]);
                let endpoint = builder.build().map_err(|e| e.to_string())?;
                let IssueWrapper { issue: detailed } =
                    redmine
                        .json_response_body::<_, IssueWrapper<Issue>>(&endpoint)
                        .map_err(|e| e.to_string())?;

                append_journals_as_comments(&detailed, &mut task);
            }

            tasks.push(task);
        }
        Ok(tasks)
    }

    fn get_remote_task(
        &self,
        domain: &String,
        project: &String,
        task_id: &String,
        with_comments: bool,
        _with_labels: bool,
        _task_statuses: &Vec<String>
    ) -> Result<Option<Task>, String> {
        let redmine = get_redmine_instance(domain)?;
        let issue_id = task_id.parse().map_err(|_| format!("Invalid task ID: {task_id}"))?;
        let include = if with_comments { Some(vec![IssueInclude::Journals]) } else { None };
        let local_project = gittask::find_task(task_id).ok().flatten().and_then(|task| task.get_property("project_id").cloned());
        let issue = match get_issue_in_project(&redmine, project, local_project.as_ref(), issue_id, include)? {
            Some(issue) => issue,
            None => return Ok(None),
        };

        let mut task = issue_to_task(&issue)?;

        if with_comments {
            append_journals_as_comments(&issue, &mut task);
        }

        Ok(Some(task))
    }

    fn create_remote_task(&self, domain: &String, _project: &String, task: &Task) -> Result<String, String> {
        let redmine = get_redmine_instance(domain)?;

        let mut builder = CreateIssue::builder();
        let project = match task.get_property("project_id") {
            Some(project_id) => project_id.to_string(),
            _ => gittask::get_config_value("task.redmine.project.id")?
        };
        let project_id = resolve_project_id(&redmine, &project)?;
        builder.project_id(project_id);
        let subject = task.get_property("name").ok_or_else(|| "Task name is missing".to_string())?.clone();
        builder.subject(subject);

        if let Some(desc) = task.get_property("description") {
            builder.description(desc.clone());
        }

        let endpoint = builder.build().map_err(|e| e.to_string())?;
        let IssueWrapper { issue } = redmine
            .json_response_body::<_, IssueWrapper<Issue>>(&endpoint)
            .map_err(|e| e.to_string())?;

        Ok(issue.id.to_string())
    }

    fn create_remote_comment(&self, domain: &String, _project: &String, task_id: &String, comment: &Comment) -> Result<String, String> {
        let redmine = get_redmine_instance(domain)?;

        let id = task_id
            .parse::<u64>()
            .map_err(|e| format!("Invalid task id '{}': {}", task_id, e))?;

        let endpoint = UpdateIssue::builder()
            .id(id)
            .notes(comment.get_text().into())
            .build()
            .map_err(|e| e.to_string())?;

        redmine.ignore_response_body(&endpoint).map_err(|e| e.to_string())?;

        let mut get_builder = GetIssue::builder();
        get_builder.id(id);
        get_builder.include(vec![IssueInclude::Journals]);
        let get_endpoint = get_builder.build().map_err(|e| e.to_string())?;
        let IssueWrapper { issue } = redmine
            .json_response_body::<_, IssueWrapper<Issue>>(&get_endpoint)
            .map_err(|e| e.to_string())?;

        if let Some(journals) = &issue.journals {
            let comment_text = comment.get_text();
            let matching_journal = journals
                .iter()
                .rev()
                .find(|j| j.notes.as_ref().map(|n| n == &comment_text).unwrap_or(false));

            if let Some(journal) = matching_journal {
                return Ok(journal.id.to_string());
            }
        }

        Err("Could not find matching journal for comment".to_string())
    }

    #[allow(unused)]
    fn create_remote_label(&self, domain: &String, project: &String, task_id: &String, label: &Label) -> Result<(), String> {
        Err("Labels are not supported for Redmine".to_string())
    }

    fn update_remote_task(
        &self,
        domain: &String,
        project: &String,
        task: &Task,
        _labels: Option<&Vec<Label>>,
        state: RemoteTaskState
    ) -> Result<(), String> {
        let redmine = get_redmine_instance(domain)?;

        let id = task.get_id()
            .ok_or_else(|| "Task id is required for update".to_string())?
            .parse::<u64>()
            .map_err(|e| format!("Invalid task id '{}': {}", task.get_id().unwrap(), e))?;

        get_issue_in_project(&redmine, project, task.get_property("project_id"), id, None)?.ok_or_else(|| format!("Issue #{id} not found in project '{project}'"))?;

        let mut builder = UpdateIssue::builder();
        builder.id(id);

        if let Some(name) = task.get_property("name") { builder.subject(name.clone()); }
        if let Some(desc) = task.get_property("description") { builder.description(desc.clone()); }

        if let Some(project_id_prop) = task.get_property("project_id") {
            let proj_id = resolve_project_id(&redmine, project_id_prop)?;
            builder.project_id(proj_id);
        }

        let (local_status, remote_status) = match state {
            RemoteTaskState::Open(s1, s2) => (s1, s2),
            RemoteTaskState::Closed(s1, s2) => (s1, s2),
            RemoteTaskState::All => (String::new(), String::new()),
        };

        if !local_status.is_empty() && !remote_status.is_empty() && local_status != remote_status {
            match get_status_id_by_name(&redmine, &local_status)? {
                Some(status_id) => { builder.status_id(status_id); },
                None => eprintln!("WARNING: no Redmine status matches '{local_status}', remote status left as '{remote_status}'"),
            }
        }

        let update_endpoint = builder.build().map_err(|e| e.to_string())?;
        redmine.ignore_response_body(&update_endpoint).map_err(|e| e.to_string())
    }

    fn update_remote_comment(&self, domain: &String, _project: &String, _task_id: &String, comment_id: &String, text: &String) -> Result<(), String> {
        let redmine = get_redmine_instance(domain)?;

        let journal_id = comment_id
            .parse::<u64>()
            .map_err(|e| format!("Invalid comment id '{}': {}", comment_id, e))?;

        let endpoint = UpdateJournal {
            journal_id,
            notes: text.clone(),
        };

        redmine.ignore_response_body(&endpoint).map_err(|e| e.to_string())
    }

    fn delete_remote_task(&self, domain: &String, _project: &String, task_id: &String) -> Result<(), String> {
        let redmine = get_redmine_instance(domain)?;

        let id = task_id
            .parse::<u64>()
            .map_err(|e| format!("Invalid task id '{}': {}", task_id, e))?;

        let endpoint = DeleteIssue::builder()
            .id(id)
            .build()
            .map_err(|e| e.to_string())?;

        redmine.ignore_response_body(&endpoint).map_err(|e| e.to_string())
    }

    #[allow(unused)]
    fn delete_remote_comment(&self, domain: &String, _project: &String, task_id: &String, comment_id: &String) -> Result<(), String> {
        Err("Comment deletion is not supported for Redmine".to_string())
    }

    #[allow(unused)]
    fn delete_remote_label(&self, domain: &String, project: &String, task_id: &String, name: &String) -> Result<(), String> {
        Err("Labels are not supported for Redmine".to_string())
    }

    fn supports_subtasks(&self) -> bool {
        true
    }

    fn supports_labels(&self) -> bool {
        false
    }

    fn list_remote_subtasks(&self, domain: &String, _project: &String, task_id: &String, _task_statuses: &Vec<String>) -> Result<Vec<Subtask>, String> {
        let redmine = get_redmine_instance(domain)?;

        let parent_id = task_id
            .parse::<u64>()
            .map_err(|e| format!("Invalid task id '{}': {}", task_id, e))?;

        let mut builder = ListIssues::builder();
        builder.parent_id(vec![parent_id]);
        builder.status_id(IssueStatusFilter::All);
        let endpoint = builder.build().map_err(|e| e.to_string())?;

        let issues = redmine
            .json_response_body_all_pages::<_, Issue>(&endpoint)
            .map_err(|e| e.to_string())?;

        let mut result = vec![];
        for issue in issues {
            let name = issue.subject.clone().unwrap_or_default();
            let status = issue.status.name.clone();
            let mut props = HashMap::new();
            props.insert("created".to_string(), issue.created_on.unix_timestamp().to_string());
            props.insert("author".to_string(), issue.author.name.clone());
            result.push(Subtask::new(Some(issue.id.to_string()), name, status, props));
        }
        Ok(result)
    }

    fn create_remote_subtask(&self, domain: &String, _project: &String, task_id: &String, subtask: &Subtask, _is_done: bool) -> Result<String, String> {
        let redmine = get_redmine_instance(domain)?;

        let parent_id = task_id
            .parse::<u64>()
            .map_err(|e| format!("Invalid task id '{}': {}", task_id, e))?;

        let mut get_builder = GetIssue::builder();
        get_builder.id(parent_id);
        let get_endpoint = get_builder.build().map_err(|e| e.to_string())?;
        let IssueWrapper { issue: parent } = redmine
            .json_response_body::<_, IssueWrapper<Issue>>(&get_endpoint)
            .map_err(|e| e.to_string())?;

        let project_id = parent.project.id;

        let mut builder = CreateIssue::builder();
        builder.project_id(project_id);
        builder.subject(subtask.get_name().to_string());
        builder.parent_issue_id(parent_id);

        if let Some(status_id) = subtask_status_id(&redmine, subtask)? {
            builder.status_id(status_id);
        }

        let endpoint = builder.build().map_err(|e| e.to_string())?;
        let IssueWrapper { issue } = redmine
            .json_response_body::<_, IssueWrapper<Issue>>(&endpoint)
            .map_err(|e| e.to_string())?;

        Ok(issue.id.to_string())
    }

    fn update_remote_subtask(&self, domain: &String, _project: &String, _task_id: &String, subtask: &Subtask, _is_done: bool) -> Result<(), String> {
        let redmine = get_redmine_instance(domain)?;

        let id = subtask.get_id()
            .ok_or_else(|| "Subtask has no ID".to_string())?
            .parse::<u64>()
            .map_err(|e| format!("Invalid subtask id: {}", e))?;

        let mut builder = UpdateIssue::builder();
        builder.id(id);
        builder.subject(subtask.get_name().to_string());

        if let Some(status_id) = subtask_status_id(&redmine, subtask)? {
            builder.status_id(status_id);
        }

        let endpoint = builder.build().map_err(|e| e.to_string())?;
        redmine.ignore_response_body(&endpoint).map_err(|e| e.to_string())
    }

    fn delete_remote_subtask(&self, domain: &String, _project: &String, _task_id: &String, subtask_id: &String) -> Result<(), String> {
        let redmine = get_redmine_instance(domain)?;

        let id = subtask_id
            .parse::<u64>()
            .map_err(|e| format!("Invalid subtask id '{}': {}", subtask_id, e))?;

        let endpoint = DeleteIssue::builder()
            .id(id)
            .build()
            .map_err(|e| e.to_string())?;

        redmine.ignore_response_body(&endpoint).map_err(|e| e.to_string())
    }
}

fn get_issue_in_project(redmine: &Redmine, project: &String, local_project: Option<&String>, issue_id: u64, include: Option<Vec<IssueInclude>>) -> Result<Option<Issue>, String> {
    let required_project = match local_project {
        Some(_) => None,
        None => Some(resolve_project_id(redmine, project)?),
    };

    let mut builder = GetIssue::builder();
    builder.id(issue_id);
    if let Some(include) = include {
        builder.include(include);
    }
    let endpoint = builder.build().map_err(|e| e.to_string())?;
    let IssueWrapper { issue } = match redmine.json_response_body::<_, IssueWrapper<Issue>>(&endpoint) {
        Ok(wrapper) => wrapper,
        Err(redmine_api::Error::HttpErrorResponse(status)) if status.as_u16() == 404 => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };

    if required_project.is_some_and(|required| issue.project.id != required) {
        eprintln!("Issue #{issue_id} belongs to project '{}' (id {}), not to project '{project}'", issue.project.name, issue.project.id);
        return Ok(None);
    }

    Ok(Some(issue))
}

static PROJECT_IDS: std::sync::OnceLock<std::sync::Mutex<HashMap<(String, String), u64>>> = std::sync::OnceLock::new();

fn resolve_project_id(redmine: &Redmine, project: &String) -> Result<u64, String> {
    if let Ok(id) = project.parse::<u64>() {
        return Ok(id);
    }

    let key = (redmine.redmine_url().to_string(), project.to_lowercase());
    let cache = PROJECT_IDS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    if let Some(id) = cache.lock().unwrap().get(&key) {
        return Ok(*id);
    }

    let id = match lookup_project_by_identifier(redmine, project) {
        Some(id) => id,
        None => find_project_in_listing(redmine, project)?,
    };
    cache.lock().unwrap().insert(key, id);
    Ok(id)
}

fn lookup_project_by_identifier(redmine: &Redmine, project: &str) -> Option<u64> {
    let endpoint = GetProject::builder().project_id_or_name(project).build().ok()?;
    redmine.json_response_body::<_, ProjectWrapper<Project>>(&endpoint).ok().map(|wrapper| wrapper.project.id)
}

fn find_project_in_listing(redmine: &Redmine, project: &str) -> Result<u64, String> {
    let endpoint = ListProjects::builder().build().map_err(|e| e.to_string())?;
    let projects = redmine
        .json_response_body_all_pages::<_, Project>(&endpoint)
        .map_err(|e| e.to_string())?;

    let lower = project.to_lowercase();
    projects.iter()
        .find(|p| p.identifier.to_lowercase() == lower || p.name.to_lowercase() == lower)
        .map(|p| p.id)
        .ok_or_else(|| format!("Project '{}' not found on Redmine server", project))
}

fn get_redmine_instance(domain: &String) -> Result<Redmine, String> {
    let client = redmine_api::reqwest::blocking::Client::builder().use_rustls_tls().build()
        .map_err(|e| e.to_string())?;
    let url = get_base_url(domain)?;
    let api_key = get_api_key()?;
    Redmine::new(client, url.parse().map_err(|e| format!("Invalid Redmine URL '{url}': {e}"))?, &*api_key).map_err(|e| e.to_string())
}

fn subtask_status_id(redmine: &Redmine, subtask: &Subtask) -> Result<Option<u64>, String> {
    let status = subtask.get_status();
    if status.is_empty() {
        return Ok(None);
    }
    let status_id = get_status_id_by_name(redmine, status).map_err(|e| format!("Failed to look up Redmine status '{status}': {e}"))?;
    if status_id.is_none() {
        eprintln!("WARNING: no Redmine status matches '{status}', remote subtask status left unchanged");
    }
    Ok(status_id)
}

fn get_status_id_by_name(redmine: &Redmine, name: &str) -> Result<Option<u64>, String> {
    let endpoint = ListIssueStatuses::builder().build().map_err(|e| e.to_string())?;
    let IssueStatusesWrapper { issue_statuses } =
        redmine
            .json_response_body::<_, IssueStatusesWrapper<IssueStatus>>(&endpoint)
            .map_err(|e| e.to_string())?;

    Ok(issue_statuses
        .iter()
        .find(|s| status_names_match(&s.name, name))
        .map(|s| s.id))
}

fn get_base_url(domain: &String) -> Result<String, String> {
    match gittask::get_config_value("task.redmine.url") {
        Ok(url) => Ok(url),
        _ => match std::env::var("REDMINE_URL") {
            Ok(url) => Ok(url),
            _ => Ok(format!("https://{}", domain)),
        }
    }
}

fn get_api_key() -> Result<String, String> {
    match gittask::get_config_value("task.redmine.api.key") {
        Ok(key) => Ok(key),
        _ => std::env::var("REDMINE_API_KEY")
            .or_else(|_| std::env::var("REDMINE_TOKEN"))
            .map_err(|_| "No Redmine API key found. Set task.redmine.api.key config option, REDMINE_API_KEY or REDMINE_TOKEN environment variable.".to_string())
    }
}

const REDMINE_PAGE_SIZE: u64 = 100;

fn collect_pages<T>(limit: usize, mut fetch: impl FnMut(u64, u64) -> Result<(Vec<T>, u64), String>) -> Result<Vec<T>, String> {
    let mut collected = Vec::new();
    while collected.len() < limit {
        let remaining = (limit - collected.len()) as u64;
        let page_size = remaining.min(REDMINE_PAGE_SIZE);
        let (values, total_count) = fetch(collected.len() as u64, page_size)?;
        let short_page = (values.len() as u64) < page_size;
        collected.extend(values);
        if short_page || collected.len() as u64 >= total_count {
            break;
        }
    }
    Ok(collected)
}

fn issue_to_task(issue: &Issue) -> Result<Task, String> {
    let mut props = HashMap::new();
    props.insert("name".to_string(), issue.subject.clone().unwrap_or_else(|| String::new()));
    props.insert("status".to_string(), issue.status.name.clone());
    props.insert("description".to_string(), issue.description.clone().unwrap_or_else(|| String::new()));
    
    let created_on= &issue.created_on;
    props.insert("created".to_string(), created_on.unix_timestamp().to_string());
    
    let author = &issue.author;
    props.insert("author".to_string(), author.name.clone());

    let project_id = issue.project.id;
    props.insert("project_id".to_string(), project_id.to_string());
    
    Task::from_properties(issue.id.to_string(), props).map_err(|e| e.to_string())
}

fn append_journals_as_comments(issue: &Issue, task: &mut Task) {
    task.set_comments(vec![]);
    if let Some(journals) = &issue.journals {
        for j in journals {
            if let Some(text) = j.notes.as_ref().filter(|text| !text.trim().is_empty()) {
                let mut props = HashMap::new();
                props.insert("author".to_string(), j.user.name.clone());
                props.insert("created".to_string(), j.created_on.unix_timestamp().to_string());
                if j.private_notes {
                    props.insert("private".to_string(), "true".to_string());
                }
                task.add_comment(Some(j.id.to_string()), props, text.clone());
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn stub_redmine(handler: impl Fn(&str, &str) -> (u16, String) + Send + 'static) -> (Redmine, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stub server");
        let addr = listener.local_addr().unwrap();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream { Ok(s) => s, Err(_) => continue };
                let mut data = Vec::new();
                let mut buf = [0u8; 4096];
                let header_end = [13u8, 10, 13, 10];
                while !data.windows(4).any(|w| w == header_end) {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => data.extend_from_slice(&buf[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&data).to_string();
                let mut parts = head.lines().next().unwrap_or_default().split_whitespace();
                let method = parts.next().unwrap_or_default().to_string();
                let path = parts.next().unwrap_or_default().split('?').next().unwrap_or_default().to_string();
                log.lock().unwrap().push(format!("{method} {path}"));
                let (status, body) = handler(&method, &path);
                let crlf = [13u8, 10];
                let status_line = format!("HTTP/1.1 {status} {}", if status < 300 { "OK" } else { "Error" });
                let length = format!("Content-Length: {}", body.len());
                let response = [status_line.as_bytes(), &crlf, b"Content-Type: application/json", &crlf, length.as_bytes(), &crlf, b"Connection: close", &crlf, &crlf, body.as_bytes()].concat();
                let _ = stream.write_all(&response);
            }
        });
        let client = redmine_api::reqwest::blocking::Client::new();
        let redmine = Redmine::new(client, format!("http://{addr}/").parse().unwrap(), "key").unwrap();
        (redmine, requests)
    }

    fn project_json(id: u64, identifier: &str, name: &str) -> String {
        format!(r#"{{"id":{id},"name":"{name}","identifier":"{identifier}","status":1,"created_on":"2026-10-16T10:00:00Z","updated_on":"2026-10-16T10:00:00Z"}}"#)
    }

    fn redmine_projects(identifier: &'static str, name: &'static str) -> impl Fn(&str, &str) -> (u16, String) + Send + 'static {
        move |_, path| {
            if path == format!("/projects/{identifier}.json") {
                (200, format!(r#"{{"project":{}}}"#, project_json(42, identifier, name)))
            } else if path == "/projects.json" {
                (200, format!(r#"{{"projects":[{},{}],"total_count":2,"offset":0,"limit":100}}"#, project_json(7, "other", "Other"), project_json(42, identifier, name)))
            } else {
                (404, r#"{"errors":["Not found"]}"#.to_string())
            }
        }
    }

    #[test]
    fn resolve_project_id_looks_the_identifier_up_directly_and_caches_it() {
        let (redmine, requests) = stub_redmine(redmine_projects("ci-direct", "Continuous Integration"));

        assert_eq!(resolve_project_id(&redmine, &"ci-direct".to_string()), Ok(42));
        assert_eq!(resolve_project_id(&redmine, &"ci-direct".to_string()), Ok(42));
        assert_eq!(resolve_project_id(&redmine, &"CI-Direct".to_string()), Ok(42), "the cache is case-insensitive like the lookup");

        assert_eq!(requests.lock().unwrap().as_slice(), ["GET /projects/ci-direct.json"], "one direct request, no listing, nothing repeated");
    }

    #[test]
    fn resolve_project_id_falls_back_to_the_listing_for_a_project_name() {
        let (redmine, requests) = stub_redmine(redmine_projects("ci-byname", "Continuous Integration"));

        assert_eq!(resolve_project_id(&redmine, &"continuous integration".to_string()), Ok(42));

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2, "a name is not a URL identifier, so the listing is needed: {requests:?}");
        assert_eq!(requests[1], "GET /projects.json");
    }

    #[test]
    fn resolve_project_id_reports_an_unknown_project() {
        let (redmine, _requests) = stub_redmine(redmine_projects("ci-unknown", "Continuous Integration"));

        let result = resolve_project_id(&redmine, &"nope".to_string());

        assert_eq!(result, Err("Project 'nope' not found on Redmine server".to_string()));
    }

    fn redmine_statuses(status_code: u16) -> impl Fn(&str, &str) -> (u16, String) + Send + 'static {
        move |method, path| {
            if path == "/issue_statuses.json" {
                (status_code, r#"{"issue_statuses":[{"id":1,"name":"New","is_closed":false},{"id":5,"name":"Closed","is_closed":true}]}"#.to_string())
            } else if method == "PUT" && path == "/issues/7.json" {
                (204, String::new())
            } else {
                (404, r#"{"errors":["Not found"]}"#.to_string())
            }
        }
    }

    fn subtask(status: &str) -> Subtask {
        Subtask::new(Some("7".to_string()), "Child".to_string(), status.to_string(), HashMap::new())
    }

    #[test]
    fn subtask_status_id_resolves_a_matching_status() {
        let (redmine, _requests) = stub_redmine(redmine_statuses(200));

        assert_eq!(subtask_status_id(&redmine, &subtask("CLOSED")), Ok(Some(5)));
        assert_eq!(subtask_status_id(&redmine, &subtask("REVIEW")), Ok(None), "an unknown status name is reported as missing, not as an error");
        assert_eq!(subtask_status_id(&redmine, &subtask("")), Ok(None));
    }

    #[test]
    fn subtask_status_id_propagates_a_failed_lookup() {
        let (redmine, _requests) = stub_redmine(redmine_statuses(403));

        let result = subtask_status_id(&redmine, &subtask("CLOSED"));

        assert!(result.as_ref().is_err_and(|e| e.starts_with("Failed to look up Redmine status 'CLOSED'")), "{result:?}");
    }

    #[test]
    fn update_remote_subtask_fails_instead_of_dropping_the_status_when_the_lookup_fails() {
        let _repo = gittask::testing::TestRepo::new();
        let (redmine, requests) = stub_redmine(redmine_statuses(403));
        gittask::set_config_value("task.redmine.url", redmine.redmine_url().as_str()).unwrap();
        gittask::set_config_value("task.redmine.api.key", "key").unwrap();

        let result = RedmineRemoteConnector.update_remote_subtask(&String::new(), &"1".to_string(), &"1".to_string(), &subtask("CLOSED"), true);

        assert!(result.is_err(), "{result:?}");
        assert_eq!(requests.lock().unwrap().as_slice(), ["GET /issue_statuses.json"], "no update may be sent once the status could not be resolved");
    }

    #[test]
    fn update_remote_subtask_sends_the_resolved_status() {
        let _repo = gittask::testing::TestRepo::new();
        let (redmine, requests) = stub_redmine(redmine_statuses(200));
        gittask::set_config_value("task.redmine.url", redmine.redmine_url().as_str()).unwrap();
        gittask::set_config_value("task.redmine.api.key", "key").unwrap();

        let result = RedmineRemoteConnector.update_remote_subtask(&String::new(), &"1".to_string(), &"1".to_string(), &subtask("CLOSED"), true);

        assert_eq!(result, Ok(()));
        assert_eq!(requests.lock().unwrap().as_slice(), ["GET /issue_statuses.json", "PUT /issues/7.json"]);
    }

    fn issue_in_project(project_id: u64) -> String {
        format!(r#"{{"issue":{{"id":7,"project":{{"id":{project_id},"name":"P{project_id}"}},"tracker":{{"id":1,"name":"Bug"}},"status":{{"id":1,"name":"New","is_closed":false}},"priority":{{"id":2,"name":"Normal"}},"author":{{"id":1,"name":"Admin"}},"subject":"Moved","description":"","start_date":null,"due_date":null,"closed_on":null,"done_ratio":0,"estimated_hours":null,"created_on":"2026-10-16T10:00:00Z","updated_on":"2026-10-16T10:00:00Z"}}}}"#)
    }

    fn redmine_issue_in_project(project_id: u64) -> impl Fn(&str, &str) -> (u16, String) + Send + 'static {
        move |method, path| {
            if method == "GET" && path == "/issues/7.json" {
                (200, issue_in_project(project_id))
            } else if method == "PUT" && path == "/issues/7.json" {
                (204, String::new())
            } else {
                (404, r#"{"errors":["Not found"]}"#.to_string())
            }
        }
    }

    fn configure_stub(redmine: &Redmine) {
        gittask::set_config_value("task.redmine.url", redmine.redmine_url().as_str()).unwrap();
        gittask::set_config_value("task.redmine.api.key", "key").unwrap();
    }

    fn local_task_in_project(project_id: &str) {
        let mut task = gittask::Task::new("Moved".to_string(), "".to_string(), "New".to_string()).unwrap();
        task.set_id("7".to_string());
        task.set_property("project_id", project_id);
        gittask::create_task(task).unwrap();
    }

    #[test]
    fn get_remote_task_rejects_a_foreign_issue_the_local_task_does_not_claim() {
        let _repo = gittask::testing::TestRepo::new();
        let (redmine, _requests) = stub_redmine(redmine_issue_in_project(2));
        configure_stub(&redmine);

        let result = RedmineRemoteConnector.get_remote_task(&String::new(), &"1".to_string(), &"7".to_string(), false, false, &vec![]);

        assert!(matches!(result, Ok(None)), "an issue of another project must not be adopted by a local task that does not point at it");
    }

    #[test]
    fn get_remote_task_finds_an_issue_in_the_project_the_local_task_points_at() {
        let _repo = gittask::testing::TestRepo::new();
        let (redmine, _requests) = stub_redmine(redmine_issue_in_project(2));
        configure_stub(&redmine);
        local_task_in_project("2");

        let result = RedmineRemoteConnector.get_remote_task(&String::new(), &"1".to_string(), &"7".to_string(), false, false, &vec![]);

        let task = result.unwrap().expect("the moved issue must still be found after the move");
        assert_eq!(task.get_property("project_id"), Some(&"2".to_string()));
    }

    #[test]
    fn get_remote_task_finds_an_issue_still_in_the_configured_project_before_a_move() {
        let _repo = gittask::testing::TestRepo::new();
        let (redmine, _requests) = stub_redmine(redmine_issue_in_project(1));
        configure_stub(&redmine);
        local_task_in_project("2");

        let result = RedmineRemoteConnector.get_remote_task(&String::new(), &"1".to_string(), &"7".to_string(), false, false, &vec![]);

        assert!(matches!(result, Ok(Some(_))), "the first push of a project change must find the issue where it still is");
    }

    #[test]
    fn update_remote_task_updates_an_issue_in_the_project_the_local_task_points_at() {
        let _repo = gittask::testing::TestRepo::new();
        let (redmine, requests) = stub_redmine(redmine_issue_in_project(2));
        configure_stub(&redmine);
        local_task_in_project("2");
        let task = gittask::find_task(&"7".to_string()).unwrap().unwrap();

        let result = RedmineRemoteConnector.update_remote_task(&String::new(), &"1".to_string(), &task, None, RemoteTaskState::All);

        assert_eq!(result, Ok(()));
        assert_eq!(requests.lock().unwrap().as_slice(), ["GET /issues/7.json", "PUT /issues/7.json"]);
    }

    #[test]
    fn a_second_move_finds_the_issue_in_the_project_of_the_first_move() {
        let _repo = gittask::testing::TestRepo::new();
        let (redmine, requests) = stub_redmine(redmine_issue_in_project(2));
        configure_stub(&redmine);
        local_task_in_project("3");

        let found = RedmineRemoteConnector.get_remote_task(&String::new(), &"1".to_string(), &"7".to_string(), false, false, &vec![]);
        let found = found.unwrap().expect("a task bound to a Redmine issue must find it wherever it currently is");
        assert_eq!(found.get_property("project_id"), Some(&"2".to_string()));

        let task = gittask::find_task(&"7".to_string()).unwrap().unwrap();
        let result = RedmineRemoteConnector.update_remote_task(&String::new(), &"1".to_string(), &task, None, RemoteTaskState::All);

        assert_eq!(result, Ok(()));
        assert_eq!(requests.lock().unwrap().as_slice(), ["GET /issues/7.json", "GET /issues/7.json", "PUT /issues/7.json"]);
    }

    #[test]
    fn an_explicit_project_selects_the_redmine_remote_even_when_it_is_not_configured() {
        let _repo = gittask::testing::TestRepo::new();
        gittask::set_config_value("task.redmine.url", "http://127.0.0.1:1/").unwrap();
        gittask::set_config_value("task.redmine.api.key", "key").unwrap();
        gittask::set_config_value("task.redmine.project.id", "1").unwrap();
        let remotes = vec!["https://example.invalid/git-task/ci.git".to_string()];

        let targets = crate::connectors::get_matching_remote_connectors(remotes.clone(), &Some("redmine".to_string()), &Some("2".to_string()), false);
        let projects: Vec<&String> = targets.iter().map(|(_, _, project)| project).collect();
        assert_eq!(projects, vec!["2"], "--project must select the requested Redmine project");

        let targets = crate::connectors::get_matching_remote_connectors(remotes, &Some("redmine".to_string()), &None, false);
        let projects: Vec<&String> = targets.iter().map(|(_, _, project)| project).collect();
        assert_eq!(projects, vec!["1"], "without --project the configured project stays the default");
    }

    #[test]
    fn an_explicit_project_needs_only_the_redmine_url() {
        let _repo = gittask::testing::TestRepo::new();
        gittask::set_config_value("task.redmine.url", "http://127.0.0.1:1/").unwrap();
        let remotes = vec!["https://example.invalid/git-task/ci.git".to_string()];

        let targets = crate::connectors::get_matching_remote_connectors(remotes.clone(), &Some("redmine".to_string()), &Some("ops".to_string()), false);
        assert_eq!(targets.len(), 1, "a project given explicitly does not need task.redmine.project.id");
        assert_eq!(targets[0].2, "ops");

        let targets = crate::connectors::get_matching_remote_connectors(remotes, &Some("redmine".to_string()), &None, false);
        assert!(targets.is_empty(), "without a default project there is no default target");
    }

    fn issue_with_journals(journals: &str) -> Issue {
        let json = format!(r#"{{
            "id": 7,
            "project": {{ "id": 1, "name": "CI" }},
            "tracker": {{ "id": 1, "name": "Bug" }},
            "status": {{ "id": 1, "name": "New", "is_closed": false }},
            "priority": {{ "id": 2, "name": "Normal" }},
            "author": {{ "id": 1, "name": "Admin" }},
            "subject": "Journals",
            "description": "",
            "start_date": null,
            "due_date": null,
            "closed_on": null,
            "done_ratio": 0,
            "estimated_hours": null,
            "created_on": "2026-10-16T10:00:00Z",
            "updated_on": "2026-10-16T10:00:00Z",
            "journals": {journals}
        }}"#);
        serde_json::from_str(&json).expect("issue JSON")
    }

    fn paged_source(available: u64) -> impl FnMut(u64, u64) -> Result<(Vec<u64>, u64), String> {
        move |offset, page_size| {
            let end = (offset + page_size).min(available);
            Ok(((offset..end).collect(), available))
        }
    }

    #[test]
    fn collect_pages_requests_further_pages_until_the_limit_is_reached() {
        let mut requests = vec![];
        let mut source = paged_source(500);
        let issues = collect_pages(150, |offset, page_size| {
            requests.push((offset, page_size));
            source(offset, page_size)
        }).unwrap();

        assert_eq!(issues.len(), 150, "a limit above the page maximum must span several pages");
        assert_eq!(issues.first(), Some(&0));
        assert_eq!(issues.last(), Some(&149));
        assert_eq!(requests, vec![(0, 100), (100, 50)], "the second page must ask only for what is still missing");
    }

    #[test]
    fn collect_pages_stops_when_redmine_runs_out_of_issues() {
        let mut requests = vec![];
        let mut source = paged_source(120);
        let issues = collect_pages(150, |offset, page_size| {
            requests.push((offset, page_size));
            source(offset, page_size)
        }).unwrap();

        assert_eq!(issues.len(), 120);
        assert_eq!(requests, vec![(0, 100), (100, 50)], "no request must be made once the total count is reached");
    }

    #[test]
    fn collect_pages_makes_a_single_request_for_a_small_limit() {
        let mut requests = vec![];
        let mut source = paged_source(500);
        let issues = collect_pages(40, |offset, page_size| {
            requests.push((offset, page_size));
            source(offset, page_size)
        }).unwrap();

        assert_eq!(issues.len(), 40);
        assert_eq!(requests, vec![(0, 40)]);
    }

    #[test]
    fn collect_pages_propagates_a_failed_page() {
        let result = collect_pages::<u64>(150, |offset, _| if offset == 0 { Ok(((0..100).collect(), 500)) } else { Err("boom".to_string()) });
        assert_eq!(result, Err("boom".to_string()));
    }

    #[test]
    fn an_issue_without_notes_yields_an_empty_comment_list_not_none() {
        let issue = issue_with_journals("[]");
        let mut task = issue_to_task(&issue).unwrap();
        assert!(task.get_comments().is_none(), "a freshly mapped issue has no comment list yet");

        append_journals_as_comments(&issue, &mut task);

        assert!(matches!(task.get_comments(), Some(comments) if comments.is_empty()), "a successful fetch with no notes must say so explicitly, so a locally kept comment is removed on pull");
    }

    #[test]
    fn journals_without_notes_do_not_become_comments() {
        let issue = issue_with_journals(r#"[
            { "id": 11, "user": { "id": 1, "name": "Admin" }, "notes": "", "private_notes": false,
              "created_on": "2026-10-16T10:01:00Z", "updated_on": "2026-10-16T10:01:00Z",
              "details": [ { "property": "attr", "name": "status_id", "old_value": "1", "new_value": "2" } ] },
            { "id": 12, "user": { "id": 1, "name": "Admin" }, "notes": null, "private_notes": false,
              "created_on": "2026-10-16T10:02:00Z", "updated_on": "2026-10-16T10:02:00Z", "details": [] },
            { "id": 13, "user": { "id": 1, "name": "Admin" }, "notes": "A real comment", "private_notes": false,
              "created_on": "2026-10-16T10:03:00Z", "updated_on": "2026-10-16T10:03:00Z", "details": [] }
        ]"#);
        let mut task = issue_to_task(&issue).unwrap();

        append_journals_as_comments(&issue, &mut task);

        let comments = task.get_comments().clone().unwrap_or_default();
        let texts = comments.iter().map(|c| c.get_text().clone()).collect::<Vec<_>>();
        assert_eq!(comments.len(), 1, "only the journal with notes must become a comment, got {texts:?}");
        assert_eq!(comments[0].get_id(), Some("13".to_string()));
        assert_eq!(comments[0].get_text(), "A real comment");
    }
}
