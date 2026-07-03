use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::DateTime;
use jira_v3_openapi::{apis::configuration::Configuration, apis::issues_api};
use jira_v3_openapi::apis::{issue_comments_api, issue_search_api};
use jira_v3_openapi::models::IssueTransition;
use regex::Regex;
use tokio::runtime::Runtime;

use gittask::{Task, Comment, Label, Subtask};

use crate::connectors::{RemoteConnector, RemoteTaskState};
use crate::util::error_message;

pub struct JiraRemoteConnector;

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    Runtime::new().unwrap()
});

impl RemoteConnector for JiraRemoteConnector {
    fn type_name(&self) -> &str {
        "jira"
    }

    fn get_config_options(&self) -> Option<Vec<String>> {
        Some(vec![
            "task.jira.url".to_string(),
            "task.jira.user".to_string(),
            "task.jira.project".to_string(),
        ])
    }

    fn supports_remote(&self, _url: &str) -> Option<(String, String)> {
        let url = get_base_url()?;
        let origin = Regex::new(r"^(https?://[^/]+)").unwrap().captures(&url)?.get(1)?.as_str().to_string();
        let project = match gittask::get_config_value("task.jira.project") {
            Ok(project) if !project.is_empty() => project,
            _ => Regex::new(r"/(?:projects|browse)/([A-Za-z][A-Za-z0-9_]*)").unwrap().captures(&url)?.get(1)?.as_str().to_string(),
        };
        Some((origin, project))
    }

    fn list_remote_tasks(
        &self,
        domain: &String,
        project: &String,
        with_comments: bool,
        with_labels: bool,
        limit: Option<usize>,
        state: RemoteTaskState,
        task_statuses: &Vec<String>
    ) -> Result<Vec<Task>, String> {
        let config = get_configuration(domain)?;

        let done_status = "Done".to_string();
        let last_status = task_statuses.last().unwrap_or_else(|| &done_status);
        let jql = match state {
            RemoteTaskState::Open(_, _) => format!("project = {} AND status != '{}'", project, last_status),
            RemoteTaskState::Closed(_, _) => format!("project = {} AND status = '{}'", project, last_status),
            RemoteTaskState::All => format!("project = {}", project),
        };
        
        let mut field_list = vec!["summary".to_string(), "description".to_string(), "status".to_string(), "created".to_string(), "creator".to_string()];
        if with_comments {
            field_list.push("comment".to_string());
        }
        if with_labels {
            field_list.push("labels".to_string());
        }

        RUNTIME.block_on(async {
            let mut tasks: Vec<Task> = vec![];
            let mut start_at: i32 = 0;
            loop {
                let max_results = match limit {
                    Some(limit) => std::cmp::min(limit.saturating_sub(tasks.len()), 100) as i32,
                    None => 100,
                };
                let response = issue_search_api::search_for_issues_using_jql(
                    &config,
                    Some(&jql),
                    Some(start_at),
                    Some(max_results),
                    None,
                    Some(field_list.clone()),
                    None,
                    None,
                    None,
                    None,
                ).await.map_err(|e| e.to_string())?;

                let issues = response.issues.unwrap_or_default();
                let batch_len = issues.len();
                for issue in issues {
                    let key = match issue.key {
                        Some(key) => key,
                        None => {
                            eprintln!("WARNING: skipping Jira issue without a key");
                            continue;
                        }
                    };
                    let fields = match issue.fields {
                        Some(fields) => fields,
                        None => continue,
                    };
                    let summary = match fields.get("summary").and_then(|v| v.as_str()) {
                        Some(summary) => summary,
                        None => {
                            eprintln!("WARNING: skipping Jira issue {key} without a summary");
                            continue;
                        }
                    };

                    let mut props = HashMap::new();
                    props.insert("name".to_string(), summary.to_string());
                    props.insert("description".to_string(), fields.get("description").map(parse_adf).unwrap_or_default());
                    props.insert("status".to_string(), fields.get("status").map(parse_status).unwrap_or_default());
                    if let Some(created) = fields.get("created").and_then(|v| v.as_str()).and_then(|s| parse_to_unix_timestamp(s).ok()) {
                        props.insert("created".to_string(), created);
                    }
                    if let Some(creator) = fields.get("creator") {
                        props.insert("author".to_string(), parse_creator(creator));
                    }

                    let mut task = match Task::from_properties(issue_key_to_task_id(&key), props) {
                        Ok(task) => task,
                        Err(e) => {
                            eprintln!("WARNING: skipping Jira issue {key}: {e}");
                            continue;
                        }
                    };

                    if with_comments {
                        if let Some(comments) = fields.get("comment") {
                            let mut task_comments = parse_comments(comments);
                            if comments_are_truncated(comments) {
                                match fetch_all_comments(&config, &key).await {
                                    Ok(all_comments) => task_comments = all_comments,
                                    Err(e) => eprintln!("WARNING: could not fetch all comments for {key}: {e}"),
                                }
                            }
                            task.set_comments(task_comments);
                        }
                    }

                    if with_labels {
                        if let Some(serde_json::Value::Array(labels)) = fields.get("labels") {
                            let task_labels = labels.iter().filter_map(|v| {
                                v.as_str().map(|name| Label::new(name.to_string(), None, None))
                            }).collect();
                            task.set_labels(task_labels);
                        }
                    }

                    tasks.push(task);
                }

                start_at += batch_len as i32;
                let total = response.total.unwrap_or(0);
                if batch_len == 0 || start_at >= total || limit.map_or(false, |l| tasks.len() >= l) {
                    break;
                }
            }

            Ok(tasks)
        })
    }

    fn get_remote_task(
        &self,
        domain: &String,
        project: &String,
        task_id: &String,
        with_comments: bool,
        with_labels: bool,
        _task_statuses: &Vec<String>
    ) -> Result<Task, String> {
        let config = get_configuration(domain)?;

        let mut field_list = vec!["summary".to_string(), "description".to_string(), "status".to_string(), "created".to_string(), "creator".to_string()];
        if with_comments {
            field_list.push("comment".to_string());
        }
        if with_labels {
            field_list.push("labels".to_string());
        }

        RUNTIME.block_on(async {
            match issues_api::get_issue(
                &config,
                task_id_to_issue_key(project, task_id).as_str(),
                Some(field_list),
                None,
                None,
                None,
                None,
                None,
            ).await {
                Ok(issue) => {
                    let mut props = HashMap::new();
                    if let Some(fields) = issue.fields {
                        let issue_key = issue.key.unwrap_or_else(|| task_id_to_issue_key(project, task_id));
                        let summary = fields.get("summary").and_then(|v| v.as_str())
                            .ok_or_else(|| format!("Issue {issue_key} has no summary"))?;
                        props.insert("name".to_string(), summary.to_string());
                        props.insert("description".to_string(), fields.get("description").map(parse_adf).unwrap_or_default());
                        props.insert("status".to_string(), fields.get("status").map(parse_status).unwrap_or_default());
                        if let Some(created) = fields.get("created").and_then(|v| v.as_str()).and_then(|s| parse_to_unix_timestamp(s).ok()) {
                            props.insert("created".to_string(), created);
                        }
                        if let Some(creator) = fields.get("creator") {
                            props.insert("author".to_string(), parse_creator(creator));
                        }

                        let mut task = Task::from_properties(issue_key_to_task_id(&issue_key), props)?;

                        if with_comments {
                            if let Some(comments) = fields.get("comment") {
                                let mut task_comments = parse_comments(comments);
                                if comments_are_truncated(comments) {
                                    match fetch_all_comments(&config, &issue_key).await {
                                        Ok(all_comments) => task_comments = all_comments,
                                        Err(e) => eprintln!("WARNING: could not fetch all comments for {issue_key}: {e}"),
                                    }
                                }
                                task.set_comments(task_comments);
                            }
                        }

                        if with_labels {
                            if let Some(serde_json::Value::Array(labels)) = fields.get("labels") {
                                let task_labels = labels.iter().filter_map(|v| {
                                    v.as_str().map(|name| Label::new(name.to_string(), None, None))
                                }).collect();
                                task.set_labels(task_labels);
                            }
                        }

                        Ok(task)
                    } else {
                        Err("Failed to get issue: no fields returned.".to_string())
                    }
                },
                Err(e) => Err(e.to_string()),
            }
        })
    }

    fn create_remote_task(
        &self,
        domain: &String,
        project: &String,
        task: &Task
    ) -> Result<String, String> {
        let config = get_configuration(domain)?;

        RUNTIME.block_on(async {
            let mut issue_details = jira_v3_openapi::models::IssueUpdateDetails {
                fields: Some(std::collections::HashMap::from([
                    ("project".to_string(), serde_json::json!({
                        "key": project
                    })),
                    ("summary".to_string(), serde_json::json!(
                        task.get_property("name").unwrap()
                    )),
                    ("description".to_string(),
                        format_adf(task.get_property("description").unwrap())
                    ),
                    ("issuetype".to_string(), serde_json::json!({
                        "name": "Task"
                    })),
                ])),
                ..Default::default()
            };
            
            if let Some(labels) = task.get_labels() {
                issue_details.fields.as_mut().unwrap().insert("labels".to_string(), serde_json::json!(
                    labels.iter().map(|l| l.get_name()).collect::<Vec<String>>()
                ));
            }

            match issues_api::create_issue(&config, issue_details, None).await {
                Ok(response) => {
                    match response.key {
                        Some(key) => {
                            let task_id = issue_key_to_task_id(&key);
                            Ok(task_id)
                        },
                        None => Err("Failed to create issue: no key returned.".to_string())
                    }
                },
                Err(e) => Err(format!("Failed to create issue: {}", e))
            }
        })
    }

    fn create_remote_comment(
        &self,
        domain: &String,
        project: &String,
        task_id: &String,
        comment: &Comment
    ) -> Result<String, String> {
        let config = get_configuration(domain)?;

        RUNTIME.block_on(async {
            let comment_body = jira_v3_openapi::models::Comment {
                body: Some(Some(format_adf(&comment.get_text()))),
                ..Default::default()
            };

            match issue_comments_api::add_comment(
                &config,
                task_id_to_issue_key(project, task_id).as_str(),
                comment_body,
                None,
            ).await {
                Ok(response) => {
                    if let Some(id) = response.id {
                        Ok(id)
                    } else {
                        Err("Failed to create comment: no ID returned.".to_string())
                    }
                },
                Err(e) => Err(format!("Failed to create comment: {}", e))
            }
        })
    }

    fn create_remote_label(
        &self,
        domain: &String,
        project: &String,
        task_id: &String,
        label: &Label,
    ) -> Result<(), String> {
        let config = get_configuration(domain)?;

        RUNTIME.block_on(async {
            let issue_result = issues_api::get_issue(
                &config,
                task_id_to_issue_key(project, task_id).as_str(),
                Some(vec!["labels".to_string()]),
                None,
                None,
                None,
                None,
                None,
            ).await;

            match issue_result {
                Ok(issue) => {
                    if let Some(fields) = issue.fields {
                        let mut current_labels = match fields.get("labels") {
                            Some(serde_json::Value::Array(labels)) => labels.iter().map(|v| v.as_str().unwrap().to_string()).collect(),
                            _ => vec![],
                        };
                        let label_name = label.get_name();

                        if !current_labels.contains(&label_name) {
                            current_labels.push(label_name);

                            let update_request = jira_v3_openapi::models::IssueUpdateDetails {
                                fields: Some(std::collections::HashMap::from([
                                    ("labels".to_string(), serde_json::json!(current_labels))
                                ])),
                                ..Default::default()
                            };

                            match issues_api::edit_issue(
                                &config,
                                task_id_to_issue_key(project, task_id).as_str(),
                                update_request,
                                None,
                                None,
                                None,
                                None,
                                None,
                            ).await {
                                Ok(_) => Ok(()),
                                Err(e) if e.to_string().starts_with("error in serde: EOF while parsing a value at line 1 column 0") => Ok(()),
                                Err(e) => Err(format!("Failed to update labels: {}", e))
                            }
                        } else {
                            Ok(())
                        }
                    } else {
                        Err("Failed to get issue: no fields returned.".to_string())
                    }
                },
                Err(e) => Err(format!("Failed to get issue: {}", e))
            }
        })
    }

    fn update_remote_task(
        &self,
        domain: &String,
        project: &String,
        task: &Task,
        labels: Option<&Vec<Label>>,
        state: RemoteTaskState
    ) -> Result<(), String> {
        let config = get_configuration(domain)?;

        RUNTIME.block_on(async {
            let mut fields = HashMap::new();
            fields.insert("summary".to_string(), serde_json::json!(task.get_property("name").unwrap()));
            fields.insert("description".to_string(), format_adf(task.get_property("description").unwrap()));
            
            if let Some(labels) = labels {
                fields.insert("labels".to_string(), serde_json::json!(
                    labels.iter().map(|l| l.get_name()).collect::<Vec<String>>()
                ));
            }

            let issue_details = jira_v3_openapi::models::IssueUpdateDetails {
                fields: Some(fields),
                ..Default::default()
            };

            let result = match issues_api::edit_issue(
                &config,
                task_id_to_issue_key(project, &task.get_id().unwrap()).as_str(),
                issue_details,
                None,
                None,
                None,
                None,
                None,
            ).await {
                Ok(_) => Ok(()),
                Err(e) if e.to_string().starts_with("error in serde: EOF while parsing a value at line 1 column 0") => Ok(()),
                Err(e) => Err(format!("Failed to update issue: {}", e))
            };
            
            if result.is_ok() {
                let (local_status, remote_status) = match state {
                    RemoteTaskState::Open(s1, s2) => (s1, s2),
                    RemoteTaskState::Closed(s1, s2) => (s1, s2),
                    _ => ("".to_string(), "".to_string())
                };
                if local_status != remote_status {
                    if let Ok(transitions) = issues_api::get_transitions(
                        &config,
                        task_id_to_issue_key(project, &task.get_id().unwrap()).as_str(),
                        None,
                        None,
                        None,
                        None,
                        None,
                    ).await {
                        for transition in transitions.transitions.unwrap_or_else(|| vec![]) {
                            if let Some(target_status) = transition.to {
                                if target_status.name.as_deref() == Some(local_status.as_str()) {
                                    let issue_details = jira_v3_openapi::models::IssueUpdateDetails {
                                        transition: Some(IssueTransition {
                                            id: transition.id.clone(),
                                            ..Default::default()
                                        }),
                                        ..Default::default()
                                    };

                                    if let Err(e) = issues_api::do_transition(
                                        &config,
                                        task_id_to_issue_key(project, &task.get_id().unwrap()).as_str(),
                                        issue_details,
                                    ).await {
                                        eprintln!("ERROR: failed to transition issue status: {e}");
                                    }

                                    break;
                                }
                            }
                        }
                    } else {
                        error_message("Failed to get transitions.".to_string());
                    }
                }
            }
            
            result
        })
    }

    fn update_remote_comment(
        &self,
        domain: &String,
        project: &String,
        task_id: &String,
        comment_id: &String,
        text: &String
    ) -> Result<(), String> {
        let config = get_configuration(domain)?;

        RUNTIME.block_on(async {
            let comment = jira_v3_openapi::models::Comment {
                body: Some(Some(format_adf(text))),
                ..Default::default()
            };

            match issue_comments_api::update_comment(
                &config,
                task_id_to_issue_key(project, task_id).as_str(),
                comment_id,
                comment,
                None,
                None,
                None,
            ).await {
                Ok(_) => Ok(()),
                Err(e) => Err(format!("Failed to update comment: {}", e))
            }
        })
    }

    fn delete_remote_task(
        &self,
        domain: &String,
        project: &String,
        task_id: &String
    ) -> Result<(), String> {
        let config = get_configuration(domain)?;

        RUNTIME.block_on(async {
            match issues_api::delete_issue(
                &config,
                task_id_to_issue_key(project, task_id).as_str(),
                Some("true"),
            ).await {
                Ok(_) => Ok(()),
                Err(e) => Err(format!("Failed to delete issue: {}", e))
            }
        })
    }

    fn delete_remote_comment(
        &self,
        domain: &String,
        project: &String,
        task_id: &String,
        comment_id: &String
    ) -> Result<(), String> {
        let config = get_configuration(domain)?;

        RUNTIME.block_on(async {
            match issue_comments_api::delete_comment(
                &config,
                task_id_to_issue_key(project, task_id).as_str(),
                comment_id,
            ).await {
                Ok(_) => Ok(()),
                Err(e) => Err(format!("Failed to delete comment: {}", e))
            }
        })
    }

    fn delete_remote_label(
        &self,
        domain: &String,
        project: &String,
        task_id: &String,
        name: &String,
    ) -> Result<(), String> {
        let config = get_configuration(domain)?;

        RUNTIME.block_on(async {
            let issue_result = issues_api::get_issue(
                &config,
                task_id_to_issue_key(project, task_id).as_str(),
                Some(vec!["labels".to_string()]),
                None,
                None,
                None,
                None,
                None,
            ).await;

            match issue_result {
                Ok(issue) => {
                    if let Some(fields) = issue.fields {
                        let current_labels = match fields.get("labels") {
                            Some(serde_json::Value::Array(labels)) => labels.iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<String>>(),
                            _ => vec![],
                        };

                        let updated_labels: Vec<String> = current_labels
                            .into_iter()
                            .filter(|l| l != name)
                            .collect();

                        let update_request = jira_v3_openapi::models::IssueUpdateDetails {
                            fields: Some(std::collections::HashMap::from([
                                ("labels".to_string(), serde_json::json!(updated_labels))
                            ])),
                            ..Default::default()
                        };

                        match issues_api::edit_issue(
                            &config,
                            task_id_to_issue_key(project, task_id).as_str(),
                            update_request,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ).await {
                            Ok(_) => Ok(()),
                            Err(e) if e.to_string().starts_with("error in serde: EOF while parsing a value at line 1 column 0") => Ok(()),
                            Err(e) => Err(format!("Failed to update labels: {}", e))
                        }
                    } else {
                        Err("Failed to get issue: no fields returned.".to_string())
                    }
                },
                Err(e) => Err(format!("Failed to get issue: {}", e))
            }
        })
    }

    fn supports_subtasks(&self) -> bool {
        true
    }

    fn list_remote_subtasks(&self, domain: &String, project: &String, task_id: &String) -> Result<Vec<Subtask>, String> {
        let config = get_configuration(domain)?;

        RUNTIME.block_on(async {
            match issues_api::get_issue(
                &config,
                task_id_to_issue_key(project, task_id).as_str(),
                Some(vec!["subtasks".to_string()]),
                None,
                None,
                None,
                None,
                None,
            ).await {
                Ok(issue) => {
                    let mut result = vec![];
                    if let Some(fields) = issue.fields
                        && let Some(serde_json::Value::Array(subtasks)) = fields.get("subtasks") {
                            for sub in subtasks {
                                if let serde_json::Value::Object(sub_obj) = sub {
                                    let key = sub_obj.get("key").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                                    let (name, status) = match sub_obj.get("fields") {
                                        Some(serde_json::Value::Object(sub_fields)) => {
                                            let name = sub_fields.get("summary").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                                            let status = sub_fields.get("status").map(parse_status).unwrap_or_default();
                                            (name, status)
                                        }
                                        _ => (String::new(), String::new()),
                                    };
                                    result.push(Subtask::new(Some(issue_key_to_task_id(&key)), name, status, HashMap::new()));
                                }
                            }
                        }
                    Ok(result)
                },
                Err(e) => Err(format!("Failed to list subtasks: {}", e))
            }
        })
    }

    fn create_remote_subtask(&self, domain: &String, project: &String, task_id: &String, subtask: &Subtask) -> Result<String, String> {
        let config = get_configuration(domain)?;

        RUNTIME.block_on(async {
            let issue_details = jira_v3_openapi::models::IssueUpdateDetails {
                fields: Some(HashMap::from([
                    ("project".to_string(), serde_json::json!({ "key": project })),
                    ("parent".to_string(), serde_json::json!({ "key": task_id_to_issue_key(project, task_id) })),
                    ("summary".to_string(), serde_json::json!(subtask.get_name())),
                    ("issuetype".to_string(), serde_json::json!({ "name": "Sub-task" })),
                ])),
                ..Default::default()
            };

            match issues_api::create_issue(&config, issue_details, None).await {
                Ok(response) => {
                    match response.key {
                        Some(key) => Ok(issue_key_to_task_id(&key)),
                        None => Err("Failed to create subtask: no key returned.".to_string())
                    }
                },
                Err(e) => Err(format!("Failed to create subtask: {}", e))
            }
        })
    }

    fn update_remote_subtask(&self, domain: &String, project: &String, _task_id: &String, subtask: &Subtask) -> Result<(), String> {
        let config = get_configuration(domain)?;
        let subtask_id = subtask.get_id().ok_or_else(|| "Subtask has no ID".to_string())?;
        let subtask_key = task_id_to_issue_key(project, &subtask_id);

        RUNTIME.block_on(async {
            let fields = HashMap::from([
                ("summary".to_string(), serde_json::json!(subtask.get_name())),
            ]);

            let issue_details = jira_v3_openapi::models::IssueUpdateDetails {
                fields: Some(fields),
                ..Default::default()
            };

            let result = match issues_api::edit_issue(
                &config,
                subtask_key.as_str(),
                issue_details,
                None,
                None,
                None,
                None,
                None,
            ).await {
                Ok(_) => Ok(()),
                Err(e) if e.to_string().starts_with("error in serde: EOF while parsing a value at line 1 column 0") => Ok(()),
                Err(e) => Err(format!("Failed to update subtask: {}", e))
            };

            if result.is_ok() && !subtask.get_status().is_empty() {
                if let Ok(transitions) = issues_api::get_transitions(
                    &config,
                    subtask_key.as_str(),
                    None,
                    None,
                    None,
                    None,
                    None,
                ).await {
                    for transition in transitions.transitions.unwrap_or_default() {
                        if let Some(target_status) = transition.to
                            && target_status.name.as_deref() == Some(subtask.get_status()) {
                                let issue_details = jira_v3_openapi::models::IssueUpdateDetails {
                                    transition: Some(IssueTransition {
                                        id: transition.id.clone(),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                };

                                if let Err(e) = issues_api::do_transition(
                                    &config,
                                    subtask_key.as_str(),
                                    issue_details,
                                ).await {
                                    eprintln!("ERROR: failed to transition subtask status: {e}");
                                }

                                break;
                        }
                    }
                } else {
                    error_message("Failed to get transitions for subtask.".to_string());
                }
            }

            result
        })
    }

    fn delete_remote_subtask(&self, domain: &String, project: &String, _task_id: &String, subtask_id: &String) -> Result<(), String> {
        let config = get_configuration(domain)?;

        RUNTIME.block_on(async {
            match issues_api::delete_issue(
                &config,
                task_id_to_issue_key(project, subtask_id).as_str(),
                Some("true"),
            ).await {
                Ok(_) => Ok(()),
                Err(e) => Err(format!("Failed to delete subtask: {}", e))
            }
        })
    }
}

fn get_token_from_env() -> Result<String, String> {
    std::env::var("JIRA_TOKEN")
        .or_else(|_| std::env::var("JIRA_API_TOKEN"))
        .map_err(|_| "No JIRA_TOKEN nor JIRA_API_TOKEN env variable set".to_string())
}

fn get_jira_user() -> Result<String, String> {
    match gittask::get_config_value("task.jira.user") {
        Ok(email) => Ok(email),
        _ => match std::env::var("JIRA_USER") {
            Ok(user) => Ok(user),
            Err(_) => Err("Unknown JIRA user: set up task.jira.user config option or JIRA_USER env variable".to_string())
        }
    }
}

fn get_base_url() -> Option<String> {
    let mut result = match gittask::get_config_value("task.jira.url") {
        Ok(url) => url,
        _ => match std::env::var("JIRA_URL").or_else(|_| std::env::var("JIRA_BASE_URL")) {
            Ok(url) => url,
            _ => return None
        }
    };

    if !result.starts_with("http") {
        result = "https://".to_string() + result.as_str();
    }

    Some(result)
}

fn get_configuration(base_url: &String) -> Result<Configuration, String> {
    let email = get_jira_user()?;
    let token = get_token_from_env()?;

    let mut config = Configuration::new();
    config.basic_auth = Some((email, Some(token)));
    config.base_path = base_url.trim_end_matches('/').to_string();
    Ok(config)
}

fn issue_key_to_task_id(key: &String) -> String {
    key.split('-').last().unwrap_or_default().to_string()
}

fn task_id_to_issue_key(project: &String, id: &String) -> String {
    format!("{}-{}", project, id)
}

fn parse_adf(description: &serde_json::Value) -> String {
    if let serde_json::Value::Object(doc) = description {
        if let Some(serde_json::Value::String(doc_type)) = doc.get("type") {
            if doc_type == "doc" {
                if let Some(serde_json::Value::Array(content)) = doc.get("content") {
                    return content.iter().map(|v| {
                        if let serde_json::Value::Object(node) = v {
                            if let Some(serde_json::Value::String(node_type)) = node.get("type") {
                                if node_type == "paragraph" {
                                    if let Some(serde_json::Value::Array(paragraph_content)) = node.get("content") {
                                        return paragraph_content.iter().map(|v| {
                                            if let serde_json::Value::Object(paragraph_node) = v {
                                                if let Some(serde_json::Value::String(paragraph_node_type)) = paragraph_node.get("type") {
                                                    if paragraph_node_type == "text" {
                                                        if let Some(serde_json::Value::String(text)) = paragraph_node.get("text") {
                                                            return text.to_string();
                                                        }
                                                    }
                                                }
                                            }
                                            "".to_string()
                                        }).collect::<Vec<String>>().join(" ");
                                    }
                                }
                            }
                        }
                        "".to_string()
                    }).collect::<Vec<String>>().join("\n");
                }
            }
        }
    }

    "".to_string()
}

fn format_adf(text: &String) -> serde_json::Value {
    serde_json::json!({
        "type": "doc",
        "version": 1,
        "content": [
            {
                "type": "paragraph",
                "content": [
                    {
                        "type": "text",
                        "text": text
                    }
                ]
            }
        ]
    })
}

fn parse_creator(creator: &serde_json::Value) -> String {
    if let serde_json::Value::Object(creator) = creator {
        if let Some(serde_json::Value::String(display_name)) = creator.get("displayName") {
            return display_name.to_string();
        }
    }

    "".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_creator_uses_display_name() {
        let creator = serde_json::json!({
            "displayName": "John Doe",
            "emailAddress": "john@example.com"
        });
        let result = parse_creator(&creator);
        assert_eq!(result, "John Doe", "parse_creator should use displayName, not emailAddress");
    }

    #[test]
    fn test_supports_remote_cloud_url() {
        let connector = JiraRemoteConnector {};
        gittask::set_config_value("task.jira.url", "https://mycompany.atlassian.net/jira/software/projects/GTPM").unwrap();
        let result = connector.supports_remote("");
        gittask::set_config_value("task.jira.url", "").unwrap();
        assert_eq!(result, Some(("https://mycompany.atlassian.net".to_string(), "GTPM".to_string())));
    }

    #[test]
    fn test_supports_remote_self_hosted_url() {
        let connector = JiraRemoteConnector {};

        gittask::set_config_value("task.jira.url", "https://jira.mycompany.com/projects/ABC").unwrap();
        let result = connector.supports_remote("");
        assert_eq!(result, Some(("https://jira.mycompany.com".to_string(), "ABC".to_string())), "self-hosted Jira projects URL must be supported");

        gittask::set_config_value("task.jira.url", "https://jira.mycompany.com/browse/ABC").unwrap();
        let result = connector.supports_remote("");
        gittask::set_config_value("task.jira.url", "").unwrap();
        assert_eq!(result, Some(("https://jira.mycompany.com".to_string(), "ABC".to_string())), "self-hosted Jira browse URL must be supported");
    }

    #[test]
    fn test_supports_remote_project_from_config() {
        let connector = JiraRemoteConnector {};
        gittask::set_config_value("task.jira.url", "https://jira.mycompany.com").unwrap();
        gittask::set_config_value("task.jira.project", "XYZ").unwrap();
        let result = connector.supports_remote("");
        gittask::set_config_value("task.jira.url", "").unwrap();
        gittask::set_config_value("task.jira.project", "").unwrap();
        assert_eq!(result, Some(("https://jira.mycompany.com".to_string(), "XYZ".to_string())), "task.jira.project config must supply the project when the URL has none");
    }

    #[test]
    fn test_comments_are_truncated() {
        let truncated = serde_json::json!({
            "comments": [{"id": "1"}, {"id": "2"}],
            "maxResults": 2,
            "startAt": 0,
            "total": 80
        });
        assert!(comments_are_truncated(&truncated), "container with total > returned comments is truncated");

        let complete = serde_json::json!({
            "comments": [{"id": "1"}, {"id": "2"}],
            "maxResults": 50,
            "startAt": 0,
            "total": 2
        });
        assert!(!comments_are_truncated(&complete));

        let no_total = serde_json::json!({
            "comments": [{"id": "1"}]
        });
        assert!(!comments_are_truncated(&no_total));
    }
}

fn parse_status(status: &serde_json::Value) -> String {
    if let serde_json::Value::Object(status) = status {
        if let Some(serde_json::Value::String(status_name)) = status.get("name") {
            return status_name.to_string();
        }
    }

    "".to_string()
}

fn parse_author(author: &serde_json::Value) -> String {
    if let serde_json::Value::Object(author) = author {
        if let Some(serde_json::Value::String(display_name)) = author.get("displayName") {
            return display_name.to_string();
        }
    }

    "".to_string()
}

fn parse_to_unix_timestamp(date_str: &str) -> Result<String, String> {
    let dt = DateTime::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%S%.3f%z")
        .map_err(|e| e.to_string())?;

    let timestamp = dt.timestamp();

    Ok(timestamp.to_string())
}

// The embedded "comment" field is a paginated container; Jira returns only the first page in it.
fn comments_are_truncated(comments: &serde_json::Value) -> bool {
    if let serde_json::Value::Object(container) = comments {
        let total = container.get("total").and_then(|v| v.as_i64()).unwrap_or(0);
        let returned = container.get("comments").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        return total > returned as i64;
    }

    false
}

async fn fetch_all_comments(config: &Configuration, issue_key: &str) -> Result<Vec<Comment>, String> {
    let mut result = vec![];
    let mut start_at: i64 = 0;
    loop {
        let page = issue_comments_api::get_comments(config, issue_key, Some(start_at), Some(100), None, None)
            .await.map_err(|e| e.to_string())?;
        let comments = page.comments.unwrap_or_default();
        let batch_len = comments.len();
        result.extend(comments.into_iter().filter_map(convert_comment));
        start_at += batch_len as i64;
        if batch_len == 0 || start_at >= page.total.unwrap_or(0) {
            break;
        }
    }

    Ok(result)
}

fn convert_comment(comment: jira_v3_openapi::models::Comment) -> Option<Comment> {
    let id = comment.id?;
    let mut props = HashMap::new();
    if let Some(display_name) = comment.author.as_ref().and_then(|a| a.display_name.clone()) {
        props.insert("author".to_string(), display_name);
    }
    if let Some(created) = comment.created.as_deref().and_then(|s| parse_to_unix_timestamp(s).ok()) {
        props.insert("created".to_string(), created);
    }
    let text = comment.body.flatten().map(|body| parse_adf(&body)).unwrap_or_default();

    Some(Comment::new(id, props, text))
}

fn parse_comments(comments: &serde_json::Value) -> Vec<Comment> {
    if let serde_json::Value::Object(comments) = comments {
        if let Some(serde_json::Value::Array(comments_array)) = comments.get("comments") {
            return comments_array.iter().filter_map(|v| {
                match v {
                    serde_json::Value::Object(comment) => {
                        let id = comment.get("id").and_then(|v| v.as_str())?;
                        let mut props = HashMap::new();
                        if let Some(author) = comment.get("author") {
                            props.insert("author".to_string(), parse_author(author));
                        }
                        if let Some(created) = comment.get("created").and_then(|v| v.as_str()).and_then(|s| parse_to_unix_timestamp(s).ok()) {
                            props.insert("created".to_string(), created);
                        }
                        Some(Comment::new(
                            id.to_string(),
                            props,
                            comment.get("body").map(parse_adf).unwrap_or_default()
                        ))
                    }
                    _ => { None }
                }
            }).collect();
        }
    }

    vec![]
}