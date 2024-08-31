use std::borrow::ToOwned;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use git2::*;
use serde_json;
use serde::{Deserialize, Serialize};

const NAME: &'static str = "name";
const DESCRIPTION: &'static str = "description";
const STATUS: &'static str = "status";
const CREATED: &'static str = "created";

#[derive(Serialize, Deserialize)]
pub struct Task {
    id: Option<String>,
    props: HashMap<String, String>,
    comments: Option<Vec<Comment>>,
}

#[derive(Serialize, Deserialize)]
pub struct Comment {
    id: Option<String>,
    props: HashMap<String, String>,
    text: String,
}

impl Task {
    pub fn new(name: String, description: String, status: String) -> Result<Task, &'static str> {
        if !name.is_empty() && !status.is_empty() {
            Ok(Self::construct_task(name, description, status, None))
        } else {
            Err("Name or status is empty")
        }
    }

    pub fn from_properties(id: String, mut props: HashMap<String, String>) -> Result<Task, &'static str> {
        let name = props.get(NAME).unwrap_or(&"".to_owned()).to_owned();
        let status = props.get(STATUS).unwrap_or(&"".to_owned()).to_owned();

        if !name.is_empty() && !status.is_empty() {
            if !props.contains_key("created") {
                props.insert("created".to_string(), get_current_timestamp().to_string());
            }

            Ok(Task{ id: Some(id), props, comments: None })
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

    pub fn set_property(&mut self, prop: String, value: String) {
        self.props.insert(prop, value);
    }

    pub fn delete_property(&mut self, prop: &str) {
        self.props.remove(prop);
    }

    pub fn get_comments(&self) -> &Option<Vec<Comment>> {
        &self.comments
    }

    pub fn add_comment(&mut self, id: Option<String>, mut props: HashMap<String, String>, text: String) {
        if self.comments.is_none() {
            self.comments = Some(vec![]);
        }

        let id = Some(id.unwrap_or_else(|| (self.comments.as_ref().unwrap().len() + 1).to_string()));

        if !props.contains_key("created") {
            props.insert("created".to_string(), get_current_timestamp().to_string());
        }

        if !props.contains_key("author") {
            if let Ok(Some(current_user)) = get_current_user() {
                props.insert("author".to_string(), current_user);
            }
        }

        self.comments.as_mut().unwrap().push(Comment {
            id,
            props,
            text,
        });
    }

    pub fn set_comments(&mut self, comments: Vec<Comment>) {
        self.comments = Some(comments);
    }

    pub fn delete_comment(&mut self, id: String) -> Result<(), String> {
        if self.comments.is_none() {
            return Err("Task has no comments".to_string());
        }

        let index = self.comments.as_ref().unwrap().iter().position(|comment| comment.get_id().unwrap() == id);

        if index.is_none() {
            return Err(format!("Comment ID {} not found", id.clone()));
        }

        self.comments.as_mut().unwrap().remove(index.unwrap());

        Ok(())
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

    pub fn get_all_properties(&self) -> &HashMap<String, String> {
        &self.props
    }

    pub fn get_text(&self) -> String {
        self.text.to_string()
    }
}

macro_rules! map_err {
    ($expr:expr) => {
        $expr.map_err(|e| e.message().to_owned())?
    }
}

pub fn list_tasks() -> Result<Vec<Task>, String> {
    let repo = map_err!(Repository::open("."));
    let task_ref = map_err!(repo.find_reference(&get_ref_path_from_repo(&repo)));
    let task_tree = map_err!(task_ref.peel_to_tree());

    let mut result = vec![];

    let _ = map_err!(task_tree.walk(TreeWalkMode::PreOrder, |_, entry| {
        let oid = entry.id();
        let blob = repo.find_blob(oid).unwrap();
        let content = blob.content();

        let task = serde_json::from_slice(content).unwrap();
        result.push(task);

        TreeWalkResult::Ok
    }));

    Ok(result)
}

pub fn find_task(id: &str) -> Result<Option<Task>, String> {
    let repo = map_err!(Repository::open("."));
    let task_ref = map_err!(repo.find_reference(&get_ref_path_from_repo(&repo)));
    let task_tree = map_err!(task_ref.peel_to_tree());

    let mut result = None;

    let _ = map_err!(task_tree.walk(TreeWalkMode::PreOrder, |_, entry| {
        let entry_name = entry.name().unwrap();
        if entry_name != id {
            return TreeWalkResult::Skip
        }

        let oid = entry.id();
        let blob = repo.find_blob(oid).unwrap();
        let content = blob.content();

        let task = serde_json::from_slice(content).unwrap();
        result = Some(task);

        TreeWalkResult::Abort
    }));

    Ok(result)
}

pub fn delete_tasks(ids: &[&str]) -> Result<(), String> {
    let repo = map_err!(Repository::open("."));
    let task_ref = map_err!(repo.find_reference(&get_ref_path_from_repo(&repo)));
    let task_tree = map_err!(task_ref.peel_to_tree());

    let mut treebuilder = map_err!(repo.treebuilder(Some(&task_tree)));
    for id in ids {
        map_err!(treebuilder.remove(id));
    }
    let tree_oid = map_err!(treebuilder.write());

    let parent_commit = map_err!(task_ref.peel_to_commit());
    let parents = vec![parent_commit];
    let me = &repo.signature().unwrap();

    map_err!(repo.commit(Some(&get_ref_path_from_repo(&repo)), me, me, "delete task", &repo.find_tree(tree_oid).unwrap(), &parents.iter().collect::<Vec<_>>()));

    Ok(())
}

pub fn clear_tasks() -> Result<u64, String> {
    let repo = map_err!(Repository::open("."));
    let task_ref = map_err!(repo.find_reference(&get_ref_path_from_repo(&repo)));
    let task_tree = map_err!(task_ref.peel_to_tree());

    let mut treebuilder = map_err!(repo.treebuilder(Some(&task_tree)));
    let task_count = treebuilder.len() as u64;
    map_err!(treebuilder.clear());
    let tree_oid = map_err!(treebuilder.write());

    let parent_commit = map_err!(task_ref.peel_to_commit());
    let parents = vec![parent_commit];
    let me = &repo.signature().unwrap();

    map_err!(repo.commit(Some(&get_ref_path_from_repo(&repo)), me, me, "delete task", &repo.find_tree(tree_oid).unwrap(), &parents.iter().collect::<Vec<_>>()));

    Ok(task_count)
}

pub fn create_task(mut task: Task) -> Result<String, String> {
    let repo = map_err!(Repository::open("."));
    let task_ref_result = repo.find_reference(&get_ref_path_from_repo(&repo));
    let source_tree = match task_ref_result {
        Ok(ref reference) => {
            match reference.peel_to_tree() {
                Ok(tree) => Some(tree),
                _ => None
            }
        }
        _ => { None }
    };

    let task_id = if task.get_id().is_some() { task.get_id() } else {
        let id = get_next_id().unwrap_or("1".to_string());
        task.set_id(id.clone());
        Some(id)
    };
    let string_content = serde_json::to_string(&task).unwrap();
    let content = string_content.as_bytes();
    let oid = repo.blob(content).unwrap();
    let mut treebuilder = map_err!(repo.treebuilder(source_tree.as_ref()));
    map_err!(treebuilder.insert(task_id.clone().unwrap(), oid, FileMode::Blob.into()));
    let tree_oid = map_err!(treebuilder.write());

    let me = &repo.signature().unwrap();
    let mut parents = vec![];
    if task_ref_result.is_ok() {
        let parent_commit = task_ref_result.unwrap().peel_to_commit();
        if parent_commit.is_ok() {
            parents.push(parent_commit.unwrap());
        }
    }
    map_err!(repo.commit(Some(&get_ref_path_from_repo(&repo)), me, me, "create task", &repo.find_tree(tree_oid).unwrap(), &parents.iter().collect::<Vec<_>>()));

    Ok(task_id.unwrap())
}

pub fn update_task(task: Task) -> Result<String, String> {
    let repo = map_err!(Repository::open("."));
    let task_ref_result = repo.find_reference(&get_ref_path_from_repo(&repo)).unwrap();
    let parent_commit = task_ref_result.peel_to_commit().unwrap();
    let source_tree = task_ref_result.peel_to_tree().unwrap();
    let string_content = serde_json::to_string(&task).unwrap();
    let content = string_content.as_bytes();
    let oid = repo.blob(content).unwrap();
    let mut treebuilder = map_err!(repo.treebuilder(Some(&source_tree)));
    let task_id = task.get_id().unwrap();
    map_err!(treebuilder.insert(task_id.clone(), oid, FileMode::Blob.into()));
    let tree_oid = map_err!(treebuilder.write());

    let me = &repo.signature().unwrap();
    let parents = vec![parent_commit];
    map_err!(repo.commit(Some(&get_ref_path_from_repo(&repo)), me, me, "update task", &repo.find_tree(tree_oid).unwrap(), &parents.iter().collect::<Vec<_>>()));

    Ok(task_id)
}

fn get_next_id() -> Result<String, String> {
    let repo = map_err!(Repository::open("."));
    let task_ref = map_err!(repo.find_reference(&get_ref_path_from_repo(&repo)));
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

pub fn update_task_id(id: &str, new_id: &str) -> Result<(), String> {
    let mut task = find_task(&id)?.unwrap();
    task.set_id(new_id.to_string());
    create_task(task)?;
    delete_tasks(&[&id])?;

    Ok(())
}

pub fn list_remotes(remote: Option<String>) -> Result<Vec<String>, String> {
    let repo = map_err!(Repository::open("."));
    let remotes = map_err!(repo.remotes());
    Ok(remotes.iter()
        .filter(|s| remote.is_none() || remote.as_ref().unwrap().as_str() == s.unwrap())
        .map(|s| repo.find_remote(s.unwrap()).unwrap().url().unwrap().to_owned())
        .collect())
}

fn get_current_timestamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn get_current_user() -> Result<Option<String>, String> {
    let repo = map_err!(Repository::open("."));
    let me = &repo.signature().unwrap();
    match me.name() {
        Some(name) => Ok(Some(String::from(name))),
        _ => match me.email() {
            Some(email) => Ok(Some(String::from(email))),
            _ => Ok(None),
        }
    }
}

pub fn get_ref_path() -> Result<String, String> {
    let repo = map_err!(Repository::open("."));
    Ok(get_ref_path_from_repo(&repo))
}

fn get_ref_path_from_repo(repo: &Repository) -> String {
    if let Ok(config) = repo.config() {
        if let Ok(ref_path) = config.get_string("task.ref") {
            return ref_path;
        }
    }

    "refs/tasks/tasks".to_string()
}

pub fn set_ref_path(ref_path: &str) -> Result<(), String> {
    let repo = map_err!(Repository::open("."));

    let current_reference = map_err!(repo.find_reference(&get_ref_path_from_repo(&repo)));
    let commit = current_reference.peel_to_commit().unwrap();
    map_err!(repo.reference(ref_path, commit.id(), true, "task.ref migrated"));

    let mut config = map_err!(repo.config());
    map_err!(config.set_str("task.ref", ref_path));

    Ok(())
}