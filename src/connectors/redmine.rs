use std::collections::HashMap;

use redmine_api::api::issues::{Issue, ListIssues};
use redmine_api::api::Redmine;

use gittask::{Task, Comment, Label};

use crate::connectors::{RemoteConnector, RemoteTaskState};

pub struct RedmineRemoteConnector;

impl RemoteConnector for RedmineRemoteConnector {
    fn type_name(&self) -> &str {
        "redmine"
    }

    fn get_config_options(&self) -> Option<Vec<String>> {
        Some(vec![
            "task.redmine.url".to_string(),
            "task.redmine.api_key".to_string(),
        ])
    }

    #[allow(unused)]
    fn supports_remote(&self, url: &str) -> Option<(String, String)> {
        Some((String::new(), String::new()))
    }

    fn list_remote_tasks(
        &self,
        domain: &String,
        _project: &String,
        _with_comments: bool,
        _with_labels: bool,
        _limit: Option<usize>,
        _state: RemoteTaskState,
        task_statuses: &Vec<String>
    ) -> Result<Vec<Task>, String> {
        let redmine = get_redmine_instance(domain)?;
        let endpoint = ListIssues::builder().build().map_err(|e| e.to_string())?;
        let issues = redmine.json_response_body_all_pages::<_, Issue>(&endpoint).map_err(|e| e.to_string())?;
        let mut tasks = Vec::new();
        for issue in issues {
            let task = issue_to_task(&issue, task_statuses)?;
            tasks.push(task);
        }
        Ok(tasks)
    }

    #[allow(unused)]
    fn get_remote_task(
        &self,
        domain: &String,
        project: &String,
        task_id: &String,
        with_comments: bool,
        with_labels: bool,
        task_statuses: &Vec<String>
    ) -> Result<Task, String> {
        let redmine = get_redmine_instance(domain)?;

        todo!()
    }

    #[allow(unused)]
    fn create_remote_task(&self, domain: &String, project: &String, task: &Task) -> Result<String, String> {
        let redmine = get_redmine_instance(domain)?;

        todo!()
    }

    #[allow(unused)]
    fn create_remote_comment(&self, domain: &String, project: &String, task_id: &String, comment: &Comment) -> Result<String, String> {
        let redmine = get_redmine_instance(domain)?;

        todo!()
    }

    #[allow(unused)]
    fn create_remote_label(&self, domain: &String, project: &String, task_id: &String, label: &Label) -> Result<(), String> {
        let redmine = get_redmine_instance(domain)?;

        todo!()
    }

    #[allow(unused)]
    fn update_remote_task(
        &self,
        domain: &String,
        project: &String,
        task: &Task,
        labels: Option<&Vec<Label>>,
        state: RemoteTaskState
    ) -> Result<(), String> {
        let redmine = get_redmine_instance(domain)?;

        todo!()
    }

    #[allow(unused)]
    fn update_remote_comment(&self, domain: &String, project: &String, task_id: &String, comment_id: &String, text: &String) -> Result<(), String> {
        let redmine = get_redmine_instance(domain)?;

        todo!()
    }

    #[allow(unused)]
    fn delete_remote_task(&self, domain: &String, project: &String, task_id: &String) -> Result<(), String> {
        let redmine = get_redmine_instance(domain)?;

        todo!()
    }

    #[allow(unused)]
    fn delete_remote_comment(&self, domain: &String, project: &String, _task_id: &String, comment_id: &String) -> Result<(), String> {
        let redmine = get_redmine_instance(domain)?;

        todo!()
    }

    #[allow(unused)]
    fn delete_remote_label(&self, domain: &String, project: &String, task_id: &String, name: &String) -> Result<(), String> {
        let redmine = get_redmine_instance(domain)?;

        todo!()
    }
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
    match gittask::get_config_value("task.redmine.api_key") {
        Ok(key) => Ok(key),
        _ => std::env::var("REDMINE_API_KEY")
            .or_else(|_| std::env::var("REDMINE_TOKEN"))
            .map_err(|_| "No Redmine API key found. Set task.redmine.api_key config or REDMINE_API_KEY environment variable.".to_string())
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
    
    Task::from_properties(issue.id.to_string(), props).map_err(|e| e.to_string())
}