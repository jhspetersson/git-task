use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

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
    description: Option<String>,
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
    #[serde(default)]
    system: bool,
}

#[derive(Serialize, Deserialize)]
struct GitlabLabel {
    name: String,
    color: String,
    description: Option<String>,
}

#[derive(Deserialize)]
struct DeleteIssueResult {}

#[derive(Deserialize)]
struct DeleteIssueNoteResult {}

impl RemoteConnector for GitlabRemoteConnector {
    fn type_name(&self) -> &str {
        "gitlab"
    }

    fn get_config_options(&self) -> Option<Vec<String>> {
        Some(vec!["task.gitlab.url".to_string()])
    }

    fn supports_remote(&self, url: &str) -> Option<(String, String)> {
        parse_remote_url(&get_base_url(), url)
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
            RemoteTaskState::Open(_, _) => Some(IssueState::Opened),
            RemoteTaskState::Closed(_, _) => Some(IssueState::Closed),
            RemoteTaskState::All => None,
        };
        let client = get_client(get_token()?.as_str())?;

        let labels = match with_labels {
            true => {
                let mut endpoint = gitlab::api::projects::labels::Labels::builder();
                let endpoint = endpoint.project(user.to_string() + "/" + repo);
                let endpoint = endpoint.build().unwrap();
                let labels: Vec<GitlabLabel> = gitlab::api::paged(endpoint, Pagination::All).query(&client).map_err(|e| e.to_string())?;
                let labels = labels.iter()
                    .map(|gl| Label::new(
                        gl.name.to_string(),
                        Some(gl.color.to_string()),
                        gl.description.clone()
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
            props.insert(String::from("description"), issue.description.unwrap_or_default());
            props.insert(String::from("status"), if issue.state == "opened" { task_statuses.first().unwrap().clone() } else { task_statuses.last().unwrap().clone() });
            props.insert(String::from("created"), parse_datetime_to_seconds(issue.created_at));
            props.insert(String::from("author"), issue.author.username);

            let mut task = Task::from_properties(issue.iid.to_string(), props).unwrap();

            if with_comments {
                let comments = list_issue_comments(&client, &user, &repo, issue.iid)?;
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
    ) -> Result<Option<Task>, String> {
        let iid = parse_task_id(task_id)?;
        let client = get_client(get_token()?.as_str())?;
        let mut endpoint = gitlab::api::projects::issues::Issue::builder();
        let mut endpoint = endpoint.project(user.to_string() + "/" + repo);
        endpoint = endpoint.issue(iid);
        let endpoint = endpoint.build().unwrap();
        match endpoint.query(&client) {
            Ok(issue) => {
                let issue: Issue = issue;
                let mut props = HashMap::new();
                props.insert(String::from("name"), issue.title);
                props.insert(String::from("description"), issue.description.unwrap_or_default());
                props.insert(String::from("status"), if issue.state == "opened" { task_statuses.first().unwrap().clone() } else { task_statuses.last().unwrap().clone() });
                props.insert(String::from("created"), parse_datetime_to_seconds(issue.created_at));
                props.insert(String::from("author"), issue.author.username);

                let mut task = Task::from_properties(task_id.to_string(), props).unwrap();

                if with_comments {
                    let comments = list_issue_comments(&client, &user, &repo, iid)?;
                    task.set_comments(comments);
                }

                if with_labels {
                    let mut endpoint = gitlab::api::projects::labels::Labels::builder();
                    let endpoint = endpoint.project(user.to_string() + "/" + repo);
                    let endpoint = endpoint.build().unwrap();
                    let labels: Vec<GitlabLabel> = gitlab::api::paged(endpoint, Pagination::All).query(&client).map_err(|e| e.to_string())?;
                    let labels = issue.labels.iter()
                        .filter_map(|l| labels.iter().find(|gl| gl.name == l.to_string()))
                        .map(|gl| Label::new(gl.name.to_string(), Some(gl.color.to_string()), gl.description.clone()))
                        .collect();
                    task.set_labels(labels);
                }

                Ok(Some(task))
            },
            Err(e) if is_not_found(&e) => Ok(None),
            Err(e) => Err(e.to_string())
        }
    }

    fn create_remote_task(&self, user: &String, repo: &String, task: &Task) -> Result<String, String> {
        let client = get_client(get_token()?.as_str())?;
        let mut endpoint = gitlab::api::projects::issues::CreateIssue::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo);
        endpoint.title(task.get_property("name").map(String::as_str).unwrap_or(""));
        endpoint.description(task.get_property("description").map(String::as_str).unwrap_or(""));
        if let Some(labels) = task.get_labels() {
            prepare_labels(&client, &user, &repo, &labels);
            let labels = labels.iter().map(|l| l.get_name()).collect::<Vec<_>>();
            endpoint.labels(labels);
        }
        let endpoint = endpoint.build().unwrap();
        let issue: Issue = endpoint.query(&client).map_err(|e| e.to_string())?;

        Ok(issue.iid.to_string())
    }

    fn create_remote_comment(&self, user: &String, repo: &String, task_id: &String, comment: &Comment) -> Result<String, String> {
        let iid = parse_task_id(task_id)?;
        let client = get_client(get_token()?.as_str())?;
        let mut endpoint = gitlab::api::projects::issues::notes::CreateIssueNote::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(iid);
        endpoint.body(comment.get_text().clone());
        let endpoint = endpoint.build().unwrap();
        let comment: GitlabComment = endpoint.query(&client).map_err(|e| e.to_string())?;

        Ok(comment.id.to_string())
    }

    fn create_remote_label(
        &self,
        user: &String,
        repo: &String,
        task_id: &String,
        label: &Label,
    ) -> Result<(), String> {
        let iid = parse_task_id(task_id)?;
        let client = get_client(get_token()?.as_str())?;
        let mut endpoint = gitlab::api::projects::issues::Issue::builder();
        let mut endpoint = endpoint.project(user.to_string() + "/" + repo);
        endpoint = endpoint.issue(iid);
        let endpoint = endpoint.build().unwrap();
        match endpoint.query(&client) {
            Ok(issue) => {
                let issue: Issue = issue;
                let label_name = label.get_name();
                if !issue.labels.contains(&label_name) {
                    prepare_labels(&client, &user, &repo, &vec![label.clone()]);
                    let mut endpoint = gitlab::api::projects::issues::EditIssue::builder();
                    let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(iid);
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
        let task_id = task.get_id().ok_or_else(|| "Task has no ID".to_string())?;
        let iid = parse_task_id(&task_id)?;
        let client = get_client(get_token()?.as_str())?;
        let mut endpoint = gitlab::api::projects::issues::EditIssue::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(iid);
        endpoint.title(task.get_property("name").map(String::as_str).unwrap_or(""));
        endpoint.description(task.get_property("description").map(String::as_str).unwrap_or(""));
        if let Some(labels) = labels {
            prepare_labels(&client, &user, &repo, &labels);
            let labels = labels.iter().map(|l| l.get_name()).collect::<Vec<_>>();
            endpoint.labels(labels);
        }
        endpoint.state_event(if let RemoteTaskState::Open(_, _) = state { IssueStateEvent::Reopen } else { IssueStateEvent::Close });
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
        let iid = parse_task_id(task_id)?;
        let note_id = parse_comment_id(comment_id)?;
        let client = get_client(get_token()?.as_str())?;
        let mut endpoint = gitlab::api::projects::issues::notes::EditIssueNote::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(iid);
        endpoint.note(note_id);
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
        let iid = parse_task_id(task_id)?;
        let client = get_client(get_token()?.as_str())?;
        let mut endpoint = gitlab::api::projects::issues::DeleteIssue::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(iid);
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
        let iid = parse_task_id(task_id)?;
        let note_id = parse_comment_id(comment_id)?;
        let client = get_client(get_token()?.as_str())?;
        let mut endpoint = gitlab::api::projects::issues::notes::DeleteIssueNote::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(iid);
        endpoint.note(note_id);
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
        let iid = parse_task_id(task_id)?;
        let client = get_client(get_token()?.as_str())?;
        let mut endpoint = gitlab::api::projects::issues::EditIssue::builder();
        let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(iid);
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

fn list_issue_comments(client: &Gitlab, user: &String, repo: &String, iid: u64) -> Result<Vec<Comment>, String> {
    let mut endpoint = gitlab::api::projects::issues::notes::IssueNotes::builder();
    let endpoint = endpoint.project(user.to_string() + "/" + repo).issue(iid);
    let endpoint = endpoint.build().unwrap();
    match gitlab::api::paged(endpoint, Pagination::All).query(client) {
        Ok(comments) => {
            let comments: Vec<GitlabComment> = comments;
            let mut result: Vec<Comment> = vec![];
            for comment in comments {
                if comment.system {
                    continue;
                }
                let comment = Comment::new(comment.id.to_string(), HashMap::from([
                    ("author".to_string(), comment.author.username),
                    ("created".to_string(), parse_datetime_to_seconds(comment.created_at)),
                ]), comment.body);
                result.push(comment);
            }
            Ok(result)
        },
        Err(e) => Err(format!("Could not fetch notes for issue {iid}: {e}")),
    }
}

fn is_not_found<E: std::error::Error + Send + Sync + 'static>(error: &gitlab::api::ApiError<E>) -> bool {
    use gitlab::api::ApiError::*;
    match error {
        GitlabService { status, .. } | GitlabWithStatus { status, .. } | GitlabObjectWithStatus { status, .. } | GitlabUnrecognizedWithStatus { status, .. } => status.as_u16() == 404,
        _ => false,
    }
}

fn prepare_labels(client: &Gitlab, user: &String, repo: &String, labels: &Vec<Label>) {
    let mut endpoint = gitlab::api::projects::labels::Labels::builder();
    let endpoint = endpoint.project(user.to_string() + "/" + repo);
    let endpoint = endpoint.build().unwrap();
    let existing_labels: Vec<GitlabLabel> = match gitlab::api::paged(endpoint, Pagination::All).query(client) {
        Ok(labels) => labels,
        Err(e) => {
            eprintln!("ERROR fetching labels: {e}");
            return;
        }
    };
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
        if let Err(e) = gitlab::api::ignore(endpoint).query(client) {
            eprintln!("ERROR creating label {}: {e}", l.get_name());
        }
    }
}

static CLIENTS: OnceLock<Mutex<HashMap<(bool, String, String), Gitlab>>> = OnceLock::new();

fn get_client(token: &str) -> Result<Gitlab, String> {
    let base_url = get_base_url();
    let (insecure, rest) = match base_url.strip_prefix("http://") {
        Some(rest) => (true, rest),
        None => (false, base_url.strip_prefix("https://").unwrap_or(&base_url)),
    };
    let host = match rest.split('/').next().unwrap_or("") {
        "" => "gitlab.com".to_string(),
        host => host.to_string(),
    };
    let key = (insecure, host.clone(), token.to_string());
    let clients = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(client) = clients.lock().unwrap().get(&key) {
        return Ok(client.clone());
    }
    let client = if insecure { Gitlab::new_insecure(&host, token) } else { Gitlab::new(&host, token) };
    let client = client.map_err(|e| format!("Could not connect to GitLab at {host}: {e}"))?;
    clients.lock().unwrap().insert(key, client.clone());
    Ok(client)
}

fn parse_task_id(task_id: &str) -> Result<u64, String> {
    task_id.parse().map_err(|_| format!("Invalid task ID: {task_id}"))
}

fn parse_comment_id(comment_id: &str) -> Result<u64, String> {
    comment_id.parse().map_err(|_| format!("Invalid comment ID: {comment_id}"))
}

fn get_token_from_env() -> Option<String> {
    std::env::var("GITLAB_TOKEN").or_else(|_| std::env::var("GITLAB_API_TOKEN")).ok()
}

fn get_token() -> Result<String, String> {
    get_token_from_env().ok_or_else(|| "Could not find GITLAB_TOKEN environment variable.".to_string())
}

/// Extracts (user, repo) from a git remote that points at the configured GitLab host.
///
/// Accepts the clone URL forms GitLab offers: `https://host/user/repo.git`,
/// `http://host/user/repo`, `ssh://git@host[:port]/user/repo.git` and the
/// scp-style `git@host:user/repo.git`. When the base URL carries a path prefix
/// (a GitLab served under a sub-path), HTTP remotes must include it; SSH remotes
/// never do.
fn parse_remote_url(base_url: &str, url: &str) -> Option<(String, String)> {
    let without_scheme = base_url
        .strip_prefix("https://")
        .or_else(|| base_url.strip_prefix("http://"))
        .unwrap_or(base_url);
    let (host, path) = match without_scheme.split_once('/') {
        Some((host, path)) => (host, path.trim_matches('/')),
        None => (without_scheme, ""),
    };
    if host.is_empty() {
        return None;
    }
    let host = regex::escape(host);
    let http_prefix = if path.is_empty() { String::new() } else { regex::escape(path) + "/" };
    let pattern = format!(
        "(?i)^(?:https?://(?:[^@/]+@)?{host}/{http_prefix}|ssh://(?:[^@/]+@)?{host}(?::\\d+)?/|[^@/:]+@{host}:)(?P<user>[a-zA-Z0-9._-]+)/(?P<repo>[a-zA-Z0-9._-]+?)(?:\\.git)?/?$"
    );
    let caps = Regex::new(&pattern).ok()?.captures(url)?;
    Some((caps.name("user")?.as_str().to_string(), caps.name("repo")?.as_str().to_string()))
}

fn get_base_url() -> String {
    let mut result = match gittask::get_config_value("task.gitlab.url") {
        Ok(url) if !url.is_empty() => url,
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
    fn test_list_issue_comments_reports_a_failed_fetch() {
        let (url, token) = match (std::env::var("GIT_TASK_GITLAB_URL"), std::env::var("GIT_TASK_GITLAB_TOKEN")) {
            (Ok(url), Ok(token)) if !url.trim().is_empty() && !token.trim().is_empty() => (url, token),
            _ => {
                eprintln!("skipped: set GIT_TASK_GITLAB_URL and GIT_TASK_GITLAB_TOKEN to run this test");
                return;
            }
        };
        let _repo = gittask::testing::TestRepo::new();
        gittask::set_config_value("task.gitlab.url", url.trim()).unwrap();
        let client = get_client(token.trim()).expect("client");

        let result = list_issue_comments(&client, &"no-such-user".to_string(), &"no-such-repo".to_string(), 999999);

        assert!(result.is_err(), "a failed note fetch must not look like an issue without notes, got {} notes", result.as_ref().map(|c| c.len()).unwrap_or(0));
    }

    fn stub_gitlab_server() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stub server");
        let addr = listener.local_addr().unwrap();
        let user_requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = user_requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream { Ok(s) => s, Err(_) => continue };
                let mut head = Vec::new();
                let mut buf = [0u8; 1024];
                let header_end = [13u8, 10, 13, 10];
                while !head.windows(4).any(|w| w == header_end) {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let request_line = String::from_utf8_lossy(&head).lines().next().unwrap_or_default().to_string();
                if request_line.contains("/api/v4/user") {
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                let crlf = [13u8, 10];
                let response = [b"HTTP/1.1 200 OK".as_slice(), &crlf, b"Content-Length: 2", &crlf, b"Connection: close", &crlf, &crlf, b"{}"].concat();
                let _ = stream.write_all(&response);
            }
        });
        (format!("http://{addr}"), user_requests)
    }

    #[test]
    fn test_get_client_is_reused_per_host_and_token() {
        let _repo = gittask::testing::TestRepo::new();
        let (url, user_requests) = stub_gitlab_server();
        gittask::set_config_value("task.gitlab.url", &url).unwrap();

        get_client("reuse-token").expect("first client");
        get_client("reuse-token").expect("second client");
        get_client("reuse-token").expect("third client");
        assert_eq!(user_requests.load(std::sync::atomic::Ordering::SeqCst), 1, "the same host and token must authenticate only once");

        get_client("another-token").expect("client for another token");
        assert_eq!(user_requests.load(std::sync::atomic::Ordering::SeqCst), 2, "a different token must get its own client");
    }

    #[test]
    fn test_remote_url() {
        let _repo = gittask::testing::TestRepo::new();
        let connector = GitlabRemoteConnector {};

        gittask::set_config_value("task.gitlab.url", "https://gitlab.com/").unwrap();
        assert_eq!(connector.supports_remote("https://gitlab.com/jhspetersson/fselect"), Some(("jhspetersson".to_string(), "fselect".to_string())));
        assert_eq!(connector.supports_remote("https://gitlab.com/jhspetersson/fselect.git"), Some(("jhspetersson".to_string(), "fselect".to_string())));

        gittask::set_config_value("task.gitlab.url", "gitlab.kitware.com").unwrap();

        let current_url = get_base_url();
        assert_eq!(current_url, "https://gitlab.kitware.com/".to_string());

        assert_eq!(connector.supports_remote("https://gitlab.kitware.com/jhspetersson/rust-gitlab.git"), Some(("jhspetersson".to_string(), "rust-gitlab".to_string())));
    }

    #[test]
    fn test_remote_url_accepts_every_clone_form_gitlab_offers() {
        let expected = Some(("jhspetersson".to_string(), "fselect".to_string()));
        for url in [
            "https://gitlab.com/jhspetersson/fselect.git",
            "https://gitlab.com/jhspetersson/fselect",
            "https://oauth2:token@gitlab.com/jhspetersson/fselect.git",
            "http://gitlab.com/jhspetersson/fselect.git",
            "git@gitlab.com:jhspetersson/fselect.git",
            "git@gitlab.com:jhspetersson/fselect",
            "ssh://git@gitlab.com/jhspetersson/fselect.git",
            "ssh://git@gitlab.com:2222/jhspetersson/fselect.git",
            "GIT@GITLAB.COM:jhspetersson/fselect.git",
        ] {
            assert_eq!(parse_remote_url("https://gitlab.com/", url), expected, "{url} must match the gitlab.com base");
        }
    }

    #[test]
    fn test_remote_url_rejects_other_hosts() {
        for url in [
            "git@github.com:jhspetersson/fselect.git",
            "https://github.com/jhspetersson/fselect.git",
            "https://gitlabXcom/jhspetersson/fselect.git",
            "https://gitlab.com.evil.org/jhspetersson/fselect.git",
            "https://gitlab.com/jhspetersson/fselect/extra",
        ] {
            assert_eq!(parse_remote_url("https://gitlab.com/", url), None, "{url} must not match the gitlab.com base");
        }
    }

    #[test]
    fn test_remote_url_with_a_self_hosted_base_and_path_prefix() {
        let expected = Some(("team".to_string(), "tool".to_string()));
        assert_eq!(parse_remote_url("https://git.example.com/gitlab/", "https://git.example.com/gitlab/team/tool.git"), expected);
        assert_eq!(parse_remote_url("https://git.example.com/gitlab/", "git@git.example.com:team/tool.git"), expected, "SSH remotes carry no path prefix");
        assert_eq!(parse_remote_url("https://git.example.com/gitlab/", "https://git.example.com/team/tool.git"), None, "an HTTP remote must include the prefix");
        assert_eq!(parse_remote_url("http://git.example.com/", "git@git.example.com:team/tool.git"), expected, "an http:// base still matches SSH remotes");
    }

    #[test]
    fn test_parse_ids_reject_non_numeric() {
        assert_eq!(parse_task_id("42"), Ok(42));
        assert_eq!(parse_task_id("1x"), Err("Invalid task ID: 1x".to_string()));
        assert_eq!(parse_task_id("ABC-7"), Err("Invalid task ID: ABC-7".to_string()));
        assert_eq!(parse_comment_id("7"), Ok(7));
        assert_eq!(parse_comment_id("abc"), Err("Invalid comment ID: abc".to_string()));
    }

    #[test]
    fn test_issue_with_null_description_deserializes() {
        let json = r#"{"iid":1,"title":"t","description":null,"author":{"username":"u"},"created_at":"2024-01-01T00:00:00Z","state":"opened","labels":[]}"#;
        let issue = serde_json::from_str::<Issue>(json);
        assert!(issue.is_ok(), "GitLab issues with null description must deserialize: {:?}", issue.err());
    }

    #[test]
    fn test_label_with_null_description_deserializes() {
        let json = r##"{"name":"bug","color":"#ff0000","description":null}"##;
        let label = serde_json::from_str::<GitlabLabel>(json);
        assert!(label.is_ok(), "GitLab labels with null description must deserialize: {:?}", label.err());
    }

    #[test]
    fn test_remote_url_with_uppercase_and_underscore() {
        let _repo = gittask::testing::TestRepo::new();
        let connector = GitlabRemoteConnector {};
        gittask::set_config_value("task.gitlab.url", "https://gitlab.com/").unwrap();
        let result = connector.supports_remote("https://gitlab.com/UserName/my_repo");
        gittask::set_config_value("task.gitlab.url", "https://gitlab.com/").unwrap();
        assert_eq!(result, Some(("UserName".to_string(), "my_repo".to_string())));
    }
}