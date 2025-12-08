use std::borrow::Cow;
use std::collections::HashMap;

use redmine_api::api::issues::{CreateIssue, GetIssue, Issue, IssueWrapper, ListIssues, UpdateIssue, DeleteIssue, IssueInclude};
use redmine_api::api::issue_statuses::{ListIssueStatuses, IssueStatusesWrapper, IssueStatus};
use redmine_api::api::projects::{Project, ListProjects};
use redmine_api::api::{Endpoint, Redmine};
use redmine_api::reqwest::Method;
use serde::Serialize;

use gittask::{Task, Comment, Label};

use crate::connectors::{RemoteConnector, RemoteTaskState};

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
        Some((String::new(), String::new()))
    }

    fn list_remote_tasks(
        &self,
        domain: &String,
        _project: &String,
        with_comments: bool,
        _with_labels: bool,
        limit: Option<usize>,
        _state: RemoteTaskState,
        task_statuses: &Vec<String>
    ) -> Result<Vec<Task>, String> {
        let redmine = get_redmine_instance(domain)?;
        let endpoint = ListIssues::builder().build().map_err(|e| e.to_string())?;
        let issues = match limit {
            Some(limit) => {
                let response = redmine
                    .json_response_body_page::<_, Issue>(&endpoint, 0, limit as u64)
                    .map_err(|e| e.to_string())?;
                response.values
            },
            None => redmine
                .json_response_body_all_pages::<_, Issue>(&endpoint)
                .map_err(|e| e.to_string())?
        };

        let mut tasks = Vec::new();
        for issue in issues {
            let mut task = issue_to_task(&issue, task_statuses)?;

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
        _project: &String,
        task_id: &String,
        with_comments: bool,
        _with_labels: bool,
        task_statuses: &Vec<String>
    ) -> Result<Task, String> {
        let redmine = get_redmine_instance(domain)?;
        let mut endpoint_builder = GetIssue::builder();
        endpoint_builder.id(task_id.parse().unwrap());
        if with_comments {
            endpoint_builder.include(vec![IssueInclude::Journals]);
        }
        let endpoint = endpoint_builder.build().map_err(|e| e.to_string())?;
        let IssueWrapper { issue } =
            redmine.json_response_body::<_, IssueWrapper<Issue>>(&endpoint)
                .map_err(|e| e.to_string())?;

        let mut task = issue_to_task(&issue, task_statuses)?;

        if with_comments {
            append_journals_as_comments(&issue, &mut task);
        }

        Ok(task)
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
        let issue: Issue = redmine
            .json_response_body::<_, Issue>(&endpoint)
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

        Ok(String::new())
    }

    #[allow(unused)]
    fn create_remote_label(&self, domain: &String, project: &String, task_id: &String, label: &Label) -> Result<(), String> {
        Err("Labels are not supported for Redmine".to_string())
    }

    fn update_remote_task(
        &self,
        domain: &String,
        _project: &String,
        task: &Task,
        _labels: Option<&Vec<Label>>,
        state: RemoteTaskState
    ) -> Result<(), String> {
        let redmine = get_redmine_instance(domain)?;

        let id = task.get_id()
            .ok_or_else(|| "Task id is required for update".to_string())?
            .parse::<u64>()
            .map_err(|e| format!("Invalid task id '{}': {}", task.get_id().unwrap(), e))?;

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
            let endpoint = ListIssueStatuses::builder().build().map_err(|e| e.to_string())?;
            let IssueStatusesWrapper { issue_statuses } =
                redmine
                    .json_response_body::<_, IssueStatusesWrapper<IssueStatus>>(&endpoint)
                    .map_err(|e| e.to_string())?;

            let local_lower = local_status.to_lowercase();

            let mut target = issue_statuses
                .iter()
                .find(|s| s.name.to_lowercase() == local_lower)
                .cloned();

            if target.is_none() {
                let want_closed = local_lower == "closed" || local_lower == "resolved";
                target = issue_statuses
                    .iter()
                    .find(|s| s.is_closed == want_closed)
                    .cloned();
            }

            if let Some(status) = target {
                builder.status_id(status.id);
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
}

fn resolve_project_id(redmine: &Redmine, project: &String) -> Result<u64, String> {
    if let Ok(id) = project.parse::<u64>() {
        return Ok(id);
    }

    let endpoint = ListProjects::builder().build().map_err(|e| e.to_string())?;
    let projects = redmine
        .json_response_body_all_pages::<_, Project>(&endpoint)
        .map_err(|e| e.to_string())?;

    let lower = project.to_lowercase();
    for p in projects {
        if p.identifier.to_lowercase() == lower { return Ok(p.id); }
        if p.name.to_lowercase() == lower { return Ok(p.id); }
    }

    Err(format!("Project '{}' not found on Redmine server", project))
}

fn get_redmine_instance(domain: &String) -> Result<Redmine, String> {
    let client = redmine_api::reqwest::blocking::Client::builder().use_rustls_tls().build()
        .map_err(|e| e.to_string())?;
    let url = get_base_url(domain)?;
    let api_key = get_api_key()?;
    Redmine::new(client, url.parse().unwrap(), &*api_key).map_err(|e| e.to_string())
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
            .map_err(|_| "No Redmine API key found. Set task.redmine.api.key config or REDMINE_API_KEY environment variable.".to_string())
    }
}

fn issue_to_task(issue: &Issue, task_statuses: &Vec<String>) -> Result<Task, String> {
    let mut props = HashMap::new();
    props.insert("name".to_string(), issue.subject.clone().unwrap_or_else(|| String::new()));
    
    let status = if issue.status.name.to_lowercase().contains("closed") 
        || issue.status.name.to_lowercase().contains("resolved") {
        task_statuses.last().unwrap_or(&"CLOSED".to_string()).clone()
    } else {
        task_statuses.first().unwrap_or(&"OPEN".to_string()).clone()
    };
    props.insert("status".to_string(), status);
    
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
    if let Some(journals) = &issue.journals {
        for j in journals {
            if let Some(text) = &j.notes {
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