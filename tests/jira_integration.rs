//! Integration tests against a real Jira Cloud site.
//!
//! They are skipped unless these variables are set:
//!
//! - `GIT_TASK_JIRA_URL`: the site, e.g. `https://example.atlassian.net`
//! - `GIT_TASK_JIRA_USER`: the e-mail of the account the token belongs to
//! - `GIT_TASK_JIRA_TOKEN`: an API token (falls back to `JIRA_TOKEN` or `JIRA_API_TOKEN`)
//! - `GIT_TASK_JIRA_PROJECT`: the key of a project the account may create and delete issues in
//!
//! Optional:
//!
//! - `GIT_TASK_JIRA_SECOND_PROJECT`: another project key, enables the multi-project tests
//! - `GIT_TASK_JIRA_STATUSES`: the open, in-progress and done status names of the project
//!   workflow, comma separated (default `To Do,In Progress,Done`)
//!
//! Every test creates its own issues with unique summaries and deletes them afterwards.
//! The Jira search API is eventually consistent, so tests wait until the issues they
//! created are visible to JQL before pulling a whole project.

use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;
use serde_json::{json, Value};

use gittask::testing::TestRepo;

const LABEL: &str = "ci-label";
const SEARCH_TIMEOUT: Duration = Duration::from_secs(90);

struct Jira {
    url: String,
    user: String,
    token: String,
    project: String,
    second_project: Option<String>,
    /// Jira status names in the order open, in progress, done.
    statuses: [String; 3],
    client: Client,
}

/// Deletes the registered issues when the test ends, even when it panics.
struct Cleanup<'a> {
    jira: &'a Jira,
    keys: Vec<String>,
}

impl<'a> Cleanup<'a> {
    fn new(jira: &'a Jira) -> Cleanup<'a> {
        Cleanup { jira, keys: vec![] }
    }

    fn track(&mut self, key: &str) {
        if !self.keys.contains(&key.to_string()) {
            self.keys.push(key.to_string());
        }
    }
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        for key in &self.keys {
            let _ = self.jira.send(reqwest::Method::DELETE, &format!("/rest/api/3/issue/{key}?deleteSubtasks=true"), None);
        }
    }
}

impl Jira {
    fn from_env() -> Option<Jira> {
        let url = std::env::var("GIT_TASK_JIRA_URL").ok()?;
        let user = std::env::var("GIT_TASK_JIRA_USER").ok()?;
        let token = std::env::var("GIT_TASK_JIRA_TOKEN")
            .or_else(|_| std::env::var("JIRA_TOKEN"))
            .or_else(|_| std::env::var("JIRA_API_TOKEN"))
            .ok()?;
        let project = std::env::var("GIT_TASK_JIRA_PROJECT").ok()?;
        if [&url, &user, &token, &project].iter().any(|v| v.trim().is_empty()) {
            return None;
        }
        let statuses = std::env::var("GIT_TASK_JIRA_STATUSES").unwrap_or_else(|_| "To Do,In Progress,Done".to_string());
        let statuses: Vec<String> = statuses.split(',').map(|s| s.trim().to_string()).collect();
        assert_eq!(statuses.len(), 3, "GIT_TASK_JIRA_STATUSES must list the open, in-progress and done status names");
        Some(Jira {
            url: url.trim().trim_end_matches('/').to_string(),
            user: user.trim().to_string(),
            token: token.trim().to_string(),
            project: project.trim().to_string(),
            second_project: std::env::var("GIT_TASK_JIRA_SECOND_PROJECT").ok().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()),
            statuses: [statuses[0].clone(), statuses[1].clone(), statuses[2].clone()],
            client: Client::builder().timeout(Duration::from_secs(60)).build().expect("http client"),
        })
    }

    fn open_status(&self) -> &str { &self.statuses[0] }
    fn in_progress_status(&self) -> &str { &self.statuses[1] }
    fn done_status(&self) -> &str { &self.statuses[2] }

    /// The local status name that `git task` uses for a Jira status: `To Do` becomes `TO_DO`.
    fn local_status(jira_status: &str) -> String {
        jira_status.trim().to_uppercase().replace([' ', '-'], "_")
    }

    fn send(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> reqwest::blocking::Response {
        let mut request = self
            .client
            .request(method, format!("{}{}", self.url, path))
            .basic_auth(&self.user, Some(&self.token))
            .header("Accept", "application/json");
        if let Some(body) = body {
            request = request.json(&body);
        }
        request.send().expect("request failed")
    }

    fn json(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Value {
        for attempt in 0..5 {
            let response = self.send(method.clone(), path, body.clone());
            let status = response.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt < 4 {
                sleep(Duration::from_secs(5 * (attempt + 1)));
                continue;
            }
            let text = response.text().expect("response body");
            assert!(status.is_success(), "{method} {path} failed with {status}: {text}");
            if text.trim().is_empty() {
                return Value::Null;
            }
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

    fn put(&self, path: &str, body: Value) {
        self.json(reqwest::Method::PUT, path, Some(body));
    }

    fn status_of(&self, path: &str) -> reqwest::StatusCode {
        self.send(reqwest::Method::GET, path, None).status()
    }

    fn create_issue(&self, project: &str, summary: &str, description: &str) -> String {
        self.create_issue_with_fields(project, summary, description, json!({}))
    }

    fn create_issue_with_fields(&self, project: &str, summary: &str, description: &str, extra: Value) -> String {
        let mut fields = json!({
            "project": { "key": project },
            "issuetype": { "name": "Task" },
            "summary": summary,
        });
        if !description.is_empty() {
            fields["description"] = paragraph_doc(description);
        }
        if let Value::Object(extra) = extra {
            for (name, value) in extra {
                fields[name] = value;
            }
        }
        let issue = self.post("/rest/api/3/issue", json!({ "fields": fields }));
        issue["key"].as_str().expect("issue key").to_string()
    }

    fn create_issue_with_adf(&self, project: &str, summary: &str, description: Value) -> String {
        let issue = self.post("/rest/api/3/issue", json!({ "fields": {
            "project": { "key": project },
            "issuetype": { "name": "Task" },
            "summary": summary,
            "description": description,
        }}));
        issue["key"].as_str().expect("issue key").to_string()
    }

    fn issue(&self, key: &str) -> Value {
        self.get(&format!("/rest/api/3/issue/{key}?fields=summary,description,status,labels,subtasks,parent,project"))["fields"].clone()
    }

    fn issue_status(&self, key: &str) -> String {
        self.issue(key)["status"]["name"].as_str().unwrap_or("").to_string()
    }

    fn issue_labels(&self, key: &str) -> Vec<String> {
        self.issue(key)["labels"].as_array().cloned().unwrap_or_default().iter().map(|l| l.as_str().unwrap_or("").to_string()).collect()
    }

    fn set_labels(&self, key: &str, labels: &[&str]) {
        self.put(&format!("/rest/api/3/issue/{key}"), json!({ "fields": { "labels": labels } }));
    }

    fn add_comment(&self, key: &str, text: &str) -> String {
        let comment = self.post(&format!("/rest/api/3/issue/{key}/comment"), json!({ "body": paragraph_doc(text) }));
        comment["id"].as_str().expect("comment id").to_string()
    }

    fn comments(&self, key: &str) -> Vec<(String, String)> {
        self.get(&format!("/rest/api/3/issue/{key}/comment"))["comments"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|c| (c["id"].as_str().unwrap_or("").to_string(), plain_text(&c["body"])))
            .collect()
    }

    fn transition(&self, key: &str, status_name: &str) {
        let transitions = self.get(&format!("/rest/api/3/issue/{key}/transitions"));
        let id = transitions["transitions"]
            .as_array()
            .expect("transitions")
            .iter()
            .find(|t| t["to"]["name"].as_str().is_some_and(|name| name.eq_ignore_ascii_case(status_name)))
            .and_then(|t| t["id"].as_str())
            .unwrap_or_else(|| panic!("no transition of {key} leads to {status_name}: {transitions}"))
            .to_string();
        self.post(&format!("/rest/api/3/issue/{key}/transitions"), json!({ "transition": { "id": id } }));
    }

    fn subtasks(&self, key: &str) -> Vec<(String, String, String)> {
        self.issue(key)["subtasks"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|s| (
                s["key"].as_str().unwrap_or("").to_string(),
                s["fields"]["summary"].as_str().unwrap_or("").to_string(),
                s["fields"]["status"]["name"].as_str().unwrap_or("").to_string(),
            ))
            .collect()
    }

    fn search(&self, jql: &str) -> Vec<String> {
        let mut keys = vec![];
        let mut next: Option<String> = None;
        loop {
            let mut path = format!("/rest/api/3/search/jql?jql={}&fields=summary&maxResults=100", urlencode(jql));
            if let Some(token) = &next {
                path.push_str(&format!("&nextPageToken={}", urlencode(token)));
            }
            let page = self.get(&path);
            keys.extend(page["issues"].as_array().cloned().unwrap_or_default().iter().filter_map(|i| i["key"].as_str().map(str::to_string)));
            next = page["nextPageToken"].as_str().map(str::to_string).filter(|t| !t.is_empty());
            if next.is_none() || page["isLast"].as_bool().unwrap_or(false) {
                return keys;
            }
        }
    }

    /// Waits until the JQL index has caught up with the given issues, which is what
    /// `git task pull` without ids depends on.
    fn wait_for_search(&self, keys: &[String]) {
        let jql = format!("key in ({})", keys.join(","));
        let started = Instant::now();
        loop {
            let found = self.search(&jql);
            if keys.iter().all(|key| found.contains(key)) {
                return;
            }
            assert!(started.elapsed() < SEARCH_TIMEOUT, "issues {keys:?} did not show up in JQL search within {SEARCH_TIMEOUT:?}, found {found:?}");
            sleep(Duration::from_secs(3));
        }
    }

    /// The number of issues whose summary is exactly `summary`, once the index has caught up with `known`.
    fn count_with_summary(&self, project: &str, summary: &str, known: &[String]) -> usize {
        self.wait_for_search(known);
        let jql = format!("project = {project} AND summary ~ \"\\\"{}\\\"\"", summary.replace('"', ""));
        let keys = self.search(&jql);
        keys.iter().filter(|key| self.issue(key)["summary"] == summary).count()
    }

    /// A custom numeric field that can be set when creating a Task in the project, if there is one.
    fn number_field(&self, project: &str) -> Option<(String, String)> {
        let types = self.get(&format!("/rest/api/3/issue/createmeta/{project}/issuetypes?maxResults=50"));
        let task_type = types["issueTypes"].as_array()?.iter().find(|t| t["name"] == "Task")?["id"].as_str()?.to_string();
        let fields = self.get(&format!("/rest/api/3/issue/createmeta/{project}/issuetypes/{task_type}?maxResults=200"));
        fields["fields"].as_array()?.iter().find_map(|f| {
            let id = f["fieldId"].as_str()?;
            if f["schema"]["type"] == "number" && id.starts_with("customfield_") {
                Some((id.to_string(), f["name"].as_str()?.to_string()))
            } else {
                None
            }
        })
    }
}

fn paragraph_doc(text: &str) -> Value {
    json!({ "type": "doc", "version": 1, "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": text }] }] })
}

fn plain_text(node: &Value) -> String {
    match node {
        Value::Object(map) => match map.get("text").and_then(Value::as_str) {
            Some(text) => text.to_string(),
            None => map.get("content").map(plain_text).unwrap_or_default(),
        },
        Value::Array(items) => items.iter().map(plain_text).collect::<Vec<_>>().join(""),
        _ => String::new(),
    }
}

fn node_types(node: &Value, out: &mut Vec<String>) {
    if let Some(kind) = node["type"].as_str() {
        out.push(kind.to_string());
    }
    for child in node["content"].as_array().into_iter().flatten() {
        node_types(child, out);
    }
    for mark in node["marks"].as_array().into_iter().flatten() {
        if let Some(kind) = mark["type"].as_str() {
            out.push(format!("mark:{kind}"));
        }
    }
}

fn urlencode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
    format!("{}-{nanos}", std::process::id())
}

/// The property name `git task` derives from a Jira field name: `Story point estimate` becomes `story_point_estimate`.
fn property_name(field_name: &str) -> String {
    let mut name = String::new();
    for c in field_name.chars() {
        if c.is_alphanumeric() {
            name.extend(c.to_lowercase());
        } else if !name.ends_with('_') && !name.is_empty() {
            name.push('_');
        }
    }
    name.trim_end_matches('_').to_string()
}

/// The numeric part of an issue key, which is the local task id in single-project mode.
fn number_of(key: &str) -> String {
    key.rsplit('-').next().unwrap_or(key).to_string()
}

fn local_repo(jira: &Jira) -> TestRepo {
    let repo = TestRepo::new();
    let git = git2::Repository::open(repo.path()).expect("open temp repo");
    git.remote("origin", "https://example.invalid/git-task/ci.git").expect("add remote");
    let mut config = git.config().expect("repo config");
    config.set_str("task.jira.url", &jira.url).expect("set url");
    config.set_str("task.jira.user", &jira.user).expect("set user");
    config.set_str("task.jira.project", &jira.project).expect("set project");
    config.set_str("task.default.connector", "jira").expect("set connector");
    let statuses = json!([
        { "name": Jira::local_status(jira.open_status()), "shortcut": "o", "color": "Red", "style": null, "is_done": false },
        { "name": Jira::local_status(jira.in_progress_status()), "shortcut": "i", "color": "Yellow", "style": null, "is_done": false },
        { "name": Jira::local_status(jira.done_status()), "shortcut": "d", "color": "Green", "style": null, "is_done": true },
    ]);
    config.set_str("task.statuses", &statuses.to_string()).expect("set statuses");
    repo
}

fn git_task(jira: &Jira, repo: &TestRepo, args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_git-task"))
        .current_dir(repo.path())
        .env("NO_COLOR", "1")
        .env("JIRA_TOKEN", &jira.token)
        .args(args)
        .output()
        .expect("failed to run git-task");
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

fn run(jira: &Jira, repo: &TestRepo, args: &[&str]) -> String {
    let (ok, out) = git_task(jira, repo, args);
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
    let entry = tree.get_name(id).unwrap_or_else(|| panic!("local task {id} not found among {:?}", local_task_ids(repo)));
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

macro_rules! jira_or_skip {
    () => {
        match Jira::from_env() {
            Some(jira) => jira,
            None => {
                eprintln!("skipped: set GIT_TASK_JIRA_URL, GIT_TASK_JIRA_USER, GIT_TASK_JIRA_TOKEN and GIT_TASK_JIRA_PROJECT to run Jira integration tests");
                return;
            }
        }
    };
}

macro_rules! second_project_or_skip {
    ($jira:expr) => {
        match &$jira.second_project {
            Some(project) => project.clone(),
            None => {
                eprintln!("skipped: set GIT_TASK_JIRA_SECOND_PROJECT to run the multi-project Jira tests");
                return;
            }
        }
    };
}

#[test]
fn push_creates_the_issue_once_and_adopts_its_number() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Pushed from CI {suffix}");
    let repo = local_repo(&jira);

    run(&jira, &repo, &["create", &title, "Pushed description"]);
    // A fresh local task is number 1, which an existing issue of the project would also be.
    run(&jira, &repo, &["set", "1", "id", "999999999"]);
    let out = run(&jira, &repo, &["push", "999999999"]);
    assert!(out.contains("Created REMOTE task ID"), "the first push must create the issue: {out}");

    let ids = local_task_ids(&repo);
    assert_eq!(ids.len(), 1, "the local task must be renamed, not duplicated: {ids:?}");
    let number = &ids[0];
    assert_ne!(number, "999999999", "the local task must adopt the issue number");
    assert!(number.chars().all(|c| c.is_ascii_digit()), "in single-project mode the local id is the issue number, got {number}");
    let key = format!("{}-{number}", jira.project);
    cleanup.track(&key);

    let issue = jira.issue(&key);
    assert_eq!(issue["summary"], title);
    assert_eq!(plain_text(&issue["description"]), "Pushed description");
    assert_eq!(issue["status"]["name"], jira.open_status(), "a new task must start in the first workflow status");

    let out = run(&jira, &repo, &["push", number]);
    assert!(!out.contains("Created REMOTE task ID"), "a second push must not create another issue: {out}");
    assert_eq!(local_task_ids(&repo), vec![number.clone()]);
    assert_eq!(jira.count_with_summary(&jira.project, &title, &[key.clone()]), 1, "exactly one issue must carry the summary");
}

#[test]
fn push_creates_a_done_task_as_a_done_issue() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Pushed done from CI {suffix}");
    let repo = local_repo(&jira);
    let done = Jira::local_status(jira.done_status());

    run(&jira, &repo, &["create", &title, "Done on creation"]);
    run(&jira, &repo, &["status", "1", &done]);
    run(&jira, &repo, &["set", "1", "id", "999999998"]);
    let out = run(&jira, &repo, &["push", "999999998"]);
    assert!(out.contains("Created REMOTE task ID"), "the first push must create the issue: {out}");

    let ids = local_task_ids(&repo);
    assert_eq!(ids.len(), 1);
    let key = format!("{}-{}", jira.project, ids[0]);
    cleanup.track(&key);
    assert_eq!(jira.issue(&key)["summary"], title);
    assert_eq!(jira.issue_status(&key), jira.done_status(), "a task pushed with a done status must be transitioned after creation");

    let out = run(&jira, &repo, &["push", &ids[0]]);
    assert!(out.contains("Nothing to sync"), "the done issue must already match the local task: {out}");
}

#[test]
fn pull_imports_the_issue_with_its_description_status_comment_and_label() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Pulled from CI {suffix}");
    let key = jira.create_issue(&jira.project, &title, "Some description");
    cleanup.track(&key);
    let comment_id = jira.add_comment(&key, "A real comment");
    jira.set_labels(&key, &[LABEL]);
    let repo = local_repo(&jira);
    let id = number_of(&key);

    run(&jira, &repo, &["pull", &id]);

    assert_eq!(local_task_ids(&repo), vec![id.clone()], "the local id must be the issue number");
    let task = local_task(&repo, &id);
    assert_eq!(task["props"]["name"], title);
    assert_eq!(task["props"]["description"], "Some description");
    assert_eq!(task["props"]["status"], jira.open_status(), "the Jira status name must be kept");
    assert!(task["props"]["author"].as_str().is_some_and(|a| !a.is_empty()), "the creator must be imported as the author");
    assert!(task["props"]["created"].as_str().is_some_and(|c| c.parse::<i64>().is_ok()), "the creation time must be a unix timestamp");
    assert_eq!(local_comments(&repo, &id), vec![(comment_id, "A real comment".to_string())], "the comment must carry the Jira comment id");
    assert_eq!(local_labels(&repo, &id), vec![LABEL.to_string()]);
}

#[test]
fn pull_accepts_the_full_issue_key_as_well() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Pulled by key from CI {suffix}");
    let key = jira.create_issue(&jira.project, &title, "");
    cleanup.track(&key);
    let repo = local_repo(&jira);

    run(&jira, &repo, &["pull", &key]);

    assert_eq!(local_task_ids(&repo), vec![number_of(&key)], "the task is stored under the issue number even when pulled by key");
    assert_eq!(local_task(&repo, &number_of(&key))["props"]["name"], title);
}

#[test]
fn rich_descriptions_round_trip_between_markdown_and_adf() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Rich text from CI {suffix}");
    let description = json!({ "type": "doc", "version": 1, "content": [
        { "type": "heading", "attrs": { "level": 2 }, "content": [{ "type": "text", "text": "Title" }] },
        { "type": "paragraph", "content": [
            { "type": "text", "text": "bold", "marks": [{ "type": "strong" }] },
            { "type": "text", "text": " and " },
            { "type": "text", "text": "code", "marks": [{ "type": "code" }] },
        ]},
        { "type": "bulletList", "content": [
            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
        ]},
        { "type": "codeBlock", "attrs": { "language": "rust" }, "content": [{ "type": "text", "text": "fn main() {}" }] },
    ]});
    let key = jira.create_issue_with_adf(&jira.project, &title, description);
    cleanup.track(&key);
    let repo = local_repo(&jira);
    let id = number_of(&key);

    run(&jira, &repo, &["pull", &id]);
    let markdown = local_task(&repo, &id)["props"]["description"].as_str().unwrap_or("").to_string();
    for fragment in ["## Title", "**bold** and `code`", "- one\n- two", "```rust\nfn main() {}\n```"] {
        assert!(markdown.contains(fragment), "pulled markdown must contain {fragment:?}, got:\n{markdown}");
    }

    let out = run(&jira, &repo, &["push", &id]);
    assert!(out.contains("Nothing to sync"), "an unchanged description must not be pushed back: {out}");

    let edited = format!("{markdown}\n\n1. first\n2. second\n\n> quoted *em*");
    run(&jira, &repo, &["set", &id, "description", &edited]);
    run(&jira, &repo, &["push", &id]);

    let remote = jira.issue(&key)["description"].clone();
    let mut kinds = vec![];
    node_types(&remote, &mut kinds);
    for kind in ["heading", "bulletList", "orderedList", "codeBlock", "blockquote", "mark:strong", "mark:code", "mark:em"] {
        assert!(kinds.contains(&kind.to_string()), "pushed ADF must contain {kind}, got {kinds:?}");
    }

    run(&jira, &repo, &["pull", &id]);
    assert_eq!(local_task(&repo, &id)["props"]["description"], edited, "the edited markdown must survive a push and pull");
    let out = run(&jira, &repo, &["push", &id]);
    assert!(out.contains("Nothing to sync"), "the round-tripped description must compare equal: {out}");
}

#[test]
fn pull_without_ids_imports_the_issues_of_the_project() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let first = jira.create_issue(&jira.project, &format!("First issue {suffix}"), "");
    cleanup.track(&first);
    let second = jira.create_issue(&jira.project, &format!("Second issue {suffix}"), "");
    cleanup.track(&second);
    jira.wait_for_search(&[first.clone(), second.clone()]);
    let repo = local_repo(&jira);

    run(&jira, &repo, &["pull"]);

    let ids = local_task_ids(&repo);
    for key in [&first, &second] {
        assert!(ids.contains(&number_of(key)), "{key} must be imported by a plain pull, got {ids:?}");
    }
    assert!(ids.iter().all(|id| id.chars().all(|c| c.is_ascii_digit())), "every imported id must be an issue number: {ids:?}");
    assert_eq!(local_task(&repo, &number_of(&first))["props"]["name"], format!("First issue {suffix}"));

    let limited = local_repo(&jira);
    run(&jira, &limited, &["pull", "--limit", "1"]);
    assert_eq!(local_task_ids(&limited).len(), 1, "--limit must cap the number of imported issues");
}

#[test]
fn pull_open_imports_only_open_issues() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let open = jira.create_issue(&jira.project, &format!("Open issue {suffix}"), "");
    cleanup.track(&open);
    let in_progress = jira.create_issue(&jira.project, &format!("In progress issue {suffix}"), "");
    cleanup.track(&in_progress);
    jira.transition(&in_progress, jira.in_progress_status());
    let done = jira.create_issue(&jira.project, &format!("Done issue {suffix}"), "");
    cleanup.track(&done);
    jira.transition(&done, jira.done_status());
    jira.wait_for_search(&[open.clone(), in_progress.clone(), done.clone()]);
    let repo = local_repo(&jira);

    run(&jira, &repo, &["pull", "--open"]);

    let ids = local_task_ids(&repo);
    assert!(ids.contains(&number_of(&open)), "{open} is open and must be imported, got {ids:?}");
    assert!(ids.contains(&number_of(&in_progress)), "{in_progress} is not done and must be imported, got {ids:?}");
    assert!(!ids.contains(&number_of(&done)), "{done} is done and must be skipped by --open");
    assert_eq!(local_task(&repo, &number_of(&in_progress))["props"]["status"], jira.in_progress_status());
}

#[test]
fn pull_status_imports_only_issues_in_that_status() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let open = jira.create_issue(&jira.project, &format!("Still open {suffix}"), "");
    cleanup.track(&open);
    let in_progress = jira.create_issue(&jira.project, &format!("Being worked on {suffix}"), "");
    cleanup.track(&in_progress);
    jira.transition(&in_progress, jira.in_progress_status());
    let done = jira.create_issue(&jira.project, &format!("Already done {suffix}"), "");
    cleanup.track(&done);
    jira.transition(&done, jira.done_status());
    jira.wait_for_search(&[open.clone(), in_progress.clone(), done.clone()]);

    let repo = local_repo(&jira);
    run(&jira, &repo, &["pull", "--status", &Jira::local_status(jira.in_progress_status())]);
    let ids = local_task_ids(&repo);
    assert!(ids.contains(&number_of(&in_progress)), "{in_progress} must be imported, got {ids:?}");
    assert!(!ids.contains(&number_of(&open)), "{open} is in another status and must be skipped");
    assert!(!ids.contains(&number_of(&done)), "{done} is in another status and must be skipped");

    let repo = local_repo(&jira);
    run(&jira, &repo, &["pull", "--status", "d"]);
    let ids = local_task_ids(&repo);
    assert!(ids.contains(&number_of(&done)), "a status shortcut must select the done issues, got {ids:?}");
    assert!(!ids.contains(&number_of(&open)));
    assert!(!ids.contains(&number_of(&in_progress)));
}

#[test]
fn push_syncs_local_edits_comments_labels_subtasks_and_status() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Synced from CI {suffix}");
    let key = jira.create_issue(&jira.project, &title, "Original description");
    cleanup.track(&key);
    let repo = local_repo(&jira);
    let id = number_of(&key);
    let renamed = format!("Renamed from CI {suffix}");
    let subtask_title = format!("Subtask added locally {suffix}");

    run(&jira, &repo, &["pull", &id]);
    run(&jira, &repo, &["set", &id, "name", &renamed]);
    run(&jira, &repo, &["set", &id, "description", "Edited description"]);
    run(&jira, &repo, &["comment", "add", &id, "Comment added locally"]);
    run(&jira, &repo, &["label", "add", &id, LABEL]);
    run(&jira, &repo, &["subtask", "add", &id, &subtask_title]);
    run(&jira, &repo, &["status", &id, &Jira::local_status(jira.in_progress_status())]);
    run(&jira, &repo, &["push", &id]);

    let issue = jira.issue(&key);
    assert_eq!(issue["summary"], renamed);
    assert_eq!(plain_text(&issue["description"]), "Edited description");
    assert_eq!(issue["status"]["name"], jira.in_progress_status(), "the local status must be pushed as a transition");
    assert_eq!(jira.issue_labels(&key), vec![LABEL.to_string()]);

    let comments = jira.comments(&key);
    assert_eq!(comments.len(), 1, "the local comment must be created on the issue");
    assert_eq!(comments[0].1, "Comment added locally");
    assert_eq!(local_comments(&repo, &id), vec![(comments[0].0.clone(), "Comment added locally".to_string())], "the local comment must adopt the Jira comment id");

    let subtasks = jira.subtasks(&key);
    assert_eq!(subtasks.len(), 1, "the local subtask must be created as a Jira sub-task");
    assert_eq!(subtasks[0].1, subtask_title);
    assert_eq!(subtasks[0].2, jira.open_status());
    let sub_key = subtasks[0].0.clone();
    let sub_id = number_of(&sub_key);
    assert_eq!(local_subtasks(&repo, &id).iter().map(|s| (s.0.clone(), s.1.clone())).collect::<Vec<_>>(), vec![(sub_id.clone(), subtask_title.clone())], "the local subtask must adopt the sub-task number");

    run(&jira, &repo, &["push", &id]);
    assert_eq!(jira.comments(&key).len(), 1, "a repeated push must not duplicate the comment");
    assert_eq!(jira.subtasks(&key).len(), 1, "a repeated push must not duplicate the sub-task");

    let done = Jira::local_status(jira.done_status());
    run(&jira, &repo, &["subtask", "set", &id, &sub_id, "status", &done]);
    run(&jira, &repo, &["push", &id]);
    assert_eq!(jira.issue_status(&sub_key), jira.done_status(), "push must transition the existing sub-task");

    run(&jira, &repo, &["status", &id, &done]);
    run(&jira, &repo, &["push", &id]);
    assert_eq!(jira.issue_status(&key), jira.done_status(), "the local done status must transition the issue");

    let out = run(&jira, &repo, &["push", &id]);
    assert!(out.contains("Nothing to sync"), "everything must be in sync after the pushes: {out}");
}

#[test]
fn pull_updates_an_existing_task_from_remote_changes() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Changed remotely {suffix}");
    let key = jira.create_issue(&jira.project, &title, "Before");
    cleanup.track(&key);
    let repo = local_repo(&jira);
    let id = number_of(&key);

    run(&jira, &repo, &["pull", &id]);
    assert_eq!(local_task(&repo, &id)["props"]["description"], "Before");

    let renamed = format!("Renamed remotely {suffix}");
    jira.put(&format!("/rest/api/3/issue/{key}"), json!({ "fields": { "summary": renamed, "description": paragraph_doc("After") } }));
    jira.transition(&key, jira.done_status());
    let comment_id = jira.add_comment(&key, "Remote comment");

    run(&jira, &repo, &["pull", &id]);
    let task = local_task(&repo, &id);
    assert_eq!(task["props"]["name"], renamed);
    assert_eq!(task["props"]["description"], "After");
    assert_eq!(task["props"]["status"], jira.done_status());
    assert_eq!(local_comments(&repo, &id), vec![(comment_id, "Remote comment".to_string())]);

    let out = run(&jira, &repo, &["push", &id]);
    assert!(out.contains("Nothing to sync"), "a freshly pulled task must not push anything: {out}");
}

#[test]
fn comment_commands_push_to_the_remote() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Commented from CI {suffix}");
    let key = jira.create_issue(&jira.project, &title, "");
    cleanup.track(&key);
    let repo = local_repo(&jira);
    let id = number_of(&key);

    run(&jira, &repo, &["pull", &id]);
    run(&jira, &repo, &["comment", "add", &id, "First version", "--push"]);

    let comments = jira.comments(&key);
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].1, "First version");
    let comment_id = comments[0].0.clone();
    assert_eq!(local_comments(&repo, &id), vec![(comment_id.clone(), "First version".to_string())]);

    run(&jira, &repo, &["comment", "set", &id, &comment_id, "Second version", "--push"]);
    let comments = jira.comments(&key);
    assert_eq!(comments.len(), 1, "editing must not add a comment");
    assert_eq!(comments[0], (comment_id.clone(), "Second version".to_string()));
    assert_eq!(local_comments(&repo, &id), vec![(comment_id.clone(), "Second version".to_string())]);

    run(&jira, &repo, &["comment", "delete", &id, &comment_id, "--push"]);
    assert!(jira.comments(&key).is_empty(), "the comment must be deleted from the issue");
    assert!(local_comments(&repo, &id).is_empty());
}

#[test]
fn label_commands_push_to_the_remote() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Labeled from CI {suffix}");
    let key = jira.create_issue(&jira.project, &title, "With labels");
    cleanup.track(&key);
    let repo = local_repo(&jira);
    let id = number_of(&key);

    run(&jira, &repo, &["pull", &id]);
    assert!(local_labels(&repo, &id).is_empty());

    run(&jira, &repo, &["label", "add", &id, LABEL, "--push"]);
    assert_eq!(jira.issue_labels(&key), vec![LABEL.to_string()]);
    assert_eq!(local_labels(&repo, &id), vec![LABEL.to_string()]);

    run(&jira, &repo, &["label", "delete", &id, LABEL, "--push"]);
    assert!(jira.issue_labels(&key).is_empty(), "the label must be removed from the issue");
    assert!(local_labels(&repo, &id).is_empty());
}

#[test]
fn pull_updates_the_labels_of_an_existing_task() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Relabeled from CI {suffix}");
    let key = jira.create_issue(&jira.project, &title, "Labels change remotely");
    cleanup.track(&key);
    let repo = local_repo(&jira);
    let id = number_of(&key);

    run(&jira, &repo, &["pull", &id]);
    assert!(local_labels(&repo, &id).is_empty());

    jira.set_labels(&key, &[LABEL]);
    run(&jira, &repo, &["pull", &id]);
    assert_eq!(local_labels(&repo, &id), vec![LABEL.to_string()], "a label added in Jira must reach the existing local task");

    jira.set_labels(&key, &[]);
    run(&jira, &repo, &["pull", &id]);
    assert!(local_labels(&repo, &id).is_empty(), "a label removed in Jira must disappear locally");
}

#[test]
fn push_syncs_label_only_changes() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Label pushed from CI {suffix}");
    let key = jira.create_issue(&jira.project, &title, "Labels change locally");
    cleanup.track(&key);
    let repo = local_repo(&jira);
    let id = number_of(&key);

    run(&jira, &repo, &["pull", &id]);
    run(&jira, &repo, &["label", "add", &id, LABEL]);
    run(&jira, &repo, &["push", &id]);
    assert_eq!(jira.issue_labels(&key), vec![LABEL.to_string()], "a label added locally must be pushed without other changes");

    run(&jira, &repo, &["label", "delete", &id, LABEL]);
    run(&jira, &repo, &["push", &id]);
    assert!(jira.issue_labels(&key).is_empty(), "a label removed locally must be removed by push");
}

#[test]
fn subtask_commands_push_to_the_remote() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Parent from CI {suffix}");
    let key = jira.create_issue(&jira.project, &title, "With subtasks");
    cleanup.track(&key);
    let repo = local_repo(&jira);
    let id = number_of(&key);
    let subtask_title = format!("Child from CI {suffix}");
    let renamed_title = format!("Renamed child from CI {suffix}");
    let done = Jira::local_status(jira.done_status());

    run(&jira, &repo, &["pull", &id]);
    run(&jira, &repo, &["subtask", "add", &id, &subtask_title, "--push"]);

    let subs = jira.subtasks(&key);
    assert_eq!(subs.len(), 1, "the subtask must be created as a sub-task issue");
    assert_eq!(subs[0].1, subtask_title);
    assert_eq!(subs[0].2, jira.open_status());
    let sub_key = subs[0].0.clone();
    let sub_id = number_of(&sub_key);
    let sub_fields = jira.issue(&sub_key);
    assert_eq!(sub_fields["parent"]["key"], key, "the sub-task must point at the parent");
    assert_eq!(sub_fields["project"]["key"], jira.project, "the sub-task must live in the parent's project");
    let local = local_subtasks(&repo, &id);
    assert_eq!(local.len(), 1);
    assert_eq!(local[0].0, sub_id, "the local subtask must adopt the sub-task number");
    assert_eq!(local[0].1, subtask_title);

    run(&jira, &repo, &["subtask", "set", &id, &sub_id, "name", &renamed_title, "--push"]);
    assert_eq!(jira.issue(&sub_key)["summary"], renamed_title);

    run(&jira, &repo, &["subtask", "set", &id, &sub_id, "status", &done, "--push"]);
    assert_eq!(jira.issue_status(&sub_key), jira.done_status());

    run(&jira, &repo, &["pull", &id]);
    assert_eq!(local_subtasks(&repo, &id), vec![(sub_id.clone(), renamed_title.clone(), jira.done_status().to_string())], "pull must bring back the sub-task with its Jira status");

    run(&jira, &repo, &["subtask", "delete", &id, &sub_id, "--push"]);
    assert!(jira.subtasks(&key).is_empty(), "the sub-task must be deleted");
    assert_eq!(jira.status_of(&format!("/rest/api/3/issue/{sub_key}")), reqwest::StatusCode::NOT_FOUND);
    assert!(local_subtasks(&repo, &id).is_empty());

    let done_title = format!("Done child from CI {suffix}");
    run(&jira, &repo, &["subtask", "add", &id, &done_title, "--status", &done, "--push"]);
    let subs = jira.subtasks(&key);
    assert_eq!(subs.len(), 1, "the done subtask must be created as a sub-task");
    assert_eq!(subs[0].2, jira.done_status(), "a subtask added with a done status must be transitioned after creation");
}

#[test]
fn delete_with_push_removes_the_remote_issue() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Deleted from CI {suffix}");
    let key = jira.create_issue(&jira.project, &title, "");
    cleanup.track(&key);
    let repo = local_repo(&jira);
    let id = number_of(&key);

    run(&jira, &repo, &["pull", &id]);
    run(&jira, &repo, &["delete", &id, "--push"]);

    assert!(local_task_ids(&repo).is_empty());
    assert_eq!(jira.status_of(&format!("/rest/api/3/issue/{key}")), reqwest::StatusCode::NOT_FOUND, "the remote issue must be deleted");
}

#[test]
fn custom_fields_are_discovered_and_synced_as_properties() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let Some((field_id, field_name)) = jira.number_field(&jira.project) else {
        eprintln!("skipped: project {} has no numeric custom field on the Task create screen", jira.project);
        return;
    };
    let property = property_name(&field_name);
    let suffix = unique_suffix();
    let title = format!("Custom field from CI {suffix}");
    let key = jira.create_issue_with_fields(&jira.project, &title, "", json!({ field_id.clone(): 5 }));
    cleanup.track(&key);
    jira.wait_for_search(&[key.clone()]);
    let repo = local_repo(&jira);
    let id = number_of(&key);

    let out = run(&jira, &repo, &["config", "fields", "list"]);
    let line = out.lines().find(|l| l.contains(&field_id)).unwrap_or_else(|| panic!("{field_id} must be listed as in use:\n{out}"));
    assert!(line.starts_with(&property), "the property name must come from the field name {field_name:?}: {line}");
    assert!(line.contains(&format!("{field_id}:number")), "the field must be typed from its schema: {line}");

    run(&jira, &repo, &["pull", &id]);
    assert_eq!(local_task(&repo, &id)["props"][&property], "5", "the field value must be pulled into the property");

    run(&jira, &repo, &["set", &id, &property, "8"]);
    run(&jira, &repo, &["push", &id]);
    assert_eq!(jira.get(&format!("/rest/api/3/issue/{key}?fields={field_id}"))["fields"][&field_id], json!(8.0), "the changed property must be written back to the field");

    let out = run(&jira, &repo, &["push", &id]);
    assert!(out.contains("Nothing to sync"), "an unchanged property must not be pushed: {out}");

    let out = run(&jira, &repo, &["config", "fields", "save"]);
    assert!(out.contains("task.jira.fields has been set"), "save must write the mapping: {out}");
    let saved = run(&jira, &repo, &["config", "get", "task.jira.fields"]);
    assert!(saved.contains(&format!("{property}={field_id}:number")), "the saved mapping must pin the field: {saved}");

    run(&jira, &repo, &["set", &id, &property, "13"]);
    run(&jira, &repo, &["push", &id]);
    assert_eq!(jira.get(&format!("/rest/api/3/issue/{key}?fields={field_id}"))["fields"][&field_id], json!(13.0), "a pinned mapping must still write the field");

    let created_title = format!("Created with custom field {suffix}");
    run(&jira, &repo, &["create", &created_title, "Carries a field"]);
    let local_id = local_task_ids(&repo).into_iter().find(|i| i != &id).expect("the created task id");
    run(&jira, &repo, &["set", &local_id, &property, "21"]);
    run(&jira, &repo, &["push", &local_id]);
    let created_id = local_task_ids(&repo).into_iter().find(|i| i != &id).expect("the pushed task id");
    let created_key = format!("{}-{created_id}", jira.project);
    cleanup.track(&created_key);
    assert_eq!(jira.get(&format!("/rest/api/3/issue/{created_key}?fields={field_id}"))["fields"][&field_id], json!(21.0), "a property set before the first push must be sent on creation");
}

#[test]
fn several_projects_sync_with_full_issue_keys() {
    let jira = jira_or_skip!();
    let second = second_project_or_skip!(jira);
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let first_key = jira.create_issue(&jira.project, &format!("First project issue {suffix}"), "");
    cleanup.track(&first_key);
    let second_key = jira.create_issue(&second, &format!("Second project issue {suffix}"), "");
    cleanup.track(&second_key);

    let repo = local_repo(&jira);
    run(&jira, &repo, &["config", "set", "task.jira.projects", &format!("{},{second}", jira.project)]);

    run(&jira, &repo, &["pull", &first_key]);
    run(&jira, &repo, &["pull", &second_key]);
    let mut ids = local_task_ids(&repo);
    ids.sort();
    let mut expected = vec![first_key.clone(), second_key.clone()];
    expected.sort();
    assert_eq!(ids, expected, "in multi-project mode the local ids are full issue keys");

    run(&jira, &repo, &["set", &second_key, "description", "Edited in the second project"]);
    run(&jira, &repo, &["comment", "add", &second_key, "Comment in the second project"]);
    run(&jira, &repo, &["push", &second_key]);
    assert_eq!(plain_text(&jira.issue(&second_key)["description"]), "Edited in the second project", "the project must be derived from the key");
    assert_eq!(jira.comments(&second_key).len(), 1);

    let created_title = format!("Created in the second project {suffix}");
    run(&jira, &repo, &["create", &created_title, "Via --project", "--project", &second, "--push"]);
    let created = local_task_ids(&repo).into_iter().find(|id| id.starts_with(&format!("{second}-")) && id != &second_key).expect("the created issue key");
    cleanup.track(&created);
    assert_eq!(jira.issue(&created)["summary"], created_title);
    assert_eq!(jira.issue(&created)["project"]["key"], second);

    let default_title = format!("Created in the default project {suffix}");
    run(&jira, &repo, &["create", &default_title, "Without --project", "--push"]);
    let defaulted = local_task_ids(&repo).into_iter().find(|id| id.starts_with(&format!("{}-", jira.project)) && id != &first_key).expect("the created issue key");
    cleanup.track(&defaulted);
    assert_eq!(jira.issue(&defaulted)["project"]["key"], jira.project, "without --project the task goes to the default project");

    jira.wait_for_search(&[first_key.clone(), second_key.clone()]);
    let only_second = local_repo(&jira);
    run(&jira, &only_second, &["config", "set", "task.jira.projects", &format!("{},{second}", jira.project)]);
    run(&jira, &only_second, &["pull", "--project", &second]);
    let ids = local_task_ids(&only_second);
    assert!(ids.contains(&second_key), "--project must pull the selected project, got {ids:?}");
    assert!(!ids.contains(&first_key), "--project must not pull the other project");
    assert!(ids.iter().all(|id| id.starts_with(&format!("{second}-"))), "every id must belong to the selected project: {ids:?}");
}

#[test]
fn renaming_tasks_when_enabling_several_projects_prefixes_the_keys() {
    let jira = jira_or_skip!();
    let second = second_project_or_skip!(jira);
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let key = jira.create_issue(&jira.project, &format!("Renamed into a key {suffix}"), "Before the switch");
    cleanup.track(&key);
    let repo = local_repo(&jira);
    let id = number_of(&key);

    run(&jira, &repo, &["pull", &id]);
    assert_eq!(local_task_ids(&repo), vec![id.clone()]);

    run(&jira, &repo, &["config", "set", "task.jira.projects", &format!("{},{second}", jira.project), "--rename-tasks"]);
    assert_eq!(local_task_ids(&repo), vec![key.clone()], "--rename-tasks must prefix the numeric ids with the default project");

    run(&jira, &repo, &["set", &key, "description", "After the switch"]);
    run(&jira, &repo, &["push", &key]);
    assert_eq!(plain_text(&jira.issue(&key)["description"]), "After the switch", "the renamed task must keep syncing the same issue");
}

impl Jira {
    fn account_id(&self) -> String {
        self.get("/rest/api/3/myself")["accountId"].as_str().expect("accountId").to_string()
    }

    fn description_adf(&self, key: &str) -> Value {
        self.issue(key)["description"].clone()
    }
}

/// Pushes `markdown` as the description of a new issue and returns its key.
fn push_description(jira: &Jira, repo: &TestRepo, title: &str, markdown: &str) -> String {
    run(jira, repo, &["create", title, markdown]);
    let local_id = local_task_ids(repo).into_iter().max_by_key(|id| id.parse::<u64>().unwrap_or(0)).expect("the created task");
    let placeholder = format!("99999{local_id}");
    run(jira, repo, &["set", &local_id, "id", &placeholder]);
    let out = run(jira, repo, &["push", &placeholder]);
    assert!(out.contains("Created REMOTE task ID"), "the push must create the issue: {out}");
    let number = local_task_ids(repo).into_iter().find(|id| id != &placeholder && local_task(repo, id)["props"]["name"] == title).expect("the adopted issue number");
    format!("{}-{number}", jira.project)
}

#[test]
fn every_markdown_construct_survives_a_push_and_a_pull() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Every construct from CI {suffix}");
    let markdown = "\
# Title

Intro with **bold**, *italic*, ~~struck~~, `code`, a [link](https://example.com/path?q=1) and **bold *and italic* inside**.
Second line after a hard break.

## Lists

- bullet
  - nested child
- second parent

  continuation paragraph

3. third
4. fourth

- [x] done task
- [ ] open task
  - [ ] nested task

> quoted
> text
>
> - quoted bullet

```rust
fn main() {}
```

| Name | Code |
| --- | --- |
| a\\| b | `x` |
| **bold** | plain |

---

The end.";
    let repo = local_repo(&jira);
    let key = push_description(&jira, &repo, &title, markdown);
    cleanup.track(&key);

    // Jira accepted the document and kept every node type.
    let mut kinds = vec![];
    node_types(&jira.description_adf(&key), &mut kinds);
    for kind in ["heading", "paragraph", "hardBreak", "bulletList", "orderedList", "taskList", "taskItem", "blockquote", "codeBlock", "table", "tableHeader", "tableCell", "rule",
                 "mark:strong", "mark:em", "mark:strike", "mark:code", "mark:link"] {
        assert!(kinds.contains(&kind.to_string()), "Jira must hold a {kind} node, got {kinds:?}");
    }

    // A fresh clone pulls the same Markdown back, and a push from it has nothing to send.
    let other = local_repo(&jira);
    let id = number_of(&key);
    run(&jira, &other, &["pull", &id]);
    assert_eq!(local_task(&other, &id)["props"]["description"], markdown, "the description must survive Jira unchanged");
    let out = run(&jira, &other, &["push", &id]);
    assert!(out.contains("Nothing to sync"), "what Jira stored must compare equal to the Markdown: {out}");

    // The original repository agrees with Jira too.
    let out = run(&jira, &repo, &["push", &id]);
    assert!(out.contains("Nothing to sync"), "the pushed Markdown must compare equal to what Jira stored: {out}");
}

#[test]
fn jira_only_nodes_pull_as_readable_markdown_and_push_back_as_valid_adf() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Jira nodes from CI {suffix}");
    let account_id = jira.account_id();
    let para = |nodes: Vec<Value>| json!({ "type": "paragraph", "content": nodes });
    let text = |t: &str| json!({ "type": "text", "text": t });
    let task = |state: &str, t: &str, local: &str| json!({ "type": "taskItem", "attrs": { "localId": local, "state": state }, "content": [text(t)] });
    let cell = |kind: &str, attrs: Value, t: &str| json!({ "type": kind, "attrs": attrs, "content": [para(vec![text(t)])] });
    let description = json!({ "type": "doc", "version": 1, "content": [
        { "type": "panel", "attrs": { "panelType": "info" }, "content": [
            para(vec![text("Checklist")]),
            { "type": "taskList", "attrs": { "localId": "list-1" }, "content": [
                task("DONE", "Finished", "item-1"),
                task("TODO", "Remaining", "item-2"),
                { "type": "taskList", "attrs": { "localId": "list-2" }, "content": [task("TODO", "Nested", "item-3")] },
            ]},
        ]},
        { "type": "expand", "attrs": { "title": " Details " }, "content": [para(vec![text("hidden text")])] },
        { "type": "table", "attrs": { "isNumberColumnEnabled": false, "layout": "default" }, "content": [
            { "type": "tableRow", "content": [cell("tableHeader", json!({ "colspan": 2 }), "wide header")] },
            { "type": "tableRow", "content": [cell("tableCell", json!({ "rowspan": 2 }), "tall"), cell("tableCell", json!({}), "r1")] },
            { "type": "tableRow", "content": [cell("tableCell", json!({}), "r2")] },
        ]},
        para(vec![
            text("Ask "),
            json!({ "type": "mention", "attrs": { "id": account_id } }),
            text(" by "),
            json!({ "type": "date", "attrs": { "timestamp": "1700000000000" } }),
            text(" "),
            json!({ "type": "status", "attrs": { "text": "BLOCKED", "color": "red", "localId": "status-1" } }),
            text(" "),
            json!({ "type": "emoji", "attrs": { "shortName": ":tada:" } }),
        ]),
        { "type": "rule" },
        { "type": "heading", "attrs": { "level": 2.0 }, "content": [text("Float level")] },
        { "type": "orderedList", "attrs": { "order": 2.0 }, "content": [
            { "type": "listItem", "content": [para(vec![text("second")])] },
        ]},
    ]});
    let key = jira.create_issue_with_adf(&jira.project, &title, description);
    cleanup.track(&key);
    let repo = local_repo(&jira);
    let id = number_of(&key);

    run(&jira, &repo, &["pull", &id]);
    let pulled = local_task(&repo, &id)["props"]["description"].as_str().unwrap_or("").to_string();
    for fragment in ["> Checklist", "> - [x] Finished", "> - [ ] Remaining", ">   - [ ] Nested", "**Details**", "hidden text",
                     "| wide header |", "| tall | r1 |", "| r2 |", "Ask @", "1700000000000", "BLOCKED", "---", "## Float level", "2. second"] {
        assert!(pulled.contains(fragment), "pulled Markdown must contain {fragment:?}, got:\n{pulled}");
    }
    assert!(pulled.contains(":tada:") || pulled.contains('\u{1F389}'), "the emoji must be kept in some form: {pulled}");
    assert!(!pulled.contains("** Details **"), "whitespace must stay outside the title delimiters: {pulled}");

    // Pushing an unrelated edit sends the flattened document back; Jira must accept it
    // and the next pull must agree with the local text.
    let edited = format!("{pulled}\n\nAppended locally.");
    run(&jira, &repo, &["set", &id, "description", &edited]);
    run(&jira, &repo, &["push", &id]);
    let mut kinds = vec![];
    node_types(&jira.description_adf(&key), &mut kinds);
    for kind in ["blockquote", "bulletList", "table", "mark:strong", "orderedList", "rule"] {
        assert!(kinds.contains(&kind.to_string()), "the flattened document must keep a {kind}, got {kinds:?}");
    }
    assert!(!kinds.contains(&"panel".to_string()) && !kinds.contains(&"expand".to_string()), "Jira-only containers are flattened on push: {kinds:?}");

    run(&jira, &repo, &["pull", &id]);
    assert_eq!(local_task(&repo, &id)["props"]["description"], edited, "the flattened document must pull back as the same Markdown");
    let out = run(&jira, &repo, &["push", &id]);
    assert!(out.contains("Nothing to sync"), "the round-tripped description must compare equal: {out}");
}

#[test]
fn deeply_nested_descriptions_stay_readable_through_jira() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let cases = [
        ("bullets", format!("{}x", "- ".repeat(200))),
        ("quotes", format!("{}x", "> ".repeat(200))),
        ("indented", (0..100).map(|k| format!("{}- item {k}", "  ".repeat(k))).collect::<Vec<_>>().join("\n")),
        ("mixed", (0..60).map(|k| format!("{}- > **deep** [l](https://example.com)", "    ".repeat(k))).collect::<Vec<_>>().join("\n")),
    ];
    for (name, markdown) in &cases {
        let title = format!("Nested {name} from CI {suffix}");
        let repo = local_repo(&jira);
        let key = push_description(&jira, &repo, &title, markdown);
        cleanup.track(&key);
        let id = number_of(&key);

        // The pull reads the document inside Jira's response wrapper, which is where
        // an over-deep document fails to deserialize.
        let other = local_repo(&jira);
        run(&jira, &other, &["pull", &id]);
        let pulled = local_task(&other, &id)["props"]["description"].as_str().unwrap_or("").to_string();
        let expected_tail = match *name { "indented" => "item 99", "mixed" => "deep", _ => "x" };
        assert!(pulled.contains(expected_tail), "{name}: the deepest content must be kept: {pulled}");

        let out = run(&jira, &other, &["push", &id]);
        assert!(out.contains("Nothing to sync"), "{name}: what Jira stored must be stable: {out}");
        run(&jira, &other, &["pull", &id]);
        assert_eq!(local_task(&other, &id)["props"]["description"], pulled, "{name}: a second pull must not change the text");
    }
}

#[test]
fn rich_text_comments_round_trip_through_jira() {
    let jira = jira_or_skip!();
    let mut cleanup = Cleanup::new(&jira);
    let suffix = unique_suffix();
    let title = format!("Rich comments from CI {suffix}");
    let key = jira.create_issue(&jira.project, &title, "");
    cleanup.track(&key);
    let repo = local_repo(&jira);
    let id = number_of(&key);
    let comment = "**Bold** start with `code` and a [link](https://example.com/):\n\n- one\n- two\n\n```sh\necho hi\n```\n\n> quoted";

    run(&jira, &repo, &["pull", &id]);
    run(&jira, &repo, &["comment", "add", &id, comment, "--push"]);

    let remote = jira.get(&format!("/rest/api/3/issue/{key}/comment"))["comments"].clone();
    assert_eq!(remote.as_array().map(Vec::len), Some(1));
    let mut kinds = vec![];
    node_types(&remote[0]["body"], &mut kinds);
    for kind in ["bulletList", "codeBlock", "blockquote", "mark:strong", "mark:code", "mark:link"] {
        assert!(kinds.contains(&kind.to_string()), "the comment must reach Jira as rich text with a {kind}, got {kinds:?}");
    }
    let comment_id = remote[0]["id"].as_str().expect("comment id").to_string();

    let other = local_repo(&jira);
    run(&jira, &other, &["pull", &id]);
    assert_eq!(local_comments(&other, &id), vec![(comment_id.clone(), comment.to_string())], "the comment must pull back as the same Markdown");

    let edited = format!("{comment}\n\n1. numbered");
    run(&jira, &other, &["comment", "set", &id, &comment_id, &edited, "--push"]);
    run(&jira, &repo, &["pull", &id]);
    assert_eq!(local_comments(&repo, &id), vec![(comment_id, edited)], "an edited rich comment must round-trip as well");
}
