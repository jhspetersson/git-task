use std::collections::HashMap;
use std::sync::Arc;

use futures_util::TryStreamExt;
use graphql_client::{reqwest::post_graphql_blocking as post_graphql, GraphQLQuery};
use octocrab::models::IssueState::Open;
use octocrab::{params, Octocrab};
use octocrab::models::IssueState;
use regex::Regex;
use tokio::pin;
use tokio::runtime::Runtime;

use gittask::{Comment, Task};

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "resources/github/schema.graphql",
    query_path = "resources/github/delete_issue.graphql",
    response_derives = "Debug"
)]
struct DeleteIssue;

pub fn get_runtime() -> Runtime {
    Runtime::new().unwrap()
}

pub fn list_github_issues(user: String, repo: String, with_comments: bool, limit: Option<usize>) -> Vec<Task> {
    Runtime::new().unwrap().block_on(list_github_issues_async(user, repo, with_comments, limit))
}

async fn list_github_issues_async(user: String, repo: String, with_comments: bool, limit: Option<usize>) -> Vec<Task> {
    let mut result = vec![];
    let crab = get_octocrab_instance().await;
    let stream = crab.issues(&user, &repo)
        .list()
        .state(params::State::All)
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
        props.insert(String::from("status"), if issue.state == Open { String::from("OPEN") } else { String::from("CLOSED") } );
        props.insert(String::from("description"), issue.body.unwrap_or(String::new()));
        props.insert(String::from("created"), issue.created_at.timestamp().to_string());
        props.insert(String::from("author"), issue.user.login);

        let mut task = Task::from_properties(issue.number.to_string(), props).unwrap();

        if with_comments {
            let task_comments = list_github_issue_comments(&user, &repo, issue.number).await;
            task.set_comments(task_comments);
        }

        result.push(task);
    }

    result
}

async fn list_github_issue_comments(user: &String, repo: &String, n: u64) -> Vec<Comment> {
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

pub fn get_github_issue(runtime: &Runtime, user: &String, repo: &String, n: u64, with_comments: bool) -> Option<Task> {
    runtime.block_on(get_issue(&user, &repo, n, with_comments))
}

async fn get_issue(user: &String, repo: &String, n: u64, with_comments: bool) -> Option<Task> {
    let crab = get_octocrab_instance().await;
    let issue = crab.issues(user, repo).get(n).await;
    match issue {
        Ok(issue) => {
            let mut props = HashMap::new();
            props.insert(String::from("name"), issue.title);
            props.insert(String::from("status"), if issue.state == Open { String::from("OPEN") } else { String::from("CLOSED") } );
            props.insert(String::from("description"), issue.body.unwrap_or(String::new()));
            props.insert(String::from("created"), issue.created_at.timestamp().to_string());
            props.insert(String::from("author"), issue.user.login);

            let mut task = Task::from_properties(n.to_string(), props).unwrap();

            if with_comments {
                let task_comments = list_github_issue_comments(user, repo, issue.number).await;
                task.set_comments(task_comments);
            }

            Some(task)
        },
        _ => None
    }
}

pub fn create_github_issue(runtime: &Runtime, user: &String, repo: &String, task: &Task) -> Result<String, String> {
    runtime.block_on(create_issue(user, repo, task))
}

async fn create_issue(user: &String, repo: &String, task: &Task) -> Result<String, String> {
    let crab = get_octocrab_instance().await;
    let crab_issues = crab.issues(user, repo);
    let mut create_builder = crab_issues.create(task.get_property("name").unwrap());
    if let Some(description) = task.get_property("description") {
        create_builder = create_builder.body(description);
    }
    match create_builder.send().await {
        Ok(issue) => Ok(issue.number.to_string()),
        Err(e) => Err(e.to_string())
    }
}

pub fn create_github_comment(runtime: &Runtime, user: &String, repo: &String, task_id: &String, comment: &Comment) -> Result<String, String> {
    runtime.block_on(create_comment(user, repo, task_id, comment))
}

async fn create_comment(user: &String, repo: &String, task_id: &String, comment: &Comment) -> Result<String, String> {
    let crab = get_octocrab_instance().await;
    match crab.issues(user, repo).create_comment(task_id.parse().unwrap(), comment.get_text()).await {
        Ok(comment) => Ok(comment.id.to_string()),
        Err(e) => Err(e.to_string())
    }
}

pub fn update_github_issue_status(runtime: &Runtime, user: &str, repo: &str, n: u64, state: IssueState) -> bool {
    runtime.block_on(update_issue_status(user, repo, n, state))
}

async fn update_issue_status(user: &str, repo: &str, n: u64, state: IssueState) -> bool {
    let crab = get_octocrab_instance().await;
    crab.issues(user, repo).update(n).state(state).send().await.is_ok()
}

pub fn delete_github_issue(user: &String, repo: &String, n: u64) -> Result<(), String> {
    match get_token_from_env() {
        Some(token) => {
            let issue_id = get_runtime().block_on(get_issue_id(user, repo, n));
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

pub fn list_github_origins(remotes: Vec<String>) -> Result<Vec<(String, String)>, String> {
    let user_repo = remotes.into_iter().map(|ref remote| {
        match Regex::new("https://github.com/([a-z0-9-]+)/([a-z0-9-]+)\\.?").unwrap().captures(&remote.to_lowercase()) {
            Some(caps) if caps.len() == 3 => {
                let user = caps.get(1).unwrap().as_str().to_string();
                let repo = caps.get(2).unwrap().as_str().to_string();
                Some((user, repo))
            },
            _ => None,
        }
    }).filter_map(|s| s).collect::<Vec<(String, String)>>();

    Ok(user_repo)
}

fn get_token_from_env() -> Option<String> {
    std::env::var("GITHUB_TOKEN").or(std::env::var("GITHUB_API_TOKEN")).ok()
}