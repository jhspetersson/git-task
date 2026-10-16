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

    fn get(&self, path: &str) -> Value {
        self.client
            .get(format!("{}{}", self.url, path))
            .header("X-Redmine-API-Key", &self.key)
            .send()
            .expect("request failed")
            .error_for_status()
            .expect("Redmine returned an error")
            .json()
            .expect("response is not JSON")
    }

    fn post(&self, path: &str, body: Value) -> Value {
        self.client
            .post(format!("{}{}", self.url, path))
            .header("X-Redmine-API-Key", &self.key)
            .json(&body)
            .send()
            .expect("request failed")
            .error_for_status()
            .expect("Redmine returned an error")
            .json()
            .expect("response is not JSON")
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
        let issue = self.post("/issues.json", json!({
            "issue": { "project_id": project_id, "subject": subject }
        }));
        issue["issue"]["id"].as_u64().expect("issue id")
    }

    fn issue(&self, id: u64) -> Value {
        self.get(&format!("/issues/{id}.json"))["issue"].clone()
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

    let (ok, out) = git_task(&repo, &["create", "Pushed from CI"]);
    assert!(ok, "create failed: {out}");

    let (ok, out) = git_task(&repo, &["push", "1"]);
    assert!(ok, "first push failed: {out}");

    let issues = redmine.issues_in_project(project);
    assert_eq!(issues.len(), 1, "exactly one issue must be created, got {issues:?}");
    let (remote_id, subject) = &issues[0];
    assert_eq!(subject, "Pushed from CI");
    assert_eq!(local_task_ids(&repo), vec![remote_id.to_string()], "the local task must adopt the Redmine issue id");

    let (ok, out) = git_task(&repo, &["push", &remote_id.to_string()]);
    assert!(ok, "second push failed: {out}");
    assert_eq!(redmine.issues_in_project(project).len(), 1, "a second push must not create another issue");
}

#[test]
fn push_never_touches_an_issue_from_another_project() {
    let redmine = redmine_or_skip!();
    let foreign_project = redmine.create_project("foreign");
    let own_project = redmine.create_project("own");
    let foreign_id = redmine.create_issue(foreign_project, "Foreign issue");
    let repo = local_repo(&redmine, own_project);

    let (ok, out) = git_task(&repo, &["create", "Own task"]);
    assert!(ok, "create failed: {out}");
    let (ok, out) = git_task(&repo, &["set", "1", "id", &foreign_id.to_string()]);
    assert!(ok, "renaming the local task failed: {out}");

    let (ok, out) = git_task(&repo, &["push", &foreign_id.to_string()]);
    assert!(ok, "push failed: {out}");

    let foreign = redmine.issue(foreign_id);
    assert_eq!(foreign["subject"], "Foreign issue", "the issue of another project must stay untouched");
    assert_eq!(foreign["project"]["id"], foreign_project, "the issue must stay in its own project");

    let own = redmine.issues_in_project(own_project);
    assert_eq!(own.len(), 1, "the task must be created in the configured project, got {own:?}");
    assert_eq!(own[0].1, "Own task");
    assert_ne!(own[0].0, foreign_id);
    assert_eq!(local_task_ids(&repo), vec![own[0].0.to_string()], "the local task must adopt the new issue id");
}
