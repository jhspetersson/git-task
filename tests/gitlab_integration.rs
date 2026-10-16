use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;
use serde_json::{json, Value};

use gittask::testing::TestRepo;

const LABEL: &str = "ci-label";

struct GitLab {
    url: String,
    token: String,
    client: Client,
}

struct Project {
    id: u64,
    namespace: String,
    path: String,
}

impl GitLab {
    fn from_env() -> Option<GitLab> {
        let url = std::env::var("GIT_TASK_GITLAB_URL").ok()?;
        let token = std::env::var("GIT_TASK_GITLAB_TOKEN").ok()?;
        if url.trim().is_empty() || token.trim().is_empty() {
            return None;
        }
        Some(GitLab {
            url: url.trim().trim_end_matches('/').to_string(),
            token: token.trim().to_string(),
            client: Client::new(),
        })
    }

    fn send(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> reqwest::blocking::Response {
        let mut request = self
            .client
            .request(method, format!("{}/api/v4{}", self.url, path))
            .header("PRIVATE-TOKEN", &self.token);
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

    fn put(&self, path: &str, body: Value) -> Value {
        self.json(reqwest::Method::PUT, path, Some(body))
    }

    fn status_of(&self, path: &str) -> reqwest::StatusCode {
        self.send(reqwest::Method::GET, path, None).status()
    }

    fn create_project(&self, tag: &str) -> Project {
        let name = format!("git-task-{tag}-{}", unique_suffix());
        let project = self.post("/projects", json!({ "name": name, "visibility": "private" }));
        let path_with_namespace = project["path_with_namespace"].as_str().expect("path_with_namespace");
        let (namespace, path) = path_with_namespace.rsplit_once('/').expect("namespace/path");
        Project {
            id: project["id"].as_u64().expect("project id"),
            namespace: namespace.to_string(),
            path: path.to_string(),
        }
    }

    fn create_issue(&self, project: &Project, title: &str, description: &str) -> u64 {
        let issue = self.post(&format!("/projects/{}/issues", project.id), json!({ "title": title, "description": description }));
        issue["iid"].as_u64().expect("issue iid")
    }

    fn issue(&self, project: &Project, iid: u64) -> Value {
        self.get(&format!("/projects/{}/issues/{iid}", project.id))
    }

    fn issue_labels(&self, project: &Project, iid: u64) -> Vec<String> {
        self.issue(project, iid)["labels"]
            .as_array()
            .expect("labels")
            .iter()
            .map(|l| l.as_str().unwrap_or("").to_string())
            .collect()
    }

    fn add_note(&self, project: &Project, iid: u64, body: &str) {
        self.post(&format!("/projects/{}/issues/{iid}/notes", project.id), json!({ "body": body }));
    }

    fn notes(&self, project: &Project, iid: u64) -> Vec<(u64, String)> {
        self.get(&format!("/projects/{}/issues/{iid}/notes?sort=asc&order_by=created_at", project.id))
            .as_array()
            .expect("notes")
            .iter()
            .filter(|n| !n["system"].as_bool().unwrap_or(false))
            .map(|n| (n["id"].as_u64().expect("note id"), n["body"].as_str().unwrap_or("").to_string()))
            .collect()
    }

    fn add_label(&self, project: &Project, iid: u64, name: &str, color: &str) {
        self.post(&format!("/projects/{}/labels", project.id), json!({ "name": name, "color": color }));
        self.put(&format!("/projects/{}/issues/{iid}", project.id), json!({ "add_labels": name }));
    }

    fn issues(&self, project: &Project) -> Vec<(u64, String, String)> {
        self.get(&format!("/projects/{}/issues?state=all", project.id))
            .as_array()
            .expect("issues")
            .iter()
            .map(|i| (
                i["iid"].as_u64().expect("issue iid"),
                i["title"].as_str().unwrap_or("").to_string(),
                i["state"].as_str().unwrap_or("").to_string(),
            ))
            .collect()
    }
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
    format!("{}-{nanos}", std::process::id())
}

fn local_repo(gitlab: &GitLab, project: &Project) -> TestRepo {
    let repo = TestRepo::new();
    let git = git2::Repository::open(repo.path()).expect("open temp repo");
    let remote_url = format!("{}/{}/{}.git", gitlab.url, project.namespace, project.path);
    git.remote("origin", &remote_url).expect("add remote");
    let mut config = git.config().expect("repo config");
    config.set_str("task.gitlab.url", &gitlab.url).expect("set url");
    config.set_str("task.default.connector", "gitlab").expect("set connector");
    repo
}

fn git_task(gitlab: &GitLab, repo: &TestRepo, args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_git-task"))
        .current_dir(repo.path())
        .env("NO_COLOR", "1")
        .env("GITLAB_TOKEN", &gitlab.token)
        .args(args)
        .output()
        .expect("failed to run git-task");
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

fn run(gitlab: &GitLab, repo: &TestRepo, args: &[&str]) -> String {
    let (ok, out) = git_task(gitlab, repo, args);
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

macro_rules! gitlab_or_skip {
    () => {
        match GitLab::from_env() {
            Some(gitlab) => gitlab,
            None => {
                eprintln!("skipped: set GIT_TASK_GITLAB_URL and GIT_TASK_GITLAB_TOKEN to run GitLab integration tests");
                return;
            }
        }
    };
}

#[test]
fn push_creates_the_issue_once_and_adopts_its_iid() {
    let gitlab = gitlab_or_skip!();
    let project = gitlab.create_project("push");
    let repo = local_repo(&gitlab, &project);

    run(&gitlab, &repo, &["create", "Pushed from CI", "Pushed description"]);
    run(&gitlab, &repo, &["set", "1", "id", "500"]);
    run(&gitlab, &repo, &["push", "500"]);

    let issues = gitlab.issues(&project);
    assert_eq!(issues.len(), 1, "exactly one issue must be created, got {issues:?}");
    let (iid, title, state) = &issues[0];
    assert_eq!(title, "Pushed from CI");
    assert_eq!(state, "opened");
    assert_eq!(gitlab.issue(&project, *iid)["description"], "Pushed description");
    assert_eq!(local_task_ids(&repo), vec![iid.to_string()], "the local task must adopt the GitLab issue iid");

    run(&gitlab, &repo, &["push", &iid.to_string()]);
    assert_eq!(gitlab.issues(&project).len(), 1, "a second push must not create another issue");
}

#[test]
fn pull_imports_the_issue_with_its_comment_and_label_but_not_system_notes() {
    let gitlab = gitlab_or_skip!();
    let project = gitlab.create_project("pull");
    let iid = gitlab.create_issue(&project, "Pulled from CI", "Some description");
    gitlab.add_note(&project, iid, "A real comment");
    gitlab.add_label(&project, iid, LABEL, "#ff6633");
    let repo = local_repo(&gitlab, &project);

    run(&gitlab, &repo, &["pull"]);

    assert_eq!(local_task_ids(&repo), vec![iid.to_string()]);
    let task = local_task(&repo, &iid.to_string());
    assert_eq!(task["props"]["name"], "Pulled from CI");
    assert_eq!(task["props"]["description"], "Some description");
    assert_eq!(task["props"]["status"], "OPEN");

    let texts = local_comments(&repo, &iid.to_string()).into_iter().map(|(_, text)| text).collect::<Vec<_>>();
    assert_eq!(texts, vec!["A real comment".to_string()], "only real notes must be imported, not system notes");
    assert_eq!(local_labels(&repo, &iid.to_string()), vec![LABEL.to_string()]);
}

#[test]
fn pull_by_id_imports_a_single_issue() {
    let gitlab = gitlab_or_skip!();
    let project = gitlab.create_project("pull-one");
    let first = gitlab.create_issue(&project, "First issue", "First");
    let second = gitlab.create_issue(&project, "Second issue", "Second");
    let repo = local_repo(&gitlab, &project);

    run(&gitlab, &repo, &["pull", &second.to_string()]);

    assert_eq!(local_task_ids(&repo), vec![second.to_string()]);
    assert_ne!(first, second);
    let task = local_task(&repo, &second.to_string());
    assert_eq!(task["props"]["name"], "Second issue");
    assert_eq!(task["props"]["description"], "Second");
}

#[test]
fn push_syncs_local_edits_comments_labels_and_closes_the_issue() {
    let gitlab = gitlab_or_skip!();
    let project = gitlab.create_project("sync");
    let iid = gitlab.create_issue(&project, "Synced from CI", "Original description");
    let repo = local_repo(&gitlab, &project);
    let id = iid.to_string();

    run(&gitlab, &repo, &["pull", &id]);
    run(&gitlab, &repo, &["set", &id, "name", "Renamed from CI"]);
    run(&gitlab, &repo, &["set", &id, "description", "Edited description"]);
    run(&gitlab, &repo, &["comment", "add", &id, "Comment added locally"]);
    run(&gitlab, &repo, &["label", "add", &id, LABEL]);
    run(&gitlab, &repo, &["push", &id]);

    let issue = gitlab.issue(&project, iid);
    assert_eq!(issue["title"], "Renamed from CI");
    assert_eq!(issue["description"], "Edited description");
    assert_eq!(issue["state"], "opened");
    assert_eq!(gitlab.issue_labels(&project, iid), vec![LABEL.to_string()]);

    let notes = gitlab.notes(&project, iid);
    assert_eq!(notes.len(), 1, "the local comment must be created remotely");
    assert_eq!(notes[0].1, "Comment added locally");
    assert_eq!(local_comments(&repo, &id), vec![(notes[0].0.to_string(), "Comment added locally".to_string())], "the local comment must adopt the remote note id");

    run(&gitlab, &repo, &["push", &id]);
    assert_eq!(gitlab.notes(&project, iid).len(), 1, "a repeated push must not duplicate the comment");

    run(&gitlab, &repo, &["set", &id, "status", "CLOSED"]);
    run(&gitlab, &repo, &["push", &id]);
    assert_eq!(gitlab.issue(&project, iid)["state"], "closed");

    run(&gitlab, &repo, &["set", &id, "status", "OPEN"]);
    run(&gitlab, &repo, &["push", &id]);
    assert_eq!(gitlab.issue(&project, iid)["state"], "opened", "push must reopen the issue");
}

#[test]
fn comment_commands_push_to_the_remote() {
    let gitlab = gitlab_or_skip!();
    let project = gitlab.create_project("comment");
    let iid = gitlab.create_issue(&project, "Commented from CI", "With comments");
    let repo = local_repo(&gitlab, &project);
    let id = iid.to_string();

    run(&gitlab, &repo, &["pull", &id]);
    run(&gitlab, &repo, &["comment", "add", &id, "First version", "--push"]);

    let notes = gitlab.notes(&project, iid);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].1, "First version");
    let note_id = notes[0].0.to_string();
    assert_eq!(local_comments(&repo, &id), vec![(note_id.clone(), "First version".to_string())]);

    run(&gitlab, &repo, &["comment", "set", &id, &note_id, "Second version", "--push"]);
    let notes = gitlab.notes(&project, iid);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].1, "Second version");
    assert_eq!(local_comments(&repo, &id), vec![(note_id.clone(), "Second version".to_string())]);

    run(&gitlab, &repo, &["comment", "delete", &id, &note_id, "--push"]);
    assert!(gitlab.notes(&project, iid).is_empty(), "the remote note must be deleted");
    assert!(local_comments(&repo, &id).is_empty());
}

#[test]
fn label_commands_push_to_the_remote() {
    let gitlab = gitlab_or_skip!();
    let project = gitlab.create_project("label");
    let iid = gitlab.create_issue(&project, "Labeled from CI", "With labels");
    let repo = local_repo(&gitlab, &project);
    let id = iid.to_string();

    run(&gitlab, &repo, &["pull", &id]);
    assert!(local_labels(&repo, &id).is_empty());

    run(&gitlab, &repo, &["label", "add", &id, LABEL, "--push"]);
    assert_eq!(gitlab.issue_labels(&project, iid), vec![LABEL.to_string()]);
    assert_eq!(local_labels(&repo, &id), vec![LABEL.to_string()]);

    run(&gitlab, &repo, &["label", "delete", &id, LABEL, "--push"]);
    assert!(gitlab.issue_labels(&project, iid).is_empty(), "the label must be removed from the remote issue");
    assert!(local_labels(&repo, &id).is_empty());
}

#[test]
fn delete_with_push_removes_the_remote_issue() {
    let gitlab = gitlab_or_skip!();
    let project = gitlab.create_project("delete");
    let iid = gitlab.create_issue(&project, "Deleted from CI", "To be deleted");
    let repo = local_repo(&gitlab, &project);
    let id = iid.to_string();

    run(&gitlab, &repo, &["pull", &id]);
    run(&gitlab, &repo, &["delete", &id, "--push"]);

    assert!(local_task_ids(&repo).is_empty());
    assert_eq!(gitlab.status_of(&format!("/projects/{}/issues/{iid}", project.id)), reqwest::StatusCode::NOT_FOUND, "the remote issue must be deleted");
    assert!(gitlab.issues(&project).is_empty());
}
