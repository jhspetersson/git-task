use std::collections::HashMap;

use nu_ansi_term::Color::DarkGray;

use crate::operations::get_user_repo;
use crate::status::StatusManager;
use crate::util::{colorize_string, error_message, get_text_from_editor, success_message};

pub(crate) fn task_subtask_add(
    task_id: String,
    name: Option<String>,
    status: Option<String>,
    push: bool,
    remote: &Option<String>,
    connector_type: &Option<String>,
) -> bool {
    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            let name = name.or_else(|| get_text_from_editor(None));
            let name = match name {
                Some(n) if !n.trim().is_empty() => n.trim().to_string(),
                _ => return error_message("Subtask name is required".to_string()),
            };

            let status_manager = StatusManager::new();
            let status = match status {
                Some(s) => status_manager.get_full_status_name(&s),
                None => status_manager.get_starting_status(),
            };

            let subtask = match task.add_subtask(None, name, status, HashMap::new()) {
                Ok(s) => s,
                Err(e) => return error_message(format!("ERROR: {e}")),
            };

            match gittask::update_task(task) {
                Ok(_) => {
                    println!("Subtask ID {} added to task ID {task_id}", subtask.get_id().unwrap());
                    let mut success = true;
                    if push {
                        success = false;
                        match get_user_repo(remote, connector_type) {
                            Ok((connector, user, repo)) => {
                                match connector.create_remote_subtask(&user, &repo, &task_id, &subtask) {
                                    Ok(remote_id) => {
                                        println!("Created REMOTE subtask ID {remote_id}");
                                        match gittask::update_subtask_id(&task_id, &subtask.get_id().unwrap(), &remote_id) {
                                            Ok(_) => {
                                                println!("Subtask ID {} -> {} updated", subtask.get_id().unwrap(), remote_id);
                                                success = true;
                                            },
                                            Err(e) => eprintln!("ERROR: {e}"),
                                        }
                                    },
                                    Err(e) => eprintln!("ERROR creating REMOTE subtask: {e}"),
                                }
                            },
                            Err(e) => eprintln!("ERROR: {e}"),
                        }
                    }
                    success
                },
                Err(e) => error_message(format!("ERROR: {e}")),
            }
        },
        Ok(None) => error_message(format!("Task ID {task_id} not found")),
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

pub(crate) fn task_subtask_set(
    task_id: String,
    subtask_id: String,
    prop_name: String,
    value: String,
    push: bool,
    remote: &Option<String>,
    connector_type: &Option<String>,
) -> bool {
    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            let status_manager = StatusManager::new();
            let resolved_value = if prop_name == "status" {
                status_manager.get_full_status_name(&value)
            } else {
                value.clone()
            };

            let update_result = task.update_subtask(&subtask_id, |s| {
                match prop_name.as_str() {
                    "name" => s.set_name(resolved_value.clone()),
                    "status" => s.set_status(resolved_value.clone()),
                    other => s.set_property(other, &resolved_value),
                }
            });

            match update_result {
                Ok(_) => {
                    match gittask::update_task(task) {
                        Ok(_) => {
                            println!("Subtask ID {subtask_id} updated");
                            let mut success = true;
                            if push {
                                success = false;
                                match get_user_repo(remote, connector_type) {
                                    Ok((connector, user, repo)) => {
                                        if let Ok(Some(task)) = gittask::find_task(&task_id) {
                                            if let Some(subtask) = task.get_subtask(&subtask_id) {
                                                match connector.update_remote_subtask(&user, &repo, &task_id, subtask) {
                                                    Ok(_) => {
                                                        println!("Sync: REMOTE subtask ID {subtask_id} has been updated");
                                                        success = true;
                                                    },
                                                    Err(e) => eprintln!("ERROR: {e}"),
                                                }
                                            }
                                        }
                                    },
                                    Err(e) => eprintln!("ERROR: {e}"),
                                }
                            }
                            success
                        },
                        Err(e) => error_message(format!("ERROR: {e}")),
                    }
                },
                Err(e) => error_message(format!("ERROR: {e}")),
            }
        },
        Ok(None) => error_message(format!("Task ID {task_id} not found")),
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

pub(crate) fn task_subtask_unset(
    task_id: String,
    subtask_id: String,
    prop_name: String,
) -> bool {
    if prop_name == "name" || prop_name == "status" {
        return error_message(format!("Property '{prop_name}' cannot be unset"));
    }

    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            let mut removed = false;
            let update_result = task.update_subtask(&subtask_id, |s| {
                removed = s.delete_property(&prop_name);
            });

            match update_result {
                Ok(_) => {
                    if !removed {
                        return error_message(format!("Subtask property {prop_name} not found"));
                    }
                    match gittask::update_task(task) {
                        Ok(_) => success_message(format!("Subtask ID {subtask_id} updated")),
                        Err(e) => error_message(format!("ERROR: {e}")),
                    }
                },
                Err(e) => error_message(format!("ERROR: {e}")),
            }
        },
        Ok(None) => error_message(format!("Task ID {task_id} not found")),
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

pub(crate) fn task_subtask_edit(
    task_id: String,
    subtask_id: String,
    prop_name: Option<String>,
) -> bool {
    let prop_name = prop_name.unwrap_or_else(|| "name".to_string());
    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            let current_value = match task.get_subtask(&subtask_id) {
                Some(s) => match prop_name.as_str() {
                    "name" => s.get_name().to_string(),
                    "status" => s.get_status().to_string(),
                    other => s.get_property(other).cloned().unwrap_or_default(),
                },
                None => return error_message(format!("Subtask ID {subtask_id} not found")),
            };

            let new_value = match get_text_from_editor(Some(&current_value)) {
                Some(text) => text,
                None => return error_message("Editing failed".to_string()),
            };

            let update_result = task.update_subtask(&subtask_id, |s| {
                match prop_name.as_str() {
                    "name" => s.set_name(new_value.clone()),
                    "status" => s.set_status(new_value.clone()),
                    other => s.set_property(other, &new_value),
                }
            });

            match update_result {
                Ok(_) => match gittask::update_task(task) {
                    Ok(_) => success_message(format!("Subtask ID {subtask_id} updated")),
                    Err(e) => error_message(format!("ERROR: {e}")),
                },
                Err(e) => error_message(format!("ERROR: {e}")),
            }
        },
        Ok(None) => error_message(format!("Task ID {task_id} not found")),
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

pub(crate) fn task_subtask_delete(
    task_id: String,
    subtask_id: String,
    push: bool,
    remote: &Option<String>,
    connector_type: &Option<String>,
) -> bool {
    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            match task.delete_subtask(&subtask_id) {
                Ok(_) => match gittask::update_task(task) {
                    Ok(_) => {
                        println!("Subtask ID {subtask_id} deleted from task ID {task_id}");
                        let mut success = true;
                        if push {
                            success = false;
                            match get_user_repo(remote, connector_type) {
                                Ok((connector, user, repo)) => {
                                    match connector.delete_remote_subtask(&user, &repo, &task_id, &subtask_id) {
                                        Ok(_) => {
                                            println!("Sync: REMOTE subtask ID {subtask_id} has been removed");
                                            success = true;
                                        },
                                        Err(e) => eprintln!("ERROR: {e}"),
                                    }
                                },
                                Err(e) => eprintln!("ERROR: {e}"),
                            }
                        }
                        success
                    },
                    Err(e) => error_message(format!("ERROR: {e}")),
                },
                Err(e) => error_message(format!("ERROR: {e}")),
            }
        },
        Ok(None) => error_message(format!("Task ID {task_id} not found")),
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

pub(crate) fn task_subtask_list(task_id: String, no_color: bool) -> bool {
    match gittask::find_task(&task_id) {
        Ok(Some(task)) => {
            let no_color = crate::operations::check_no_color(no_color);
            let status_manager = StatusManager::new();
            match task.get_subtasks() {
                Some(subtasks) if !subtasks.is_empty() => {
                    for subtask in subtasks {
                        let id_title = colorize_string("ID", DarkGray, no_color);
                        let id = subtask.get_id().unwrap_or_else(|| "---".to_string());
                        let status = status_manager.format_status(subtask.get_status(), no_color);
                        println!("{}: {} | {} | {}", id_title, id, status, subtask.get_name());
                    }
                    true
                },
                _ => success_message(format!("No subtasks for task ID {task_id}")),
            }
        },
        Ok(None) => error_message(format!("Task ID {task_id} not found")),
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}
