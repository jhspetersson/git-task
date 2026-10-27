use crate::status;
use crate::status::StatusManager;
use crate::util::{error_message, read_from_pipe, success_message};

pub(crate) fn task_config_status_add(name: String, shortcut: String, color: String, is_done: Option<bool>) -> bool {
    let mut status_manager = StatusManager::new();
    match status_manager.add_status(name, shortcut, color, is_done.unwrap_or(false)) {
        Ok(_) => success_message("Status has been added".to_string()),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_config_status_delete(name: String, force: bool) -> bool {
    let mut status_manager = StatusManager::new();
    let name = status_manager.get_full_status_name(&name);

    if !force {
        if let Ok(tasks) = gittask::list_tasks() {
            let task_exists = tasks.iter().any(|task| task.get_property("status").map_or(false, |s| s == name.as_str()));
            if task_exists {
                return error_message("Can't delete a status, some tasks still have it. Use --force option to override.".to_string());
            }
        }
    }

    match status_manager.delete_status(name) {
        Ok(_) => success_message("Status has been deleted".to_string()),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_config_status_get(name: String, param: String) -> bool {
    let status_manager = StatusManager::new();
    let name = status_manager.get_full_status_name(&name);
    match status_manager.get_property(&name, &param) {
        Some(value) => success_message(value),
        None => error_message(format!("Unknown status {name} or property: {param}"))
    }
}

pub(crate) fn task_config_status_set(name: String, param: String, value: String) -> bool {
    let mut status_manager = StatusManager::new();
    let name = status_manager.get_full_status_name(&name);
    match status_manager.set_property(&name, &param, &value) {
        Ok(prev_value) => {
            println!("{name} {param} has been updated");

            match (param.as_str(), prev_value) {
                ("name", Some(prev_status)) => rename_status_in_tasks(&prev_status, &value),
                _ => true,
            }
        },
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

fn rename_status_in_tasks(from: &str, to: &str) -> bool {
    let tasks = match gittask::list_tasks() {
        Ok(tasks) => tasks,
        Err(e) => return error_message(format!("ERROR: {e}")),
    };
    let changed: Vec<_> = tasks.into_iter().filter_map(|mut task| {
        let mut changed = false;
        if task.get_property("status").map_or(false, |s| s == from) {
            task.set_property("status", to);
            changed = true;
        }
        if let Some(subtasks) = task.get_subtasks() {
            if subtasks.iter().any(|s| s.get_status() == from) {
                let subtasks = subtasks.iter().cloned().map(|mut subtask| {
                    if subtask.get_status() == from {
                        subtask.set_status(to.to_string());
                    }
                    subtask
                }).collect();
                task.set_subtasks(subtasks);
                changed = true;
            }
        }
        if changed { Some(task) } else { None }
    }).collect();
    match gittask::update_tasks(changed) {
        Ok(updated) => {
            for task in updated {
                println!("Task ID {} updated", task.get_id().unwrap());
            }
            true
        },
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn test_renaming_a_status_migrates_tasks_and_their_subtasks() {
        let _repo = gittask::testing::TestRepo::new();
        let mut parent = gittask::Task::new("Parent".to_string(), "".to_string(), "CLOSED".to_string()).unwrap();
        parent.add_subtask(Some("1".to_string()), "Done child".to_string(), "CLOSED".to_string(), HashMap::new()).unwrap();
        parent.add_subtask(Some("2".to_string()), "Open child".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        let parent_id = gittask::create_task(parent).unwrap().get_id().unwrap();
        let mut open_parent = gittask::Task::new("Open parent".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        open_parent.add_subtask(Some("1".to_string()), "Done child".to_string(), "CLOSED".to_string(), HashMap::new()).unwrap();
        let open_parent_id = gittask::create_task(open_parent).unwrap().get_id().unwrap();

        assert!(task_config_status_set("CLOSED".to_string(), "name".to_string(), "DONE".to_string()));

        let parent = gittask::find_task(&parent_id).unwrap().unwrap();
        assert_eq!(parent.get_property("status").unwrap(), "DONE");
        let statuses: Vec<&str> = parent.get_subtasks().as_ref().unwrap().iter().map(|s| s.get_status()).collect();
        assert_eq!(statuses, vec!["DONE", "OPEN"], "only subtasks with the renamed status must change");

        let open_parent = gittask::find_task(&open_parent_id).unwrap().unwrap();
        assert_eq!(open_parent.get_property("status").unwrap(), "OPEN", "a parent with another status must keep it");
        let statuses: Vec<&str> = open_parent.get_subtasks().as_ref().unwrap().iter().map(|s| s.get_status()).collect();
        assert_eq!(statuses, vec!["DONE"], "subtasks must migrate even when the parent status does not change");

        assert_eq!(StatusManager::new().resolve_status("DONE"), Ok("DONE".to_string()));
    }
}

pub(crate) fn task_config_status_list() -> bool {
    let status_manager = StatusManager::new();
    println!("Name\tShortcut\tColor\tStyle\tIs DONE");
    status_manager.get_statuses().iter().for_each(|status| {
        println!("{}\t{}\t{}\t{}\t{}", status.get_name(), status.get_shortcut(), status.get_color(), status.get_style().unwrap_or_else(|| ""), status.is_done());
    });
    true
}

pub(crate) fn task_config_status_import() -> bool {
    if let Some(input) = read_from_pipe() {
        match status::parse_statuses(input) {
            Ok(statuses) => {
                let mut status_manager = StatusManager::new();
                match status_manager.set_statuses(statuses) {
                    Ok(_) => success_message("Import successful".to_string()),
                    Err(e) => error_message(format!("ERROR: {e}"))
                }
            },
            Err(e) => error_message(e)
        }
    } else {
        error_message("Can't read from pipe".to_string())
    }
}

pub(crate) fn task_config_status_export(pretty: bool) -> bool {
    let status_manager = StatusManager::new();
    let func = if pretty { serde_json::to_string_pretty } else { serde_json::to_string };

    if let Ok(result) = func(&status_manager.get_statuses()) {
        success_message(result)
    } else {
        error_message("ERROR serializing status list".to_string())
    }
}

pub(crate) fn task_config_status_reset() -> bool {
    let mut status_manager = StatusManager::new();
    match status_manager.set_defaults() {
        Ok(_) => success_message("Statuses have been reset".to_string()),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}
