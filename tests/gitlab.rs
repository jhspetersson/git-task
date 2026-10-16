use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;
use serde_json::{json, Value};

use gittask::testing::TestRepo;

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

    fn get(&self, path: &str) -> Value {
        self.client
            .get(format!("{}/api/v4{}", self.url, path))
            .header("PRIVATE-TOKEN", &self.token)
            .send()
            .expect("request failed")
            .error_for_status()
            .expect("GitLab returned an error")
            .json()
            .expect("response is not JSON")
    }

    fn send(&self, method: reqwest::Method, path: &str, body: Value) -> Value {
        self.client
            .request(method, format!("{}/api/v4{}", self.url, path))
            .header("PRIVATE-TOKEN", &self.token)
            .json(&body)
            .send()
            .expect("request failed")
            .error_for_status()
            .expect("GitLab returned an error")
            .json()
            .expect("response is not JSON")
    }

    fn post(&self, path: &str, body: Value) -> Value {
        self.send(reqwest::Method::POST, path, body)
    }

    fn put(&self, path: &str, body: Value) -> Value {
        self.send(reqwest::Method::PUT, path, body)
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

    fn add_note(&self, project: &Project, iid: u64, body: &str) {
        self.post(&format!("/projects/{}/issues/{iid}/notes", project.id), json!({ "body": body }));
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

    let (ok, out) = git_task(&gitlab, &repo, &["create", "Pushed from CI"]);
    assert!(ok, "create failed: {out}");
    let (ok, out) = git_task(&gitlab, &repo, &["set", "1", "id", "500"]);
    assert!(ok, "renaming the local task failed: {out}");

    let (ok, out) = git_task(&gitlab, &repo, &["push", "500"]);
    assert!(ok, "first push failed: {out}");

    let issues = gitlab.issues(&project);
    assert_eq!(issues.len(), 1, "exactly one issue must be created, got {issues:?}");
    let (iid, title, state) = &issues[0];
    assert_eq!(title, "Pushed from CI");
    assert_eq!(state, "opened");
    assert_eq!(local_task_ids(&repo), vec![iid.to_string()], "the local task must adopt the GitLab issue iid");

    let (ok, out) = git_task(&gitlab, &repo, &["push", &iid.to_string()]);
    assert!(ok, "second push failed: {out}");
    assert_eq!(gitlab.issues(&project).len(), 1, "a second push must not create another issue");
}

#[test]
fn pull_imports_the_issue_with_its_comment_and_label_but_not_system_notes() {
    let gitlab = gitlab_or_skip!();
    let project = gitlab.create_project("pull");
    let iid = gitlab.create_issue(&project, "Pulled from CI", "Some description");
    gitlab.add_note(&project, iid, "A real comment");
    gitlab.add_label(&project, iid, "ci-label", "#ff6633");
    let repo = local_repo(&gitlab, &project);

    let (ok, out) = git_task(&gitlab, &repo, &["pull"]);
    assert!(ok, "pull failed: {out}");

    assert_eq!(local_task_ids(&repo), vec![iid.to_string()]);
    let task = local_task(&repo, &iid.to_string());
    assert_eq!(task["props"]["name"], "Pulled from CI");
    assert_eq!(task["props"]["description"], "Some description");
    assert_eq!(task["props"]["status"], "OPEN");

    let comments = task["comments"].as_array().expect("comments");
    let texts = comments.iter().map(|c| c["text"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>();
    assert_eq!(texts, vec!["A real comment".to_string()], "only real notes must be imported, not system notes");

    let labels = task["labels"].as_array().expect("labels");
    let names = labels.iter().map(|l| l["name"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>();
    assert_eq!(names, vec!["ci-label".to_string()]);
}
