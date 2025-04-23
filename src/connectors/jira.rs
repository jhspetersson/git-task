use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::DateTime;
use jira_v3_openapi::{apis::configuration::Configuration, apis::issues_api};
use jira_v3_openapi::apis::{issue_comments_api, issue_search_api};
use regex::Regex;
use tokio::runtime::Runtime;

use gittask::{Task, Comment, Label};

use crate::connectors::{RemoteConnector, RemoteTaskState};

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
        ])
    }

    fn supports_remote(&self, _url: &str) -> Option<(String, String)> {
        if let Some(url) = get_base_url() {
            match Regex::new(r"https://([^/]+)\.atlassian\.net/jira/software/projects/([^/]+)").unwrap().captures(&url) {
                Some(caps) if caps.len() >= 3 => {
                    let domain = caps.get(1)?.as_str().to_string();
                    let project = caps.get(2)?.as_str().to_string();
                    Some((domain, project))
                },
                _ => None,
            }
        } else {
            None
        }
    }

    fn list_remote_tasks(
        &self,
        domain: &String,
        project: &String,
        with_comments: bool,
        with_labels: bool,
        limit: Option<usize>,
        state: RemoteTaskState,
        _task_statuses: &Vec<String>
    ) -> Result<Vec<Task>, String> {
        let config = get_configuration(domain)?;

        let jql = match state {
            RemoteTaskState::Open => format!("project = {} AND status != Done", project),
            RemoteTaskState::Closed => format!("project = {} AND status = Done", project),
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
            let issues = issue_search_api::search_for_issues_using_jql(
                &config,
                Some(&jql),
                None,
                if let Some(limit) = limit { Some(limit as i32) } else { None },
                None,
                Some(field_list),
                None,
                None,
                None,
                None,
            ).await;
            match issues {
                Ok(response) => {
                    let mut tasks = vec![];
                    for issue in response.issues.unwrap_or_default() {
                        let mut props = HashMap::new();
                        if let Some(fields) = issue.fields {
                            props.insert("name".to_string(), fields.get("summary").unwrap().as_str().unwrap().to_string());
                            props.insert("description".to_string(), parse_description(fields.get("description").unwrap()));
                            props.insert("status".to_string(), parse_status(fields.get("status").unwrap()));
                            props.insert("created".to_string(), parse_to_unix_timestamp(fields.get("created").unwrap().as_str().unwrap()).unwrap());
                            props.insert("author".to_string(), parse_creator(fields.get("creator").unwrap()));

                            let mut task = Task::from_properties(issue_key_to_task_id(&issue.key.unwrap()), props).unwrap();

                            if with_comments {
                                if let Some(comment) = fields.get("comment") {
                                    if let Some(comment_obj) = comment.as_object() {
                                        if let Some(serde_json::Value::Array(comments)) = comment_obj.get("comments") {
                                            let task_comments = comments.iter().map(|v| {
                                                if let serde_json::Value::Object(comment) = v {
                                                    Comment::new(
                                                        comment.get("id").unwrap().as_str().unwrap().to_string(),
                                                        HashMap::from([
                                                            ("author".to_string(), parse_author(comment.get("author").unwrap())),
                                                            ("created".to_string(), parse_to_unix_timestamp(comment.get("created").unwrap().as_str().unwrap()).unwrap()),
                                                        ]),
                                                        parse_description(comment.get("body").unwrap())
                                                    )
                                                } else {
                                                    Comment::new(String::new(), HashMap::new(), String::new())
                                                }
                                            }).collect();
                                            task.set_comments(task_comments);
                                        }
                                    }
                                }
                            }

                            if with_labels {
                                if let Some(serde_json::Value::Array(labels)) = fields.get("labels") {
                                    let task_labels = labels.iter().map(|v| {
                                        Label::new(v.as_str().unwrap().to_string(), None, None)
                                    }).collect();
                                    task.set_labels(task_labels);
                                }
                            }

                            tasks.push(task);
                        }
                    }

                    Ok(tasks)
                },
                Err(e) => Err(e.to_string()),
            }
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

        RUNTIME.block_on(async {
            match issues_api::get_issue(
                &config,
                task_id_to_issue_key(project, task_id).as_str(),
                Some(vec!["summary".to_string(), "description".to_string(), "status".to_string(), "created".to_string(), "creator".to_string()]),
                None,
                None,
                None,
                None,
                None,
            ).await {
                Ok(issue) => {
                    let mut props = HashMap::new();
                    let mut task_labels = None;
                    if let Some(fields) = issue.fields {
                        props.insert("name".to_string(), fields.get("summary").unwrap().as_str().unwrap().to_string());
                        props.insert("description".to_string(), parse_description(fields.get("description").unwrap()));
                        props.insert("status".to_string(), parse_status(fields.get("status").unwrap()));
                        props.insert("created".to_string(), parse_to_unix_timestamp(fields.get("created").unwrap().as_str().unwrap())?);
                        props.insert("author".to_string(), parse_creator(fields.get("creator").unwrap()));

                        if with_labels {
                            if let Some(serde_json::Value::Array(labels)) = fields.get("labels") {
                                task_labels = Some(labels.iter().map(|v| {
                                    Label::new(v.as_str().unwrap().to_string(), None, None)
                                }).collect());
                            }
                        }
                    }

                    let mut task = Task::from_properties(issue_key_to_task_id(&issue.key.unwrap()), props)?;

                    if with_comments {
                        if let Ok(comments) = list_issue_comments(&config, project, task_id).await {
                            task.set_comments(comments);
                        }
                    }

                    if let Some(labels) = task_labels {
                        task.set_labels(labels);
                    }

                    Ok(task)
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
                        format_description(task.get_property("description").unwrap())
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
                body: Some(Some(serde_json::json!(comment.get_text().clone()))),
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
                                update: None,
                                fields: Some(std::collections::HashMap::from([
                                    ("labels".to_string(), serde_json::json!(current_labels))
                                ])),
                                ..Default::default()
                            };

                            match issues_api::edit_issue(
                                &config,
                                task_id,
                                update_request,
                                None,
                                None,
                                None,
                                None,
                                None,
                            ).await {
                                Ok(_) => Ok(()),
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

            fields.insert("summary".to_string(),
                          serde_json::json!(task.get_property("name").unwrap()));
            fields.insert("description".to_string(),
                          serde_json::json!(task.get_property("description").unwrap()));

            if let Some(labels) = labels {
                fields.insert(
                    "labels".to_string(),
                    serde_json::json!(
                        labels.iter()
                            .map(|l| l.get_name())
                            .collect::<Vec<String>>()
                    )
                );
            }

            let transition = match state {
                RemoteTaskState::Closed => Some(serde_json::json!({
                    "id": "31" // Typically "31" is Close in Jira, but might need configuration
                })),
                RemoteTaskState::Open => Some(serde_json::json!({
                    "id": "11" // Typically "11" is Reopen in Jira, but might need configuration
                })),
                _ => None
            };

            if let Some(transition_value) = transition {
                fields.insert("transition".to_string(), transition_value);
            }

            let issue_details = jira_v3_openapi::models::IssueUpdateDetails {
                fields: Some(fields),
                ..Default::default()
            };

            match issues_api::edit_issue(
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
                Err(e) => Err(format!("Failed to update issue: {}", e))
            }
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
                body: Some(Some(serde_json::json!(text.clone()))),
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
                None
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
                            update: None,
                            fields: Some(std::collections::HashMap::from([
                                ("labels".to_string(), serde_json::json!(updated_labels))
                            ])),
                            ..Default::default()
                        };

                        match issues_api::edit_issue(
                            &config,
                            task_id,
                            update_request,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ).await {
                            Ok(_) => Ok(()),
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
}

async fn list_issue_comments(config: &Configuration, project: &String, task_id: &String) -> Result<Vec<Comment>, ()> {
    let comments_result = issue_comments_api::get_comments(
        config,
        task_id_to_issue_key(project, task_id).as_str(),
        None,
        None,
        None,
        None,
    ).await;

    match comments_result {
        Ok(comments_response) => {
            let comments = comments_response.comments.unwrap_or_default().into_iter().map(|comment| {
                Comment::new(
                    comment.id.unwrap(),
                    HashMap::from([
                        ("author".to_string(), comment.author.unwrap().display_name.unwrap()),
                        ("created".to_string(), comment.created.unwrap().to_string()),
                    ]),
                    comment.body.unwrap().map_or_else(|| String::new(), |s| s.to_string())
                )
            }).collect();
            Ok(comments)
        },
        Err(_) => Err(()),
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

fn get_configuration(domain: &String) -> Result<Configuration, String> {
    let email = get_jira_user()?;
    let token = get_token_from_env()?;

    let mut config = Configuration::new();
    config.basic_auth = Some((email, Some(token)));
    config.base_path = format!("https://{}.atlassian.net", domain);
    Ok(config)
}

fn issue_key_to_task_id(key: &String) -> String {
    key.split('-').last().unwrap_or_default().to_string()
}

fn task_id_to_issue_key(project: &String, id: &String) -> String {
    format!("{}-{}", project, id)
}

fn parse_description(description: &serde_json::Value) -> String {
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

fn format_description(description: &String) -> serde_json::Value {
    serde_json::json!({
        "type": "doc",
        "version": 1,
        "content": [
            {
                "type": "paragraph",
                "content": [
                    {
                        "type": "text",
                        "text": description
                    }
                ]
            }
        ]
    })
}

fn parse_creator(creator: &serde_json::Value) -> String {
    if let serde_json::Value::Object(creator) = creator {
        if let Some(serde_json::Value::String(display_name)) = creator.get("emailAddress") {
            return display_name.to_string();
        }
    }

    "".to_string()
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