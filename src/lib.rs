use std::borrow::ToOwned;
use std::collections::HashMap;
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
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
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

        let id = Some(id.unwrap_or_else(|| (self.comments.as_ref().unwrap().len() + 1).to_string()));

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

        let index = self.comments.as_ref().unwrap().iter().position(|comment| comment.get_id().unwrap() == id.deref());

        if index.is_none() {
            return Err(format!("Comment ID {id} not found"));
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

macro_rules! map_err {
    ($expr:expr) => {
        $expr.map_err(|e| e.message().to_owned())?
    }
}

pub fn list_tasks() -> Result<Vec<Task>, String> {
    let repo = map_err!(Repository::discover("."));
    let task_ref = map_err!(repo.find_reference(&get_ref_path()));
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
    let repo = map_err!(Repository::discover("."));
    let task_ref = repo.find_reference(&get_ref_path());
    match task_ref {
        Ok(task_ref) => {
            let task_tree = map_err!(task_ref.peel_to_tree());
            let result = match task_tree.get_name(id) {
                Some(entry) => {
                    let oid = entry.id();
                    let blob = map_err!(repo.find_blob(oid));
                    let content = blob.content();
                    let task = serde_json::from_slice(content).unwrap();

                    Some(task)
                },
                None => None,
            };

            Ok(result)
        },
        Err(_) => Ok(None)
    }
}

pub fn delete_tasks(ids: &[&str]) -> Result<(), String> {
    let repo = map_err!(Repository::discover("."));
    let task_ref = map_err!(repo.find_reference(&get_ref_path()));
    let task_tree = map_err!(task_ref.peel_to_tree());

    let mut treebuilder = map_err!(repo.treebuilder(Some(&task_tree)));
    for id in ids {
        map_err!(treebuilder.remove(id));
    }
    let tree_oid = map_err!(treebuilder.write());

    let parent_commit = map_err!(task_ref.peel_to_commit());
    let parents = vec![parent_commit];
    let me = &map_err!(repo.signature());

    let mut ids = ids.iter().map(|id| id.parse::<u64>().unwrap()).collect::<Vec<_>>();
    ids.sort();
    let ids = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(", ");
    map_err!(repo.commit(Some(&get_ref_path()), me, me, format!("Delete task {}", ids).as_str(), &map_err!(repo.find_tree(tree_oid)), &parents.iter().collect::<Vec<_>>()));

    Ok(())
}

pub fn clear_tasks() -> Result<u64, String> {
    let repo = map_err!(Repository::discover("."));
    let task_ref = map_err!(repo.find_reference(&get_ref_path()));
    let task_tree = map_err!(task_ref.peel_to_tree());

    let mut treebuilder = map_err!(repo.treebuilder(Some(&task_tree)));
    let task_count = treebuilder.len() as u64;
    map_err!(treebuilder.clear());
    let tree_oid = map_err!(treebuilder.write());

    let parent_commit = map_err!(task_ref.peel_to_commit());
    let parents = vec![parent_commit];
    let me = &map_err!(repo.signature());

    map_err!(repo.commit(Some(&get_ref_path()), me, me, "Clear tasks", &map_err!(repo.find_tree(tree_oid)), &parents.iter().collect::<Vec<_>>()));

    Ok(task_count)
}

pub fn create_task(mut task: Task) -> Result<Task, String> {
    let repo = map_err!(Repository::discover("."));
    let task_ref_result = repo.find_reference(&get_ref_path());
    let source_tree = match task_ref_result {
        Ok(ref reference) => {
            match reference.peel_to_tree() {
                Ok(tree) => Some(tree),
                _ => None
            }
        }
        _ => { None }
    };

    if task.get_id().is_none() {
        let id = get_next_id().unwrap_or_else(|_| "1".to_string());
        task.set_id(id);
    }
    let string_content = serde_json::to_string(&task).unwrap();
    let content = string_content.as_bytes();
    let oid = map_err!(repo.blob(content));
    let mut treebuilder = map_err!(repo.treebuilder(source_tree.as_ref()));
    map_err!(treebuilder.insert(&task.get_id().unwrap(), oid, FileMode::Blob.into()));
    let tree_oid = map_err!(treebuilder.write());

    let me = &map_err!(repo.signature());
    let mut parents = vec![];
    if task_ref_result.is_ok() {
        let parent_commit = map_err!(task_ref_result).peel_to_commit();
        if parent_commit.is_ok() {
            parents.push(map_err!(parent_commit));
        }
    }
    map_err!(repo.commit(Some(&get_ref_path()), me, me, format!("Create task {}", &task.get_id().unwrap_or_else(|| String::from("?"))).as_str(), &map_err!(repo.find_tree(tree_oid)), &parents.iter().collect::<Vec<_>>()));

    Ok(task)
}

pub fn update_task(task: Task) -> Result<String, String> {
    let repo = map_err!(Repository::discover("."));
    let task_ref_result = map_err!(repo.find_reference(&get_ref_path()));
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
    map_err!(repo.commit(Some(&get_ref_path()), me, me, format!("Update task {}", &task.get_id().unwrap()).as_str(), &map_err!(repo.find_tree(tree_oid)), &parents.iter().collect::<Vec<_>>()));

    Ok(task.get_id().unwrap())
}

fn get_next_id() -> Result<String, String> {
    let repo = map_err!(Repository::discover("."));
    let task_ref = map_err!(repo.find_reference(&get_ref_path()));
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

pub fn update_comment_id(task_id: &str, id: &str, new_id: &str) -> Result<(), String> {
    let mut task = find_task(&task_id)?.unwrap().clone();
    let comments = task.get_comments();
    match comments {
        Some(comments) => {
            let updated_comments = comments.iter().map(|c| {
                if c.get_id().unwrap() == id {
                    let mut c = c.clone();
                    c.set_id(new_id.to_string());
                    c
                } else {
                    c.clone()
                }
            }).collect::<Vec<_>>();
            task.set_comments(updated_comments);
            update_task(task)?;
        },
        None => {}
    }

    Ok(())
}

pub fn list_remotes(remote: &Option<String>) -> Result<Vec<String>, String> {
    let repo = map_err!(Repository::discover("."));
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
    let repo = map_err!(Repository::discover("."));
    let me = &map_err!(repo.signature());
    match me.name() {
        Some(name) => Ok(Some(String::from(name))),
        _ => match me.email() {
            Some(email) => Ok(Some(String::from(email))),
            _ => Ok(None),
        }
    }
}

pub fn get_ref_path() -> String {
    get_config_value("task.ref").unwrap_or_else(|_| "refs/tasks/tasks".to_string())
}

pub fn get_config_value(key: &str) -> Result<String, String> {
    let repo = map_err!(Repository::discover("."));
    let config = map_err!(repo.config());
    Ok(map_err!(config.get_string(key)))
}

pub fn set_config_value(key: &str, value: &str) -> Result<(), String> {
    let repo = map_err!(Repository::discover("."));
    let mut config = map_err!(repo.config());
    map_err!(config.set_str(key, value));
    Ok(())
}

pub fn set_ref_path(ref_path: &str, move_ref: bool) -> Result<(), String> {
    let repo = map_err!(Repository::discover("."));

    let current_reference = repo.find_reference(&get_ref_path());
    if let Ok(current_reference) = &current_reference {
        let commit = map_err!(current_reference.peel_to_commit());
        map_err!(repo.reference(ref_path, commit.id(), true, "task.ref migrated"));
    }

    let mut config = map_err!(repo.config());
    map_err!(config.set_str("task.ref", ref_path));

    if move_ref && current_reference.is_ok() {
        map_err!(current_reference.unwrap().delete());
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use crate::{create_task, delete_tasks, find_task, get_current_timestamp, get_next_id, get_ref_path, set_ref_path, update_task, Task};

    #[test]
    fn test_ref_path() {
        let ref_path = get_ref_path();
        assert!(set_ref_path("refs/heads/test-git-task", true).is_ok());
        assert_eq!(get_ref_path(), "refs/heads/test-git-task");
        assert!(set_ref_path(&ref_path, true).is_ok());
        assert_eq!(get_ref_path(), ref_path);
    }

    #[test]
    fn test_create_update_delete_task() {
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
}