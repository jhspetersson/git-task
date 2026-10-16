use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;
use serde_json::{json, Value};

use gittask::testing::TestRepo;

struct Redmine {
    url: String,
    key: String,
    client: Client,
}

impl Redmine {
    fn from_env() -> Option<Redmine> {
        let url = std::env::var("GIT_TASK_REDMINE_URL").ok()?;
        let key = std::env::var("GIT_TASK_REDMINE_API_KEY").ok()?;
        if url.trim().is_empty() || key.trim().is_empty() {
            return None;
        }
        Some(Redmine {
            url: url.trim().trim_end_matches('/').to_string(),
            key: key.trim().to_string(),
            client: Client::new(),
        })
    }

    fn send(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> reqwest::blocking::Response {
        let mut request = self
            .client
            .request(method, format!("{}{}", self.url, path))
            .header("X-Redmine-API-Key", &self.key);
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
        if text.trim().is_empty() {
            return Value::Null;
        }
        serde_json::from_str(&text).expect("response is not JSON")
    }

    fn get(&self, path: &str) -> Value {
        self.json(reqwest::Method::GET, path, None)
    }

    fn post(&self, path: &str, body: Value) -> Value {
        self.json(reqwest::Method::POST, path, Some(body))
    }

    fn put(&self, path: &str, body: Value) -> Value {
        self.json(reqwest::Method::PUT, path, Some(body))
    }

    fn status_of(&self, path: &str) -> reqwest::StatusCode {
        self.send(reqwest::Method::GET, path, None).status()
    }

    fn create_project(&self, tag: &str) -> u64 {
        let trackers = self.get("/trackers.json")["trackers"]
            .as_array()
            .expect("trackers")
            .iter()
            .map(|t| t["id"].as_u64().expect("tracker id"))
            .collect::<Vec<_>>();
        assert!(!trackers.is_empty(), "Redmine has no trackers, load the default data first");

        let identifier = format!("git-task-{tag}-{}", unique_suffix());
        let project = self.post("/projects.json", json!({
            "project": { "name": identifier, "identifier": identifier, "tracker_ids": trackers }
        }));
        project["project"]["id"].as_u64().expect("project id")
    }

    fn create_issue(&self, project_id: u64, subject: &str) -> u64 {
        self.create_issue_with_description(project_id, subject, "")
    }

    fn create_issue_with_description(&self, project_id: u64, subject: &str, description: &str) -> u64 {
        let issue = self.post("/issues.json", json!({
            "issue": { "project_id": project_id, "subject": subject, "description": description }
        }));
        issue["issue"]["id"].as_u64().expect("issue id")
    }

    fn issue(&self, id: u64) -> Value {
        self.get(&format!("/issues/{id}.json?include=journals"))["issue"].clone()
    }

    fn add_note(&self, id: u64, text: &str) {
        self.put(&format!("/issues/{id}.json"), json!({ "issue": { "notes": text } }));
    }

    fn notes(&self, id: u64) -> Vec<(u64, String)> {
        self.issue(id)["journals"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter(|j| !j["notes"].as_str().unwrap_or("").is_empty())
            .map(|j| (j["id"].as_u64().expect("journal id"), j["notes"].as_str().unwrap_or("").to_string()))
            .collect()
    }

    fn children(&self, parent_id: u64) -> Vec<(u64, String, String)> {
        self.get(&format!("/issues.json?parent_id={parent_id}&status_id=*"))["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .map(|i| (
                i["id"].as_u64().expect("issue id"),
                i["subject"].as_str().unwrap_or("").to_string(),
                i["status"]["name"].as_str().unwrap_or("").to_string(),
            ))
            .collect()
    }

    fn issues_in_project(&self, project_id: u64) -> Vec<(u64, String)> {
        self.get(&format!("/issues.json?project_id={project_id}&status_id=*"))["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .map(|i| (i["id"].as_u64().expect("issue id"), i["subject"].as_str().unwrap_or("").to_string()))
            .collect()
    }
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
    format!("{}-{nanos}", std::process::id())
}

fn local_repo(redmine: &Redmine, project_id: u64) -> TestRepo {
    let repo = TestRepo::new();
    let git = git2::Repository::open(repo.path()).expect("open temp repo");
    git.remote("origin", "https://example.invalid/git-task/ci.git").expect("add remote");
    let mut config = git.config().expect("repo config");
    config.set_str("task.redmine.url", &redmine.url).expect("set url");
    config.set_str("task.redmine.api.key", &redmine.key).expect("set key");
    config.set_str("task.redmine.project.id", &project_id.to_string()).expect("set project");
    config.set_str("task.default.connector", "redmine").expect("set connector");
    repo
}

fn git_task(repo: &TestRepo, args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_git-task"))
        .current_dir(repo.path())
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .expect("failed to run git-task");
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

fn run(repo: &TestRepo, args: &[&str]) -> String {
    let (ok, out) = git_task(repo, args);
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

macro_rules! redmine_or_skip {
    () => {
        match Redmine::from_env() {
            Some(redmine) => redmine,
            None => {
                eprintln!("skipped: set GIT_TASK_REDMINE_URL and GIT_TASK_REDMINE_API_KEY to run Redmine integration tests");
                return;
            }
        }
    };
}

#[test]
fn push_creates_the_issue_once_and_adopts_its_id() {
    let redmine = redmine_or_skip!();
    let project = redmine.create_project("push");
    let repo = local_repo(&redmine, project);

    run(&repo, &["create", "Pushed from CI", "Pushed description"]);
    run(&repo, &["push", "1"]);

    let issues = redmine.issues_in_project(project);
    assert_eq!(issues.len(), 1, "exactly one issue must be created, got {issues:?}");
    let (remote_id, subject) = &issues[0];
    assert_eq!(subject, "Pushed from CI");
    assert_eq!(redmine.issue(*remote_id)["description"], "Pushed description");
    assert_eq!(local_task_ids(&repo), vec![remote_id.to_string()], "the local task must adopt the Redmine issue id");

    run(&repo, &["push", &remote_id.to_string()]);
    assert_eq!(redmine.issues_in_project(project).len(), 1, "a second push must not create another issue");
}

#[test]
fn push_never_touches_an_issue_from_another_project() {
    let redmine = redmine_or_skip!();
    let foreign_project = redmine.create_project("foreign");
    let own_project = redmine.create_project("own");
    let foreign_id = redmine.create_issue(foreign_project, "Foreign issue");
    let repo = local_repo(&redmine, own_project);

    run(&repo, &["create", "Own task", "Own description"]);
    run(&repo, &["set", "1", "id", &foreign_id.to_string()]);
    run(&repo, &["push", &foreign_id.to_string()]);

    let foreign = redmine.issue(foreign_id);
    assert_eq!(foreign["subject"], "Foreign issue", "the issue of another project must stay untouched");
    assert_eq!(foreign["project"]["id"], foreign_project, "the issue must stay in its own project");

    let own = redmine.issues_in_project(own_project);
    assert_eq!(own.len(), 1, "the task must be created in the configured project, got {own:?}");
    assert_eq!(own[0].1, "Own task");
    assert_ne!(own[0].0, foreign_id);
    assert_eq!(local_task_ids(&repo), vec![own[0].0.to_string()], "the local task must adopt the new issue id");
}

#[test]
fn pull_imports_the_issue_with_its_status_and_comment() {
    let redmine = redmine_or_skip!();
    let project = redmine.create_project("pull");
    let id = redmine.create_issue_with_description(project, "Pulled from CI", "Some description");
    redmine.add_note(id, "A real comment");
    let repo = local_repo(&redmine, project);

    run(&repo, &["pull", &id.to_string()]);

    assert_eq!(local_task_ids(&repo), vec![id.to_string()]);
    let task = local_task(&repo, &id.to_string());
    assert_eq!(task["props"]["name"], "Pulled from CI");
    assert_eq!(task["props"]["description"], "Some description");
    assert_eq!(task["props"]["status"], "New", "the Redmine status name must be kept");

    let notes = redmine.notes(id);
    assert_eq!(notes.len(), 1);
    assert_eq!(local_comments(&repo, &id.to_string()), vec![(notes[0].0.to_string(), "A real comment".to_string())], "the comment must carry the journal id");
}

#[test]
fn pull_without_ids_imports_every_issue_of_the_project() {
    let redmine = redmine_or_skip!();
    let project = redmine.create_project("pull-all");
    let other_project = redmine.create_project("other");
    let first = redmine.create_issue(project, "First issue");
    let second = redmine.create_issue(project, "Second issue");
    let foreign = redmine.create_issue(other_project, "Foreign issue");
    let repo = local_repo(&redmine, project);

    run(&repo, &["pull"]);

    let mut ids = local_task_ids(&repo);
    ids.sort();
    let mut expected = vec![first.to_string(), second.to_string()];
    expected.sort();
    assert_eq!(ids, expected, "only the issues of the configured project must be imported");
    assert!(!ids.contains(&foreign.to_string()));
}

#[test]
fn push_syncs_local_edits_comments_subtasks_and_status() {
    let redmine = redmine_or_skip!();
    let project = redmine.create_project("sync");
    let id = redmine.create_issue_with_description(project, "Synced from CI", "Original description");
    let repo = local_repo(&redmine, project);
    let id_str = id.to_string();

    run(&repo, &["pull", &id_str]);
    run(&repo, &["set", &id_str, "name", "Renamed from CI"]);
    run(&repo, &["set", &id_str, "description", "Edited description"]);
    run(&repo, &["comment", "add", &id_str, "Comment added locally"]);
    run(&repo, &["subtask", "add", &id_str, "Subtask added locally"]);
    run(&repo, &["push", &id_str]);

    let issue = redmine.issue(id);
    assert_eq!(issue["subject"], "Renamed from CI");
    assert_eq!(issue["description"], "Edited description");
    assert_eq!(issue["status"]["name"], "New");

    let notes = redmine.notes(id);
    assert_eq!(notes.len(), 1, "the local comment must be created as a journal note");
    assert_eq!(notes[0].1, "Comment added locally");
    assert_eq!(local_comments(&repo, &id_str), vec![(notes[0].0.to_string(), "Comment added locally".to_string())], "the local comment must adopt the journal id");

    let children = redmine.children(id);
    assert_eq!(children.len(), 1, "the local subtask must be created as a child issue");
    assert_eq!(children[0].1, "Subtask added locally");
    let child_id = children[0].0.to_string();
    let subtasks = local_subtasks(&repo, &id_str);
    assert_eq!(subtasks.len(), 1);
    assert_eq!(subtasks[0].0, child_id, "the local subtask must adopt the child issue id");
    assert_eq!(subtasks[0].1, "Subtask added locally");

    run(&repo, &["push", &id_str]);
    assert_eq!(redmine.notes(id).len(), 1, "a repeated push must not duplicate the comment");
    assert_eq!(redmine.children(id).len(), 1, "a repeated push must not duplicate the child issue");

    run(&repo, &["subtask", "set", &id_str, &child_id, "status", "CLOSED"]);
    run(&repo, &["push", &id_str]);
    assert_eq!(redmine.children(id)[0].2, "Closed", "push must update the existing child issue");

    run(&repo, &["set", &id_str, "status", "CLOSED"]);
    run(&repo, &["push", &id_str]);
    assert_eq!(redmine.issue(id)["status"]["name"], "Closed", "the local CLOSED status must map to the Redmine Closed status");
}

#[test]
fn comment_commands_push_to_the_remote() {
    let redmine = redmine_or_skip!();
    let project = redmine.create_project("comment");
    let id = redmine.create_issue(project, "Commented from CI");
    let repo = local_repo(&redmine, project);
    let id_str = id.to_string();

    run(&repo, &["pull", &id_str]);
    run(&repo, &["comment", "add", &id_str, "First version", "--push"]);

    let notes = redmine.notes(id);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].1, "First version");
    let journal_id = notes[0].0.to_string();
    assert_eq!(local_comments(&repo, &id_str), vec![(journal_id.clone(), "First version".to_string())]);

    run(&repo, &["comment", "set", &id_str, &journal_id, "Second version", "--push"]);
    let notes = redmine.notes(id);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].1, "Second version");
    assert_eq!(local_comments(&repo, &id_str), vec![(journal_id.clone(), "Second version".to_string())]);

    let (ok, out) = git_task(&repo, &["comment", "delete", &id_str, &journal_id, "--push"]);
    assert!(!ok, "Redmine cannot delete journal notes, the command must report a failure");
    assert!(out.contains("not supported"), "unexpected output: {out}");
    assert!(local_comments(&repo, &id_str).is_empty(), "the local comment is still deleted");
    assert_eq!(redmine.notes(id).len(), 1, "the remote note must stay in place");
}

#[test]
fn label_commands_report_that_labels_are_unsupported() {
    let redmine = redmine_or_skip!();
    let project = redmine.create_project("label");
    let id = redmine.create_issue(project, "Labeled from CI");
    let repo = local_repo(&redmine, project);
    let id_str = id.to_string();

    run(&repo, &["pull", &id_str]);

    let (ok, out) = git_task(&repo, &["label", "add", &id_str, "ci-label", "--push"]);
    assert!(!ok, "Redmine has no labels, the command must report a failure");
    assert!(out.contains("not supported"), "unexpected output: {out}");

    let (ok, out) = git_task(&repo, &["label", "delete", &id_str, "ci-label", "--push"]);
    assert!(!ok, "Redmine has no labels, the command must report a failure");
    assert!(out.contains("not supported"), "unexpected output: {out}");

    assert_eq!(redmine.issue(id)["subject"], "Labeled from CI", "the remote issue must stay untouched");
}

#[test]
fn subtask_commands_push_to_the_remote() {
    let redmine = redmine_or_skip!();
    let project = redmine.create_project("subtask");
    let id = redmine.create_issue(project, "Parent from CI");
    let repo = local_repo(&redmine, project);
    let id_str = id.to_string();

    run(&repo, &["pull", &id_str]);
    run(&repo, &["subtask", "add", &id_str, "Child from CI", "--push"]);

    let children = redmine.children(id);
    assert_eq!(children.len(), 1, "the subtask must be created as a child issue");
    assert_eq!(children[0].1, "Child from CI");
    assert_eq!(children[0].2, "New");
    let child_id = children[0].0;
    let child_id_str = child_id.to_string();
    let subtasks = local_subtasks(&repo, &id_str);
    assert_eq!(subtasks.len(), 1);
    assert_eq!(subtasks[0].0, child_id_str, "the local subtask must adopt the child issue id");
    assert_eq!(redmine.issue(child_id)["parent"]["id"], id);
    assert_eq!(redmine.issue(child_id)["project"]["id"], project, "the child must be created in the parent's project");

    run(&repo, &["subtask", "set", &id_str, &child_id_str, "name", "Renamed child from CI", "--push"]);
    assert_eq!(redmine.issue(child_id)["subject"], "Renamed child from CI");

    run(&repo, &["subtask", "set", &id_str, &child_id_str, "status", "CLOSED", "--push"]);
    assert_eq!(redmine.issue(child_id)["status"]["name"], "Closed");

    run(&repo, &["pull", &id_str]);
    assert_eq!(local_subtasks(&repo, &id_str), vec![(child_id_str.clone(), "Renamed child from CI".to_string(), "Closed".to_string())], "pull must bring back the child issue with its Redmine status");

    run(&repo, &["subtask", "delete", &id_str, &child_id_str, "--push"]);
    assert!(redmine.children(id).is_empty(), "the child issue must be deleted");
    assert_eq!(redmine.status_of(&format!("/issues/{child_id}.json")), reqwest::StatusCode::NOT_FOUND);
    assert!(local_subtasks(&repo, &id_str).is_empty());
}

#[test]
fn delete_with_push_removes_the_remote_issue() {
    let redmine = redmine_or_skip!();
    let project = redmine.create_project("delete");
    let id = redmine.create_issue(project, "Deleted from CI");
    let repo = local_repo(&redmine, project);
    let id_str = id.to_string();

    run(&repo, &["pull", &id_str]);
    run(&repo, &["delete", &id_str, "--push"]);

    assert!(local_task_ids(&repo).is_empty());
    assert_eq!(redmine.status_of(&format!("/issues/{id}.json")), reqwest::StatusCode::NOT_FOUND, "the remote issue must be deleted");
    assert!(redmine.issues_in_project(project).is_empty());
}
