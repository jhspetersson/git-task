use std::collections::HashMap;

use gitlab::api::issues::{IssueScope, IssueState};
use gitlab::api::Query;
use gitlab::Gitlab;
use regex::Regex;
use serde::{Deserialize, Serialize};

use gittask::{Comment, Task};
use crate::connectors::{RemoteConnector, RemoteTaskState};
use crate::util::parse_datetime_to_seconds;

pub struct GitlabRemoteConnector;

#[derive(Serialize, Deserialize)]
struct Author {
    username: String,
}

#[derive(Serialize, Deserialize)]
struct Issue {
    iid: u64,
    title: String,
    description: String,
    author: Author,
    created_at: String,
    state: String,
}

impl RemoteConnector for GitlabRemoteConnector {
    fn supports_remote(&self, url: &str) -> Option<(String, String)> {
        match Regex::new("https://gitlab.com/([a-z0-9-]+)/([a-z0-9-]+)\\.?").unwrap().captures(url) {
            Some(caps) if caps.len() == 3 => {
                let user = caps.get(1)?.as_str().to_string();
                let repo = caps.get(2)?.as_str().to_string();
                Some((user, repo))
            },
            _ => None,
        }
    }

    fn list_remote_tasks(&self, user: &String, repo: &String, with_comments: bool, limit: Option<usize>, state: RemoteTaskState, task_statuses: &Vec<String>) -> Vec<Task> {
        let state = match state {
            RemoteTaskState::Open => Some(IssueState::Opened),
            RemoteTaskState::Closed => Some(IssueState::Closed),
            RemoteTaskState::All => None
        };
        let client = get_client(get_token_from_env().unwrap().as_str());
        let mut endpoint = gitlab::api::issues::ProjectIssues::builder();
        let mut endpoint = endpoint.project(user.to_string() + "/" + repo).scope(IssueScope::All);
        endpoint = match state {
            Some(state) => endpoint.state(state),
            None => endpoint
        };
        let endpoint = endpoint.build().unwrap();
        let issues: Vec<Issue> = gitlab::api::paged(endpoint, gitlab::api::Pagination::Limit(limit.unwrap_or_else(|| 100))).query(&client).unwrap();
        let mut result = vec![];
        for issue in issues {
            let mut props = HashMap::new();
            props.insert(String::from("name"), issue.title);
            props.insert(String::from("description"), issue.description);
            props.insert(String::from("status"), if issue.state == "opened" { task_statuses.get(0).unwrap().clone() } else { task_statuses.get(1).unwrap().clone() });
            props.insert(String::from("created"), parse_datetime_to_seconds(issue.created_at));
            props.insert(String::from("author"), issue.author.username);

            let task = Task::from_properties(issue.iid.to_string(), props).unwrap();

            if with_comments {
                eprintln!("Comments are not supported for Gitlab");
            }

            result.push(task);
        }

        if result.len() == 100 {
            eprintln!("Only last 100 issues are supported for Gitlab");
        }

        result
    }

    fn get_remote_task(&self, user: &String, repo: &String, task_id: &String, with_comments: bool, task_statuses: &Vec<String>) -> Option<Task> {
        let client = get_client(get_token_from_env().unwrap().as_str());
        let mut endpoint = gitlab::api::issues::ProjectIssues::builder();
        let mut endpoint = endpoint.project(user.to_string() + "/" + repo);
        endpoint = endpoint.iid(task_id.parse().unwrap());
        let endpoint = endpoint.build().unwrap();

        match endpoint.query(&client) {
            Ok(issue) => {
                let issue: Issue = issue;
                let mut props = HashMap::new();
                props.insert(String::from("name"), issue.title);
                props.insert(String::from("description"), issue.description);
                props.insert(String::from("status"), if issue.state == "opened" { task_statuses.get(0).unwrap().clone() } else { task_statuses.get(1).unwrap().clone() });
                props.insert(String::from("created"), parse_datetime_to_seconds(issue.created_at));
                props.insert(String::from("author"), issue.author.username);

                let task = Task::from_properties(task_id.to_string(), props).unwrap();

                if with_comments {
                    eprintln!("Comments are not supported for Gitlab");
                }

                Some(task)
            },
            Err(_) => None
        }
    }

    fn create_remote_task(&self, user: &String, repo: &String, task: &Task) -> Result<String, String> {
        let client = get_client(get_token_from_env().unwrap().as_str());
        let mut endpoint = gitlab::api::projects::issues::CreateIssue::builder();
        let mut endpoint = endpoint.project(user.to_string() + "/" + repo);
        endpoint.title(task.get_property("name").unwrap());
        endpoint.description(task.get_property("description").unwrap());
        let endpoint = endpoint.build().unwrap();
        let issue: Issue = endpoint.query(&client).unwrap();

        if let Some(comments) = task.get_comments() {
            if !comments.is_empty() {
                eprintln!("Comments are not supported for Gitlab");
            }
        }

        Ok(issue.iid.to_string())
    }

    fn create_remote_comment(&self, user: &String, repo: &String, task_id: &String, comment: &Comment) -> Result<String, String> {
        todo!()
    }

    fn update_remote_task(&self, user: &String, repo: &String, task_id: &String, name: &String, text: &String, state: RemoteTaskState) -> Result<(), String> {
        todo!()
    }

    fn update_remote_comment(&self, user: &String, repo: &String, comment_id: &String, text: &String) -> Result<(), String> {
        todo!()
    }

    fn delete_remote_task(&self, user: &String, repo: &String, task_id: &String) -> Result<(), String> {
        todo!()
    }

    fn delete_remote_comment(&self, user: &String, repo: &String, comment_id: &String) -> Result<(), String> {
        todo!()
    }
}

fn get_client(token: &str) -> Gitlab {
    Gitlab::new("gitlab.com", token).unwrap()
}

fn get_token_from_env() -> Option<String> {
    std::env::var("GITLAB_TOKEN").or_else(|_| std::env::var("GITLAB_API_TOKEN")).ok()
}