use crate::connectors::{get_config_options_from_connectors, get_matching_remote_connectors};
use crate::util::{error_message, success_message};

pub(crate) mod status;
pub(crate) mod properties;

pub(crate) fn task_config_get(param: String) -> bool {
    match param.as_str() {
        "task.list.columns" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("id, created, status, name")))),
        "task.list.show.headers" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("false")))),
        "task.list.sort" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("id desc")))),
        "task.status.open" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("OPEN")))),
        "task.status.in_progress" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("IN_PROGRESS")))),
        "task.status.closed" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("CLOSED")))),
        "task.default.connector" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::new()))),
        "task.ref" => success_message(format!("{}", gittask::get_ref_path())),
        _ => {
            if get_config_options_from_connectors().contains(&param) {
                match gittask::get_config_value(&param) {
                    Ok(value) => success_message(format!("{}", value)),
                    Err(e) => error_message(format!("ERROR: {e}"))
                }
            } else {
                error_message(format!("Unknown parameter: {param}"))
            }
        }
    }
}

pub(crate) fn task_config_set(param: String, value: String, move_ref: bool, force: bool, rename_tasks: bool) -> bool {
    if rename_tasks && param != "task.jira.projects" {
        return error_message("--rename-tasks only applies to task.jira.projects".to_string());
    }

    if (move_ref || force) && param != "task.ref" {
        return error_message("--move and --force only apply to task.ref".to_string());
    }

    match param.as_str() {
        "task.jira.projects" => {
            match gittask::set_config_value(&param, &value) {
                Ok(_) => {
                    println!("{param} has been updated");
                    if rename_tasks {
                        rename_numeric_tasks_with_jira_project()
                    } else {
                        println!("NOTE: Jira tasks are now identified by their full issue key (e.g. ABC-42). Run with --rename-tasks to prefix existing numeric task IDs with the default project.");
                        true
                    }
                },
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.list.columns" => {
            match gittask::set_config_value(&param, &value) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.list.show.headers" => {
            match gittask::set_config_value(&param, &value) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.list.sort" => {
            match gittask::set_config_value(&param, &value) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.status.open" => {
            match gittask::set_config_value(&param, &value) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.status.in_progress" => {
            match gittask::set_config_value(&param, &value) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.status.closed" => {
            match gittask::set_config_value(&param, &value) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.default.connector" => {
            match gittask::set_config_value(&param, &value) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.ref" => {
            let value = match value {
                value if !value.contains('/') => "refs/heads/".to_string() + value.as_str(),
                value if value.chars().filter(|c| *c == '/').count() == 1 && !value.starts_with('/') && !value.ends_with('/') => "refs/".to_string() + value.as_str(),
                value => value,
            };

            match gittask::set_ref_path(&value, move_ref, force) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        _ => {
            if get_config_options_from_connectors().contains(&param) {
                match gittask::set_config_value(&param, &value) {
                    Ok(_) => success_message(format!("{param} has been updated")),
                    Err(e) => error_message(format!("ERROR: {e}"))
                }
            } else {
                error_message(format!("Unknown parameter: {param}"))
            }
        }
    }
}

/// Migration to multi-project Jira mode: numeric task IDs belonged to the single
/// (default) project, so they become `PROJECT-<id>` to match the IDs pulled from now on.
fn rename_numeric_tasks_with_jira_project() -> bool {
    let project = match crate::connectors::jira::get_default_project() {
        Some(project) => project,
        None => return error_message("No default Jira project found: set task.jira.project, a project URL in task.jira.url or task.jira.projects".to_string()),
    };

    match gittask::list_tasks() {
        Ok(tasks) => {
            let mut renamed = 0;
            let mut success = true;
            for task in tasks {
                let id = match task.get_id() {
                    Some(id) if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) => id,
                    _ => continue,
                };
                let new_id = format!("{project}-{id}");
                match gittask::update_task_id(&id, &new_id) {
                    Ok(_) => {
                        println!("Task ID {id} -> {new_id} updated");
                        renamed += 1;
                    },
                    Err(e) => {
                        eprintln!("ERROR: {e}");
                        success = false;
                    }
                }
            }
            println!("{renamed} task(s) renamed");
            success
        },
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

pub(crate) fn task_config_list() -> bool {
    let from_connectors = get_config_options_from_connectors().join("\n");
    success_message("task.list.columns\ntask.list.show.headers\ntask.list.sort\ntask.status.open\ntask.status.in_progress\ntask.status.closed\ntask.default.connector\ntask.ref\n".to_string() + &from_connectors)
}

pub(crate) fn task_config_connectors_list(name: Option<String>) -> bool {
    match gittask::list_remotes(&name) {
        Ok(remotes) => {
            let connectors = get_matching_remote_connectors(remotes, &None, &None, true);
            let mut result = vec![];
            for (connector, user, repo) in connectors {
                result.push(format!("{}: {}/{}", connector.type_name(), user, repo));
            }
            success_message(result.join("\n"))
        },
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}