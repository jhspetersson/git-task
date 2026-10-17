use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
        for attempt in 0..5 {
            let response = self.send(method.clone(), path, body.clone());
            let status = response.status();
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(60);
            let text = response.text().expect("response body");
            let rate_limited = (status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS)
                && text.contains("rate limit");
            if rate_limited && attempt < 4 {
                eprintln!("{method} {path} hit a rate limit, retrying in {retry_after}s");
                std::thread::sleep(Duration::from_secs(retry_after));
                continue;
            }
            assert!(status.is_success(), "{method} {path} failed with {status}: {text}");
            return serde_json::from_str(&text).expect("response is not JSON");
        }
        unreachable!()
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

    fn status_of(&self, path: &str) -> reqwest::StatusCode {
        self.send(reqwest::Method::GET, path, None).status()
    }

    fn remote_url(&self) -> String {
        format!("https://github.com/{}/{}.git", self.owner, self.repo)
    }

    fn create_issue(&self, title: &str, body: &str) -> u64 {
        let issue = self.post("/issues", json!({ "title": title, "body": body }));
        issue["number"].as_u64().expect("issue number")
    }

    fn issue(&self, number: u64) -> Value {
        self.get(&format!("/issues/{number}"))
    }

    fn issue_labels(&self, number: u64) -> Vec<String> {
        self.issue(number)["labels"]
            .as_array()
            .expect("labels")
            .iter()
            .map(|l| l["name"].as_str().unwrap_or("").to_string())
            .collect()
    }

    fn add_comment(&self, number: u64, body: &str) {
        self.post(&format!("/issues/{number}/comments"), json!({ "body": body }));
    }

    fn comments(&self, number: u64) -> Vec<(u64, String)> {
        self.get(&format!("/issues/{number}/comments"))
            .as_array()
            .expect("comments")
            .iter()
            .map(|c| (c["id"].as_u64().expect("comment id"), c["body"].as_str().unwrap_or("").to_string()))
            .collect()
    }

    fn ensure_label(&self, name: &str) {
        let response = self.send(reqwest::Method::POST, "/labels", Some(json!({ "name": name, "color": "ff6633" })));
        assert!(
            response.status().is_success() || response.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY,
            "could not create label {name}: {}",
            response.status()
        );
    }

    fn add_label(&self, number: u64, name: &str) {
        self.ensure_label(name);
        self.post(&format!("/issues/{number}/labels"), json!({ "labels": [name] }));
    }

    fn remove_label(&self, number: u64, name: &str) {
        let response = self.send(reqwest::Method::DELETE, &format!("/issues/{number}/labels/{name}"), None);
        assert!(response.status().is_success(), "could not remove label {name}: {}", response.status());
    }

    fn create_sub_issue(&self, parent: u64, title: &str) -> u64 {
        let issue = self.post("/issues", json!({ "title": title }));
        let database_id = issue["id"].as_u64().expect("issue id");
        self.post(&format!("/issues/{parent}/sub_issues"), json!({ "sub_issue_id": database_id }));
        issue["number"].as_u64().expect("issue number")
    }

    fn sub_issues(&self, number: u64) -> Vec<Issue> {
        let mut result = vec![];
        for page in 1.. {
            let batch = self.get(&format!("/issues/{number}/sub_issues?per_page=100&page={page}"));
            let batch = batch.as_array().expect("sub issues");
            result.extend(batch.iter().map(|i| Issue {
                number: i["number"].as_u64().expect("issue number"),
                title: i["title"].as_str().unwrap_or("").to_string(),
                state: i["state"].as_str().unwrap_or("").to_string(),
            }));
            if batch.len() < 100 {
                break;
            }
        }
        result
    }

    fn close_issue(&self, number: u64) {
        self.patch(&format!("/issues/{number}"), json!({ "state": "closed" }));
    }

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

    fn wait_for_issues_with(&self, marker: &str, expected: usize) -> Vec<Issue> {
        let mut issues = vec![];
        for _ in 0..15 {
            issues = self.issues_with(marker);
            if issues.len() >= expected {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        issues
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

fn run(github: &GitHub, repo: &TestRepo, args: &[&str]) -> String {
    let (ok, out) = git_task(github, repo, args);
    assert!(ok, "git task {} failed: {out}", args.join(" "));
    out
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

fn local_comments(repo: &TestRepo, id: &str) -> Vec<(String, String)> {
    local_task(repo, id)["comments"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|c| (c["id"].as_str().unwrap_or("").to_string(), c["text"].as_str().unwrap_or("").to_string()))
        .collect()
}

fn local_labels(repo: &TestRepo, id: &str) -> Vec<String> {
    local_task(repo, id)["labels"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|l| l["name"].as_str().unwrap_or("").to_string())
        .collect()
}

fn local_subtasks(repo: &TestRepo, id: &str) -> Vec<(String, String, String)> {
    local_task(repo, id)["subtasks"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|s| (
            s["id"].as_str().unwrap_or("").to_string(),
            s["name"].as_str().unwrap_or("").to_string(),
            s["status"].as_str().unwrap_or("").to_string(),
        ))
        .collect()
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

    run(&github, &repo, &["create", &title, "Pushed description"]);
    run(&github, &repo, &["set", "1", "id", "999999999"]);
    let out = run(&github, &repo, &["push", "999999999"]);
    assert!(out.contains("Created REMOTE task ID"), "the first push must create the issue: {out}");

    let ids = local_task_ids(&repo);
    assert_eq!(ids.len(), 1);
    assert_ne!(ids[0], "999999999", "the local task must adopt the GitHub issue number");
    let number: u64 = ids[0].parse().expect("issue number");
    let issue = github.issue(number);
    assert_eq!(issue["title"], title);
    assert_eq!(issue["body"], "Pushed description");
    assert_eq!(issue["state"], "open");

    let out = run(&github, &repo, &["push", &ids[0]]);
    assert!(!out.contains("Created REMOTE task ID"), "a second push must not create another issue: {out}");
    assert_eq!(local_task_ids(&repo), ids);
    assert_eq!(github.wait_for_issues_with(&suffix, 1).len(), 1, "exactly one issue must exist for {suffix}");

    github.close_issue(number);
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

    run(&github, &repo, &["pull", &number.to_string()]);

    assert_eq!(local_task_ids(&repo), vec![number.to_string()]);
    let task = local_task(&repo, &number.to_string());
    assert_eq!(task["props"]["name"], title);
    assert_eq!(task["props"]["description"], "Some description");
    assert_eq!(task["props"]["status"], "OPEN");

    let texts = local_comments(&repo, &number.to_string()).into_iter().map(|(_, text)| text).collect::<Vec<_>>();
    assert_eq!(texts, vec!["A real comment".to_string()]);
    assert_eq!(local_labels(&repo, &number.to_string()), vec![LABEL.to_string()]);

    github.close_issue(number);
}

#[test]
fn push_closes_the_issue_when_the_local_task_is_closed() {
    let github = github_or_skip!();
    let suffix = unique_suffix();
    let title = format!("Closed from CI {suffix}");
    let number = github.create_issue(&title, "To be closed");
    let repo = local_repo(&github);
    let id = number.to_string();

    run(&github, &repo, &["pull", &id]);
    run(&github, &repo, &["set", &id, "status", "CLOSED"]);
    let out = run(&github, &repo, &["push", &id]);
    assert!(!out.contains("Created REMOTE task ID"), "no new issue must be created: {out}");

    assert_eq!(local_task_ids(&repo), vec![id.clone()]);
    assert_eq!(github.issue(number)["state"], "closed", "the remote issue must be closed by the push");
    assert_eq!(github.wait_for_issues_with(&suffix, 1).len(), 1, "exactly one issue must exist for {suffix}");
}

#[test]
fn push_syncs_local_edits_comments_labels_and_subtasks() {
    let github = github_or_skip!();
    let suffix = unique_suffix();
    let title = format!("Synced from CI {suffix}");
    let number = github.create_issue(&title, "Original description");
    let repo = local_repo(&github);
    let id = number.to_string();
    let new_title = format!("Renamed from CI {suffix}");
    let subtask_title = format!("Subtask from CI {suffix}");

    run(&github, &repo, &["pull", &id]);
    run(&github, &repo, &["set", &id, "name", &new_title]);
    run(&github, &repo, &["set", &id, "description", "Edited description"]);
    run(&github, &repo, &["comment", "add", &id, "Comment added locally"]);
    run(&github, &repo, &["label", "add", &id, LABEL]);
    run(&github, &repo, &["subtask", "add", &id, &subtask_title]);
    run(&github, &repo, &["push", &id]);

    let issue = github.issue(number);
    assert_eq!(issue["title"], new_title);
    assert_eq!(issue["body"], "Edited description");
    assert_eq!(issue["state"], "open");
    assert_eq!(github.issue_labels(number), vec![LABEL.to_string()]);

    let comments = github.comments(number);
    assert_eq!(comments.len(), 1, "the local comment must be created remotely");
    assert_eq!(comments[0].1, "Comment added locally");
    assert_eq!(local_comments(&repo, &id), vec![(comments[0].0.to_string(), "Comment added locally".to_string())], "the local comment must adopt the remote comment id");

    let subs = github.sub_issues(number);
    assert_eq!(subs.len(), 1, "the local subtask must be created as a sub-issue");
    assert_eq!(subs[0].title, subtask_title);
    assert_eq!(subs[0].state, "open");
    let sub_id = subs[0].number.to_string();
    assert_eq!(local_subtasks(&repo, &id), vec![(sub_id.clone(), subtask_title.clone(), "OPEN".to_string())], "the local subtask must adopt the sub-issue number");

    run(&github, &repo, &["subtask", "set", &id, &sub_id, "status", "CLOSED"]);
    run(&github, &repo, &["push", &id]);
    let subs = github.sub_issues(number);
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].state, "closed", "push must update the existing sub-issue");

    run(&github, &repo, &["push", &id]);
    assert_eq!(github.comments(number).len(), 1, "a repeated push must not duplicate the comment");
    assert_eq!(github.sub_issues(number).len(), 1, "a repeated push must not duplicate the sub-issue");

    github.close_issue(subs[0].number);
    github.close_issue(number);
}

#[test]
fn comment_commands_push_to_the_remote() {
    let github = github_or_skip!();
    let suffix = unique_suffix();
    let title = format!("Commented from CI {suffix}");
    let number = github.create_issue(&title, "With comments");
    let repo = local_repo(&github);
    let id = number.to_string();

    run(&github, &repo, &["pull", &id]);
    run(&github, &repo, &["comment", "add", &id, "First version", "--push"]);

    let comments = github.comments(number);
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].1, "First version");
    let comment_id = comments[0].0.to_string();
    assert_eq!(local_comments(&repo, &id), vec![(comment_id.clone(), "First version".to_string())]);

    run(&github, &repo, &["comment", "set", &id, &comment_id, "Second version", "--push"]);
    let comments = github.comments(number);
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].1, "Second version");
    assert_eq!(local_comments(&repo, &id), vec![(comment_id.clone(), "Second version".to_string())]);

    run(&github, &repo, &["comment", "delete", &id, &comment_id, "--push"]);
    assert!(github.comments(number).is_empty(), "the remote comment must be deleted");
    assert!(local_comments(&repo, &id).is_empty());

    github.close_issue(number);
}

#[test]
fn label_commands_push_to_the_remote() {
    let github = github_or_skip!();
    let suffix = unique_suffix();
    let title = format!("Labeled from CI {suffix}");
    let number = github.create_issue(&title, "With labels");
    let repo = local_repo(&github);
    let id = number.to_string();

    run(&github, &repo, &["pull", &id]);
    assert!(local_labels(&repo, &id).is_empty());

    run(&github, &repo, &["label", "add", &id, LABEL, "--push"]);
    assert_eq!(github.issue_labels(number), vec![LABEL.to_string()]);
    assert_eq!(local_labels(&repo, &id), vec![LABEL.to_string()]);

    run(&github, &repo, &["label", "delete", &id, LABEL, "--push"]);
    assert!(github.issue_labels(number).is_empty(), "the label must be removed from the remote issue");
    assert!(local_labels(&repo, &id).is_empty());

    github.close_issue(number);
}

#[test]
fn pull_updates_the_labels_of_an_existing_task() {
    let github = github_or_skip!();
    let suffix = unique_suffix();
    let title = format!("Relabeled from CI {suffix}");
    let number = github.create_issue(&title, "Labels change remotely");
    let repo = local_repo(&github);
    let id = number.to_string();

    run(&github, &repo, &["pull", &id]);
    assert!(local_labels(&repo, &id).is_empty());

    github.add_label(number, LABEL);
    run(&github, &repo, &["pull", &id]);
    assert_eq!(local_labels(&repo, &id), vec![LABEL.to_string()], "a label added on GitHub must reach the existing local task");

    github.remove_label(number, LABEL);
    run(&github, &repo, &["pull", &id]);
    assert!(local_labels(&repo, &id).is_empty(), "a label removed on GitHub must disappear locally");

    github.close_issue(number);
}

#[test]
fn push_syncs_label_only_changes() {
    let github = github_or_skip!();
    let suffix = unique_suffix();
    let title = format!("Label pushed from CI {suffix}");
    let number = github.create_issue(&title, "Labels change locally");
    let repo = local_repo(&github);
    let id = number.to_string();

    run(&github, &repo, &["pull", &id]);
    run(&github, &repo, &["label", "add", &id, LABEL]);
    run(&github, &repo, &["push", &id]);
    assert_eq!(github.issue_labels(number), vec![LABEL.to_string()], "a label added locally must be pushed without other changes");

    run(&github, &repo, &["label", "delete", &id, LABEL]);
    run(&github, &repo, &["push", &id]);
    assert!(github.issue_labels(number).is_empty(), "a label removed locally must be removed by push");

    github.close_issue(number);
}

#[test]
fn subtask_commands_push_to_the_remote() {
    let github = github_or_skip!();
    let suffix = unique_suffix();
    let title = format!("Parent from CI {suffix}");
    let number = github.create_issue(&title, "With subtasks");
    let repo = local_repo(&github);
    let id = number.to_string();
    let subtask_title = format!("Child from CI {suffix}");
    let renamed_title = format!("Renamed child from CI {suffix}");

    run(&github, &repo, &["pull", &id]);
    run(&github, &repo, &["subtask", "add", &id, &subtask_title, "--push"]);

    let subs = github.sub_issues(number);
    assert_eq!(subs.len(), 1, "the subtask must be created as a sub-issue");
    assert_eq!(subs[0].title, subtask_title);
    assert_eq!(subs[0].state, "open");
    let sub_number = subs[0].number;
    let sub_id = sub_number.to_string();
    assert_eq!(local_subtasks(&repo, &id), vec![(sub_id.clone(), subtask_title.clone(), "OPEN".to_string())]);

    run(&github, &repo, &["subtask", "set", &id, &sub_id, "name", &renamed_title, "--push"]);
    assert_eq!(github.issue(sub_number)["title"], renamed_title);

    run(&github, &repo, &["subtask", "set", &id, &sub_id, "status", "CLOSED", "--push"]);
    assert_eq!(github.issue(sub_number)["state"], "closed");

    run(&github, &repo, &["pull", &id]);
    assert_eq!(local_subtasks(&repo, &id), vec![(sub_id.clone(), renamed_title.clone(), "CLOSED".to_string())], "pull must bring back the sub-issue");

    run(&github, &repo, &["subtask", "delete", &id, &sub_id, "--push"]);
    assert!(github.sub_issues(number).is_empty(), "the sub-issue must be unlinked from the parent");
    assert!(local_subtasks(&repo, &id).is_empty());

    github.close_issue(number);
}

#[test]
fn pull_and_push_handle_more_than_one_page_of_sub_issues() {
    let github = github_or_skip!();
    let suffix = unique_suffix();
    let title = format!("Big parent from CI {suffix}");
    let number = github.create_issue(&title, "With many sub-issues");
    let repo = local_repo(&github);
    let id = number.to_string();
    let count = 31;

    let mut children = vec![];
    for i in 1..=count {
        children.push(github.create_sub_issue(number, &format!("Child {i} from CI {suffix}")));
        std::thread::sleep(Duration::from_millis(500));
    }
    assert_eq!(github.sub_issues(number).len(), count);

    run(&github, &repo, &["pull", &id]);
    let mut local_ids = local_subtasks(&repo, &id).into_iter().map(|(id, _, _)| id).collect::<Vec<_>>();
    local_ids.sort();
    let mut expected = children.iter().map(|c| c.to_string()).collect::<Vec<_>>();
    expected.sort();
    assert_eq!(local_ids, expected, "pull must import every sub-issue, not only the first page");

    let out = run(&github, &repo, &["push", &id]);
    assert!(!out.contains("Created REMOTE subtask"), "push must not duplicate sub-issues beyond the first page: {out}");
    assert_eq!(github.sub_issues(number).len(), count, "push must not create new sub-issues");

    for child in children {
        github.close_issue(child);
        std::thread::sleep(Duration::from_millis(500));
    }
    github.close_issue(number);
}

#[test]
fn delete_with_push_removes_the_remote_issue() {
    let github = github_or_skip!();
    let suffix = unique_suffix();
    let title = format!("Deleted from CI {suffix}");
    let number = github.create_issue(&title, "To be deleted");
    let repo = local_repo(&github);
    let id = number.to_string();

    run(&github, &repo, &["pull", &id]);
    run(&github, &repo, &["delete", &id, "--push"]);

    assert!(local_task_ids(&repo).is_empty());
    let status = github.status_of(&format!("/issues/{number}"));
    assert!(
        status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE,
        "the remote issue must be deleted, got {status}"
    );
    assert!(github.issues_with(&suffix).is_empty());
}
