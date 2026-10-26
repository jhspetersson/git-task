use std::borrow::ToOwned;
use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::time::{SystemTime, UNIX_EPOCH};
use git2::*;
use serde_json;
use serde::{Deserialize, Serialize};

const NAME: &'static str = "name";
const DESCRIPTION: &'static str = "description";
const STATUS: &'static str = "status";
const CREATED: &'static str = "created";

#[derive(Clone, Serialize, Deserialize)]
pub struct Task {
    id: Option<String>,
    props: HashMap<String, String>,
    comments: Option<Vec<Comment>>,
    labels: Option<Vec<Label>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subtasks: Option<Vec<Subtask>>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    id: Option<String>,
    props: HashMap<String, String>,
    text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Label {
    name: String,
    color: Option<String>,
    description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subtask {
    id: Option<String>,
    name: String,
    status: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    props: HashMap<String, String>,
}

impl Task {
    pub fn new(name: String, description: String, status: String) -> Result<Task, &'static str> {
        if name.trim().is_empty() {
            return Err("Task name is empty");
        }
        if status.trim().is_empty() {
            return Err("Task status is empty");
        }
        Ok(Self::construct_task(name, description, status, None))
    }

    pub fn from_properties(id: String, mut props: HashMap<String, String>) -> Result<Task, &'static str> {
        let name = props.get(NAME).unwrap_or(&"".to_owned()).to_owned();
        let status = props.get(STATUS).unwrap_or(&"".to_owned()).to_owned();

        if !name.is_empty() && !status.is_empty() {
            if !props.contains_key("created") {
                props.insert("created".to_string(), get_current_timestamp().to_string());
            }

            Ok(Task{ id: Some(id), props, comments: None, labels: None, subtasks: None })
        } else {
            Err("Name or status is empty")
        }
    }

    fn construct_task(name: String, description: String, status: String, created: Option<u64>) -> Task {
        let mut props = HashMap::from([
            (NAME.to_owned(), name),
            (DESCRIPTION.to_owned(), description),
            (STATUS.to_owned(), status),
            (CREATED.to_owned(), created.unwrap_or(get_current_timestamp()).to_string()),
        ]);

        if let Ok(Some(current_user)) = get_current_user() {
            props.insert("author".to_string(), current_user);
        }

        Task {
            id: None,
            props,
            comments: None,
            labels: None,
            subtasks: None,
        }
    }

    pub fn get_id(&self) -> Option<String> {
        match &self.id {
            Some(id) => Some(id.clone()),
            _ => None
        }
    }

    pub fn set_id(&mut self, id: String) {
        self.id = Some(id);
    }

    pub fn get_property(&self, prop: &str) -> Option<&String> {
        self.props.get(prop)
    }

    pub fn get_all_properties(&self) -> &HashMap<String, String> {
        &self.props
    }

    pub fn set_property(&mut self, prop: &str, value: &str) {
        self.props.insert(prop.to_string(), value.to_string());
    }

    pub fn has_property(&self, prop: &str) -> bool {
        self.props.contains_key(prop)
    }

    pub fn delete_property(&mut self, prop: &str) -> bool {
        self.props.remove(prop).is_some()
    }

    pub fn get_comments(&self) -> &Option<Vec<Comment>> {
        &self.comments
    }

    pub fn add_comment(&mut self, id: Option<String>, mut props: HashMap<String, String>, text: String) -> Comment {
        if self.comments.is_none() {
            self.comments = Some(vec![]);
        }

        let id = Some(id.unwrap_or_else(|| {
            let max_id = self.comments.as_ref().unwrap().iter()
                .filter_map(|c| c.get_id().and_then(|id| id.parse::<u64>().ok()))
                .max()
                .unwrap_or(0);
            (max_id + 1).to_string()
        }));

        if !props.contains_key("created") {
            props.insert("created".to_string(), get_current_timestamp().to_string());
        }

        if !props.contains_key("author") {
            if let Ok(Some(current_user)) = get_current_user() {
                props.insert("author".to_string(), current_user);
            }
        }

        let comment = Comment {
            id,
            props,
            text,
        };

        self.comments.as_mut().unwrap().push(comment.clone());

        comment
    }

    pub fn set_comments(&mut self, comments: Vec<Comment>) {
        self.comments = Some(comments);
    }

    pub fn delete_comment(&mut self, id: &String) -> Result<(), String> {
        if self.comments.is_none() {
            return Err("Task has no comments".to_string());
        }

        let index = self.comments.as_ref().unwrap().iter().position(|comment| comment.get_id().map(|cid| cid == id.deref()).unwrap_or(false));

        if index.is_none() {
            return Err(format!("Comment ID {id} not found"));
        }

        self.comments.as_mut().unwrap().remove(index.unwrap());

        Ok(())
    }

    pub fn get_labels(&self) -> &Option<Vec<Label>> {
        &self.labels
    }

    pub fn add_label(&mut self, name: String, description: Option<String>, color: Option<String>) -> Label {
        if self.labels.is_none() {
            self.labels = Some(vec![]);
        }

        if let Some(existing) = self.labels.as_ref().unwrap().iter().find(|l| l.name == name) {
            return existing.clone();
        }

        let label = Label {
            name: name.clone(),
            description,
            color,
        };

        self.labels.as_mut().unwrap().push(label.clone());

        label
    }

    pub fn set_labels(&mut self, labels: Vec<Label>) {
        self.labels = Some(labels);
    }

    pub fn delete_label(&mut self, name: &str) -> Result<(), String> {
        if self.labels.is_none() {
            return Err("Task has no labels".to_string());
        }

        let index = self.labels.as_ref().unwrap().iter().position(|label| label.name == name);

        if index.is_none() {
            return Err(format!("Label with name '{name}' not found"));
        }

        self.labels.as_mut().unwrap().remove(index.unwrap());

        Ok(())
    }

    pub fn get_label_by_name(&self, name: &str) -> Option<&Label> {
        self.labels
            .as_ref()
            .and_then(|labels| labels.iter().find(|label| label.name == name))
    }

    pub fn get_subtasks(&self) -> &Option<Vec<Subtask>> {
        &self.subtasks
    }

    pub fn add_subtask(&mut self, id: Option<String>, name: String, status: String, mut props: HashMap<String, String>) -> Result<Subtask, String> {
        if name.is_empty() || status.is_empty() {
            return Err("Name or status is empty".to_string());
        }

        if self.subtasks.is_none() {
            self.subtasks = Some(vec![]);
        }

        let id = Some(id.unwrap_or_else(|| {
            let max_id = self.subtasks.as_ref().unwrap().iter()
                .filter_map(|s| s.get_id().and_then(|id| id.parse::<u64>().ok()))
                .max()
                .unwrap_or(0);
            (max_id + 1).to_string()
        }));

        if !props.contains_key("created") {
            props.insert("created".to_string(), get_current_timestamp().to_string());
        }

        if !props.contains_key("author") {
            if let Ok(Some(current_user)) = get_current_user() {
                props.insert("author".to_string(), current_user);
            }
        }

        let subtask = Subtask { id, name, status, props };

        self.subtasks.as_mut().unwrap().push(subtask.clone());

        Ok(subtask)
    }

    pub fn set_subtasks(&mut self, subtasks: Vec<Subtask>) {
        self.subtasks = Some(subtasks);
    }

    pub fn delete_subtask(&mut self, id: &String) -> Result<(), String> {
        if self.subtasks.is_none() {
            return Err("Task has no subtasks".to_string());
        }

        let index = self.subtasks.as_ref().unwrap().iter()
            .position(|subtask| subtask.get_id().map(|sid| sid.as_str() == id.as_str()).unwrap_or(false));

        if index.is_none() {
            return Err(format!("Subtask ID {id} not found"));
        }

        self.subtasks.as_mut().unwrap().remove(index.unwrap());

        Ok(())
    }

    pub fn get_subtask(&self, id: &str) -> Option<&Subtask> {
        self.subtasks
            .as_ref()
            .and_then(|subtasks| subtasks.iter().find(|s| s.get_id().map(|sid| sid == id).unwrap_or(false)))
    }

    pub fn update_subtask<F>(&mut self, id: &str, updater: F) -> Result<(), String>
    where
        F: FnOnce(&mut Subtask),
    {
        if self.subtasks.is_none() {
            return Err("Task has no subtasks".to_string());
        }

        let subtasks = self.subtasks.as_mut().unwrap();
        let subtask = subtasks.iter_mut()
            .find(|s| s.get_id().map(|sid| sid == id).unwrap_or(false));

        match subtask {
            Some(s) => { updater(s); Ok(()) },
            None => Err(format!("Subtask ID {id} not found")),
        }
    }
}

impl Comment {
    pub fn new(id: String, props: HashMap<String, String>, text: String) -> Comment {
        Comment {
            id: Some(id),
            props,
            text,
        }
    }

    pub fn get_id(&self) -> Option<String> {
        match &self.id {
            Some(id) => Some(id.clone()),
            _ => None
        }
    }

    pub fn set_id(&mut self, id: String) {
        self.id = Some(id);
    }

    pub fn get_all_properties(&self) -> &HashMap<String, String> {
        &self.props
    }

    pub fn get_text(&self) -> String {
        self.text.to_string()
    }

    pub fn set_text(&mut self, text: String) {
        self.text = text;
    }
}

impl Subtask {
    pub fn new(id: Option<String>, name: String, status: String, props: HashMap<String, String>) -> Subtask {
        Subtask { id, name, status, props }
    }

    pub fn get_id(&self) -> Option<String> {
        self.id.clone()
    }

    pub fn set_id(&mut self, id: String) {
        self.id = Some(id);
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn get_status(&self) -> &str {
        &self.status
    }

    pub fn set_status(&mut self, status: String) {
        self.status = status;
    }

    pub fn get_all_properties(&self) -> &HashMap<String, String> {
        &self.props
    }

    pub fn get_property(&self, key: &str) -> Option<&String> {
        self.props.get(key)
    }

    pub fn set_property(&mut self, key: &str, value: &str) {
        self.props.insert(key.to_string(), value.to_string());
    }

    pub fn delete_property(&mut self, key: &str) -> bool {
        self.props.remove(key).is_some()
    }
}

impl Label {
    pub fn new(name: String, color: Option<String>, description: Option<String>) -> Label {
        Label {
            name, color, description
        }
    }

    pub fn get_name(&self) -> String {
        self.name.to_string()
    }

    pub fn get_color(&self) -> String {
        self.color.clone().unwrap_or_else(|| String::from(""))
    }

    pub fn set_color(&mut self, color: String) {
        self.color = Some(color);
    }

    pub fn get_description(&self) -> Option<String> {
        self.description.clone()
    }

    pub fn set_description(&mut self, description: String) {
        self.description = Some(description);
    }
}

macro_rules! map_err {
    ($expr:expr) => {
        $expr.map_err(|e| e.message().to_owned())?
    }
}

thread_local! {
    static REPOSITORY_PATH: std::cell::RefCell<std::path::PathBuf> = std::cell::RefCell::new(std::path::PathBuf::from("."));
}

pub fn set_repository_path(path: impl Into<std::path::PathBuf>) {
    REPOSITORY_PATH.with(|p| *p.borrow_mut() = path.into());
}

fn open_repo() -> Result<Repository, String> {
    REPOSITORY_PATH.with(|p| Repository::discover(&*p.borrow())).map_err(|e| e.message().to_owned())
}

pub fn list_tasks() -> Result<Vec<Task>, String> {
    let repo = open_repo()?;
    let ref_path = get_ref_path();
    let task_ref = match repo.find_reference(&ref_path) {
        Ok(task_ref) => task_ref,
        Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.to_string()),
    };
    let task_tree = map_err!(task_ref.peel_to_tree());

    let mut result = vec![];

    let _ = map_err!(task_tree.walk(TreeWalkMode::PreOrder, |_, entry| {
        let oid = entry.id();
        match repo.find_blob(oid) {
            Ok(blob) => match serde_json::from_slice::<Task>(blob.content()) {
                Ok(task) => result.push(task),
                Err(_) => eprintln!("WARNING: skipping malformed task entry {}", entry.name().unwrap_or("?")),
            },
            Err(_) => eprintln!("WARNING: skipping non-blob task entry {}", entry.name().unwrap_or("?")),
        }

        TreeWalkResult::Ok
    }));

    Ok(result)
}

pub fn find_task(id: &str) -> Result<Option<Task>, String> {
    let repo = open_repo()?;
    let task_ref = repo.find_reference(&get_ref_path());
    match task_ref {
        Ok(task_ref) => {
            let task_tree = map_err!(task_ref.peel_to_tree());
            let result = match task_tree.get_name(id) {
                Some(entry) => {
                    let oid = entry.id();
                    let blob = map_err!(repo.find_blob(oid));
                    let content = blob.content();
                    let task = serde_json::from_slice(content).map_err(|e| format!("Malformed task {id}: {e}"))?;

                    Some(task)
                },
                None => None,
            };

            Ok(result)
        },
        Err(_) => Ok(None)
    }
}

/// Deletes the tasks with the given ids and returns the ids that were actually deleted.
/// Unknown ids are skipped, and nothing is committed when none of the ids exist.
pub fn delete_tasks(ids: &[&str]) -> Result<Vec<String>, String> {
    let repo = open_repo()?;
    let ref_path = get_ref_path();
    let task_ref = map_err!(repo.find_reference(&ref_path));
    let task_tree = map_err!(task_ref.peel_to_tree());

    let mut treebuilder = map_err!(repo.treebuilder(Some(&task_tree)));
    let mut deleted = vec![];
    for id in ids {
        if task_tree.get_name(id).is_none() || deleted.iter().any(|d| d == id) {
            continue;
        }
        map_err!(treebuilder.remove(id));
        deleted.push(id.to_string());
    }
    if deleted.is_empty() {
        return Ok(deleted);
    }
    let tree_oid = map_err!(treebuilder.write());

    let parent_commit = map_err!(task_ref.peel_to_commit());
    let parents = vec![parent_commit];
    let me = &map_err!(repo.signature());

    deleted.sort_by(|a, b| compare_ids(a, b));
    let message = format!("Delete task {}", deleted.join(", "));
    map_err!(repo.commit(Some(&ref_path), me, me, &message, &map_err!(repo.find_tree(tree_oid)), &parents.iter().collect::<Vec<_>>()));

    Ok(deleted)
}

pub fn clear_tasks() -> Result<u64, String> {
    let repo = open_repo()?;
    let ref_path = get_ref_path();
    let task_ref = map_err!(repo.find_reference(&ref_path));
    let task_tree = map_err!(task_ref.peel_to_tree());

    let mut treebuilder = map_err!(repo.treebuilder(Some(&task_tree)));
    let task_count = treebuilder.len() as u64;
    map_err!(treebuilder.clear());
    let tree_oid = map_err!(treebuilder.write());

    let parent_commit = map_err!(task_ref.peel_to_commit());
    let parents = vec![parent_commit];
    let me = &map_err!(repo.signature());

    map_err!(repo.commit(Some(&ref_path), me, me, "Clear tasks", &map_err!(repo.find_tree(tree_oid)), &parents.iter().collect::<Vec<_>>()));

    Ok(task_count)
}

pub fn create_task(task: Task) -> Result<Task, String> {
    let mut tasks = write_tasks(vec![task], |ids| format!("Create task {}", ids.join(", ")))?;
    Ok(tasks.pop().unwrap())
}

pub fn import_tasks(tasks: Vec<Task>) -> Result<Vec<Task>, String> {
    write_tasks(tasks, |ids| format!("Import task {}", ids.join(", ")))
}

fn write_tasks(mut tasks: Vec<Task>, message: impl Fn(&[String]) -> String) -> Result<Vec<Task>, String> {
    if tasks.is_empty() {
        return Ok(tasks);
    }
    let repo = open_repo()?;
    let ref_path = get_ref_path();
    let task_ref_result = repo.find_reference(&ref_path);
    let source_tree = task_ref_result.as_ref().ok().and_then(|reference| reference.peel_to_tree().ok());
    let mut next_id = get_next_id_in(&repo, &ref_path).ok().and_then(|id| id.parse::<i64>().ok()).unwrap_or(1);

    let mut treebuilder = map_err!(repo.treebuilder(source_tree.as_ref()));
    let mut ids = vec![];
    for task in tasks.iter_mut() {
        if task.get_id().is_none() {
            task.set_id(next_id.to_string());
        }
        let id = task.get_id().unwrap();
        if let Ok(numeric) = id.parse::<i64>() {
            if numeric >= next_id {
                next_id = numeric + 1;
            }
        }
        let content = serde_json::to_string(&*task).unwrap();
        let oid = map_err!(repo.blob(content.as_bytes()));
        map_err!(treebuilder.insert(&id, oid, FileMode::Blob.into()));
        ids.push(id);
    }
    let tree_oid = map_err!(treebuilder.write());

    let me = &map_err!(repo.signature());
    let mut parents = vec![];
    if let Ok(reference) = task_ref_result {
        if let Ok(parent_commit) = reference.peel_to_commit() {
            parents.push(parent_commit);
        }
    }
    ids.sort_by(|a, b| compare_ids(a, b));
    map_err!(repo.commit(Some(&ref_path), me, me, &message(&ids), &map_err!(repo.find_tree(tree_oid)), &parents.iter().collect::<Vec<_>>()));

    Ok(tasks)
}

pub fn update_task(task: Task) -> Result<String, String> {
    let repo = open_repo()?;
    let ref_path = get_ref_path();
    let task_ref_result = map_err!(repo.find_reference(&ref_path));
    let parent_commit = map_err!(task_ref_result.peel_to_commit());
    let source_tree = map_err!(task_ref_result.peel_to_tree());
    let string_content = serde_json::to_string(&task).unwrap();
    let content = string_content.as_bytes();
    let oid = map_err!(repo.blob(content));
    let mut treebuilder = map_err!(repo.treebuilder(Some(&source_tree)));
    map_err!(treebuilder.insert(&task.get_id().unwrap(), oid, FileMode::Blob.into()));
    let tree_oid = map_err!(treebuilder.write());

    let me = &map_err!(repo.signature());
    let parents = vec![parent_commit];
    map_err!(repo.commit(Some(&ref_path), me, me, format!("Update task {}", &task.get_id().unwrap()).as_str(), &map_err!(repo.find_tree(tree_oid)), &parents.iter().collect::<Vec<_>>()));

    Ok(task.get_id().unwrap())
}

fn get_next_id() -> Result<String, String> {
    let repo = open_repo()?;
    let ref_path = get_ref_path();
    get_next_id_in(&repo, &ref_path)
}

fn get_next_id_in(repo: &Repository, ref_path: &str) -> Result<String, String> {
    let task_ref = map_err!(repo.find_reference(ref_path));
    let task_tree = map_err!(task_ref.peel_to_tree());

    let mut result = 0;

    let _ = map_err!(task_tree.walk(TreeWalkMode::PreOrder, |_, entry| {
        let entry_name = entry.name().unwrap();
        match entry_name.parse::<i64>() {
            Ok(id) => {
                if id > result {
                    result = id;
                }
            },
            _ => return TreeWalkResult::Skip
        };

        TreeWalkResult::Ok
    }));

    Ok((result + 1).to_string())
}

pub fn update_task_id(id: &str, new_id: &str, force: bool) -> Result<(), String> {
    let mut task = find_task(&id)?.ok_or_else(|| format!("Task ID {id} not found"))?;
    if !force && id != new_id && find_task(&new_id)?.is_some() {
        return Err(format!("Task ID {new_id} already exists"));
    }
    task.set_id(new_id.to_string());
    create_task(task)?;
    if id != new_id {
        delete_tasks(&[&id])?;
    }

    Ok(())
}

fn apply_id_updates(ids: &[Option<String>], id_updates: &[(String, String)], kind: &str) -> Result<Vec<Option<String>>, String> {
    for (old, _) in id_updates {
        if !ids.iter().any(|id| id.as_deref() == Some(old.as_str())) {
            return Err(format!("{kind} ID {old} not found"));
        }
    }
    let new_ids = ids.iter().map(|id| id.as_ref().map(|id| {
        id_updates.iter().find(|(old, _)| old == id).map(|(_, new)| new.clone()).unwrap_or_else(|| id.clone())
    })).collect::<Vec<_>>();
    let mut seen = HashSet::new();
    for id in new_ids.iter().flatten() {
        if !seen.insert(id) {
            return Err(format!("{kind} ID {id} already exists"));
        }
    }
    Ok(new_ids)
}

fn next_free_numeric_id(ids: &[Option<String>]) -> String {
    (ids.iter().flatten().filter_map(|id| id.parse::<u64>().ok()).max().unwrap_or(0) + 1).to_string()
}

fn single_id_update(ids: &[Option<String>], id: &str, new_id: &str) -> Vec<(String, String)> {
    let mut id_updates = vec![(id.to_string(), new_id.to_string())];
    if id != new_id && ids.iter().any(|existing| existing.as_deref() == Some(new_id)) {
        id_updates.push((new_id.to_string(), next_free_numeric_id(ids)));
    }
    id_updates
}

pub fn update_subtask_id(task_id: &str, id: &str, new_id: &str) -> Result<(), String> {
    let task = find_task(&task_id)?.ok_or_else(|| format!("Task ID {task_id} not found"))?;
    let ids = task.get_subtasks().as_ref().map(|subtasks| subtasks.iter().map(|s| s.get_id()).collect::<Vec<_>>()).unwrap_or_default();
    update_subtask_ids(task_id, &single_id_update(&ids, id, new_id))
}

pub fn update_subtask_ids(task_id: &str, id_updates: &[(String, String)]) -> Result<(), String> {
    if id_updates.is_empty() {
        return Ok(());
    }
    let mut task = find_task(&task_id)?.ok_or_else(|| format!("Task ID {task_id} not found"))?;
    let subtasks = task.get_subtasks().clone().unwrap_or_default();
    let ids = subtasks.iter().map(|s| s.get_id()).collect::<Vec<_>>();
    let new_ids = apply_id_updates(&ids, id_updates, "Subtask")?;
    let updated = subtasks.into_iter().zip(new_ids).map(|(mut subtask, id)| {
        if let Some(id) = id {
            subtask.set_id(id);
        }
        subtask
    }).collect();
    task.set_subtasks(updated);
    update_task(task)?;
    Ok(())
}

pub fn update_comment_id(task_id: &str, id: &str, new_id: &str) -> Result<(), String> {
    let task = find_task(&task_id)?.ok_or_else(|| format!("Task ID {task_id} not found"))?;
    let ids = task.get_comments().as_ref().map(|comments| comments.iter().map(|c| c.get_id()).collect::<Vec<_>>()).unwrap_or_default();
    update_comment_ids(task_id, &single_id_update(&ids, id, new_id))
}

pub fn update_comment_ids(task_id: &str, id_updates: &[(String, String)]) -> Result<(), String> {
    if id_updates.is_empty() {
        return Ok(());
    }
    let mut task = find_task(&task_id)?.ok_or_else(|| format!("Task ID {task_id} not found"))?;
    let comments = task.get_comments().clone().unwrap_or_default();
    let ids = comments.iter().map(|c| c.get_id()).collect::<Vec<_>>();
    let new_ids = apply_id_updates(&ids, id_updates, "Comment")?;
    let updated = comments.into_iter().zip(new_ids).map(|(mut comment, id)| {
        if let Some(id) = id {
            comment.set_id(id);
        }
        comment
    }).collect();
    task.set_comments(updated);
    update_task(task)?;
    Ok(())
}

/// Orders task IDs by project prefix (none first), then numerically: `7`, `42`, `ABC-7`, `ABC-42`, `XYZ-1`.
/// Prefixed IDs are Jira issue keys kept as local IDs when several Jira projects are synced.
pub fn compare_ids(a: &str, b: &str) -> std::cmp::Ordering {
    fn split(id: &str) -> (&str, u64) {
        match id.rsplit_once('-') {
            Some((prefix, number)) if !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) => (prefix, number.parse().unwrap_or(0)),
            _ => ("", id.parse().unwrap_or(0)),
        }
    }

    let (prefix_a, number_a) = split(a);
    let (prefix_b, number_b) = split(b);
    prefix_a.cmp(prefix_b).then(number_a.cmp(&number_b)).then_with(|| a.cmp(b))
}

pub fn list_remotes(remote: &Option<String>) -> Result<Vec<String>, String> {
    let repo = open_repo()?;
    let remotes = map_err!(repo.remotes());
    Ok(remotes.iter()
        .filter_map(|s| s.ok().flatten())
        .filter(|name| remote.is_none() || remote.as_ref().unwrap().as_str() == *name)
        .filter_map(|name| repo.find_remote(name).ok())
        .filter_map(|remote| non_empty_url(remote.url().ok()).or_else(|| non_empty_url(remote.pushurl().ok().flatten())))
        .collect())
}

fn non_empty_url(url: Option<&str>) -> Option<String> {
    url.filter(|url| !url.trim().is_empty()).map(str::to_owned)
}

fn get_current_timestamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn get_current_user() -> Result<Option<String>, String> {
    let repo = open_repo()?;
    let me = &map_err!(repo.signature());
    match me.name() {
        Ok(name) => Ok(Some(String::from(name))),
        _ => match me.email() {
            Ok(email) => Ok(Some(String::from(email))),
            _ => Ok(None),
        }
    }
}

pub fn get_ref_path() -> String {
    get_config_value("task.ref").unwrap_or_else(|_| "refs/tasks/tasks".to_string())
}

pub fn get_config_value(key: &str) -> Result<String, String> {
    let repo = open_repo()?;
    let config = map_err!(repo.config());
    Ok(map_err!(config.get_string(key)))
}

pub fn set_config_value(key: &str, value: &str) -> Result<(), String> {
    let repo = open_repo()?;
    let mut config = map_err!(repo.config());
    map_err!(config.set_str(key, value));
    Ok(())
}

pub fn unset_config_value(key: &str) -> Result<(), String> {
    let repo = open_repo()?;
    let mut config = map_err!(repo.config());
    match config.remove(key) {
        Ok(_) => Ok(()),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn set_ref_path(ref_path: &str, move_ref: bool, force: bool) -> Result<(), String> {
    let repo = open_repo()?;
    let same_path = get_ref_path() == ref_path;

    let current_reference = repo.find_reference(&get_ref_path());
    if !same_path {
        if let Ok(current_reference) = &current_reference {
            let commit = map_err!(current_reference.peel_to_commit());
            match repo.find_reference(ref_path) {
                Ok(existing) if existing.target() == Some(commit.id()) => {},
                Ok(_) if !force => return Err(format!("Reference {ref_path} already exists and points to a different commit, use --force to overwrite it")),
                _ => { map_err!(repo.reference(ref_path, commit.id(), force, "task.ref migrated")); },
            }
        }
    }

    let mut config = map_err!(repo.config());
    map_err!(config.set_str("task.ref", ref_path));

    if move_ref && !same_path && current_reference.is_ok() {
        map_err!(current_reference.unwrap().delete());
    }

    Ok(())
}

#[doc(hidden)]
pub mod testing {
    use std::path::Path;
    use tempfile::TempDir;

    pub struct TestRepo {
        dir: TempDir,
    }

    impl TestRepo {
        pub fn new() -> TestRepo {
            let dir = tempfile::Builder::new().prefix("git-task-test").tempdir().expect("failed to create a temporary directory");
            let repo = git2::Repository::init(dir.path()).expect("failed to init a temporary repository");
            let mut config = repo.config().expect("failed to open the repository config");
            config.set_str("user.name", "git-task tests").expect("failed to set user.name");
            config.set_str("user.email", "tests@git-task.invalid").expect("failed to set user.email");
            let me = repo.signature().expect("failed to build a signature");
            let tree_oid = repo.treebuilder(None).expect("failed to create a tree builder").write().expect("failed to write the empty tree");
            let tree = repo.find_tree(tree_oid).expect("failed to find the empty tree");
            repo.commit(Some("HEAD"), &me, &me, "Initial commit", &tree, &[]).expect("failed to create the initial commit");
            super::set_repository_path(dir.path());
            TestRepo { dir }
        }

        pub fn path(&self) -> &Path {
            self.dir.path()
        }
    }

    impl Drop for TestRepo {
        fn drop(&mut self) {
            super::set_repository_path(".");
        }
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use crate::*;

    #[test]
    fn test_ref_path() {
        let _repo = testing::TestRepo::new();
        let ref_path = get_ref_path();
        assert!(set_ref_path("refs/heads/test-git-task", true, false).is_ok());
        assert_eq!(get_ref_path(), "refs/heads/test-git-task");
        assert!(set_ref_path(&ref_path, true, false).is_ok());
        assert_eq!(get_ref_path(), ref_path);
    }

    #[test]
    fn test_set_ref_path_refuses_to_overwrite_existing_ref() {
        let _repo = testing::TestRepo::new();
        let ref_path = get_ref_path();
        let task = Task::construct_task("Ref clash".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        let task = create_task(task).unwrap();
        let id = task.get_id().unwrap();

        let repo = open_repo().unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let other_path = "refs/heads/test-git-task-existing";
        repo.reference(other_path, head.id(), true, "test").unwrap();

        let result = set_ref_path(other_path, true, false);
        let other_target = repo.find_reference(other_path).unwrap().target();
        let _ = repo.find_reference(other_path).unwrap().delete();
        let _ = delete_tasks(&[&id]);

        assert!(result.is_err(), "setting task.ref to an existing, unrelated ref must fail");
        assert_eq!(other_target, Some(head.id()), "the existing ref must not be moved");
        assert_eq!(get_ref_path(), ref_path, "task.ref must stay unchanged after a refused move");
        assert!(find_task(&id).is_ok(), "tasks must still be reachable under the original ref");
    }

    #[test]
    fn test_set_ref_path_force_overwrites_existing_ref() {
        let _repo = testing::TestRepo::new();
        let ref_path = get_ref_path();
        let task = Task::construct_task("Ref force".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        let task = create_task(task).unwrap();
        let id = task.get_id().unwrap();

        let repo = open_repo().unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let tasks_commit = repo.find_reference(&ref_path).unwrap().peel_to_commit().unwrap();
        let other_path = "refs/heads/test-git-task-forced";
        repo.reference(other_path, head.id(), true, "test").unwrap();

        let result = set_ref_path(other_path, false, true);
        let other_target = repo.find_reference(other_path).unwrap().target();
        let found = find_task(&id);
        let _ = set_ref_path(&ref_path, false, false);
        let _ = repo.find_reference(other_path).unwrap().delete();
        let _ = delete_tasks(&[&id]);

        assert!(result.is_ok(), "--force must overwrite an existing ref: {:?}", result.err());
        assert_eq!(other_target, Some(tasks_commit.id()), "the existing ref must now point at the tasks commit");
        assert!(found.unwrap().is_some(), "tasks must be visible under the forced ref");
    }

    #[test]
    fn test_set_ref_path_accepts_existing_ref_at_same_commit() {
        let _repo = testing::TestRepo::new();
        let ref_path = get_ref_path();
        let task = Task::construct_task("Ref copy".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        let task = create_task(task).unwrap();
        let id = task.get_id().unwrap();

        let repo = open_repo().unwrap();
        let tasks_commit = repo.find_reference(&ref_path).unwrap().peel_to_commit().unwrap();
        let other_path = "refs/heads/test-git-task-copy";
        repo.reference(other_path, tasks_commit.id(), true, "test").unwrap();

        let result = set_ref_path(other_path, false, false);
        let found = find_task(&id);
        let _ = set_ref_path(&ref_path, false, false);
        let _ = repo.find_reference(other_path).unwrap().delete();
        let _ = delete_tasks(&[&id]);

        assert!(result.is_ok(), "a ref that already points at the tasks commit is fine: {:?}", result.err());
        assert!(found.unwrap().is_some(), "tasks must be visible under the adopted ref");
    }

    #[test]
    fn test_set_ref_path_same_path_keeps_tasks() {
        let _repo = testing::TestRepo::new();
        let ref_path = get_ref_path();
        let task = Task::construct_task("Ref move".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        let task = create_task(task).unwrap();
        let id = task.get_id().unwrap();

        assert!(set_ref_path(&ref_path, true, false).is_ok());
        assert_eq!(get_ref_path(), ref_path);
        let found = find_task(&id);
        assert!(found.is_ok());
        assert!(found.unwrap().is_some(), "tasks must survive setting task.ref to its current value with move");

        let _ = delete_tasks(&[&id]);
    }

    #[test]
    fn test_create_update_delete_task() {
        let _repo = testing::TestRepo::new();
        let id = get_next_id().unwrap_or_else(|_| "1".to_string());
        let task = Task::construct_task("Test task".to_string(), "Description goes here".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        let create_result = create_task(task);
        assert!(create_result.is_ok());
        let mut task = create_result.unwrap();
        assert_eq!(task.get_id(), Some(id.clone()));
        assert_eq!(task.get_property("name").unwrap(), "Test task");
        assert_eq!(task.get_property("description").unwrap(), "Description goes here");
        assert_eq!(task.get_property("status").unwrap(), "OPEN");
        assert!(task.has_property("created"));

        task.set_property("description", "Updated description");
        let comment_props = HashMap::from([("author".to_string(), "Some developer".to_string())]);
        task.add_comment(None, comment_props, "This is a comment".to_string());
        task.set_property("custom_prop", "Custom content");
        let update_result = update_task(task);
        assert!(update_result.is_ok());
        assert_eq!(update_result.unwrap(), id.clone());

        let find_result = find_task(&id);
        assert!(find_result.is_ok());
        let task = find_result.unwrap();
        assert!(task.is_some());
        let task = task.unwrap();
        assert_eq!(task.get_id(), Some(id.clone()));
        assert_eq!(task.get_property("description").unwrap(), "Updated description");
        let comments = task.get_comments().clone();
        assert!(comments.is_some());
        let comments = comments.unwrap();
        assert_eq!(comments.len(), 1);
        let comment = comments.first().unwrap();
        assert_eq!(comment.get_text(), "This is a comment".to_string());
        let comment_props = comment.clone().props;
        assert_eq!(comment_props.get("author").unwrap(), &"Some developer".to_string());
        assert_eq!(task.get_property("custom_prop").unwrap(), "Custom content");

        let delete_result = delete_tasks(&[&id]);
        assert!(delete_result.is_ok());

        let find_result = find_task(&id);
        assert!(find_result.is_ok());
        let task = find_result.unwrap();
        assert!(task.is_none());
    }

    #[test]
    fn test_update_comment_id() {
        let _repo = testing::TestRepo::new();
        // Create a task first
        let id = get_next_id().unwrap_or_else(|_| "1".to_string());
        let task = Task::construct_task(
            "Test task".to_string(),
            "Description goes here".to_string(),
            "OPEN".to_string(),
            Some(get_current_timestamp())
        );
        let create_result = create_task(task);
        assert!(create_result.is_ok());
        let mut task = create_result.unwrap();

        // Add a comment to the task
        let comment_props = HashMap::from([("author".to_string(), "Some developer".to_string())]);
        let comment = task.add_comment(Some("1".to_string()), comment_props, "Test comment".to_string());
        assert_eq!(comment.get_id().unwrap(), "1");
        let update_result = update_task(task);
        assert!(update_result.is_ok());

        // Update the comment ID
        let result = update_comment_id(&id, "1", "2");
        assert!(result.is_ok());

        // Verify the comment ID was updated
        let updated_task = find_task(&id).unwrap().unwrap();
        let updated_comments = updated_task.get_comments().as_ref().unwrap();
        assert_eq!(updated_comments.len(), 1);
        assert_eq!(updated_comments[0].get_id().unwrap(), "2");

        // Clean up
        let delete_result = delete_tasks(&[&id]);
        assert!(delete_result.is_ok());
    }

    #[test]
    fn test_list_tasks_skips_malformed_blob() {
        let _repo = testing::TestRepo::new();
        let task = Task::construct_task("Good task".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        let task = create_task(task).unwrap();
        let id = task.get_id().unwrap();

        // inject a malformed blob into the tasks tree
        let repo = open_repo().unwrap();
        let task_ref = repo.find_reference(&get_ref_path()).unwrap();
        let task_tree = task_ref.peel_to_tree().unwrap();
        let oid = repo.blob(b"not json").unwrap();
        let mut treebuilder = repo.treebuilder(Some(&task_tree)).unwrap();
        treebuilder.insert("999999", oid, FileMode::Blob.into()).unwrap();
        let tree_oid = treebuilder.write().unwrap();
        let me = repo.signature().unwrap();
        let parent = task_ref.peel_to_commit().unwrap();
        repo.commit(Some(&get_ref_path()), &me, &me, "inject bad blob", &repo.find_tree(tree_oid).unwrap(), &[&parent]).unwrap();

        let result = std::panic::catch_unwind(|| list_tasks());
        assert!(result.is_ok(), "list_tasks must not panic on a malformed blob");
        let tasks = result.unwrap().unwrap();
        assert!(tasks.iter().any(|t| t.get_id() == Some(id.clone())), "valid tasks must still be listed");
        assert!(!tasks.iter().any(|t| t.get_id() == Some("999999".to_string())));

        let find_result = std::panic::catch_unwind(|| find_task("999999"));
        assert!(find_result.is_ok(), "find_task must not panic on a malformed blob");
        assert!(find_result.unwrap().is_err(), "find_task should report an error for a malformed blob");

        let _ = delete_tasks(&[&id, "999999"]);
    }

    fn task_with_subtasks(names: &[&str]) -> String {
        let mut task = Task::new("Parent".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        for name in names {
            task.add_subtask(None, name.to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        }
        create_task(task).unwrap().get_id().unwrap()
    }

    fn subtask_ids_by_name(task_id: &str) -> Vec<(String, String)> {
        find_task(task_id).unwrap().unwrap().get_subtasks().clone().unwrap_or_default()
            .iter().map(|s| (s.get_name().to_string(), s.get_id().unwrap_or_default())).collect()
    }

    fn task_with_comments(texts: &[&str]) -> String {
        let mut task = Task::new("Parent".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        for text in texts {
            task.add_comment(None, HashMap::new(), text.to_string());
        }
        create_task(task).unwrap().get_id().unwrap()
    }

    fn comment_ids_by_text(task_id: &str) -> Vec<(String, String)> {
        find_task(task_id).unwrap().unwrap().get_comments().clone().unwrap_or_default()
            .iter().map(|c| (c.get_text().to_string(), c.get_id().unwrap_or_default())).collect()
    }

    #[test]
    fn test_update_subtask_id_displaces_a_colliding_provisional_subtask() {
        let _repo = testing::TestRepo::new();
        let task_id = task_with_subtasks(&["A", "B"]);
        assert_eq!(subtask_ids_by_name(&task_id), vec![("A".to_string(), "1".to_string()), ("B".to_string(), "2".to_string())]);

        update_subtask_id(&task_id, "1", "2").unwrap();

        assert_eq!(subtask_ids_by_name(&task_id), vec![("A".to_string(), "2".to_string()), ("B".to_string(), "3".to_string())], "the updated subtask takes the id and the unsynced holder moves to a fresh id");
    }

    #[test]
    fn test_update_subtask_ids_applies_updates_against_the_original_ids() {
        let _repo = testing::TestRepo::new();
        let task_id = task_with_subtasks(&["A", "B"]);

        update_subtask_ids(&task_id, &[("1".to_string(), "2".to_string()), ("2".to_string(), "3".to_string())]).unwrap();

        assert_eq!(subtask_ids_by_name(&task_id), vec![("A".to_string(), "2".to_string()), ("B".to_string(), "3".to_string())]);
    }

    #[test]
    fn test_update_subtask_ids_refuses_duplicates_without_writing() {
        let _repo = testing::TestRepo::new();
        let task_id = task_with_subtasks(&["A", "B", "C"]);
        let before = open_repo().unwrap().find_reference(&get_ref_path()).unwrap().target().unwrap();

        let result = update_subtask_ids(&task_id, &[("1".to_string(), "3".to_string())]);

        assert!(result.is_err(), "renaming onto an id that stays in use must fail");
        let after = open_repo().unwrap().find_reference(&get_ref_path()).unwrap().target().unwrap();
        assert_eq!(before, after, "nothing must be written");
        assert_eq!(subtask_ids_by_name(&task_id)[0].1, "1");
    }

    #[test]
    fn test_update_comment_id_displaces_a_colliding_provisional_comment() {
        let _repo = testing::TestRepo::new();
        let task_id = task_with_comments(&["first", "second"]);

        update_comment_id(&task_id, "1", "2").unwrap();

        assert_eq!(comment_ids_by_text(&task_id), vec![("first".to_string(), "2".to_string()), ("second".to_string(), "3".to_string())]);
    }

    #[test]
    fn test_update_comment_ids_applies_updates_against_the_original_ids() {
        let _repo = testing::TestRepo::new();
        let task_id = task_with_comments(&["first", "second"]);

        update_comment_ids(&task_id, &[("1".to_string(), "2".to_string()), ("2".to_string(), "3".to_string())]).unwrap();

        assert_eq!(comment_ids_by_text(&task_id), vec![("first".to_string(), "2".to_string()), ("second".to_string(), "3".to_string())]);
    }

    #[test]
    fn test_update_comment_ids_refuses_duplicates_without_writing() {
        let _repo = testing::TestRepo::new();
        let task_id = task_with_comments(&["first", "second", "third"]);

        let result = update_comment_ids(&task_id, &[("1".to_string(), "3".to_string())]);

        assert!(result.is_err(), "renaming onto an id that stays in use must fail");
        assert_eq!(comment_ids_by_text(&task_id)[0].1, "1");
    }

    #[test]
    fn test_list_remotes_tolerates_a_push_only_remote() {
        let repo_dir = testing::TestRepo::new();
        let repo = Repository::open(repo_dir.path()).unwrap();
        repo.remote("origin", "https://github.com/user/fetch.git").unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("remote.mirror.pushurl", "https://github.com/user/push-only.git").unwrap();

        let result = std::panic::catch_unwind(|| list_remotes(&None));

        assert!(result.is_ok(), "a remote with only a pushurl must not crash remote listing");
        let mut urls = result.unwrap().unwrap();
        urls.sort();
        assert_eq!(urls, vec!["https://github.com/user/fetch.git".to_string(), "https://github.com/user/push-only.git".to_string()], "the push url stands in for the missing fetch url");
    }

    #[test]
    fn test_delete_tasks_skips_absent_ids() {
        let _repo = testing::TestRepo::new();
        let a = create_task(Task::new("A".to_string(), "".to_string(), "OPEN".to_string()).unwrap()).unwrap().get_id().unwrap();
        let b = create_task(Task::new("B".to_string(), "".to_string(), "OPEN".to_string()).unwrap()).unwrap().get_id().unwrap();
        let c = create_task(Task::new("C".to_string(), "".to_string(), "OPEN".to_string()).unwrap()).unwrap().get_id().unwrap();
        delete_tasks(&[&b]).unwrap();

        let result = delete_tasks(&[&a, &b, &c]);

        assert!(result.is_ok(), "an already deleted id must not abort the batch: {result:?}");
        assert_eq!(result.unwrap(), vec![a.clone(), c.clone()], "only the ids that existed are reported as deleted");
        assert!(find_task(&a).unwrap().is_none());
        assert!(find_task(&c).unwrap().is_none());
    }

    #[test]
    fn test_delete_tasks_without_matching_ids_writes_no_commit() {
        let _repo = testing::TestRepo::new();
        let a = create_task(Task::new("A".to_string(), "".to_string(), "OPEN".to_string()).unwrap()).unwrap().get_id().unwrap();
        let before = open_repo().unwrap().find_reference(&get_ref_path()).unwrap().target().unwrap();

        let result = delete_tasks(&["999999"]);

        assert_eq!(result, Ok(vec![]));
        let after = open_repo().unwrap().find_reference(&get_ref_path()).unwrap().target().unwrap();
        assert_eq!(before, after, "no commit must be written when nothing is deleted");
        assert!(find_task(&a).unwrap().is_some());
    }

    #[test]
    fn test_clear_tasks() {
        let _repo = testing::TestRepo::new();
        // Clear any leftover tasks from other tests first
        let _ = crate::clear_tasks();

        let id = get_next_id().unwrap_or_else(|_| "1".to_string());
        let task = Task::construct_task("Test task".to_string(), "Description goes here".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        let create_result = create_task(task);
        assert!(create_result.is_ok());
        let task = create_result.unwrap();
        assert_eq!(task.get_id(), Some(id.clone()));

        let id = get_next_id().unwrap_or_else(|_| "2".to_string());
        let task2 = Task::construct_task("Another task".to_string(), "Another description".to_string(), "IN_PROGRESS".to_string(), Some(get_current_timestamp()));
        let create_result2 = create_task(task2);
        assert!(create_result2.is_ok());
        let task2 = create_result2.unwrap();
        assert_eq!(task2.get_id(), Some(id.clone()));

        let id = get_next_id().unwrap_or_else(|_| "3".to_string());
        let task3 = Task::construct_task("Third task".to_string(), "Third description".to_string(), "CLOSED".to_string(), Some(get_current_timestamp()));
        let create_result3 = create_task(task3);
        assert!(create_result3.is_ok());
        let task3 = create_result3.unwrap();
        assert_eq!(task3.get_id(), Some(id.clone()));

        let clear_result = crate::clear_tasks();
        assert!(clear_result.is_ok());
        assert_eq!(clear_result.unwrap(), 3);

        let find_result = find_task(&id);
        assert!(find_result.is_ok());
        let task = find_result.unwrap();
        assert!(task.is_none());

        let find_result = find_task(&task2.get_id().unwrap());
        assert!(find_result.is_ok());
        let task = find_result.unwrap();
        assert!(task.is_none());

        let find_result = find_task(&task3.get_id().unwrap());
        assert!(find_result.is_ok());
        let task = find_result.unwrap();
        assert!(task.is_none());
    }

    #[test]
    fn test_delete_comment_without_id() {
        let _repo = testing::TestRepo::new();
        let mut task = Task::construct_task(
            "Test task".to_string(),
            "Description".to_string(),
            "OPEN".to_string(),
            Some(get_current_timestamp()),
        );
        let comment = Comment {
            id: None,
            props: HashMap::new(),
            text: "A comment without ID".to_string(),
        };
        task.comments = Some(vec![comment]);

        let result = task.delete_comment(&"1".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_add_comment_id_no_collision_after_deletion() {
        let _repo = testing::TestRepo::new();
        let mut task = Task::construct_task(
            "Test task".to_string(),
            "Description".to_string(),
            "OPEN".to_string(),
            Some(get_current_timestamp()),
        );

        task.add_comment(None, HashMap::new(), "Comment 1".to_string());
        task.add_comment(None, HashMap::new(), "Comment 2".to_string());
        task.add_comment(None, HashMap::new(), "Comment 3".to_string());

        task.delete_comment(&"2".to_string()).unwrap();

        let _new_comment = task.add_comment(None, HashMap::new(), "Comment 4".to_string());

        let all_ids: Vec<String> = task.comments.as_ref().unwrap()
            .iter()
            .map(|c| c.get_id().unwrap())
            .collect();
        let unique_ids: std::collections::HashSet<String> = all_ids.iter().cloned().collect();
        assert_eq!(all_ids.len(), unique_ids.len(), "Comment IDs must be unique, got: {:?}", all_ids);
    }

    #[test]
    fn test_delete_task_with_non_numeric_id() {
        let _repo = testing::TestRepo::new();
        let mut task = Task::construct_task(
            "Remote task".to_string(),
            "Description".to_string(),
            "OPEN".to_string(),
            Some(get_current_timestamp()),
        );
        task.set_id("GH-42".to_string());
        let create_result = create_task(task);
        assert!(create_result.is_ok());

        let delete_result = delete_tasks(&["GH-42"]);
        assert!(delete_result.is_ok());
    }

    #[test]
    fn test_update_comment_id_nonexistent_comment() {
        let _repo = testing::TestRepo::new();
        let mut task = Task::construct_task("Test task".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        task.add_comment(Some("1".to_string()), HashMap::new(), "a comment".to_string());
        let task = create_task(task).unwrap();
        let id = task.get_id().unwrap();

        let result = update_comment_id(&id, "99", "100");
        assert!(result.is_err(), "updating a nonexistent comment id must fail");

        let _ = delete_tasks(&[&id]);
    }

    #[test]
    fn test_update_subtask_id_nonexistent_subtask() {
        let _repo = testing::TestRepo::new();
        let mut task = Task::construct_task("Test task".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        task.add_subtask(None, "Sub".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        let task = create_task(task).unwrap();
        let id = task.get_id().unwrap();

        let result = update_subtask_id(&id, "99", "100");
        assert!(result.is_err(), "updating a nonexistent subtask id must fail");

        let _ = delete_tasks(&[&id]);
    }

    fn commit_count() -> usize {
        let repo = open_repo().unwrap();
        let mut commit = repo.find_reference(&get_ref_path()).unwrap().peel_to_commit().unwrap();
        let mut count = 1;
        while let Some(parent) = commit.parents().next() {
            count += 1;
            commit = parent;
        }
        count
    }

    #[test]
    fn test_import_tasks_writes_one_commit_for_the_whole_batch() {
        let _repo = testing::TestRepo::new();
        let mut with_id = Task::construct_task("With id".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        with_id.set_id("7".to_string());
        let without_id = Task::construct_task("Without id".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        let another = Task::construct_task("Another".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));

        let imported = import_tasks(vec![with_id, without_id, another]).unwrap();

        let ids: Vec<String> = imported.iter().map(|t| t.get_id().unwrap()).collect();
        assert_eq!(ids, vec!["7".to_string(), "8".to_string(), "9".to_string()], "tasks without an id continue after the highest id in the batch");
        assert_eq!(commit_count(), 1, "a batch must produce a single commit");
        assert_eq!(find_task("8").unwrap().unwrap().get_property("name").unwrap(), "Without id");

        let mut replaced = Task::construct_task("Replaced".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        replaced.set_id("7".to_string());
        import_tasks(vec![replaced, Task::construct_task("Fourth".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()))]).unwrap();

        assert_eq!(commit_count(), 2);
        assert_eq!(find_task("7").unwrap().unwrap().get_property("name").unwrap(), "Replaced", "an imported task overwrites the one with the same id");
        assert_eq!(find_task("10").unwrap().unwrap().get_property("name").unwrap(), "Fourth");
        assert_eq!(list_tasks().unwrap().len(), 4);
    }

    #[test]
    fn test_import_tasks_with_an_empty_batch_writes_nothing() {
        let _repo = testing::TestRepo::new();
        assert_eq!(import_tasks(vec![]).unwrap().len(), 0);
        assert!(open_repo().unwrap().find_reference(&get_ref_path()).is_err(), "no commit must be created for an empty batch");
    }

    #[test]
    fn test_update_task_id_existing_target() {
        let _repo = testing::TestRepo::new();
        let task_a = Task::construct_task("Task A".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        let task_a = create_task(task_a).unwrap();
        let id_a = task_a.get_id().unwrap();
        let task_b = Task::construct_task("Task B".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        let task_b = create_task(task_b).unwrap();
        let id_b = task_b.get_id().unwrap();

        let result = update_task_id(&id_b, &id_a, false);
        assert!(result.is_err(), "renaming onto an existing task id must fail");
        assert_eq!(find_task(&id_a).unwrap().unwrap().get_property("name").unwrap(), "Task A");
        assert_eq!(find_task(&id_b).unwrap().unwrap().get_property("name").unwrap(), "Task B");

        let _ = delete_tasks(&[&id_a, &id_b]);
    }

    #[test]
    fn test_update_task_id_force_overwrites_existing_target() {
        let _repo = testing::TestRepo::new();
        let task_a = Task::construct_task("Task A".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        let id_a = create_task(task_a).unwrap().get_id().unwrap();
        let task_b = Task::construct_task("Task B".to_string(), "".to_string(), "OPEN".to_string(), Some(get_current_timestamp()));
        let id_b = create_task(task_b).unwrap().get_id().unwrap();

        let result = update_task_id(&id_b, &id_a, true);
        assert_eq!(result, Ok(()));
        assert_eq!(find_task(&id_a).unwrap().unwrap().get_property("name").unwrap(), "Task B", "the forced rename must replace the task owning the id");
        assert!(find_task(&id_b).unwrap().is_none(), "the old id must be gone");
    }

    #[test]
    fn test_update_task_id_nonexistent() {
        let _repo = testing::TestRepo::new();
        let result = update_task_id("99999", "99998", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_update_comment_id_with_no_id_comment() {
        let _repo = testing::TestRepo::new();
        let mut task = Task::construct_task(
            "Test task".to_string(),
            "Description".to_string(),
            "OPEN".to_string(),
            Some(get_current_timestamp()),
        );
        let comment_with_id = Comment::new("1".to_string(), HashMap::new(), "Comment with ID".to_string());
        let comment_without_id = Comment { id: None, props: HashMap::new(), text: "Comment without ID".to_string() };
        task.set_comments(vec![comment_without_id, comment_with_id]);
        let create_result = create_task(task);
        assert!(create_result.is_ok());
        let task = create_result.unwrap();
        let task_id = task.get_id().unwrap();

        let result = update_comment_id(&task_id, "1", "2");
        assert!(result.is_ok());

        let _ = delete_tasks(&[&task_id]);
    }

    #[test]
    fn test_find_comment_by_id_with_no_id_comment() {
        let _repo = testing::TestRepo::new();
        let mut task = Task::construct_task(
            "Test task".to_string(),
            "Description".to_string(),
            "OPEN".to_string(),
            Some(get_current_timestamp()),
        );
        let comment_no_id = Comment { id: None, props: HashMap::new(), text: "No ID".to_string() };
        let comment_with_id = Comment::new("5".to_string(), HashMap::new(), "Has ID".to_string());
        task.set_comments(vec![comment_no_id, comment_with_id]);

        let comments = task.get_comments().clone().unwrap();
        let result = std::panic::catch_unwind(|| {
            comments.iter().any(|c| c.get_id().map(|id| id == "5").unwrap_or(false))
        });
        assert!(result.is_ok(), "Finding comment by ID should not panic when other comments have no ID");
        assert!(result.unwrap(), "Should find the comment with ID 5");
    }

    #[test]
    fn test_collect_comment_ids_with_no_id_comment() {
        let _repo = testing::TestRepo::new();
        let mut task = Task::construct_task(
            "Test task".to_string(),
            "Description".to_string(),
            "OPEN".to_string(),
            Some(get_current_timestamp()),
        );
        let comment_no_id = Comment { id: None, props: HashMap::new(), text: "No ID".to_string() };
        let comment_with_id = Comment::new("5".to_string(), HashMap::new(), "Has ID".to_string());
        task.set_comments(vec![comment_no_id, comment_with_id]);

        let comments = task.get_comments().as_ref().unwrap();
        let result = std::panic::catch_unwind(|| {
            let ids: Vec<String> = comments.iter().filter_map(|c| c.get_id()).collect();
            ids
        });
        assert!(result.is_ok(), "Collecting comment IDs should not panic when some comments have no ID");
        assert_eq!(result.unwrap(), vec!["5".to_string()]);
    }

    #[test]
    fn test_update_task_id_same_id() {
        let _repo = testing::TestRepo::new();
        let task = Task::construct_task(
            "Test task".to_string(),
            "Description".to_string(),
            "OPEN".to_string(),
            Some(get_current_timestamp()),
        );
        let create_result = create_task(task);
        assert!(create_result.is_ok());
        let task = create_result.unwrap();
        let id = task.get_id().unwrap();

        let result = update_task_id(&id, &id, false);
        assert!(result.is_ok());

        let find_result = find_task(&id);
        assert!(find_result.is_ok());
        assert!(find_result.unwrap().is_some(), "Task should still exist after updating ID to the same value");

        let _ = delete_tasks(&[&id]);
    }

    #[test]
    fn test_add_and_delete_subtask() {
        let _repo = testing::TestRepo::new();
        let mut task = Task::construct_task(
            "Parent task".to_string(),
            "Description".to_string(),
            "OPEN".to_string(),
            Some(get_current_timestamp()),
        );

        let added = task.add_subtask(None, "Subtask one".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        assert_eq!(added.get_id().unwrap(), "1");
        assert_eq!(added.get_name(), "Subtask one");
        assert_eq!(added.get_status(), "OPEN");
        assert!(added.get_all_properties().contains_key("created"));

        let second = task.add_subtask(None, "Subtask two".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        assert_eq!(second.get_id().unwrap(), "2");

        assert_eq!(task.get_subtasks().as_ref().unwrap().len(), 2);

        task.delete_subtask(&"1".to_string()).unwrap();
        let subtasks = task.get_subtasks().as_ref().unwrap();
        assert_eq!(subtasks.len(), 1);
        assert_eq!(subtasks[0].get_id().unwrap(), "2");

        let err = task.delete_subtask(&"99".to_string());
        assert!(err.is_err());
    }

    #[test]
    fn test_subtask_empty_name_or_status() {
        let _repo = testing::TestRepo::new();
        let mut task = Task::construct_task(
            "Parent".to_string(),
            "".to_string(),
            "OPEN".to_string(),
            Some(get_current_timestamp()),
        );
        assert!(task.add_subtask(None, "".to_string(), "OPEN".to_string(), HashMap::new()).is_err());
        assert!(task.add_subtask(None, "Name".to_string(), "".to_string(), HashMap::new()).is_err());
    }

    #[test]
    fn test_subtask_id_no_collision_after_deletion() {
        let _repo = testing::TestRepo::new();
        let mut task = Task::construct_task(
            "Parent".to_string(),
            "".to_string(),
            "OPEN".to_string(),
            Some(get_current_timestamp()),
        );
        task.add_subtask(None, "A".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        task.add_subtask(None, "B".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        task.add_subtask(None, "C".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        task.delete_subtask(&"2".to_string()).unwrap();
        task.add_subtask(None, "D".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();

        let ids: Vec<String> = task.get_subtasks().as_ref().unwrap().iter()
            .map(|s| s.get_id().unwrap())
            .collect();
        let unique: std::collections::HashSet<String> = ids.iter().cloned().collect();
        assert_eq!(ids.len(), unique.len(), "Subtask IDs must be unique, got: {:?}", ids);
    }

    #[test]
    fn test_update_subtask_in_place() {
        let _repo = testing::TestRepo::new();
        let mut task = Task::construct_task(
            "Parent".to_string(),
            "".to_string(),
            "OPEN".to_string(),
            Some(get_current_timestamp()),
        );
        task.add_subtask(None, "A".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        task.update_subtask("1", |s| {
            s.set_name("AA".to_string());
            s.set_status("CLOSED".to_string());
            s.set_property("priority", "HIGH");
        }).unwrap();

        let subtask = task.get_subtask("1").unwrap();
        assert_eq!(subtask.get_name(), "AA");
        assert_eq!(subtask.get_status(), "CLOSED");
        assert_eq!(subtask.get_property("priority").unwrap(), "HIGH");
    }

    #[test]
    fn test_compare_ids_orders_by_project_then_number() {
        let mut ids = vec!["XYZ-1", "ABC-42", "10", "ABC-7", "2", "abc-1"];
        ids.sort_by(|a, b| compare_ids(a, b));
        assert_eq!(ids, vec!["2", "10", "ABC-7", "ABC-42", "XYZ-1", "abc-1"], "numeric IDs first, then issue keys by project and number");
    }

    #[test]
    fn test_subtask_roundtrip_serialization() {
        let _repo = testing::TestRepo::new();
        let mut task = Task::construct_task(
            "Parent".to_string(),
            "Desc".to_string(),
            "OPEN".to_string(),
            Some(123),
        );
        task.add_subtask(
            Some("GH-10".to_string()),
            "Remote subtask".to_string(),
            "IN_PROGRESS".to_string(),
            HashMap::new(),
        ).unwrap();

        let json = serde_json::to_string(&task).unwrap();
        let restored: Task = serde_json::from_str(&json).unwrap();
        let subtasks = restored.get_subtasks().as_ref().unwrap();
        assert_eq!(subtasks.len(), 1);
        assert_eq!(subtasks[0].get_id().unwrap(), "GH-10");
        assert_eq!(subtasks[0].get_name(), "Remote subtask");
        assert_eq!(subtasks[0].get_status(), "IN_PROGRESS");
    }

    #[test]
    fn test_task_without_subtasks_deserializes() {
        let json = r#"{"id":"5","props":{"name":"x","status":"OPEN","description":"","created":"1"},"comments":null,"labels":null}"#;
        let task: Task = serde_json::from_str(json).unwrap();
        assert!(task.get_subtasks().is_none());
    }

    #[test]
    fn test_update_subtask_id_nonexistent_task() {
        let _repo = testing::TestRepo::new();
        let result = update_subtask_id("99999", "1", "2");
        assert!(result.is_err());
    }

    #[test]
    fn test_new_task_rejects_blank_name_or_status() {
        let _repo = testing::TestRepo::new();
        assert!(Task::new("".to_string(), "".to_string(), "OPEN".to_string()).is_err());
        assert!(Task::new("   ".to_string(), "".to_string(), "OPEN".to_string()).is_err(), "whitespace-only name must be rejected");
        assert!(Task::new("Name".to_string(), "".to_string(), "".to_string()).is_err());
        assert!(Task::new("Name".to_string(), "".to_string(), " \t".to_string()).is_err(), "whitespace-only status must be rejected");
        assert!(Task::new("Name".to_string(), "".to_string(), "OPEN".to_string()).is_ok());
    }

    #[test]
    fn test_add_duplicate_label() {
        let _repo = testing::TestRepo::new();
        let mut task = Task::construct_task(
            "Test task".to_string(),
            "Description".to_string(),
            "OPEN".to_string(),
            Some(get_current_timestamp()),
        );

        task.add_label("bug".to_string(), None, None);
        task.add_label("bug".to_string(), None, None);

        let labels = task.get_labels().as_ref().unwrap();
        let unique_names: std::collections::HashSet<String> = labels.iter().map(|l| l.get_name()).collect();
        assert_eq!(labels.len(), unique_names.len(), "Label names must be unique, got duplicates");
    }
}