use std::borrow::ToOwned;
use std::collections::HashMap;

use git2::*;
use serde_json;

const NAME: &'static str = "name";
const DESCRIPTION: &'static str = "description";
const STATUS: &'static str = "status";

//#[derive(Deserialize, Debug)]
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
}

macro_rules! map_err {
    ($expr:expr) => {
        $expr.map_err(|e| e.message().to_owned())?
    }
}

pub fn list_tasks() -> Result<Vec<Task>, String> {
    let repo = map_err!(Repository::open("."));
    let task_ref = map_err!(repo.find_reference("refs/tasks/tasks"));
    let task_tree = map_err!(task_ref.peel_to_tree());

    let mut result = vec![];

    let _ = map_err!(task_tree.walk(TreeWalkMode::PreOrder, |_, entry| {
        let entry_name = entry.name().unwrap().to_owned();
        let oid = entry.id();
        let blob = repo.find_blob(oid).unwrap();
        let content = blob.content();

        let map: HashMap<String, String> = serde_json::from_slice(content).unwrap();

        let task = Task::from_properties(entry_name, map).unwrap();
        result.push(task);

        TreeWalkResult::Ok
    }));

    Ok(result)
}

pub fn find_task(id: &str) -> Result<Option<Task>, String> {
    let repo = map_err!(Repository::open("."));
    let task_ref = map_err!(repo.find_reference("refs/tasks/tasks"));
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

        let map: HashMap<String, String> = serde_json::from_slice(content).unwrap();

        let task = Task::from_properties(entry_name.to_string(), map).unwrap();
        result = Some(task);

        TreeWalkResult::Abort
    }));

    Ok(result)
}

pub fn delete_tasks(ids: &[&str]) -> Result<(), String> {
    let repo = map_err!(Repository::open("."));
    let task_ref = map_err!(repo.find_reference("refs/tasks/tasks"));
    let task_tree = map_err!(task_ref.peel_to_tree());

    let mut treebuilder = map_err!(repo.treebuilder(Some(&task_tree)));
    for id in ids {
        map_err!(treebuilder.remove(id));
    }
    let tree_oid = map_err!(treebuilder.write());

    let parent_commit = map_err!(task_ref.peel_to_commit());
    let parents = vec![parent_commit];
    let me = &repo.signature().unwrap();

    map_err!(repo.commit(Some("refs/tasks/tasks"), me, me, "delete task", &repo.find_tree(tree_oid).unwrap(), &parents.iter().collect::<Vec<_>>()));

    Ok(())
}

pub fn clear_tasks() -> Result<u64, String> {
    let repo = map_err!(Repository::open("."));
    let task_ref = map_err!(repo.find_reference("refs/tasks/tasks"));
    let task_tree = map_err!(task_ref.peel_to_tree());

    let mut treebuilder = map_err!(repo.treebuilder(Some(&task_tree)));
    let task_count = treebuilder.len() as u64;
    map_err!(treebuilder.clear());
    let tree_oid = map_err!(treebuilder.write());

    let parent_commit = map_err!(task_ref.peel_to_commit());
    let parents = vec![parent_commit];
    let me = &repo.signature().unwrap();

    map_err!(repo.commit(Some("refs/tasks/tasks"), me, me, "delete task", &repo.find_tree(tree_oid).unwrap(), &parents.iter().collect::<Vec<_>>()));

    Ok(task_count)
}

pub fn create_task(task: Task) -> Result<String, String> {
    let repo = map_err!(Repository::open("."));
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
    let mut treebuilder = map_err!(repo.treebuilder(source_tree.as_ref()));
    let task_id = if task.get_id().is_some() { task.get_id() } else { Some(get_next_id().unwrap_or("1".to_string())) };
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
    map_err!(repo.commit(Some("refs/tasks/tasks"), me, me, "create task", &repo.find_tree(tree_oid).unwrap(), &parents.iter().collect::<Vec<_>>()));

    Ok(task_id.unwrap())
}

pub fn update_task(task: Task) -> Result<String, String> {
    let repo = map_err!(Repository::open("."));
    let task_ref_result = repo.find_reference("refs/tasks/tasks").unwrap();
    let parent_commit = task_ref_result.peel_to_commit().unwrap();
    let source_tree = task_ref_result.peel_to_tree().unwrap();
    let string_content = serde_json::to_string(task.get_all_properties()).unwrap();
    let content = string_content.as_bytes();
    let oid = repo.blob(content).unwrap();
    let mut treebuilder = map_err!(repo.treebuilder(Some(&source_tree)));
    let task_id = task.get_id().unwrap();
    map_err!(treebuilder.insert(task_id.clone(), oid, FileMode::Blob.into()));
    let tree_oid = map_err!(treebuilder.write());

    let me = &repo.signature().unwrap();
    let parents = vec![parent_commit];
    map_err!(repo.commit(Some("refs/tasks/tasks"), me, me, "update task", &repo.find_tree(tree_oid).unwrap(), &parents.iter().collect::<Vec<_>>()));

    Ok(task_id)
}

fn get_next_id() -> Result<String, String> {
    let repo = map_err!(Repository::open("."));
    let task_ref = map_err!(repo.find_reference("refs/tasks/tasks"));
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

pub fn list_remotes() -> Result<Vec<String>, String> {
    let repo = map_err!(Repository::open("."));
    let remotes = map_err!(repo.remotes());
    Ok(remotes.iter().map(|s| repo.find_remote(s.unwrap()).unwrap().url().unwrap().to_owned()).collect())
}