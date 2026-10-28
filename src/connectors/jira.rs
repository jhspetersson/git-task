use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::DateTime;
use jira_v3_openapi::{apis::configuration::Configuration, apis::issues_api};
use jira_v3_openapi::apis::{issue_comments_api, issue_fields_api, issue_search_api, projects_api};
use jira_v3_openapi::models::IssueTransition;
use regex::Regex;
use tokio::runtime::Runtime;

use gittask::{Task, Comment, Label, Subtask};

use crate::connectors::adf;
use crate::connectors::{property_name, status_names_match, RemoteConnector, RemoteField, RemoteTaskState, BUILT_IN_PROPERTIES};

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
            "task.jira.projects".to_string(),
            "task.jira.fields".to_string(),
        ])
    }

    fn supports_remote(&self, _url: &str) -> Option<(String, String)> {
        let origin = get_origin()?;
        let project = get_default_project()?;
        Some((origin, project))
    }

    fn supports_remotes(&self, _url: &str) -> Vec<(String, String)> {
        let Some(origin) = get_origin() else { return vec![] };
        let mut projects: Vec<String> = get_default_project().into_iter().collect();
        for project in get_configured_projects() {
            if !projects.iter().any(|p| p.eq_ignore_ascii_case(&project)) {
                projects.push(project);
            }
        }
        projects.into_iter().map(|project| (origin.clone(), project)).collect()
    }

    fn supports_multiple_projects(&self) -> bool {
        true
    }

    fn fields_config_option(&self) -> Option<String> {
        Some("task.jira.fields".to_string())
    }

    fn discover_remote_fields(&self, domain: &String, project: &String) -> Result<Vec<RemoteField>, String> {
        let config = get_configuration(domain)?;
        RUNTIME.block_on(async {
            let jql = build_jql(project, &RemoteTaskState::All, &[]);
            let response = issue_search_api::search_and_reconsile_issues_using_jql(
                &config,
                Some(&jql),
                None,
                Some(100),
                Some(vec!["*navigable".to_string()]),
                None,
                None,
                None,
                None,
                None,
            ).await.map_err(|e| e.to_string())?;
            let issues = response.issues.unwrap_or_default();
            let samples: Vec<&HashMap<String, serde_json::Value>> = issues.iter().filter_map(|issue| issue.fields.as_ref()).collect();
            let catalog = fetch_field_catalog(&config).await?;
            let mappings = build_field_mappings(&catalog, Some(&used_custom_fields(&samples)));
            Ok(mappings.into_iter().map(|mapping| RemoteField {
                name: catalog.iter()
                    .find(|field| field.id.as_deref() == Some(mapping.field.as_str()))
                    .and_then(|field| field.name.clone())
                    .unwrap_or_default(),
                kind: mapping.kind.map(FieldKind::config_name).unwrap_or("string").to_string(),
                property: mapping.property,
                id: mapping.field,
            }).collect())
        })
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
        let prefixed = multi_project_mode();
        let configured = get_field_mappings()?;
        let discover = configured.is_none();
        let mut mappings = configured.unwrap_or_default();

        let jql = RUNTIME.block_on(async {
            let jira_statuses = jira_status_names(&config, &state).await?;
            Ok::<String, String>(build_jql(project, &state, &jira_statuses))
        })?;
        
        let mut field_list = vec!["summary".to_string(), "description".to_string(), "status".to_string(), "created".to_string(), "creator".to_string()];
        if with_comments {
            field_list.push("comment".to_string());
        }
        if with_labels {
            field_list.push("labels".to_string());
        }
        field_list.extend(mappings.iter().map(|m| m.field.clone()));
        if discover {
            // Every navigable field, so the custom fields in use can be found.
            field_list.push("*navigable".to_string());
        }

        RUNTIME.block_on(async {
            let mut issues = vec![];
            let mut next_page_token: Option<String> = None;
            loop {
                let max_results = match limit {
                    Some(limit) => std::cmp::min(limit.saturating_sub(issues.len()), 100) as i32,
                    None => 100,
                };
                let response = issue_search_api::search_and_reconsile_issues_using_jql(
                    &config,
                    Some(&jql),
                    next_page_token.as_deref(),
                    Some(max_results),
                    Some(field_list.clone()),
                    None,
                    None,
                    None,
                    None,
                    None,
                ).await.map_err(|e| e.to_string())?;

                let batch = response.issues.unwrap_or_default();
                let batch_len = batch.len();
                issues.extend(batch);

                next_page_token = match next_search_page(response.is_last, response.next_page_token, batch_len, issues.len(), limit) {
                    Some(token) => Some(token),
                    None => break,
                };
            }

            if discover {
                let samples: Vec<&HashMap<String, serde_json::Value>> = issues.iter().filter_map(|issue| issue.fields.as_ref()).collect();
                mappings = discover_field_mappings(&config, &samples).await?;
            }

            let mut tasks: Vec<Task> = vec![];
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
                insert_custom_properties(&mut props, &fields, &mappings);

                let mut task = match Task::from_properties(issue_key_to_task_id(&key, prefixed), props) {
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
                                Err(e) => return Err(format!("Could not fetch all comments for {key}: {e}")),
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
    ) -> Result<Option<Task>, String> {
        let config = get_configuration(domain)?;
        let prefixed = multi_project_mode();
        let configured = get_field_mappings()?;
        let discover = configured.is_none();
        let mut mappings = configured.unwrap_or_default();

        let mut field_list = vec!["summary".to_string(), "description".to_string(), "status".to_string(), "created".to_string(), "creator".to_string()];
        if with_comments {
            field_list.push("comment".to_string());
        }
        if with_labels {
            field_list.push("labels".to_string());
        }
        field_list.extend(mappings.iter().map(|m| m.field.clone()));
        if discover {
            field_list.push("*navigable".to_string());
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
                        if discover {
                            mappings = discover_field_mappings(&config, &[&fields]).await?;
                        }
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
                        insert_custom_properties(&mut props, &fields, &mappings);

                        let mut task = Task::from_properties(issue_key_to_task_id(&issue_key, prefixed), props)?;

                        if with_comments {
                            if let Some(comments) = fields.get("comment") {
                                let mut task_comments = parse_comments(comments);
                                if comments_are_truncated(comments) {
                                    match fetch_all_comments(&config, &issue_key).await {
                                        Ok(all_comments) => task_comments = all_comments,
                                        Err(e) => return Err(format!("Could not fetch all comments for {issue_key}: {e}")),
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

                        Ok(Some(task))
                    } else {
                        Err("Failed to get issue: no fields returned.".to_string())
                    }
                },
                Err(jira_v3_openapi::apis::Error::ResponseError(content)) if content.status.as_u16() == 404 => Ok(None),
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
        let prefixed = multi_project_mode();
        let configured = get_field_mappings()?;
        let name = task.get_property("name").ok_or_else(|| "Task has no name".to_string())?;
        let description = task.get_property("description").map(String::as_str).unwrap_or("");

        RUNTIME.block_on(async {
            let mappings = match configured {
                Some(mappings) => mappings,
                None => mappings_for_task(&config, task).await?,
            };
            let mut issue_details = jira_v3_openapi::models::IssueUpdateDetails {
                fields: Some(std::collections::HashMap::from([
                    ("project".to_string(), serde_json::json!({
                        "key": project
                    })),
                    ("summary".to_string(), serde_json::json!(name)),
                    ("description".to_string(), format_adf(description)),
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
            for (field, value) in custom_fields_for_create(task.get_all_properties(), &mappings)? {
                issue_details.fields.as_mut().unwrap().insert(field, value);
            }

            match issues_api::create_issue(&config, issue_details, None).await {
                Ok(response) => {
                    match response.key {
                        Some(key) => {
                            let task_id = issue_key_to_task_id(&key, prefixed);
                            Ok(task_id)
                        },
                        None => Err("Failed to create issue: no key returned.".to_string())
                    }
                },
                Err(e) => Err(format!("Failed to create issue: {}", api_error(e)))
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
        let name = task.get_property("name").ok_or_else(|| "Task has no name".to_string())?;
        let description = task.get_property("description").map(String::as_str).unwrap_or("");
        let issue_key = task_id_to_issue_key(project, &task.get_id().unwrap());
        let configured = get_field_mappings()?;

        RUNTIME.block_on(async {
            let mappings = match configured {
                Some(mappings) => mappings,
                None => mappings_for_task(&config, task).await?,
            };
            let current = fetch_current_fields(&config, &issue_key, &mappings).await?;
            let fields = changed_fields(&current, name, description, labels, task.get_all_properties(), &mappings)?;

            let mut result = if fields.is_empty() {
                Ok(())
            } else {
                let issue_details = jira_v3_openapi::models::IssueUpdateDetails {
                    fields: Some(fields),
                    ..Default::default()
                };
                match issues_api::edit_issue(
                    &config,
                    issue_key.as_str(),
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
                }
            };

            if result.is_ok() {
                let (local_status, remote_status) = match state {
                    RemoteTaskState::Open(s1, s2) => (s1, s2),
                    RemoteTaskState::Closed(s1, s2) => (s1, s2),
                    _ => ("".to_string(), "".to_string())
                };
                result = transition_issue(&config, &issue_key, &local_status, &remote_status).await;
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

    fn list_remote_subtasks(&self, domain: &String, project: &String, task_id: &String, _task_statuses: &Vec<String>) -> Result<Vec<Subtask>, String> {
        let config = get_configuration(domain)?;
        let prefixed = multi_project_mode();

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
                                    result.push(Subtask::new(Some(issue_key_to_task_id(&key, prefixed)), name, status, HashMap::new()));
                                }
                            }
                        }
                    Ok(result)
                },
                Err(e) => Err(format!("Failed to list subtasks: {}", e))
            }
        })
    }

    fn create_remote_subtask(&self, domain: &String, project: &String, task_id: &String, subtask: &Subtask, _is_done: bool) -> Result<String, String> {
        let config = get_configuration(domain)?;
        let prefixed = multi_project_mode();

        let key = RUNTIME.block_on(create_subtask(&config, project, &task_id_to_issue_key(project, task_id), subtask))?;
        Ok(issue_key_to_task_id(&key, prefixed))
    }

    fn update_remote_subtask(&self, domain: &String, project: &String, _task_id: &String, subtask: &Subtask, _is_done: bool) -> Result<(), String> {
        let config = get_configuration(domain)?;
        let subtask_id = subtask.get_id().ok_or_else(|| "Subtask has no ID".to_string())?;
        let subtask_key = task_id_to_issue_key(project, &subtask_id);

        RUNTIME.block_on(update_subtask(&config, &subtask_key, subtask))
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

fn get_origin() -> Option<String> {
    let url = get_base_url()?;
    Some(Regex::new(r"^(https?://[^/]+)").unwrap().captures(&url)?.get(1)?.as_str().to_string())
}

/// Project keys listed in `task.jira.projects` (comma separated).
fn get_configured_projects() -> Vec<String> {
    match gittask::get_config_value("task.jira.projects") {
        Ok(projects) => projects
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
        _ => vec![],
    }
}

/// The project used when none is specified: `task.jira.project`, then the project
/// from the `task.jira.url` path, then the first entry of `task.jira.projects`.
pub(crate) fn get_default_project() -> Option<String> {
    if let Ok(project) = gittask::get_config_value("task.jira.project") {
        if !project.is_empty() {
            return Some(project);
        }
    }

    if let Some(url) = get_base_url() {
        if let Some(project) = Regex::new(r"/(?:projects|browse)/([A-Za-z][A-Za-z0-9_]*)").unwrap().captures(&url).and_then(|c| c.get(1)) {
            return Some(project.as_str().to_string());
        }
    }

    get_configured_projects().into_iter().next()
}

/// Several projects can be synced into one repository only when local task IDs keep
/// the full issue key (`ABC-42`), otherwise `ABC-42` and `XYZ-42` would collide as `42`.
/// This mode is enabled by setting `task.jira.projects`.
pub(crate) fn multi_project_mode() -> bool {
    !get_configured_projects().is_empty()
}

/// Returns the full issue key when `prefixed`, otherwise only the numeric part.
fn issue_key_to_task_id(key: &String, prefixed: bool) -> String {
    if prefixed {
        key.to_string()
    } else {
        key.split('-').last().unwrap_or_default().to_string()
    }
}

/// Accepts both a bare number (`42`) and a full issue key (`ABC-42`).
fn task_id_to_issue_key(project: &String, id: &String) -> String {
    if is_issue_key(id) {
        id.to_string()
    } else {
        format!("{}-{}", project, id)
    }
}

static ISSUE_KEY_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9_]*-\d+$").unwrap());

pub(crate) fn is_issue_key(id: &str) -> bool {
    ISSUE_KEY_REGEX.is_match(id)
}

/// Converts an ADF document (description or comment body) to the markdown
/// subset stored in local tasks. See `connectors::adf` for what survives.
fn parse_adf(description: &serde_json::Value) -> String {
    adf::adf_to_markdown(description)
}

/// Converts local markdown text to an ADF document for Jira.
fn format_adf(text: &str) -> serde_json::Value {
    adf::markdown_to_adf(text)
}

/// The remote state of the fields `update_remote_task` may write.
struct CurrentFields {
    summary: Option<String>,
    description: String,
    labels: Vec<String>,
    /// Raw values of the mapped custom fields, by Jira field id.
    custom: HashMap<String, serde_json::Value>,
}

async fn update_subtask(config: &Configuration, subtask_key: &str, subtask: &Subtask) -> Result<(), String> {
    let current_status = fetch_current_status(config, subtask_key).await?;

    let issue_details = jira_v3_openapi::models::IssueUpdateDetails {
        fields: Some(HashMap::from([
            ("summary".to_string(), serde_json::json!(subtask.get_name())),
        ])),
        ..Default::default()
    };
    match issues_api::edit_issue(config, subtask_key, issue_details, None, None, None, None, None).await {
        Ok(_) => {},
        Err(e) if e.to_string().starts_with("error in serde: EOF while parsing a value at line 1 column 0") => {},
        Err(e) => return Err(format!("Failed to update subtask: {}", e)),
    }

    ensure_status(config, subtask_key, current_status.as_deref(), subtask.get_status()).await
}

async fn create_subtask(config: &Configuration, project: &str, parent_key: &str, subtask: &Subtask) -> Result<String, String> {
    let issue_type = subtask_issue_type(config, project).await?;
    let issue_details = jira_v3_openapi::models::IssueUpdateDetails {
        fields: Some(HashMap::from([
            ("project".to_string(), serde_json::json!({ "key": project })),
            ("parent".to_string(), serde_json::json!({ "key": parent_key })),
            ("summary".to_string(), serde_json::json!(subtask.get_name())),
            ("issuetype".to_string(), serde_json::json!({ "id": issue_type })),
        ])),
        ..Default::default()
    };
    match issues_api::create_issue(config, issue_details, None).await {
        Ok(response) => response.key.ok_or_else(|| "Failed to create subtask: no key returned.".to_string()),
        Err(e) => Err(format!("Failed to create subtask: {}", api_error(e))),
    }
}

static SUBTASK_ISSUE_TYPES: LazyLock<std::sync::Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

/// The id of the project's sub-task issue type. Its name is not fixed: company-managed
/// projects call it "Sub-task", team-managed ones "Subtask", and both can be renamed,
/// so the type flagged as a sub-task in the project is used. Looked up once per project.
async fn subtask_issue_type(config: &Configuration, project: &str) -> Result<String, String> {
    let cache_key = format!("{}#{project}", config.base_path);
    if let Some(id) = SUBTASK_ISSUE_TYPES.lock().unwrap().get(&cache_key) {
        return Ok(id.clone());
    }
    let details = projects_api::get_project(config, project, None, None)
        .await
        .map_err(|e| format!("Failed to get project {project}: {}", api_error(e)))?;
    let id = details.issue_types.unwrap_or_default().into_iter()
        .find(|t| t.subtask == Some(true))
        .and_then(|t| t.id)
        .ok_or_else(|| format!("Project {project} has no sub-task issue type"))?;
    SUBTASK_ISSUE_TYPES.lock().unwrap().insert(cache_key, id.clone());
    Ok(id)
}

/// The client's own error text drops the response body, which is where Jira explains
/// what was wrong with a request.
fn api_error<T>(error: jira_v3_openapi::apis::Error<T>) -> String {
    match error {
        jira_v3_openapi::apis::Error::ResponseError(content) if !content.content.trim().is_empty() => {
            format!("status code {}: {}", content.status, content.content.trim())
        }
        other => other.to_string(),
    }
}

async fn ensure_status(config: &Configuration, issue_key: &str, current_status: Option<&str>, wanted_status: &str) -> Result<(), String> {
    let already_there = current_status.is_some_and(|current| status_names_match(current, wanted_status));
    if wanted_status.is_empty() || already_there {
        return Ok(());
    }
    transition_issue(config, issue_key, wanted_status, "").await
}

async fn fetch_current_status(config: &Configuration, issue_key: &str) -> Result<Option<String>, String> {
    match issues_api::get_issue(config, issue_key, Some(vec!["status".to_string()]), None, None, None, None, None).await {
        Ok(issue) => Ok(issue.fields.unwrap_or_default()
            .get("status")
            .and_then(|status| status.get("name"))
            .and_then(|name| name.as_str())
            .map(str::to_string)),
        Err(e) => Err(format!("Could not fetch issue {issue_key} before updating it: {e}")),
    }
}

async fn fetch_current_fields(config: &Configuration, issue_key: &str, mappings: &[FieldMapping]) -> Result<CurrentFields, String> {
    let mut field_list = vec!["summary".to_string(), "description".to_string(), "labels".to_string()];
    field_list.extend(mappings.iter().map(|m| m.field.clone()));
    match issues_api::get_issue(
        config,
        issue_key,
        Some(field_list),
        None,
        None,
        None,
        None,
        None,
    ).await {
        Ok(issue) => {
            let fields = issue.fields.unwrap_or_default();
            Ok(CurrentFields {
                summary: fields.get("summary").and_then(|v| v.as_str()).map(str::to_string),
                description: fields.get("description").map(parse_adf).unwrap_or_default(),
                labels: fields.get("labels").and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                    .unwrap_or_default(),
                custom: mappings.iter()
                    .filter_map(|m| fields.get(&m.field).map(|v| (m.field.clone(), v.clone())))
                    .collect(),
            })
        }
        Err(e) => Err(format!("Could not fetch issue {issue_key} before updating it: {e}")),
    }
}

/// Builds the edit payload with only the fields that differ from Jira.
///
/// The description is the important one: the local text is a markdown
/// approximation of the Jira document, so writing it back when it has not
/// changed would strip whatever formatting the conversion could not keep.
fn changed_fields(
    current: &CurrentFields,
    name: &str,
    description: &str,
    labels: Option<&Vec<Label>>,
    properties: &HashMap<String, String>,
    mappings: &[FieldMapping],
) -> Result<HashMap<String, serde_json::Value>, String> {
    let mut fields = HashMap::new();
    if current.summary.as_deref() != Some(name) {
        fields.insert("summary".to_string(), serde_json::json!(name));
    }
    if current.description != description {
        fields.insert("description".to_string(), format_adf(description));
    }
    if let Some(labels) = labels {
        let mut wanted: Vec<String> = labels.iter().map(|l| l.get_name()).collect();
        wanted.sort();
        let mut have = current.labels.clone();
        have.sort();
        if wanted != have {
            fields.insert("labels".to_string(), serde_json::json!(wanted));
        }
    }
    for mapping in mappings {
        // A property the task does not have leaves the Jira field alone.
        let Some(local) = properties.get(&mapping.property) else { continue };
        let remote = current.custom.get(&mapping.field);
        let remote_text = remote.and_then(custom_field_to_string).unwrap_or_default();
        if remote_text != *local {
            fields.insert(mapping.field.clone(), custom_field_payload(mapping, remote, local)?);
        }
    }
    Ok(fields)
}

/// How a local property is written to a Jira custom field.
#[derive(Clone, Copy, Debug, PartialEq)]
enum FieldKind {
    /// A plain string.
    Text,
    /// A rich text (paragraph) field, sent as an ADF document.
    Doc,
    Number,
    Bool,
    /// A select list: `{"value": "..."}`.
    Option,
    /// An object chosen by name: `{"name": "..."}` (priorities, versions, components).
    Named,
}

/// One `property=field[:type]` entry of `task.jira.fields`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FieldMapping {
    property: String,
    field: String,
    kind: Option<FieldKind>,
}

/// Parses `task.jira.fields`: comma separated `property=field[:type]` entries, for example
/// `points=customfield_10016:number,team=customfield_10001:option`.
pub(crate) fn parse_field_mappings(spec: &str) -> Result<Vec<FieldMapping>, String> {
    let mut mappings: Vec<FieldMapping> = vec![];
    for entry in spec.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let invalid = || format!("Invalid Jira field mapping '{entry}': expected property=field[:type]");
        let (property, target) = entry.split_once('=').ok_or_else(invalid)?;
        let (field, kind) = match target.split_once(':') {
            Some((field, kind)) => (field.trim(), Some(parse_field_kind(kind.trim(), entry)?)),
            None => (target.trim(), None),
        };
        let property = property.trim();
        if property.is_empty() || field.is_empty() {
            return Err(invalid());
        }
        if BUILT_IN_PROPERTIES.contains(&property) {
            return Err(format!("Invalid Jira field mapping '{entry}': '{property}' is a built-in property"));
        }
        if mappings.iter().any(|m| m.property == property) {
            return Err(format!("Invalid Jira field mapping '{entry}': property '{property}' is mapped twice"));
        }
        if mappings.iter().any(|m| m.field == field) {
            return Err(format!("Invalid Jira field mapping '{entry}': field '{field}' is mapped twice"));
        }
        mappings.push(FieldMapping { property: property.to_string(), field: field.to_string(), kind });
    }
    Ok(mappings)
}

fn parse_field_kind(kind: &str, entry: &str) -> Result<FieldKind, String> {
    match kind.to_ascii_lowercase().as_str() {
        "string" | "text" => Ok(FieldKind::Text),
        "doc" | "paragraph" => Ok(FieldKind::Doc),
        "number" => Ok(FieldKind::Number),
        "bool" | "boolean" => Ok(FieldKind::Bool),
        "option" | "select" => Ok(FieldKind::Option),
        "name" | "named" => Ok(FieldKind::Named),
        _ => Err(format!("Invalid Jira field mapping '{entry}': unknown type '{kind}' (expected string, doc, number, bool, option or name)")),
    }
}

/// The custom field mappings configured in `task.jira.fields`, or `None` when the option
/// is not set and the mapping is discovered from Jira instead.
fn get_field_mappings() -> Result<Option<Vec<FieldMapping>>, String> {
    match gittask::get_config_value("task.jira.fields") {
        Ok(spec) => parse_field_mappings(&spec).map(Some),
        Err(_) => Ok(None),
    }
}

/// Serializes mappings in the `task.jira.fields` syntax.
#[cfg(test)]
fn format_field_mappings(mappings: &[FieldMapping]) -> String {
    mappings.iter().map(|m| match m.kind {
        Some(kind) => format!("{}={}:{}", m.property, m.field, kind.config_name()),
        None => format!("{}={}", m.property, m.field),
    }).collect::<Vec<_>>().join(",")
}

impl FieldKind {
    fn config_name(self) -> &'static str {
        match self {
            FieldKind::Text => "string",
            FieldKind::Doc => "doc",
            FieldKind::Number => "number",
            FieldKind::Bool => "bool",
            FieldKind::Option => "option",
            FieldKind::Named => "name",
        }
    }
}

async fn fetch_field_catalog(config: &Configuration) -> Result<Vec<jira_v3_openapi::models::FieldDetails>, String> {
    issue_fields_api::get_fields(config).await.map_err(|e| format!("Could not list Jira fields: {e}"))
}

/// Discovers a mapping for the custom fields that carry a value on the given issues:
/// each property is named after the Jira field, typed from the field schema.
async fn discover_field_mappings(config: &Configuration, issues: &[&HashMap<String, serde_json::Value>]) -> Result<Vec<FieldMapping>, String> {
    let catalog = fetch_field_catalog(config).await?;
    Ok(build_field_mappings(&catalog, Some(&used_custom_fields(issues))))
}

/// Discovers the mapping for the properties of a task about to be pushed: every custom
/// field of the instance whose derived property name the task has.
async fn mappings_for_task(config: &Configuration, task: &Task) -> Result<Vec<FieldMapping>, String> {
    let catalog = fetch_field_catalog(config).await?;
    let mut mappings = build_field_mappings(&catalog, None);
    mappings.retain(|mapping| task.has_property(&mapping.property));
    Ok(mappings)
}

/// The custom fields holding a value on at least one of the issues.
fn used_custom_fields(issues: &[&HashMap<String, serde_json::Value>]) -> Vec<String> {
    let mut used: Vec<String> = issues.iter()
        .flat_map(|fields| fields.iter())
        .filter(|(id, value)| id.starts_with("customfield_") && !value.is_null())
        .map(|(id, _)| id.clone())
        .collect();
    used.sort();
    used.dedup();
    used
}

/// Maps the custom fields of the catalog that can be synchronized, or only those in
/// `used_fields`. Property names are derived over the whole catalog, in field id order,
/// so a name does not depend on which fields happen to hold a value.
fn build_field_mappings(catalog: &[jira_v3_openapi::models::FieldDetails], used_fields: Option<&[String]>) -> Vec<FieldMapping> {
    let mut custom_fields: Vec<&jira_v3_openapi::models::FieldDetails> = catalog.iter()
        .filter(|field| field.id.as_deref().is_some_and(|id| id.starts_with("customfield_")))
        .collect();
    custom_fields.sort_by_key(|field| custom_field_number(field));

    let mut mappings: Vec<FieldMapping> = vec![];
    for field in custom_fields {
        let id = field.id.as_deref().unwrap_or_default();
        let Some(kind) = field.schema.as_deref().and_then(field_kind_from_schema) else { continue };
        let mut property = property_name(field.name.as_deref().unwrap_or(id));
        if BUILT_IN_PROPERTIES.contains(&property.as_str()) || mappings.iter().any(|m| m.property == property) {
            property = format!("{property}_{}", id.trim_start_matches("customfield_"));
        }
        mappings.push(FieldMapping { property, field: id.to_string(), kind: Some(kind) });
    }
    if let Some(used) = used_fields {
        mappings.retain(|mapping| used.contains(&mapping.field));
    }
    mappings.sort_by(|a, b| a.property.cmp(&b.property));
    mappings
}

fn custom_field_number(field: &jira_v3_openapi::models::FieldDetails) -> i64 {
    field.schema.as_deref().and_then(|schema| schema.custom_id)
        .or_else(|| field.id.as_deref().and_then(|id| id.trim_start_matches("customfield_").parse().ok()))
        .unwrap_or(i64::MAX)
}

/// The writable shape of a field, from its schema. Fields that cannot be written back by
/// value (users, cascading selects, sprints, ranks, development info) are not mapped.
fn field_kind_from_schema(schema: &jira_v3_openapi::models::JsonTypeBean) -> Option<FieldKind> {
    let custom_type = schema.custom.as_deref().and_then(|c| c.rsplit(':').next()).unwrap_or("");
    match (schema.r#type.as_str(), schema.items.as_deref()) {
        (_, _) if custom_type == "gh-lexo-rank" => None,
        ("string", _) if custom_type == "textarea" => Some(FieldKind::Doc),
        ("string", _) | ("date", _) | ("datetime", _) => Some(FieldKind::Text),
        ("number", _) => Some(FieldKind::Number),
        ("option", _) | ("array", Some("option")) => Some(FieldKind::Option),
        ("array", Some("string")) => Some(FieldKind::Text),
        ("version", _) | ("component", _) | ("array", Some("version")) | ("array", Some("component")) => Some(FieldKind::Named),
        _ => None,
    }
}

/// Copies the mapped Jira fields of an issue into task properties.
fn insert_custom_properties(props: &mut HashMap<String, String>, fields: &HashMap<String, serde_json::Value>, mappings: &[FieldMapping]) {
    for mapping in mappings {
        if let Some(value) = fields.get(&mapping.field).and_then(custom_field_to_string) {
            props.insert(mapping.property.clone(), value);
        }
    }
}

fn is_adf_document(value: &serde_json::Value) -> bool {
    value.get("type").and_then(|t| t.as_str()) == Some("doc") && value.get("content").is_some()
}

/// Renders a Jira field value as a task property. `None` for an empty (null) field,
/// so that the local property, if any, is left alone.
fn custom_field_to_string(value: &serde_json::Value) -> Option<String> {
    use serde_json::Value;
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => i.to_string(),
            (None, Some(f)) if f.fract() == 0.0 && f.abs() < 9e15 => format!("{}", f as i64),
            _ => n.to_string(),
        }),
        Value::Array(items) => Some(items.iter().filter_map(custom_field_to_string).collect::<Vec<_>>().join(", ")),
        Value::Object(_) if is_adf_document(value) => Some(parse_adf(value)),
        Value::Object(obj) => ["value", "name", "displayName", "key", "id"].iter()
            .find_map(|k| obj.get(*k))
            .and_then(custom_field_to_string),
    }
}

/// Guesses how to write a field from the value Jira currently holds in it.
fn infer_field_kind(value: &serde_json::Value) -> Option<FieldKind> {
    use serde_json::Value;
    match value {
        Value::Null => None,
        Value::String(_) => Some(FieldKind::Text),
        Value::Number(_) => Some(FieldKind::Number),
        Value::Bool(_) => Some(FieldKind::Bool),
        Value::Array(items) => items.first().and_then(infer_field_kind),
        Value::Object(_) if is_adf_document(value) => Some(FieldKind::Doc),
        Value::Object(obj) if obj.contains_key("value") => Some(FieldKind::Option),
        Value::Object(obj) if obj.contains_key("name") => Some(FieldKind::Named),
        Value::Object(_) => Some(FieldKind::Text),
    }
}

/// The value written to a Jira field for a local property. The shape follows the type
/// given in the mapping, else the field's current value, else plain text. An empty
/// property clears the field; a multi-value field takes a comma separated list.
fn custom_field_payload(mapping: &FieldMapping, current: Option<&serde_json::Value>, local: &str) -> Result<serde_json::Value, String> {
    use serde_json::Value;
    if local.trim().is_empty() {
        return Ok(Value::Null);
    }
    let kind = mapping.kind.or_else(|| current.and_then(infer_field_kind)).unwrap_or(FieldKind::Text);
    if let Some(Value::Array(_)) = current {
        return local.split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| scalar_payload(mapping, kind, part))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array);
    }
    scalar_payload(mapping, kind, local)
}

fn scalar_payload(mapping: &FieldMapping, kind: FieldKind, local: &str) -> Result<serde_json::Value, String> {
    let bad = |expected: &str| format!("Property '{}' must be {expected} to be pushed to Jira field {}, got '{local}'", mapping.property, mapping.field);
    Ok(match kind {
        FieldKind::Text => serde_json::json!(local),
        FieldKind::Doc => format_adf(local),
        FieldKind::Number => match local.parse::<i64>() {
            Ok(i) => serde_json::json!(i),
            Err(_) => serde_json::json!(local.parse::<f64>().map_err(|_| bad("a number"))?),
        },
        FieldKind::Bool => match local.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => serde_json::json!(true),
            "false" | "no" | "0" => serde_json::json!(false),
            _ => return Err(bad("true or false")),
        },
        FieldKind::Option => serde_json::json!({"value": local}),
        FieldKind::Named => serde_json::json!({"name": local}),
    })
}

/// The mapped custom fields to send when creating an issue. Without a current value to
/// learn the shape from, a mapping needs an explicit type for anything but text.
fn custom_fields_for_create(properties: &HashMap<String, String>, mappings: &[FieldMapping]) -> Result<Vec<(String, serde_json::Value)>, String> {
    let mut fields = vec![];
    for mapping in mappings {
        if let Some(value) = properties.get(&mapping.property).filter(|v| !v.trim().is_empty()) {
            fields.push((mapping.field.clone(), custom_field_payload(mapping, None, value)?));
        }
    }
    Ok(fields)
}

fn parse_creator(creator: &serde_json::Value) -> String {
    if let serde_json::Value::Object(creator) = creator {
        if let Some(serde_json::Value::String(display_name)) = creator.get("displayName") {
            return display_name.to_string();
        }
    }

    "".to_string()
}

fn next_search_page(is_last: Option<bool>, next_page_token: Option<String>, batch_len: usize, collected: usize, limit: Option<usize>) -> Option<String> {
    if batch_len == 0 || is_last.unwrap_or(false) || limit.is_some_and(|limit| collected >= limit) {
        return None;
    }
    next_page_token.filter(|token| !token.is_empty())
}

fn build_jql(project: &str, state: &RemoteTaskState, jira_statuses: &[String]) -> String {
    let (status, is_done) = match state {
        RemoteTaskState::Open(status, _) => (status, false),
        RemoteTaskState::Closed(status, _) => (status, true),
        RemoteTaskState::All => return format!("project = {project}"),
    };
    let jira_status = jira_statuses.iter().find(|name| !status.is_empty() && status_names_match(name, status));
    match jira_status {
        Some(name) => format!("project = {project} AND status = \"{}\"", name.replace('\\', "\\\\").replace('"', "\\\"")),
        None if is_done => format!("project = {project} AND statusCategory = Done"),
        None => format!("project = {project} AND statusCategory != Done"),
    }
}

async fn jira_status_names(config: &Configuration, state: &RemoteTaskState) -> Result<Vec<String>, String> {
    let needs_lookup = match state {
        RemoteTaskState::Open(status, _) | RemoteTaskState::Closed(status, _) => !status.is_empty(),
        RemoteTaskState::All => false,
    };
    if !needs_lookup {
        return Ok(vec![]);
    }
    let statuses = jira_v3_openapi::apis::workflow_statuses_api::get_statuses(config)
        .await
        .map_err(|e| format!("Failed to list Jira statuses: {e}"))?;
    Ok(statuses.into_iter().filter_map(|status| status.name).collect())
}

fn find_transition(transitions: &[IssueTransition], wanted_status: &str) -> Result<String, String> {
    let target_name = |transition: &IssueTransition| transition.to.as_ref().and_then(|to| to.name.clone());
    match transitions.iter().find(|t| target_name(t).is_some_and(|name| status_names_match(&name, wanted_status))).and_then(|t| t.id.clone()) {
        Some(id) => Ok(id),
        None => {
            let reachable = transitions.iter().filter_map(|t| target_name(t)).collect::<Vec<_>>();
            Err(match reachable.is_empty() {
                true => format!("No transition leads to status '{wanted_status}'"),
                false => format!("No transition leads to status '{wanted_status}', reachable statuses: {}", reachable.join(", ")),
            })
        }
    }
}

async fn transition_issue(config: &Configuration, issue_key: &str, local_status: &str, remote_status: &str) -> Result<(), String> {
    if local_status.is_empty() || local_status == remote_status {
        return Ok(());
    }
    let transitions = issues_api::get_transitions(config, issue_key, None, None, None, None, None)
        .await
        .map_err(|e| format!("Failed to get transitions for {issue_key}: {e}"))?;
    let transition_id = find_transition(&transitions.transitions.unwrap_or_default(), local_status)?;
    let issue_details = jira_v3_openapi::models::IssueUpdateDetails {
        transition: Some(IssueTransition {
            id: Some(transition_id),
            ..Default::default()
        }),
        ..Default::default()
    };
    match issues_api::do_transition(config, issue_key, issue_details).await {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().starts_with("error in serde: EOF while parsing a value at line 1 column 0") => Ok(()),
        Err(e) => Err(format!("Failed to transition {issue_key} to status '{local_status}': {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transition(id: &str, target: &str) -> IssueTransition {
        IssueTransition {
            id: Some(id.to_string()),
            to: Some(jira_v3_openapi::models::StatusDetails { name: Some(target.to_string()), ..Default::default() }),
            ..Default::default()
        }
    }

    fn jira_statuses() -> Vec<String> {
        vec!["To Do".to_string(), "In Progress".to_string(), "Done".to_string()]
    }

    fn stub_jira(handler: impl Fn(&str, &str) -> (u16, String) + Send + 'static) -> (Configuration, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
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
                loop {
                    if let Some(pos) = data.windows(4).position(|w| w == header_end) {
                        let head = String::from_utf8_lossy(&data[..pos]).to_string();
                        let content_length = head.lines().find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            if key.eq_ignore_ascii_case("content-length") { value.trim().parse::<usize>().ok() } else { None }
                        }).unwrap_or(0);
                        if data.len() >= pos + 4 + content_length {
                            break;
                        }
                    }
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
        let mut config = Configuration::new();
        config.base_path = format!("http://{addr}");
        (config, requests)
    }

    fn subtask_with_status(status: &str) -> Subtask {
        Subtask::new(Some("ABC-2".to_string()), "Child".to_string(), status.to_string(), HashMap::new())
    }

    fn jira_subtask_flow(current_status: &'static str, transition_status: u16) -> impl Fn(&str, &str) -> (u16, String) + Send + 'static {
        move |method, path| match (method, path) {
            // A team-managed project: the sub-task type is called "Subtask", not "Sub-task".
            ("GET", "/rest/api/3/project/ABC") => (200, r#"{"id":"10000","key":"ABC","issueTypes":[{"id":"10001","name":"Task","subtask":false},{"id":"10003","name":"Subtask","subtask":true}]}"#.to_string()),
            ("POST", "/rest/api/3/issue") => (201, r#"{"id":"10002","key":"ABC-2"}"#.to_string()),
            ("GET", "/rest/api/3/issue/ABC-2") => (200, format!(r#"{{"fields":{{"status":{{"name":"{current_status}"}}}}}}"#)),
            ("PUT", "/rest/api/3/issue/ABC-2") => (204, String::new()),
            ("GET", "/rest/api/3/issue/ABC-2/transitions") => (200, r#"{"transitions":[{"id":"31","to":{"name":"Done"}}]}"#.to_string()),
            ("POST", "/rest/api/3/issue/ABC-2/transitions") => (transition_status, if transition_status < 300 { String::new() } else { r#"{"errorMessages":["Forbidden"]}"#.to_string() }),
            _ => (404, String::new()),
        }
    }

    #[test]
    fn test_update_subtask_fails_when_the_transition_is_rejected() {
        let (config, requests) = stub_jira(jira_subtask_flow("To Do", 403));

        let result = RUNTIME.block_on(update_subtask(&config, "ABC-2", &subtask_with_status("DONE")));

        assert!(result.is_err(), "a rejected transition must fail the subtask update");
        assert!(result.as_ref().unwrap_err().contains("Failed to transition ABC-2"), "unexpected error: {result:?}");
        assert!(requests.lock().unwrap().contains(&"POST /rest/api/3/issue/ABC-2/transitions".to_string()), "the transition must have been attempted");
    }

    #[test]
    fn test_update_subtask_transitions_when_the_status_differs() {
        let (config, requests) = stub_jira(jira_subtask_flow("To Do", 204));

        let result = RUNTIME.block_on(update_subtask(&config, "ABC-2", &subtask_with_status("DONE")));

        assert_eq!(result, Ok(()));
        assert!(requests.lock().unwrap().contains(&"POST /rest/api/3/issue/ABC-2/transitions".to_string()));
    }

    #[test]
    fn test_update_subtask_skips_the_transition_when_the_status_already_matches() {
        let (config, requests) = stub_jira(jira_subtask_flow("Done", 403));

        let result = RUNTIME.block_on(update_subtask(&config, "ABC-2", &subtask_with_status("DONE")));

        assert_eq!(result, Ok(()));
        let requests = requests.lock().unwrap();
        assert!(requests.contains(&"PUT /rest/api/3/issue/ABC-2".to_string()), "the summary must still be updated");
        assert!(!requests.iter().any(|r| r.ends_with("/transitions")), "no transition request must be made, got {requests:?}");
    }

    #[test]
    fn test_create_subtask_only_creates_the_issue_and_returns_its_key() {
        let (config, requests) = stub_jira(jira_subtask_flow("To Do", 403));

        let result = RUNTIME.block_on(create_subtask(&config, "ABC", "ABC-1", &subtask_with_status("DONE")));

        assert_eq!(result, Ok("ABC-2".to_string()));
        assert_eq!(requests.lock().unwrap().as_slice(), ["GET /rest/api/3/project/ABC", "POST /rest/api/3/issue"], "the status is applied by the caller once the key is recorded, so a rejected transition cannot lose the key");

        // The sub-task issue type of the project is looked up once.
        let result = RUNTIME.block_on(create_subtask(&config, "ABC", "ABC-1", &subtask_with_status("DONE")));
        assert_eq!(result, Ok("ABC-2".to_string()));
        assert_eq!(requests.lock().unwrap().iter().filter(|r| r.starts_with("GET /rest/api/3/project")).count(), 1);
    }

    #[test]
    fn test_create_subtask_reports_a_project_without_a_subtask_type() {
        let (config, requests) = stub_jira(|method, path| match (method, path) {
            ("GET", "/rest/api/3/project/NOSUB") => (200, r#"{"id":"10000","key":"NOSUB","issueTypes":[{"id":"10001","name":"Sub-task","subtask":false}]}"#.to_string()),
            _ => (404, String::new()),
        });

        let result = RUNTIME.block_on(create_subtask(&config, "NOSUB", "NOSUB-1", &subtask_with_status("DONE")));

        assert_eq!(result, Err("Project NOSUB has no sub-task issue type".to_string()), "the type is found by its sub-task flag, not by its name");
        assert!(!requests.lock().unwrap().contains(&"POST /rest/api/3/issue".to_string()), "nothing must be created without a type");
    }

    #[test]
    fn test_api_errors_include_what_jira_said() {
        let (config, _) = stub_jira(|method, path| match (method, path) {
            ("GET", "/rest/api/3/project/ERR") => (400, r#"{"errorMessages":["Issue type is required"]}"#.to_string()),
            _ => (404, String::new()),
        });

        let result = RUNTIME.block_on(create_subtask(&config, "ERR", "ERR-1", &subtask_with_status("DONE")));

        let error = result.unwrap_err();
        assert!(error.contains("400") && error.contains("Issue type is required"), "the response body must be part of the error: {error}");
    }

    #[test]
    fn test_next_search_page_follows_the_token_until_the_last_page() {
        assert_eq!(next_search_page(Some(false), Some("page2".to_string()), 100, 100, None), Some("page2".to_string()));
        assert_eq!(next_search_page(None, Some("page2".to_string()), 100, 100, None), Some("page2".to_string()), "a missing is_last flag does not stop paging while a token is present");
        assert_eq!(next_search_page(Some(true), Some("stale".to_string()), 7, 107, None), None, "the last page ends paging even if a token is present");
        assert_eq!(next_search_page(Some(false), None, 100, 100, None), None, "no token means no further page");
        assert_eq!(next_search_page(Some(false), Some(String::new()), 100, 100, None), None, "an empty token means no further page");
        assert_eq!(next_search_page(Some(false), Some("page2".to_string()), 0, 0, None), None, "an empty page ends paging");
    }

    #[test]
    fn test_next_search_page_stops_at_the_limit() {
        assert_eq!(next_search_page(Some(false), Some("page2".to_string()), 50, 50, Some(50)), None);
        assert_eq!(next_search_page(Some(false), Some("page2".to_string()), 50, 50, Some(80)), Some("page2".to_string()));
    }

    #[test]
    fn test_build_jql_without_a_status_filter() {
        assert_eq!(build_jql("ABC", &RemoteTaskState::All, &jira_statuses()), "project = ABC");
    }

    #[test]
    fn test_build_jql_for_any_open_or_closed_status_uses_the_status_category() {
        assert_eq!(build_jql("ABC", &RemoteTaskState::Open(String::new(), String::new()), &jira_statuses()), "project = ABC AND statusCategory != Done");
        assert_eq!(build_jql("ABC", &RemoteTaskState::Closed(String::new(), String::new()), &jira_statuses()), "project = ABC AND statusCategory = Done");
    }

    #[test]
    fn test_build_jql_for_a_status_known_to_jira_filters_by_that_status() {
        let state = RemoteTaskState::Open("IN_PROGRESS".to_string(), "IN_PROGRESS".to_string());
        assert_eq!(build_jql("ABC", &state, &jira_statuses()), "project = ABC AND status = \"In Progress\"", "the requested status must be used, spelled as Jira spells it");
        let state = RemoteTaskState::Closed("Done".to_string(), "Done".to_string());
        assert_eq!(build_jql("ABC", &state, &jira_statuses()), "project = ABC AND status = \"Done\"");
    }

    #[test]
    fn test_build_jql_for_a_status_unknown_to_jira_falls_back_to_the_category() {
        let state = RemoteTaskState::Open("OPEN".to_string(), "OPEN".to_string());
        assert_eq!(build_jql("ABC", &state, &jira_statuses()), "project = ABC AND statusCategory != Done");
        let state = RemoteTaskState::Closed("CLOSED".to_string(), "CLOSED".to_string());
        assert_eq!(build_jql("ABC", &state, &jira_statuses()), "project = ABC AND statusCategory = Done");
    }

    #[test]
    fn test_find_transition_picks_the_one_leading_to_the_wanted_status() {
        let transitions = vec![transition("11", "To Do"), transition("21", "In Progress"), transition("31", "Done")];

        assert_eq!(find_transition(&transitions, "Done"), Ok("31".to_string()));
        assert_eq!(find_transition(&transitions, "IN_PROGRESS"), Ok("21".to_string()), "status names match loosely");
    }

    #[test]
    fn test_find_transition_reports_the_reachable_statuses_when_none_matches() {
        let transitions = vec![transition("11", "To Do"), transition("21", "In Progress")];

        let error = find_transition(&transitions, "Done").unwrap_err();

        assert!(error.contains("Done"), "the wanted status must be named: {error}");
        assert!(error.contains("To Do") && error.contains("In Progress"), "the reachable statuses must be listed: {error}");
        assert!(find_transition(&[], "Done").is_err());
    }

    fn current(summary: &str, description: &str, labels: &[&str]) -> CurrentFields {
        CurrentFields {
            summary: Some(summary.to_string()),
            description: description.to_string(),
            labels: labels.iter().map(|l| l.to_string()).collect(),
            custom: HashMap::new(),
        }
    }

    fn no_custom_fields() -> (HashMap<String, String>, Vec<FieldMapping>) {
        (HashMap::new(), vec![])
    }

    fn props(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn test_update_sends_nothing_when_only_the_status_changed() {
        let remote = current("Title", "# Heading\n\n- a list", &["bug"]);
        let labels = vec![Label::new("bug".to_string(), None, None)];
        let (properties, mappings) = no_custom_fields();
        let fields = changed_fields(&remote, "Title", "# Heading\n\n- a list", Some(&labels), &properties, &mappings).unwrap();
        assert!(fields.is_empty(), "an unchanged description must never be written back: {fields:?}");
    }

    #[test]
    fn test_update_sends_only_the_changed_fields() {
        let remote = current("Title", "body", &["bug"]);
        let labels = vec![Label::new("bug".to_string(), None, None), Label::new("ui".to_string(), None, None)];

        let (properties, mappings) = no_custom_fields();
        let fields = changed_fields(&remote, "New title", "body", Some(&labels), &properties, &mappings).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields["summary"], serde_json::json!("New title"));
        assert_eq!(fields["labels"], serde_json::json!(["bug", "ui"]));
        assert!(!fields.contains_key("description"));

        let fields = changed_fields(&remote, "Title", "edited body", None, &properties, &mappings).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields["description"], format_adf("edited body"));
    }

    #[test]
    fn test_update_ignores_label_order() {
        let remote = current("Title", "body", &["ui", "bug"]);
        let labels = vec![Label::new("bug".to_string(), None, None), Label::new("ui".to_string(), None, None)];
        let (properties, mappings) = no_custom_fields();
        assert!(changed_fields(&remote, "Title", "body", Some(&labels), &properties, &mappings).unwrap().is_empty());
    }

    #[test]
    fn test_parse_field_mappings() {
        let mappings = parse_field_mappings(" points = customfield_10016:number, team=customfield_10001 ,, due=duedate:text").unwrap();
        assert_eq!(mappings, vec![
            FieldMapping { property: "points".to_string(), field: "customfield_10016".to_string(), kind: Some(FieldKind::Number) },
            FieldMapping { property: "team".to_string(), field: "customfield_10001".to_string(), kind: None },
            FieldMapping { property: "due".to_string(), field: "duedate".to_string(), kind: Some(FieldKind::Text) },
        ]);
        assert_eq!(parse_field_mappings("").unwrap(), vec![]);
    }

    #[test]
    fn test_parse_field_mappings_rejects_bad_entries() {
        for spec in ["points", "=customfield_1", "points=", "points=customfield_1:money", "name=customfield_1", "a=customfield_1,a=customfield_2", "a=customfield_1,b=customfield_1"] {
            let error = parse_field_mappings(spec).unwrap_err();
            assert!(error.starts_with("Invalid Jira field mapping"), "{spec}: {error}");
        }
    }

    #[test]
    fn test_custom_field_values_become_text() {
        assert_eq!(custom_field_to_string(&serde_json::json!(null)), None);
        assert_eq!(custom_field_to_string(&serde_json::json!("plain")), Some("plain".to_string()));
        assert_eq!(custom_field_to_string(&serde_json::json!(5.0)), Some("5".to_string()));
        assert_eq!(custom_field_to_string(&serde_json::json!(2.5)), Some("2.5".to_string()));
        assert_eq!(custom_field_to_string(&serde_json::json!(true)), Some("true".to_string()));
        assert_eq!(custom_field_to_string(&serde_json::json!({"self": "...", "value": "Backend", "id": "10100"})), Some("Backend".to_string()));
        assert_eq!(custom_field_to_string(&serde_json::json!({"accountId": "x", "displayName": "Alice"})), Some("Alice".to_string()));
        assert_eq!(custom_field_to_string(&serde_json::json!([{"value": "a"}, {"value": "b"}])), Some("a, b".to_string()));
        assert_eq!(custom_field_to_string(&serde_json::json!([])), Some(String::new()));
        let doc = format_adf("**bold** text");
        assert_eq!(custom_field_to_string(&doc), Some("**bold** text".to_string()), "paragraph fields are converted like descriptions");
    }

    #[test]
    fn test_pulled_issue_fields_fill_the_mapped_properties() {
        let mappings = parse_field_mappings("points=customfield_10016,team=customfield_10001,empty=customfield_10002").unwrap();
        let fields: HashMap<String, serde_json::Value> = serde_json::from_value(serde_json::json!({
            "summary": "Title",
            "customfield_10016": 3,
            "customfield_10001": {"value": "Core"},
            "customfield_10002": null,
        })).unwrap();
        let mut collected = HashMap::new();
        insert_custom_properties(&mut collected, &fields, &mappings);
        assert_eq!(collected, props(&[("points", "3"), ("team", "Core")]));
    }

    #[test]
    fn test_update_sends_only_the_changed_custom_fields() {
        let mut remote = current("Title", "body", &[]);
        remote.custom = HashMap::from([
            ("customfield_10016".to_string(), serde_json::json!(3.0)),
            ("customfield_10001".to_string(), serde_json::json!({"value": "Core", "id": "1"})),
            ("customfield_10002".to_string(), serde_json::json!(null)),
            ("customfield_10003".to_string(), serde_json::json!([{"value": "a"}, {"value": "b"}])),
        ]);
        let mappings = parse_field_mappings("points=customfield_10016,team=customfield_10001,note=customfield_10002,tags=customfield_10003,untouched=customfield_10004").unwrap();

        let unchanged = props(&[("points", "3"), ("team", "Core"), ("tags", "a, b")]);
        assert!(changed_fields(&remote, "Title", "body", None, &unchanged, &mappings).unwrap().is_empty(), "equal values must not be written back");

        let edited = props(&[("points", "5"), ("team", "Platform"), ("note", "hello"), ("tags", "b, c")]);
        let fields = changed_fields(&remote, "Title", "body", None, &edited, &mappings).unwrap();
        assert_eq!(fields.len(), 4, "{fields:?}");
        assert_eq!(fields["customfield_10016"], serde_json::json!(5), "the number shape is learnt from the current value");
        assert_eq!(fields["customfield_10001"], serde_json::json!({"value": "Platform"}), "options are set by value");
        assert_eq!(fields["customfield_10002"], serde_json::json!("hello"), "an empty field with no type is written as text");
        assert_eq!(fields["customfield_10003"], serde_json::json!([{"value": "b"}, {"value": "c"}]), "multi-select lists are split on commas");

        let cleared = props(&[("team", "")]);
        let fields = changed_fields(&remote, "Title", "body", None, &cleared, &mappings).unwrap();
        assert_eq!(fields["customfield_10001"], serde_json::json!(null), "an empty property clears the field");
    }

    #[test]
    fn test_explicit_field_types_win_and_are_validated() {
        let mappings = parse_field_mappings("points=customfield_1:number,flag=customfield_2:bool,team=customfield_3:option,prio=customfield_4:name,notes=customfield_5:doc").unwrap();
        let remote = current("Title", "body", &[]);

        let fields = changed_fields(&remote, "Title", "body", None, &props(&[("points", "2.5"), ("flag", "yes"), ("team", "Core"), ("prio", "High"), ("notes", "# Head")]), &mappings).unwrap();
        assert_eq!(fields["customfield_1"], serde_json::json!(2.5));
        assert_eq!(fields["customfield_2"], serde_json::json!(true));
        assert_eq!(fields["customfield_3"], serde_json::json!({"value": "Core"}));
        assert_eq!(fields["customfield_4"], serde_json::json!({"name": "High"}));
        assert_eq!(fields["customfield_5"], format_adf("# Head"));

        let error = changed_fields(&remote, "Title", "body", None, &props(&[("points", "many")]), &mappings).unwrap_err();
        assert!(error.contains("points") && error.contains("customfield_1") && error.contains("many"), "{error}");
        assert!(changed_fields(&remote, "Title", "body", None, &props(&[("flag", "maybe")]), &mappings).is_err());
    }

    #[test]
    fn test_create_sends_the_mapped_properties_that_are_set() {
        let mappings = parse_field_mappings("points=customfield_1:number,team=customfield_2,blank=customfield_3").unwrap();
        let fields = custom_fields_for_create(&props(&[("points", "8"), ("team", "Core"), ("blank", ""), ("other", "x")]), &mappings).unwrap();
        let fields: HashMap<String, serde_json::Value> = fields.into_iter().collect();
        assert_eq!(fields, HashMap::from([
            ("customfield_1".to_string(), serde_json::json!(8)),
            ("customfield_2".to_string(), serde_json::json!("Core")),
        ]));
    }

    fn catalog_field(id: &str, name: &str, kind: &str, items: Option<&str>, custom_type: Option<&str>) -> jira_v3_openapi::models::FieldDetails {
        jira_v3_openapi::models::FieldDetails {
            id: Some(id.to_string()),
            name: Some(name.to_string()),
            custom: Some(id.starts_with("customfield_")),
            schema: Some(Box::new(jira_v3_openapi::models::JsonTypeBean {
                r#type: kind.to_string(),
                items: items.map(str::to_string),
                custom: custom_type.map(|t| format!("com.atlassian.jira.plugin.system.customfieldtypes:{t}")),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn sample_catalog() -> Vec<jira_v3_openapi::models::FieldDetails> {
        vec![
            catalog_field("summary", "Summary", "string", None, None),
            catalog_field("customfield_10016", "Story point estimate", "number", None, Some("float")),
            catalog_field("customfield_10001", "Team", "option", None, Some("select")),
            catalog_field("customfield_10002", "Flagged", "array", Some("option"), Some("multicheckboxes")),
            catalog_field("customfield_10003", "Acceptance Criteria", "string", None, Some("textarea")),
            catalog_field("customfield_10004", "Reviewer", "user", None, Some("userpicker")),
            catalog_field("customfield_10005", "Rank", "string", None, Some("gh-lexo-rank")),
            catalog_field("customfield_10006", "Status", "string", None, Some("textfield")),
            catalog_field("customfield_10007", "Team", "string", None, Some("textfield")),
            catalog_field("customfield_10008", "Fix Versions (custom)", "array", Some("version"), Some("multiversion")),
            catalog_field("customfield_10009", "Unused", "number", None, Some("float")),
        ]
    }

    #[test]
    fn test_build_field_mappings_names_and_types_the_used_custom_fields() {
        let used: Vec<String> = (1..=8).map(|n| format!("customfield_1000{n}")).chain(["customfield_10016".to_string()]).collect();

        let mappings = build_field_mappings(&sample_catalog(), Some(&used));

        assert_eq!(format_field_mappings(&mappings), "acceptance_criteria=customfield_10003:doc,fix_versions_custom=customfield_10008:name,flagged=customfield_10002:option,status_10006=customfield_10006:string,story_point_estimate=customfield_10016:number,team=customfield_10001:option,team_10007=customfield_10007:string");
        assert!(mappings.iter().all(|m| m.field != "customfield_10004"), "user fields cannot be written by value and are skipped");
        assert!(mappings.iter().all(|m| m.field != "customfield_10005"), "the rank is noise");
        assert!(mappings.iter().all(|m| m.field != "customfield_10009"), "fields without a value on any issue are skipped");
    }

    #[test]
    fn test_property_names_do_not_depend_on_the_fields_in_use() {
        let only_second_team = build_field_mappings(&sample_catalog(), Some(&["customfield_10007".to_string()]));
        assert_eq!(format_field_mappings(&only_second_team), "team_10007=customfield_10007:string", "the name is the same as when both Team fields are in use");

        let everything = build_field_mappings(&sample_catalog(), None);
        assert_eq!(everything.len(), 8, "every writable custom field of the catalog: {everything:?}");
        assert!(everything.iter().any(|m| m.field == "customfield_10009"), "unused fields are included when no usage is given");
    }

    #[test]
    fn test_formatted_mappings_parse_back() {
        let mappings = build_field_mappings(&sample_catalog(), Some(&["customfield_10016".to_string(), "customfield_10001".to_string()]));
        assert_eq!(parse_field_mappings(&format_field_mappings(&mappings)).unwrap(), mappings);
        let untyped = vec![FieldMapping { property: "a".to_string(), field: "customfield_1".to_string(), kind: None }];
        assert_eq!(format_field_mappings(&untyped), "a=customfield_1");
    }

    #[test]
    fn test_property_names_come_from_field_names() {
        assert_eq!(property_name("Story point estimate"), "story_point_estimate");
        assert_eq!(property_name("  Épic -- Link!"), "épic_link");
        assert_eq!(property_name("???"), "field");
    }

    #[test]
    fn test_discover_field_mappings_uses_the_fields_with_values_on_the_issues() {
        let (config, requests) = stub_jira(|method, path| match (method, path) {
            ("GET", "/rest/api/3/field") => (200, r#"[
                {"id":"customfield_10016","name":"Story point estimate","custom":true,"schema":{"type":"number","custom":"com.atlassian.jira.plugin.system.customfieldtypes:float","customId":10016}},
                {"id":"customfield_10001","name":"Team","custom":true,"schema":{"type":"option","custom":"com.atlassian.jira.plugin.system.customfieldtypes:select","customId":10001}},
                {"id":"customfield_10009","name":"Unused","custom":true,"schema":{"type":"number","custom":"com.atlassian.jira.plugin.system.customfieldtypes:float","customId":10009}},
                {"id":"summary","name":"Summary","custom":false,"schema":{"type":"string","system":"summary"}}
            ]"#.to_string()),
            _ => (404, String::new()),
        });
        let first: HashMap<String, serde_json::Value> = serde_json::from_value(serde_json::json!({"summary": "A", "customfield_10016": 3, "customfield_10001": null, "customfield_10009": null})).unwrap();
        let second: HashMap<String, serde_json::Value> = serde_json::from_value(serde_json::json!({"summary": "B", "customfield_10016": null, "customfield_10001": {"value": "Core"}})).unwrap();

        let mappings = RUNTIME.block_on(discover_field_mappings(&config, &[&first, &second])).unwrap();

        assert_eq!(requests.lock().unwrap().as_slice(), ["GET /rest/api/3/field"]);
        assert_eq!(format_field_mappings(&mappings), "story_point_estimate=customfield_10016:number,team=customfield_10001:option");
    }

    #[test]
    fn test_fetch_current_fields_requests_and_keeps_the_mapped_fields() {
        let (config, requests) = stub_jira(|method, path| match (method, path) {
            ("GET", "/rest/api/3/issue/ABC-1") => (200, r#"{"fields":{"summary":"Title","labels":["bug"],"customfield_10016":5,"customfield_10001":{"value":"Core"}}}"#.to_string()),
            _ => (404, String::new()),
        });
        let mappings = parse_field_mappings("points=customfield_10016,team=customfield_10001,missing=customfield_10099").unwrap();

        let current = RUNTIME.block_on(fetch_current_fields(&config, "ABC-1", &mappings)).unwrap();

        assert_eq!(requests.lock().unwrap().as_slice(), ["GET /rest/api/3/issue/ABC-1"]);
        assert_eq!(current.summary.as_deref(), Some("Title"));
        assert_eq!(current.custom, HashMap::from([
            ("customfield_10016".to_string(), serde_json::json!(5)),
            ("customfield_10001".to_string(), serde_json::json!({"value": "Core"})),
        ]), "fields Jira did not return are simply absent");
    }

    #[test]
    fn test_rich_description_round_trips_through_pull_and_push() {
        let jira_doc = serde_json::json!({"type": "doc", "version": 1, "content": [
            {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Steps"}]},
            {"type": "orderedList", "content": [
                {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "open the "}, {"type": "text", "text": "page", "marks": [{"type": "link", "attrs": {"href": "https://example.com"}}]}]}]},
                {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "run "}, {"type": "text", "text": "cargo test", "marks": [{"type": "code"}]}]}]}
            ]},
            {"type": "codeBlock", "attrs": {"language": "sh"}, "content": [{"type": "text", "text": "git task list"}]}
        ]});
        let local = parse_adf(&jira_doc);
        assert_eq!(local, "## Steps\n\n1. open the [page](https://example.com)\n2. run `cargo test`\n\n```sh\ngit task list\n```");
        assert_eq!(format_adf(&local), jira_doc, "pushing the pulled text must rebuild the same document");
    }

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
        let _repo = gittask::testing::TestRepo::new();
        let connector = JiraRemoteConnector {};
        gittask::set_config_value("task.jira.url", "https://mycompany.atlassian.net/jira/software/projects/GTPM").unwrap();
        let result = connector.supports_remote("");
        assert_eq!(result, Some(("https://mycompany.atlassian.net".to_string(), "GTPM".to_string())));
    }

    #[test]
    fn test_supports_remote_self_hosted_url() {
        let _repo = gittask::testing::TestRepo::new();
        let connector = JiraRemoteConnector {};

        gittask::set_config_value("task.jira.url", "https://jira.mycompany.com/projects/ABC").unwrap();
        let result = connector.supports_remote("");
        assert_eq!(result, Some(("https://jira.mycompany.com".to_string(), "ABC".to_string())), "self-hosted Jira projects URL must be supported");

        gittask::set_config_value("task.jira.url", "https://jira.mycompany.com/browse/ABC").unwrap();
        let result = connector.supports_remote("");
        assert_eq!(result, Some(("https://jira.mycompany.com".to_string(), "ABC".to_string())), "self-hosted Jira browse URL must be supported");
    }

    #[test]
    fn test_supports_remote_project_from_config() {
        let _repo = gittask::testing::TestRepo::new();
        let connector = JiraRemoteConnector {};
        gittask::set_config_value("task.jira.url", "https://jira.mycompany.com").unwrap();
        gittask::set_config_value("task.jira.project", "XYZ").unwrap();
        let result = connector.supports_remote("");
        assert_eq!(result, Some(("https://jira.mycompany.com".to_string(), "XYZ".to_string())), "task.jira.project config must supply the project when the URL has none");
    }

    #[test]
    fn test_supports_remotes_lists_configured_projects() {
        let _repo = gittask::testing::TestRepo::new();
        let connector = JiraRemoteConnector {};
        gittask::set_config_value("task.jira.url", "https://jira.mycompany.com/browse/ABC").unwrap();
        gittask::set_config_value("task.jira.projects", "XYZ, abc ,QRS").unwrap();
        let all = connector.supports_remotes("");
        let default = connector.supports_remote("");
        let multi = multi_project_mode();
        let origin = "https://jira.mycompany.com".to_string();
        assert_eq!(default, Some((origin.clone(), "ABC".to_string())), "URL project stays the default");
        assert_eq!(all, vec![
            (origin.clone(), "ABC".to_string()),
            (origin.clone(), "XYZ".to_string()),
            (origin.clone(), "QRS".to_string()),
        ], "default first, then configured projects without duplicates");
        assert!(multi, "task.jira.projects enables multi-project mode");
    }

    #[test]
    fn test_default_project_falls_back_to_first_configured() {
        let _repo = gittask::testing::TestRepo::new();
        let connector = JiraRemoteConnector {};
        gittask::set_config_value("task.jira.url", "https://jira.mycompany.com").unwrap();
        gittask::set_config_value("task.jira.projects", "XYZ,QRS").unwrap();
        let default = connector.supports_remote("");
        assert_eq!(default, Some(("https://jira.mycompany.com".to_string(), "XYZ".to_string())));
    }

    #[test]
    fn test_issue_key_task_id_conversion() {
        let project = "ABC".to_string();
        assert_eq!(issue_key_to_task_id(&"ABC-42".to_string(), false), "42");
        assert_eq!(issue_key_to_task_id(&"ABC-42".to_string(), true), "ABC-42");
        assert_eq!(task_id_to_issue_key(&project, &"42".to_string()), "ABC-42");
        assert_eq!(task_id_to_issue_key(&project, &"XYZ-7".to_string()), "XYZ-7", "a full key is used as is");
        assert!(is_issue_key("ABC-42"));
        assert!(is_issue_key("A_1-7"));
        assert!(!is_issue_key("42"));
        assert!(!is_issue_key("1..5"));
        assert!(!is_issue_key("ABC-"));
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