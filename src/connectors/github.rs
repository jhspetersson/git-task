use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use futures_util::{StreamExt, TryStreamExt};
use graphql_client::{reqwest::post_graphql_blocking as post_graphql, GraphQLQuery};
use octocrab::Octocrab;
use octocrab::models::{CommentId, IssueState};
use octocrab::params::State;
use regex::Regex;
use tokio::pin;
use tokio::runtime::Runtime;

use gittask::{Comment, Label, Task};
use crate::connectors::{RemoteConnector, RemoteTaskState};

pub struct GithubRemoteConnector;

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    Runtime::new().unwrap()
});

impl RemoteConnector for GithubRemoteConnector {
    fn supports_remote(&self, url: &str) -> Option<(String, String)> {
        match Regex::new("((https://)|(git@))github.com[/:](?P<user>[a-zA-Z0-9-]+)/(?P<repo>[a-zA-Z0-9-]+)(\\.git)?").unwrap().captures(url) {
            Some(caps) if caps.len() >= 3 => {
                let user = caps.name("user")?.as_str().to_string();
                let repo = caps.name("repo")?.as_str().to_string();
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
    ) -> Vec<Task> {
        let state = match state {
            RemoteTaskState::Open => State::Open,
            RemoteTaskState::Closed => State::Closed,
            RemoteTaskState::All => State::All,
        };
        RUNTIME.block_on(
            list_issues(
                user,
                repo,
                with_comments,
                with_labels,
                limit,
                state,
                task_statuses
            ))
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
        RUNTIME.block_on(
            get_issue(
                &user, &repo, task_id.parse().unwrap(), with_comments, with_labels, task_statuses
            )
        )
    }

    fn create_remote_task(&self, user: &String, repo: &String, task: &Task) -> Result<String, String> {
        match get_token_from_env() {
            Some(_) => RUNTIME.block_on(create_issue(user, repo, task)),
            None => Err("Could not find GITHUB_TOKEN environment variable.".to_string())
        }
    }

    fn create_remote_comment(&self, user: &String, repo: &String, task_id: &String, comment: &Comment) -> Result<String, String> {
        match get_token_from_env() {
            Some(_) => RUNTIME.block_on(create_comment(user, repo, task_id, comment)),
            None => Err("Could not find GITHUB_TOKEN environment variable.".to_string())
        }
    }

    #[allow(unused_variables)]
    fn create_remote_label(&self, user: &String, repo: &String, task_id: &String, label: &Label) -> Result<(), String> {
        match get_token_from_env() {
            Some(_) => RUNTIME.block_on(add_label(user, repo, task_id.parse().unwrap(), &label.get_name(), Some(&label.get_color()), label.get_description().as_ref())),
            None => Err("Could not find GITHUB_TOKEN environment variable.".to_string())
        }
    }

    fn update_remote_task(&self, user: &String, repo: &String, task_id: &String, name: &String, text: &String, state: RemoteTaskState) -> Result<(), String> {
        match get_token_from_env() {
            Some(_) => {
                let state = match state {
                    RemoteTaskState::Closed => IssueState::Closed,
                    _ => IssueState::Open,
                };
                RUNTIME.block_on(update_issue(user, repo, task_id.parse().unwrap(), name, text, state))
            },
            None => Err("Could not find GITHUB_TOKEN environment variable.".to_string())
        }
    }

    fn update_remote_comment(&self, user: &String, repo: &String, _task_id: &String, comment_id: &String, text: &String) -> Result<(), String> {
        match get_token_from_env() {
            Some(_) => RUNTIME.block_on(update_comment(user, repo, comment_id.parse().unwrap(), text)),
            None => Err("Could not find GITHUB_TOKEN environment variable.".to_string())
        }
    }

    fn delete_remote_task(&self, user: &String, repo: &String, task_id: &String) -> Result<(), String> {
        match get_token_from_env() {
            Some(token) => {
                let issue_id = RUNTIME.block_on(get_issue_id(user, repo, task_id.parse().unwrap()));
                if issue_id.is_err() {
                    return Err("Could not match task ID with GitHub internal issue ID.".to_string());
                }
                let issue_id = issue_id?;
                let variables = delete_issue::Variables {
                    issue_id,
                };

                let client = reqwest::blocking::Client::builder()
                    .user_agent("git-task/".to_owned() + env!("CARGO_PKG_VERSION"))
                    .default_headers(
                        std::iter::once((
                            reqwest::header::AUTHORIZATION,
                            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
                        )).collect(),
                    )
                    .build().unwrap();

                let response_body = post_graphql::<DeleteIssue, _>(&client, "https://api.github.com/graphql", variables).expect("Failed to make GraphQL request");

                if let Some(errors) = response_body.errors {
                    if !errors.is_empty() {
                        return Err(errors.first().unwrap().message.clone());
                    }
                }

                let response_data: Option<delete_issue::ResponseData> = response_body.data;

                if response_data.is_none() {
                    return Err("Missing response data.".to_string());
                }

                match response_data {
                    Some(_) => Ok(()),
                    None => Err("Response data not found".to_string())
                }
            },
            None => Err("Could not find GITHUB_TOKEN environment variable.".to_string()),
        }
    }

    fn delete_remote_comment(&self, user: &String, repo: &String, _task_id: &String, comment_id: &String) -> Result<(), String> {
        match get_token_from_env() {
            Some(_) => RUNTIME.block_on(delete_comment(user, repo, comment_id.parse().unwrap())),
            None => Err("Could not find GITHUB_TOKEN environment variable.".to_string())
        }
    }

    fn delete_remote_label(&self, user: &String, repo: &String, task_id: &String, name: &String) -> Result<(), String> {
        match get_token_from_env() {
            Some(_) => RUNTIME.block_on(delete_label(user, repo, task_id.parse().unwrap(), name)),
            None => Err("Could not find GITHUB_TOKEN environment variable.".to_string())
        }
    }
}

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "resources/github/schema.graphql",
    query_path = "resources/github/delete_issue.graphql",
    response_derives = "Debug"
)]
struct DeleteIssue;

async fn list_issues(
    user: &String,
    repo: &String,
    with_comments: bool,
    with_labels: bool,
    limit: Option<usize>,
    state: State,
    task_statuses: &Vec<String>
) -> Vec<Task> {
    let mut result = vec![];
    let crab = get_octocrab_instance().await;
    let stream = crab.issues(user, repo)
        .list()
        .state(state)
        .per_page(100)
        .send()
        .await.unwrap()
        .into_stream(&crab);
    pin!(stream);
    let mut count = 0;
    while let Some(issue) = stream.try_next().await.unwrap() {
        if limit.is_some() && count >= limit.unwrap() {
            break;
        }
        count += 1;
        let mut props = HashMap::new();
        props.insert(String::from("name"), issue.title);
        props.insert(String::from("status"), if issue.state == IssueState::Open { task_statuses.get(0).unwrap().clone() } else { task_statuses.get(1).unwrap().clone() } );
        props.insert(String::from("description"), issue.body.unwrap_or(String::new()));
        props.insert(String::from("created"), issue.created_at.timestamp().to_string());
        props.insert(String::from("author"), issue.user.login);

        let mut task = Task::from_properties(issue.number.to_string(), props).unwrap();

        if with_comments {
            let task_comments = list_issue_comments(&user, &repo, issue.number).await;
            task.set_comments(task_comments);
        }

        if with_labels {
            if !issue.labels.is_empty() {
                let labels = issue.labels.iter()
                    .map(|l| Label::new(
                        l.name.clone(),
                        Some(l.color.clone()),
                        l.description.clone()
                    ))
                    .collect();
                task.set_labels(labels);
            }
        }

        result.push(task);
    }

    result
}

async fn list_issue_comments(user: &String, repo: &String, n: u64) -> Vec<Comment> {
    let mut result = vec![];
    let crab = get_octocrab_instance().await;
    let stream = crab.issues(user, repo)
        .list_comments(n)
        .per_page(100)
        .send()
        .await.unwrap()
        .into_stream(&crab);
    pin!(stream);
    while let Some(comment) = stream.try_next().await.unwrap() {
        let comment = Comment::new(comment.id.to_string(), HashMap::from([
            ("author".to_string(), comment.user.login),
            ("created".to_string(), comment.created_at.timestamp().to_string()),
        ]), comment.body.unwrap());
        result.push(comment);
    }

    result
}

async fn get_issue(
    user: &String,
    repo: &String,
    n: u64,
    with_comments: bool,
    with_labels: bool,
    task_statuses: &Vec<String>
) -> Option<Task> {
    let crab = get_octocrab_instance().await;
    let issue = crab.issues(user, repo).get(n).await;
    match issue {
        Ok(issue) => {
            let mut props = HashMap::new();
            props.insert(String::from("name"), issue.title);
            props.insert(String::from("status"), if issue.state == IssueState::Open { task_statuses.get(0).unwrap().clone() } else { task_statuses.get(1).unwrap().clone() } );
            props.insert(String::from("description"), issue.body.unwrap_or(String::new()));
            props.insert(String::from("created"), issue.created_at.timestamp().to_string());
            props.insert(String::from("author"), issue.user.login);

            let mut task = Task::from_properties(n.to_string(), props).unwrap();

            if with_comments {
                let task_comments = list_issue_comments(user, repo, issue.number).await;
                task.set_comments(task_comments);
            }

            if with_labels {
                let labels = issue.labels.iter()
                    .map(|l| Label::new(
                        l.name.to_string(),
                        Some(l.color.to_string()),
                        l.description.clone()
                    )).collect();
                task.set_labels(labels);
            }

            Some(task)
        },
        _ => None
    }
}

async fn create_issue(user: &String, repo: &String, task: &Task) -> Result<String, String> {
    let crab = get_octocrab_instance().await;
    let crab_issues = crab.issues(user, repo);
    let mut create_builder = crab_issues.create(task.get_property("name").unwrap());
    if let Some(description) = task.get_property("description") {
        create_builder = create_builder.body(description);
    }
    if let Some(labels) = task.get_labels() {
        let labels = labels.iter().map(|l| l.get_name()).collect::<Vec<_>>();
        create_builder = create_builder.labels(labels);
    }
    match create_builder.send().await {
        Ok(issue) => Ok(issue.number.to_string()),
        Err(e) => Err(e.to_string())
    }
}

async fn create_comment(user: &String, repo: &String, task_id: &String, comment: &Comment) -> Result<String, String> {
    let crab = get_octocrab_instance().await;
    match crab.issues(user, repo).create_comment(task_id.parse().unwrap(), comment.get_text()).await {
        Ok(comment) => Ok(comment.id.to_string()),
        Err(e) => Err(e.to_string())
    }
}

async fn add_label(
    user: &String,
    repo: &String,
    n: u64,
    label_name: &String,
    label_color: Option<&String>,
    label_description: Option<&String>,
) -> Result<(), String> {
    let crab = get_octocrab_instance().await;
    let existing_labels_stream = crab
        .issues(user, repo)
        .list_labels_for_repo()
        .per_page(100)
        .send()
        .await.unwrap()
        .into_stream(&crab);
    pin!(existing_labels_stream);
    let mut existing_label = None;
    while let Some(label) = existing_labels_stream.next().await {
        if label.as_ref().unwrap().name == label_name.to_string() {
            existing_label = Some(Label::new(
                label.as_ref().unwrap().name.clone(),
                Some(label.as_ref().unwrap().color.clone()),
                label.as_ref().unwrap().description.clone(),
            ));
            break;
        }
    }

    if existing_label.is_none() {
        let _ = crab
            .issues(user, repo)
            .create_label(
                label_name,
                label_color.unwrap_or(&"000000".to_string()).to_string(),
                label_description.unwrap_or(&"".to_string()).to_string(),
            )
            .await;
    }

    let add_label_body = vec![label_name.to_string()];
    crab
        .issues(user, repo)
        .add_labels(n, &add_label_body)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

async fn update_issue(user: &String, repo: &String, n: u64, title: &String, body: &String, state: IssueState) -> Result<(), String> {
    let crab = get_octocrab_instance().await;
    match crab.issues(user, repo).update(n).title(title).body(body).state(state).send().await {
        Ok(_) => Ok(()),
        Err(e) => Err(e.to_string())
    }
}

async fn update_comment(user: &String, repo: &String, n: u64, text: &String) -> Result<(), String> {
    let crab = get_octocrab_instance().await;
    match crab.issues(user, repo).update_comment(CommentId(n), text).await {
        Ok(_) => Ok(()),
        Err(e) => Err(e.to_string())
    }
}

async fn delete_comment(user: &String, repo: &String, n: u64) -> Result<(), String> {
    let crab = get_octocrab_instance().await;
    match crab.issues(user, repo).delete_comment(CommentId(n)).await {
        Ok(_) => Ok(()),
        Err(e) => Err(e.to_string())
    }
}

pub async fn delete_label(
    user: &String,
    repo: &String,
    n: u64,
    label_name: &str,
) -> Result<(), String> {
    let crab = get_octocrab_instance().await;

    crab
        .issues(user, repo)
        .remove_label(n, label_name)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

async fn get_issue_id(user: &String, repo: &String, n: u64) -> Result<String, String> {
    let crab = get_octocrab_instance().await;
    let issue = crab.issues(user, repo).get(n).await;
    match issue {
        Ok(issue) => Ok(issue.node_id),
        Err(e) => Err(e.to_string()),
    }
}

async fn get_octocrab_instance() -> Arc<Octocrab> {
    match get_token_from_env() {
        Some(token) => Arc::new(Octocrab::builder().personal_token(token).build().unwrap()),
        None => octocrab::instance()
    }
}

fn get_token_from_env() -> Option<String> {
    std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GITHUB_API_TOKEN")).ok()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_remote_url() {
        let connector = GithubRemoteConnector {};

        assert!(connector.supports_remote("git@github.com:VIK-777/java-telegram-meetup-bot.git").is_some());
        assert!(connector.supports_remote("https://github.com/jhspetersson/fselect.git").is_some());
    }
}