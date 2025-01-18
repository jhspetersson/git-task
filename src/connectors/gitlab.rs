use std::collections::HashMap;

use gitlab::api::issues::{IssueScope, IssueState};
use gitlab::api::projects::issues::IssueStateEvent;
use gitlab::api::{Pagination, Query};
use gitlab::Gitlab;
use regex::Regex;
use serde::{Deserialize, Serialize};

use gittask::{Comment, Label, Task};
use crate::connectors::{RemoteConnector, RemoteTaskState};
use crate::util::{color_str_to_rgb_str, parse_datetime_to_seconds};

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
    labels: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct GitlabComment {
    id: u64,
    body: String,
    author: Author,
    created_at: String,
}

#[derive(Serialize, Deserialize)]
struct GitlabLabel {
    name: String,
    color: String,
    description: String,
}

#[derive(Deserialize)]
struct DeleteIssueResult {}

#[derive(Deserialize)]
struct DeleteIssueNoteResult {}

impl RemoteConnector for GitlabRemoteConnector {
    fn supports_remote(&self, url: &str) -> Option<(String, String)> {
        match Regex::new(&(get_base_url() + "([a-z0-9-]+)/([a-z0-9-]+)\\.?")).unwrap().captures(url) {
            Some(caps) if caps.len() == 3 => {
                let user = caps.get(1)?.as_str().to_string();
                let repo = caps.get(2)?.as_str().to_string();
                Some((user, repo))
            },
            _ => None,
        }
    }

    fn list_remote_tasks(
        &self,
        user: &String,
        repo: &String,
        with_comments: bool,
        with_labels: bool,
        limit: Option<usize>,
        state: RemoteTaskState,
        task_statuses: &Vec<String>
    ) -> Result<Vec<Task>, String> {
        let state = match state {
            RemoteTaskState::Open => Some(IssueState::Opened),
            RemoteTaskState::Closed => Some(IssueState::Closed),
            RemoteTaskState::All => None
        };
        let client = get_client(get_token_from_env().unwrap().as_str());

        let labels = match with_labels {
            true => {
                let mut endpoint = gitlab::api::projects::labels::Labels::builder();
                let endpoint = endpoint.project(user.to_string() + "/" + repo);
                let endpoint = endpoint.build().unwrap();
                let labels: Vec<GitlabLabel> = gitlab::api::paged(endpoint, Pagination::All).query(&client).unwrap();
                let labels = labels.iter()
                    .map(|gl| Label::new(
                        gl.name.to_string(),
                        Some(gl.color.to_string()),
                        Some(gl.description.to_string())
                    ))
                    .collect::<Vec<_>>();
                labels
            },
            false => vec![]
        };

        let mut endpoint = gitlab::api::issues::ProjectIssues::builder();
        let mut endpoint = endpoint.project(user.to_string() + "/" + repo).scope(IssueScope::All);
        endpoint = match state {
            Some(state) => endpoint.state(state),
            None => endpoint
        };
        let endpoint = endpoint.build().unwrap();
        let pagination = match limit {
            Some(limit) => Pagination::Limit(limit),
            None => Pagination::All
        };
        let issues: Vec<Issue> = gitlab::api::paged(endpoint, pagination).query(&client).map_err(|e| e.to_string())?;
        let mut result = vec![];
        for issue in issues {
            let mut props = HashMap::new();
            props.insert(String::from("name"), issue.title);
            props.insert(String::from("description"), issue.description);
            props.insert(String::from("status"), if issue.state == "opened" { task_statuses.get(0).unwrap().clone() } else { task_statuses.get(1).unwrap().clone() });
            props.insert(String::from("created"), parse_datetime_to_seconds(issue.created_at));
            props.insert(String::from("author"), issue.author.username);

            let mut task = Task::from_properties(issue.iid.to_string(), props).unwrap();

            if with_comments {
                let comments = list_issue_comments(&client, &user, &repo, &issue.iid.to_string());
                task.set_comments(comments);
            }

            if with_labels {
                let labels = issue.labels.iter()
                    .filter_map(|l| match labels.iter().find(|label| label.get_name() == l.to_string()) {
                        Some(label) => Some(label.clone()),
                        None => None
                    })
                    .collect::<Vec<_>>();
                task.set_labels(labels);
            }

            result.push(task);
        }

        Ok(result)
    }

    fn get_remote_task(
        &self,
        user: &String,
        repo: &String,
        task_id: &String,
        with_comments: bool,
        with_labels: bool,
        task_statuses: &Vec<String>
    ) -> Option<Task> {
        let client = get_client(get_token_from_env().unwrap().as_str());
        let mut endpoint = gitlab::api::projects::issues::Issue::builder();
        let mut endpoint = endpoint.project(user.to_string() + "/" + repo);
        endpoint = endpoint.issue(task_id.parse().unwrap());
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

                let mut task = Task::from_properties(task_id.to_string(), props).unwrap();

                if with_comments {
                    let comments = list_issue_comments(&client, &user, &repo, task_id);
                    task.set_comments(comments);
                }

                if with_labels {
                    let mut endpoint = gitlab::api::projects::labels::Labels::builder();
                    let endpoint = endpoint.project(user.to_string() + "/" + repo);
                    let endpoint = endpoint.build().unwrap();
                    let labels: Vec<GitlabLabel> = gitlab::api::paged(endpoint, Pagination::All).query(&client).unwrap();
                    let labels = issue.labels.iter()
                        .map(|l| labels.iter().find(|gl| gl.name == l.to_string()).unwrap())
                        .map(|gl| Label::new(gl.name.to_string(), Some(gl.color.to_string()), Some(gl.description.to_string())))
                        .collect();
                    task.set_labels(labels);
                }

                Some(task)
            },
            Err(_) => {
                None
            }
        }
    }

    fn create_remote_task(&self, user: &String, repo: &String, task: &Task) -> Result<String, String> {
        let client = get_client(get_token_from_env().unwrap().as_str());
        let mut endpoint = gitlab::api::projects::issues::CreateIssue::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo);
        endpoint.title(task.get_property("name").unwrap());
        endpoint.description(task.get_property("description").unwrap());
        if let Some(labels) = task.get_labels() {
            prepare_labels(&client, &user, &repo, &labels);
            let labels = labels.iter().map(|l| l.get_name()).collect::<Vec<_>>();
            endpoint.labels(labels);
        }
        let endpoint = endpoint.build().unwrap();
        let issue: Issue = endpoint.query(&client).unwrap();

        Ok(issue.iid.to_string())
    }

    fn create_remote_comment(&self, user: &String, repo: &String, task_id: &String, comment: &Comment) -> Result<String, String> {
        let client = get_client(get_token_from_env().unwrap().as_str());
        let mut endpoint = gitlab::api::projects::issues::notes::CreateIssueNote::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(task_id.parse().unwrap());
        endpoint.body(comment.get_text().clone());
        let endpoint = endpoint.build().unwrap();
        let comment: GitlabComment = endpoint.query(&client).unwrap();

        Ok(comment.id.to_string())
    }

    fn create_remote_label(
        &self,
        user: &String,
        repo: &String,
        task_id: &String,
        label: &Label,
    ) -> Result<(), String> {
        let client = get_client(get_token_from_env().unwrap().as_str());
        let mut endpoint = gitlab::api::projects::issues::Issue::builder();
        let mut endpoint = endpoint.project(user.to_string() + "/" + repo);
        endpoint = endpoint.issue(task_id.parse().unwrap());
        let endpoint = endpoint.build().unwrap();
        match endpoint.query(&client) {
            Ok(issue) => {
                let issue: Issue = issue;
                let label_name = label.get_name();
                if !issue.labels.contains(&label_name) {
                    prepare_labels(&client, &user, &repo, &vec![label.clone()]);
                    let mut endpoint = gitlab::api::projects::issues::EditIssue::builder();
                    let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(task_id.parse().unwrap());
                    endpoint.add_label(label_name);
                    let endpoint = endpoint.build().unwrap();
                    match endpoint.query(&client) {
                        Ok(issue) => {
                            let _: Issue = issue;
                            Ok(())
                        },
                        Err(e) => Err(e.to_string())
                    }
                } else {
                    Ok(())
                }
            },
            Err(e) => Err(e.to_string())
        }
    }

    fn update_remote_task(
        &self,
        user: &String,
        repo: &String,
        task: &Task,
        labels: Option<&Vec<Label>>,
        state: RemoteTaskState
    ) -> Result<(), String> {
        let client = get_client(get_token_from_env().unwrap().as_str());
        let mut endpoint = gitlab::api::projects::issues::EditIssue::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(task.get_id().unwrap().parse().unwrap());
        endpoint.title(task.get_property("name").unwrap());
        endpoint.description(task.get_property("description").unwrap());
        if let Some(labels) = labels {
            prepare_labels(&client, &user, &repo, &labels);
            let labels = labels.iter().map(|l| l.get_name()).collect::<Vec<_>>();
            endpoint.labels(labels);
        }
        endpoint.state_event(if state == RemoteTaskState::Open { IssueStateEvent::Reopen } else { IssueStateEvent::Close });
        let endpoint = endpoint.build().unwrap();
        match endpoint.query(&client) {
            Ok(issue) => {
                let _: Issue = issue;
                Ok(())
            },
            Err(e) => Err(e.to_string())
        }
    }

    fn update_remote_comment(&self, user: &String, repo: &String, task_id: &String, comment_id: &String, text: &String) -> Result<(), String> {
        let client = get_client(get_token_from_env().unwrap().as_str());
        let mut endpoint = gitlab::api::projects::issues::notes::EditIssueNote::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(task_id.parse().unwrap());
        endpoint.note(comment_id.parse().unwrap());
        endpoint.body(text.as_str());
        let endpoint = endpoint.build().unwrap();
        match endpoint.query(&client) {
            Ok(comment) => {
                let _: GitlabComment = comment;
                Ok(())
            },
            Err(e) => Err(e.to_string())
        }
    }

    fn delete_remote_task(&self, user: &String, repo: &String, task_id: &String) -> Result<(), String> {
        let client = get_client(get_token_from_env().unwrap().as_str());
        let mut endpoint = gitlab::api::projects::issues::DeleteIssue::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(task_id.parse().unwrap());
        let endpoint = endpoint.build().unwrap();
        match endpoint.query(&client) {
            Ok(result) => {
                let _: DeleteIssueResult = result;
                Ok(())
            },
            Err(e) => if e.to_string().contains("204") { Ok(()) } else { Err(e.to_string()) }
        }
    }

    fn delete_remote_comment(&self, user: &String, repo: &String, task_id: &String, comment_id: &String) -> Result<(), String> {
        let client = get_client(get_token_from_env().unwrap().as_str());
        let mut endpoint = gitlab::api::projects::issues::notes::DeleteIssueNote::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(task_id.parse().unwrap());
        endpoint.note(comment_id.parse().unwrap());
        let endpoint = endpoint.build().unwrap();
        match endpoint.query(&client) {
            Ok(result) => {
                let _: DeleteIssueNoteResult = result;
                Ok(())
            },
            Err(e) => if e.to_string().contains("204") { Ok(()) } else { Err(e.to_string()) }
        }
    }

    fn delete_remote_label(
        &self,
        user: &String,
        repo: &String,
        task_id: &String,
        label_name: &String,
    ) -> Result<(), String> {
        let client = get_client(get_token_from_env().unwrap().as_str());
        let mut endpoint = gitlab::api::projects::issues::EditIssue::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(task_id.parse().unwrap());
        endpoint.remove_label(label_name);
        let endpoint = endpoint.build().unwrap();
        match endpoint.query(&client) {
            Ok(issue) => {
                let _: Issue = issue;
                Ok(())
            },
            Err(e) => Err(e.to_string())
        }
    }
}

fn list_issue_comments(client: &Gitlab, user: &String, repo: &String, task_id: &String) -> Vec<Comment> {
    let mut endpoint = gitlab::api::projects::issues::notes::IssueNotes::builder();
    let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(task_id.parse().unwrap());
    let endpoint = endpoint.build().unwrap();
    match gitlab::api::paged(endpoint, Pagination::All).query(client) {
        Ok(comments) => {
            let comments: Vec<GitlabComment> = comments;
            let mut result: Vec<Comment> = vec![];
            for comment in comments {
                let comment = Comment::new(comment.id.to_string(), HashMap::from([
                    ("author".to_string(), comment.author.username),
                    ("created".to_string(), parse_datetime_to_seconds(comment.created_at)),
                ]), comment.body);
                result.push(comment);
            }
            result
        },
        Err(e) => {
            eprintln!("{}", e);
            vec![]
        }
    }
}

fn prepare_labels(client: &Gitlab, user: &String, repo: &String, labels: &Vec<Label>) {
    let mut endpoint = gitlab::api::projects::labels::Labels::builder();
    let endpoint = endpoint.project(user.to_string() + "/" + repo);
    let endpoint = endpoint.build().unwrap();
    let existing_labels: Vec<GitlabLabel> = gitlab::api::paged(endpoint, Pagination::All).query(client).unwrap();
    let mut labels_to_create = labels.clone();
    for label in existing_labels {
        if let Some(pos) = labels_to_create.iter().position(|l| l.get_name() == label.name) {
            labels_to_create.remove(pos);
        }
    }
    for l in labels_to_create.iter() {
        let mut endpoint = gitlab::api::projects::labels::CreateLabel::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo);
        endpoint.name(l.get_name());
        endpoint.color("#".to_string() + &color_str_to_rgb_str(&l.get_color()));
        if let Some(description) = l.get_description() {
            endpoint.description(description);
        }
        let endpoint = endpoint.build().unwrap();
        gitlab::api::ignore(endpoint).query(client).unwrap();
    }
}

fn get_client(token: &str) -> Gitlab {
    let base_url = get_base_url();
    let gitlab_domain = match Regex::new("(https://)?(?P<domain>[^/]+)").unwrap().captures(&base_url) {
        Some(caps) if caps.name("domain").is_some() => caps.name("domain").unwrap().as_str().to_string(),
        _ => "gitlab.com".to_string(),
    };
    Gitlab::new(gitlab_domain, token).unwrap()
}

fn get_token_from_env() -> Option<String> {
    std::env::var("GITLAB_TOKEN").or_else(|_| std::env::var("GITLAB_API_TOKEN")).ok()
}

fn get_base_url() -> String {
    let mut result = match gittask::get_config_value("task.gitlab.url") {
        Ok(url) => url,
        _ => match std::env::var("GITLAB_URL") {
            Ok(url) => url,
            _ => "https://gitlab.com".to_string(),
        }
    };

    if !result.starts_with("http") {
        result = "https://".to_string() + result.as_str();
    }

    if !result.ends_with('/') {
        result += "/";
    }

    result
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_remote_url() {
        let connector = GitlabRemoteConnector {};

        gittask::set_config_value("task.gitlab.url", "https://gitlab.com/").unwrap();
        assert!(connector.supports_remote("https://gitlab.com/jhspetersson/fselect").is_some());

        let gitlab_url = get_base_url();
        gittask::set_config_value("task.gitlab.url", "gitlab.kitware.com").unwrap();

        let current_url = get_base_url();
        assert_eq!(current_url, "https://gitlab.kitware.com/".to_string());

        assert!(connector.supports_remote("https://gitlab.kitware.com/jhspetersson/rust-gitlab.git").is_some());

        gittask::set_config_value("task.gitlab.url", &gitlab_url).unwrap();
    }
}