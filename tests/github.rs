//! Integration tests against the real GitHub API.
//!
//! They run only when `GIT_TASK_GITHUB_TOKEN` is set. The token needs write
//! access to the issues of the test repository, which defaults to
//! `jhspetersson/git-task-ci` and can be overridden with `GIT_TASK_GITHUB_REPO`
//! in the `owner/name` form. Without the token the tests are skipped and report
//! success.
//!
//! GitHub cannot be spun up per run, so every test shares one repository. Each
//! test tags its issue titles with a unique suffix, asserts only on issues that
//! carry its suffix, and closes them afterwards.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;
use serde_json::{json, Value};

use gittask::testing::TestRepo;

const DEFAULT_REPO: &str = "jhspetersson/git-task-ci";
const LABEL: &str = "ci-label";

struct GitHub {
    token: String,
    owner: String,
    repo: String,
    client: Client,
}

struct Issue {
    number: u64,
    title: String,
    state: String,
}

impl GitHub {
    fn from_env() -> Option<GitHub> {
        let token = std::env::var("GIT_TASK_GITHUB_TOKEN").ok()?;
        if token.trim().is_empty() {
            return None;
        }
        let repo = std::env::var("GIT_TASK_GITHUB_REPO").unwrap_or_else(|_| DEFAULT_REPO.to_string());
        let (owner, repo) = repo.trim().split_once('/').expect("GIT_TASK_GITHUB_REPO must be owner/name");
        Some(GitHub {
            token: token.trim().to_string(),
            owner: owner.to_string(),
            repo: repo.to_string(),
            client: Client::builder()
                .user_agent("git-task-integration-tests")
                .build()
                .expect("http client"),
        })
    }

    fn send(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> reqwest::blocking::Response {
        let mut request = self
            .client
            .request(method, format!("https://api.github.com/repos/{}/{}{}", self.owner, self.repo, path))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(body) = body {
            request = request.json(&body);
        }
        request.send().expect("request failed")
    }

    fn json(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Value {
        let response = self.send(method.clone(), path, body);
        let status = response.status();
        let text = response.text().expect("response body");
        assert!(status.is_success(), "{method} {path} failed with {status}: {text}");
        serde_json::from_str(&text).expect("response is not JSON")
    }

    fn get(&self, path: &str) -> Value {
        self.json(reqwest::Method::GET, path, None)
    }

    fn post(&self, path: &str, body: Value) -> Value {
        self.json(reqwest::Method::POST, path, Some(body))
    }

    fn patch(&self, path: &str, body: Value) -> Value {
        self.json(reqwest::Method::PATCH, path, Some(body))
    }

    fn remote_url(&self) -> String {
        format!("https://github.com/{}/{}.git", self.owner, self.repo)
    }

    fn create_issue(&self, title: &str, body: &str) -> u64 {
        let issue = self.post("/issues", json!({ "title": title, "body": body }));
        issue["number"].as_u64().expect("issue number")
    }

    fn add_comment(&self, number: u64, body: &str) {
        self.post(&format!("/issues/{number}/comments"), json!({ "body": body }));
    }

    fn add_label(&self, number: u64, name: &str) {
        // The label is repository-wide and survives between runs, so creating it may fail with 422.
        let response = self.send(reqwest::Method::POST, "/labels", Some(json!({ "name": name, "color": "ff6633" })));
        assert!(
            response.status().is_success() || response.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY,
            "could not create label {name}: {}",
            response.status()
        );
        self.post(&format!("/issues/{number}/labels"), json!({ "labels": [name] }));
    }

    fn close_issue(&self, number: u64) {
        self.patch(&format!("/issues/{number}"), json!({ "state": "closed" }));
    }

    /// Issues whose title contains `marker`, newest first. Pull requests are excluded.
    fn issues_with(&self, marker: &str) -> Vec<Issue> {
        self.get("/issues?state=all&per_page=100&sort=created&direction=desc")
            .as_array()
            .expect("issues")
            .iter()
            .filter(|i| i.get("pull_request").is_none())
            .filter(|i| i["title"].as_str().unwrap_or("").contains(marker))
            .map(|i| Issue {
                number: i["number"].as_u64().expect("issue number"),
                title: i["title"].as_str().unwrap_or("").to_string(),
                state: i["state"].as_str().unwrap_or("").to_string(),
            })
            .collect()
    }
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
    format!("{}-{nanos}", std::process::id())
}

fn local_repo(github: &GitHub) -> TestRepo {
    let repo = TestRepo::new();
    let git = git2::Repository::open(repo.path()).expect("open temp repo");
    git.remote("origin", &github.remote_url()).expect("add remote");
    let mut config = git.config().expect("repo config");
    config.set_str("task.default.connector", "github").expect("set connector");
    repo
}

fn git_task(github: &GitHub, repo: &TestRepo, args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_git-task"))
        .current_dir(repo.path())
        .env("NO_COLOR", "1")
        .env("GITHUB_TOKEN", &github.token)
        .args(args)
        .output()
        .expect("failed to run git-task");
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

fn local_task_ids(repo: &TestRepo) -> Vec<String> {
    let git = git2::Repository::open(repo.path()).expect("open temp repo");
    match git.find_reference("refs/tasks/tasks") {
        Ok(reference) => reference
            .peel_to_tree()
            .expect("tasks tree")
            .iter()
            .map(|entry| entry.name().unwrap_or("").to_string())
            .collect(),
        Err(_) => vec![],
    }
}

fn local_task(repo: &TestRepo, id: &str) -> Value {
    let git = git2::Repository::open(repo.path()).expect("open temp repo");
    let tree = git.find_reference("refs/tasks/tasks").expect("tasks ref").peel_to_tree().expect("tasks tree");
    let entry = tree.get_name(id).unwrap_or_else(|| panic!("local task {id} not found"));
    let blob = git.find_blob(entry.id()).expect("task blob");
    serde_json::from_slice(blob.content()).expect("task JSON")
}

macro_rules! github_or_skip {
    () => {
        match GitHub::from_env() {
            Some(github) => github,
            None => {
                eprintln!("skipped: set GIT_TASK_GITHUB_TOKEN to run GitHub integration tests");
                return;
            }
        }
    };
}

#[test]
fn push_creates_the_issue_once_and_adopts_its_number() {
    let github = github_or_skip!();
    let suffix = unique_suffix();
    let title = format!("Pushed from CI {suffix}");
    let repo = local_repo(&github);

    let (ok, out) = git_task(&github, &repo, &["create", &title]);
    assert!(ok, "create failed: {out}");
    // Rename the local task to a number no issue in the shared repository will ever have.
    let (ok, out) = git_task(&github, &repo, &["set", "1", "id", "999999999"]);
    assert!(ok, "renaming the local task failed: {out}");

    let (ok, out) = git_task(&github, &repo, &["push", "999999999"]);
    assert!(ok, "first push failed: {out}");

    let issues = github.issues_with(&suffix);
    assert_eq!(issues.len(), 1, "exactly one issue must be created for {suffix}");
    let issue = &issues[0];
    assert_eq!(issue.title, title);
    assert_eq!(issue.state, "open");
    assert_eq!(local_task_ids(&repo), vec![issue.number.to_string()], "the local task must adopt the GitHub issue number");

    let (ok, out) = git_task(&github, &repo, &["push", &issue.number.to_string()]);
    assert!(ok, "second push failed: {out}");
    assert_eq!(github.issues_with(&suffix).len(), 1, "a second push must not create another issue");

    github.close_issue(issue.number);
}

#[test]
fn pull_imports_the_issue_with_its_comment_and_label() {
    let github = github_or_skip!();
    let suffix = unique_suffix();
    let title = format!("Pulled from CI {suffix}");
    let number = github.create_issue(&title, "Some description");
    github.add_comment(number, "A real comment");
    github.add_label(number, LABEL);
    let repo = local_repo(&github);

    let (ok, out) = git_task(&github, &repo, &["pull", &number.to_string()]);
    assert!(ok, "pull failed: {out}");

    assert_eq!(local_task_ids(&repo), vec![number.to_string()]);
    let task = local_task(&repo, &number.to_string());
    assert_eq!(task["props"]["name"], title);
    assert_eq!(task["props"]["description"], "Some description");
    assert_eq!(task["props"]["status"], "OPEN");

    let comments = task["comments"].as_array().expect("comments");
    let texts = comments.iter().map(|c| c["text"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>();
    assert_eq!(texts, vec!["A real comment".to_string()]);

    let labels = task["labels"].as_array().expect("labels");
    let names = labels.iter().map(|l| l["name"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>();
    assert_eq!(names, vec![LABEL.to_string()]);

    github.close_issue(number);
}

#[test]
fn push_closes_the_issue_when_the_local_task_is_closed() {
    let github = github_or_skip!();
    let suffix = unique_suffix();
    let title = format!("Closed from CI {suffix}");
    let number = github.create_issue(&title, "To be closed");
    let repo = local_repo(&github);

    let (ok, out) = git_task(&github, &repo, &["pull", &number.to_string()]);
    assert!(ok, "pull failed: {out}");
    let (ok, out) = git_task(&github, &repo, &["set", &number.to_string(), "status", "CLOSED"]);
    assert!(ok, "closing the local task failed: {out}");

    let (ok, out) = git_task(&github, &repo, &["push", &number.to_string()]);
    assert!(ok, "push failed: {out}");

    let issues = github.issues_with(&suffix);
    assert_eq!(issues.len(), 1, "no new issue must be created for {suffix}");
    assert_eq!(issues[0].number, number);
    assert_eq!(issues[0].state, "closed", "the remote issue must be closed by the push");
}
