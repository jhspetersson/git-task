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
            let task_exists = tasks.iter().any(|task| task.get_property("status").unwrap() == name.as_str());
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

            if param.as_str() == "name" && prev_value.is_some() {
                let prev_status = prev_value.unwrap();
                match gittask::list_tasks() {
                    Ok(tasks) => {
                        for mut task in tasks {
                            if task.get_property("status").unwrap() == prev_status.as_str() {
                                task.set_property("status", &value);
                                if let Err(e) = gittask::update_task(task) {
                                    eprintln!("ERROR: {e}");
                                }
                            }
                        }
                    },
                    Err(e) => eprintln!("ERROR: {e}")
                }
            }

            true
        },
        Err(e) => error_message(format!("ERROR: {e}"))
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
