use std::collections::HashMap;
use std::sync::Arc;

use futures_util::TryStreamExt;
use octocrab::models::IssueState::Open;
use octocrab::{params, Octocrab};
use octocrab::models::IssueState;
use regex::Regex;
use tokio::pin;
use tokio::runtime::Runtime;

use gittask::{Comment, Task};

pub fn get_runtime() -> Runtime {
    Runtime::new().unwrap()
}

pub fn list_github_issues(user: String, repo: String, with_comments: bool) -> Vec<Task> {
    Runtime::new().unwrap().block_on(list_github_issues_async(user, repo, with_comments))
}

async fn list_github_issues_async(user: String, repo: String, with_comments: bool) -> Vec<Task> {
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
    while let Some(issue) = stream.try_next().await.unwrap() {
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

pub fn update_github_issue_status(runtime: &Runtime, user: &str, repo: &str, n: u64, state: IssueState) -> bool {
    runtime.block_on(update_issue_status(user, repo, n, state))
}

async fn update_issue_status(user: &str, repo: &str, n: u64, state: IssueState) -> bool {
    let crab = get_octocrab_instance().await;
    crab.issues(user, repo).update(n).state(state).send().await.is_ok()
}

async fn get_octocrab_instance() -> Arc<Octocrab> {
    match std::env::var("GITHUB_TOKEN") {
        Ok(token) => Arc::new(Octocrab::builder().personal_token(token).build().unwrap()),
        _ => octocrab::instance()
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