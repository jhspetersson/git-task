use std::borrow::ToOwned;
use std::collections::HashMap;

use git2::*;
use serde_json;

const NAME: &'static str = "name";
const DESCRIPTION: &'static str = "description";
const STATUS: &'static str = "status";

#[derive(Debug)]
pub struct Task {
    id: Option<String>,
    props: HashMap<String, String>,
}

impl Task {
    pub fn new(name: String, description: String, status: String) -> Result<Task, &'static str> {
        if !name.is_empty() && !status.is_empty() {
            Ok(Self::construct_task(None, name, description, status))
        } else {
            Err("Name or status is empty")
        }
    }

    pub fn from_properties(id: String, map: HashMap<String, String>) -> Result<Task, &'static str> {
        let name = map.get(NAME).unwrap_or(&"".to_owned()).to_owned();
        let status = map.get(STATUS).unwrap_or(&"".to_owned()).to_owned();

        if !name.is_empty() && !status.is_empty() {
            Ok(Task{ id: Some(id), props: map})
        } else {
            Err("Name or status is empty")
        }
    }

    fn construct_task(id: Option<String>, name: String, description: String, status: String) -> Task {
        Task {
            id,
            props: HashMap::from([(NAME.to_owned(), name), (DESCRIPTION.to_owned(), description), (STATUS.to_owned(), status)])
        }
    }

    pub fn get_id(&self) -> Option<String> {
        match &self.id {
            Some(id) => Some(id.clone()),
            _ => None
        }
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
}

pub fn list_tasks() -> Result<Vec<Task>, String> {
    let repo = git2::Repository::open(".").map_err(|e| e.message().to_owned())?;
    let task_ref = repo.find_reference("refs/tasks/tasks").map_err(|e| e.message().to_owned())?;
    let task_tree = task_ref.peel_to_tree().map_err(|e| e.message().to_owned())?;

    let mut result = vec![];

    let _ = task_tree.walk(TreeWalkMode::PreOrder, |_, entry| {
        let entry_name = entry.name().unwrap().to_owned();
        let oid = entry.id();
        let blob = repo.find_blob(oid).unwrap();
        let content = blob.content();

        let map: HashMap<String, String> = serde_json::from_slice(content).unwrap();

        let task = Task::from_properties(entry_name, map).unwrap();
        result.push(task);

        TreeWalkResult::Ok
    }).map_err(|e| e.message().to_owned())?;

    Ok(result)
}

pub fn find_task(id: &str) -> Result<Option<Task>, String> {
    let repo = git2::Repository::open(".").map_err(|e| e.message().to_owned())?;
    let task_ref = repo.find_reference("refs/tasks/tasks").map_err(|e| e.message().to_owned())?;
    let task_tree = task_ref.peel_to_tree().map_err(|e| e.message().to_owned())?;

    let mut result = None;

    let _ = task_tree.walk(TreeWalkMode::PreOrder, |_, entry| {
        let entry_name = entry.name().unwrap();
        if entry_name != id {
            return TreeWalkResult::Skip
        }

        let oid = entry.id();
        let blob = repo.find_blob(oid).unwrap();
        let content = blob.content();

        let map: HashMap<String, String> = serde_json::from_slice(content).unwrap();

        let task = Task::from_properties(entry_name.to_string(), map).unwrap();
        result = Some(task);

        TreeWalkResult::Abort
    }).map_err(|e| e.message().to_owned())?;

    Ok(result)
}

pub fn delete_task(id: &str) -> Result<(), String> {
    let repo = git2::Repository::open(".").map_err(|e| e.message().to_owned())?;
    let task_ref = repo.find_reference("refs/tasks/tasks").map_err(|e| e.message().to_owned())?;
    let task_tree = task_ref.peel_to_tree().map_err(|e| e.message().to_owned())?;

    let mut treebuilder = repo.treebuilder(Some(&task_tree)).map_err(|e| e.message().to_owned())?;
    treebuilder.remove(id).map_err(|e| e.message().to_owned())?;
    let tree_oid = treebuilder.write().map_err(|e| e.message().to_owned())?;

    let parent_commit = task_ref.peel_to_commit().map_err(|e| e.message().to_owned())?;
    let parents = vec![parent_commit];
    let me = &repo.signature().unwrap();

    repo.commit(Some("refs/tasks/tasks"), me, me, "delete task", &repo.find_tree(tree_oid).unwrap(), &parents.iter().collect::<Vec<_>>()).map_err(|e| e.message().to_owned())?;

    Ok(())
}

pub fn clear_tasks() -> Result<u64, String> {
    let repo = git2::Repository::open(".").map_err(|e| e.message().to_owned())?;
    let task_ref = repo.find_reference("refs/tasks/tasks").map_err(|e| e.message().to_owned())?;
    let task_tree = task_ref.peel_to_tree().map_err(|e| e.message().to_owned())?;

    let mut treebuilder = repo.treebuilder(Some(&task_tree)).map_err(|e| e.message().to_owned())?;
    let task_count = treebuilder.len() as u64;
    treebuilder.clear().map_err(|e| e.message().to_owned())?;
    let tree_oid = treebuilder.write().map_err(|e| e.message().to_owned())?;

    let parent_commit = task_ref.peel_to_commit().map_err(|e| e.message().to_owned())?;
    let parents = vec![parent_commit];
    let me = &repo.signature().unwrap();

    repo.commit(Some("refs/tasks/tasks"), me, me, "delete task", &repo.find_tree(tree_oid).unwrap(), &parents.iter().collect::<Vec<_>>()).map_err(|e| e.message().to_owned())?;

    Ok(task_count)
}

pub fn create_task(task: Task) -> Result<String, String> {
    let repo = git2::Repository::open(".").map_err(|e| e.message().to_owned())?;
    let task_ref_result = repo.find_reference("refs/tasks/tasks");
    let source_tree = match task_ref_result {
        Ok(ref reference) => {
            match reference.peel_to_tree() {
                Ok(tree) => Some(tree),
                _ => None
            }
        }
        _ => { None }
    };

    let string_content = serde_json::to_string(task.get_all_properties()).unwrap();
    let content = string_content.as_bytes();
    let oid = repo.blob(content).unwrap();
    let mut treebuilder = repo.treebuilder(source_tree.as_ref()).map_err(|e| e.message().to_owned())?;
    let task_id = if task.get_id().is_some() { task.get_id() } else { Some(get_next_id().unwrap_or("1".to_string())) };
    treebuilder.insert(task_id.clone().unwrap(), oid, FileMode::Blob.into()).map_err(|e| e.message().to_owned())?;
    let tree_oid = treebuilder.write().map_err(|e| e.message().to_owned())?;

    let me = &repo.signature().unwrap();
    let mut parents = vec![];
    if task_ref_result.is_ok() {
        let parent_commit = task_ref_result.unwrap().peel_to_commit();
        if parent_commit.is_ok() {
            parents.push(parent_commit.unwrap());
        }
    }
    repo.commit(Some("refs/tasks/tasks"), me, me, "create task", &repo.find_tree(tree_oid).unwrap(), &parents.iter().collect::<Vec<_>>()).map_err(|e| e.message().to_owned())?;

    Ok(task_id.unwrap())
}

pub fn update_task(task: Task) -> Result<String, String> {
    let repo = git2::Repository::open(".").map_err(|e| e.message().to_owned())?;
    let task_ref_result = repo.find_reference("refs/tasks/tasks").unwrap();
    let parent_commit = task_ref_result.peel_to_commit().unwrap();
    let source_tree = task_ref_result.peel_to_tree().unwrap();
    let string_content = serde_json::to_string(task.get_all_properties()).unwrap();
    let content = string_content.as_bytes();
    let oid = repo.blob(content).unwrap();
    let mut treebuilder = repo.treebuilder(Some(&source_tree)).map_err(|e| e.message().to_owned())?;
    let task_id = task.get_id().unwrap();
    treebuilder.insert(task_id.clone(), oid, FileMode::Blob.into()).map_err(|e| e.message().to_owned())?;
    let tree_oid = treebuilder.write().map_err(|e| e.message().to_owned())?;

    let me = &repo.signature().unwrap();
    let parents = vec![parent_commit];
    repo.commit(Some("refs/tasks/tasks"), me, me, "update task", &repo.find_tree(tree_oid).unwrap(), &parents.iter().collect::<Vec<_>>()).map_err(|e| e.message().to_owned())?;

    Ok(task_id)
}

fn get_next_id() -> Result<String, String> {
    let repo = git2::Repository::open(".").map_err(|e| e.message().to_owned())?;
    let task_ref = repo.find_reference("refs/tasks/tasks").map_err(|e| e.message().to_owned())?;
    let task_tree = task_ref.peel_to_tree().map_err(|e| e.message().to_owned())?;

    let mut result = 0;

    let _ = task_tree.walk(TreeWalkMode::PreOrder, |_, entry| {
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
    }).map_err(|e| e.message().to_owned())?;

    Ok((result + 1).to_string())
}

pub fn list_remotes() -> Result<Vec<String>, String> {
    let repo = Repository::open(".").map_err(|e| e.message().to_owned())?;
    let remotes = repo.remotes().map_err(|e| e.message().to_owned())?;
    Ok(remotes.iter().map(|s| repo.find_remote(s.unwrap()).unwrap().url().unwrap().to_owned()).collect())
}