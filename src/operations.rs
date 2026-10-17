pub(crate) mod comment;
pub(crate) mod config;
pub(crate) mod label;
pub(crate) mod subtask;

use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{Local, TimeZone};
use nu_ansi_term::Color::DarkGray;
use regex::Regex;

use evalexpr::{ContextWithMutableVariables, HashMapContext};
use gittask::{Comment, Label, Subtask, Task};

use crate::connectors::{get_matching_remote_connectors, status_names_match, RemoteConnector, RemoteTarget, RemoteTaskState};
use crate::property::{PropertyManager, PropertyValueType};
use crate::status::StatusManager;
use crate::util::{capitalize, colorize_string, error_message, get_text_from_editor, parse_bool, parse_date, parse_ids, read_from_pipe, str_to_color, success_message};

/// Default value of the `task.list.columns` setting, shared by `task list` and `task config get`.
pub(crate) const DEFAULT_LIST_COLUMNS: &str = "id, created, status, name, labels";

pub(crate) fn task_create(
    name: String,
    description: Option<String>,
    no_desc: bool,
    push: bool,
    remote: &Option<String>,
    connector_type: &Option<String>,
    project: &Option<String>,
) -> bool {
    let description = match description {
        Some(description) => description,
        None => match no_desc {
            true => String::from(""),
            false => get_text_from_editor(None).unwrap_or_else(|| String::from(""))
        }
    };

    let status_manager = StatusManager::new();
    let task = match Task::new(name, description, status_manager.get_starting_status()) {
        Ok(task) => task,
        Err(e) => return error_message(format!("ERROR: {e}")),
    };

    match gittask::create_task(task) {
        Ok(task) => {
            println!("Task ID {} created", task.get_id().unwrap());
            let mut success = true;
            if push {
                success = false;
                match get_user_repo(remote, connector_type, project) {
                    Ok((connector, user, repo)) => {
                        match connector.create_remote_task(&user, &repo, &task) {
                            Ok(id) => {
                                println!("Sync: Created REMOTE task ID {id}");
                                match gittask::update_task_id(&task.get_id().unwrap(), &id) {
                                    Ok(_) => {
                                        println!("Task ID {} -> {} updated", task.get_id().unwrap(), id);
                                        success = true;
                                    },
                                    Err(e) => eprintln!("ERROR: {e}")
                                }
                            },
                            Err(e) => eprintln!("ERROR: {e}")
                        }
                    },
                    Err(e) => eprintln!("ERROR: {e}")
                }
            }
            success
        },
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_status(
    ids: String,
    status: String,
    create: bool,
    push: bool,
    remote: &Option<String>,
    connector_type: &Option<String>,
    project: &Option<String>,
    no_color: bool,
) -> bool {
    let mut status_manager = StatusManager::new();
    let status = match status_manager.resolve_status(&status) {
        Ok(status) => status,
        Err(e) if create => {
            let name = status.trim().to_string();
            match status_manager.add_status_with_defaults(&name) {
                Ok(shortcut) => {
                    println!("Status {name} created with shortcut '{shortcut}' and color Default, adjust it with `git task config status set`");
                    name
                },
                Err(create_error) => return error_message(format!("ERROR: {e}. Could not create it: {create_error}")),
            }
        },
        Err(e) => return error_message(format!("ERROR: {e}. Use --create to add it")),
    };

    task_set(ids, "status".to_string(), status, push, remote, connector_type, project, no_color)
}

pub(crate) fn task_get(id: String, prop_name: String) -> bool {
    match gittask::find_task(&id) {
        Ok(Some(task)) => {
            match task.get_property(&prop_name) {
                Some(value) => success_message(format!("{value}")),
                None => error_message(format!("Task property {prop_name} not found"))
            }
        },
        Ok(None) => error_message(format!("Task ID {id} not found")),
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

pub(crate) fn task_set(
    ids: String,
    prop_name: String,
    value: String,
    push: bool,
    remote: &Option<String>,
    connector_type: &Option<String>,
    project: &Option<String>,
    no_color: bool
) -> bool {
    if (prop_name == "name" || prop_name == "status") && value.trim().is_empty() {
        return error_message(format!("Property '{prop_name}' cannot be empty"));
    }

    let value = match prop_name.as_str() {
        "status" => match StatusManager::new().resolve_status(&value) {
            Ok(status) => status,
            Err(e) => return error_message(format!("ERROR: {e}")),
        },
        _ => value,
    };

    let ids = parse_ids(ids);
    let mut success = true;
    match prop_name.as_str() {
        "id" => {
            for id in &ids {
                match gittask::update_task_id(&id, &value) {
                    Ok(_) => {
                        println!("Task ID {id} -> {value} updated");

                        if push {
                            success &= task_push(value.clone(), remote, connector_type, project, false, false, false, no_color);
                        }
                    },
                    Err(e) => {
                        error_message(format!("ERROR: {e}"));
                        success = false;
                    }
                }
            }
        },
        _ => {
            for id in &ids {
                match gittask::find_task(&id) {
                    Ok(Some(mut task)) => {
                        task.set_property(&prop_name, &value);

                        match gittask::update_task(task) {
                            Ok(_) => {
                                println!("Task ID {id} updated");

                                if push {
                                    success &= task_push(id.to_string(), remote, connector_type, project, false, false, false, no_color);
                                }
                            },
                            Err(e) => {
                                error_message(format!("ERROR: {e}"));
                                success = false;
                            },
                        }
                    },
                    Ok(None) => {
                        error_message(format!("Task ID {id} not found"));
                        success = false;
                    },
                    Err(e) =>{
                        error_message(format!("ERROR: {e}"));
                        success = false;
                    }
                }
            }
        }
    }

    success
}

pub(crate) fn task_replace(
    ids: String,
    prop_name: String,
    search: String,
    replace: String,
    regex: bool,
    push: bool,
    remote: &Option<String>,
    connector_type: &Option<String>,
    project: &Option<String>,
    no_color: bool
) -> bool {
    let ids = parse_ids(ids);
    let regex = match regex {
        true => match Regex::new(search.as_str()) {
            Ok(r) => Some(Box::new(r)),
            Err(e) => return error_message(format!("Invalid regex: {e}")),
        },
        false => None
    };
    let mut success = true;
    for id in ids {
        match gittask::find_task(&id) {
            Ok(Some(mut task)) => {
                if let Some(value) = task.get_property(&prop_name) {
                    let new_value = match regex {
                        Some(ref regex) => regex.replace_all(value.as_str(), replace.as_str()).to_string(),
                        None => value.replace(&search, &replace)
                    };
                    if (prop_name == "name" || prop_name == "status") && new_value.trim().is_empty() {
                        eprintln!("Task ID {id}: property '{prop_name}' cannot be empty");
                        success = false;
                        continue;
                    }
                    if prop_name == "status" {
                        if let Err(e) = StatusManager::new().resolve_status(&new_value) {
                            eprintln!("Task ID {id}: {e}");
                            success = false;
                            continue;
                        }
                    }
                    task.set_property(&prop_name, &new_value);
                    match gittask::update_task(task) {
                        Ok(_) => {
                            println!("Task ID {id} updated");
                            if push {
                                success &= task_push(id.to_string(), remote, connector_type, project, false, false, false, no_color);
                            }
                        },
                        Err(e) => { eprintln!("ERROR: {e}"); success = false; }
                    }
                } else {
                    eprintln!("Task ID {id}: property not found");
                    success = false;
                }
            },
            Ok(None) => { eprintln!("Task ID {id} not found"); success = false; },
            Err(e) => { eprintln!("ERROR: {e}"); success = false; }
        }
    }

    success
}

pub(crate) fn task_unset(ids: String, prop_name: String) -> bool {
    if prop_name == "name" || prop_name == "status" {
        return error_message(format!("Property '{prop_name}' cannot be unset"));
    }

    let ids = parse_ids(ids);
    let mut success = true;
    for id in ids {
        match gittask::find_task(&id) {
            Ok(Some(mut task)) => {
                if task.delete_property(&prop_name) {
                    match gittask::update_task(task) {
                        Ok(_) => println!("Task ID {id} updated"),
                        Err(e) => { eprintln!("ERROR: {e}"); success = false; }
                    }
                } else {
                    eprintln!("Task ID {id}: property not found");
                    success = false;
                }
            },
            Ok(None) => { eprintln!("Task ID {id} not found"); success = false; },
            Err(e) => { eprintln!("ERROR: {e}"); success = false; }
        }
    };

    success
}

pub(crate) fn task_edit(id: String, prop_name: String) -> bool {
    match gittask::find_task(&id) {
        Ok(Some(mut task)) => {
            match prop_name.as_str() {
                "id" => {
                    match get_text_from_editor(Some(&task.get_id().unwrap())) {
                        Some(text) => {
                            let text = text.trim().to_string();
                            if text.is_empty() {
                                return error_message("Task ID cannot be empty".to_string());
                            }
                            task.set_id(text.clone());
                            match gittask::update_task(task) {
                                Ok(_) => {
                                    println!("Task ID {id} -> {text} updated");
                                    if text != id {
                                        if let Err(e) = gittask::delete_tasks(&[&id]) {
                                            eprintln!("ERROR: {e}");
                                        }
                                    }
                                    true
                                },
                                Err(e) => error_message(format!("ERROR: {e}")),
                            }
                        },
                        None => error_message("Editing failed".to_string()),
                    }
                },
                _ => {
                    match task.get_property(&prop_name) {
                        Some(value) => {
                            match get_text_from_editor(Some(value)) {
                                Some(text) => {
                                    let text = match prop_name.as_str() {
                                        "name" | "status" => {
                                            let trimmed = text.trim();
                                            if trimmed.is_empty() {
                                                return error_message(format!("Property '{prop_name}' cannot be empty"));
                                            }
                                            trimmed.to_string()
                                        },
                                        _ => text,
                                    };
                                    let text = match prop_name.as_str() {
                                        "status" => match StatusManager::new().resolve_status(&text) {
                                            Ok(status) => status,
                                            Err(e) => return error_message(format!("ERROR: {e}")),
                                        },
                                        _ => text,
                                    };
                                    task.set_property(&prop_name, &text);
                                    match gittask::update_task(task) {
                                        Ok(_) => success_message(format!("Task ID {id} updated")),
                                        Err(e) => error_message(format!("ERROR: {e}")),
                                    }
                                },
                                None => error_message("Editing failed".to_string()),
                            }
                        },
                        None => error_message(format!("Task property {prop_name} not found"))
                    }
                }
            }
        },
        Ok(None) => error_message(format!("Task ID {id} not found")),
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

pub(crate) fn task_import(ids: Option<String>, format: Option<String>) -> bool {
    if let Some(format) = format {
        if format.to_lowercase() != "json" {
            return error_message("Only JSON format is supported".to_string());
        }
    }

    if let Some(input) = read_from_pipe() {
        import_from_input(ids, &input)
    } else {
        error_message("Can't read from pipe".to_string())
    }
}

fn import_from_input(ids: Option<String>, input: &String) -> bool {
    if let Ok(tasks) = serde_json::from_str::<Vec<Task>>(input) {
        let ids = ids.map(parse_ids);

        for task in tasks {
            let id = match task.get_id() {
                Some(id) => id,
                None => { eprintln!("ERROR: task has no id, skipping"); continue; }
            };

            if let Some(ids) = &ids {
                if !ids.contains(&id) {
                    continue;
                }
            }

            match gittask::create_task(task) {
                Ok(_) => println!("Task ID {id} imported"),
                Err(e) => eprintln!("ERROR: {e}"),
            }
        }
        true
    } else {
        error_message("Can't deserialize input".to_string())
    }
}

pub(crate) fn task_pull(
    ids: Option<String>,
    limit: Option<usize>,
    status: Option<String>,
    remote: &Option<String>,
    connector_type: &Option<String>,
    project: &Option<String>,
    no_comments: bool,
    no_labels: bool,
    no_subtasks: bool,
) -> bool {
    let status_manager = StatusManager::new();
    let mut task_statuses = vec![
        status_manager.get_starting_status(),
        status_manager.get_final_status(),
    ];
    if let Some(status_in_progress) = status_manager.get_in_progress_status() {
        task_statuses.insert(1, status_in_progress);
    }

    if let Some(ids) = ids {
        let mut success = true;
        for id in parse_ids(ids) {
            let (connector, user, repo) = match get_user_repo(remote, connector_type, &resolve_project(project, &id)) {
                Ok(target) => target,
                Err(e) => {
                    eprintln!("Task ID {id}: {e}");
                    success = false;
                    continue;
                }
            };
            success &= pull_task(&connector, &user, &repo, &id, no_comments, no_labels, no_subtasks, &task_statuses);
        }
        return success;
    }

    // Without IDs and without an explicit project, pull every project the connector manages.
    let targets = match get_user_repos(remote, connector_type, project) {
        Ok(targets) => targets,
        Err(e) => return error_message(format!("ERROR: {e}")),
    };

    let state = match status {
        Some(s) => {
            let status = status_manager.get_full_status_name(&s);
            let is_done = status_manager.get_property(&status, "is_done").and_then(|v| parse_bool(&v).ok()).unwrap_or(false);
            if is_done { RemoteTaskState::Closed(status.clone(), status) } else { RemoteTaskState::Open(status.clone(), status) }
        },
        None => RemoteTaskState::All
    };

    let mut success = true;
    for (connector, user, repo) in targets {
        println!("Pulling tasks from {user}/{repo}...");
        let fetch_subtasks = !no_subtasks && connector.supports_subtasks();

        match connector.list_remote_tasks(&user, &repo, !no_comments, !no_labels, limit, state.clone(), &task_statuses) {
            Ok(tasks) => {
                if tasks.is_empty() {
                    success_message("No tasks found".to_string());
                } else {
                    for mut task in tasks {
                        let task_id = task.get_id().unwrap();
                        if fetch_subtasks {
                            match connector.list_remote_subtasks(&user, &repo, &task_id, &task_statuses) {
                                Ok(subtasks) => task.set_subtasks(subtasks),
                                Err(e) => {
                                    eprintln!("Task ID {task_id}: {e}");
                                    success = false;
                                }
                            }
                        }
                        match import_remote_task(task, no_comments, fetch_subtasks) {
                            Ok(Some(id)) => println!("Task ID {id} updated"),
                            Ok(None) => println!("Task ID {task_id} skipped, nothing to update"),
                            Err(e) => {
                                eprintln!("ERROR: {e}");
                                success = false;
                            }
                        }
                    }
                }
            },
            Err(e) => {
                error_message(format!("ERROR: {e}"));
                success = false;
            }
        }
    }

    success
}

fn pull_task(
    connector: &Box<&'static dyn RemoteConnector>,
    user: &String,
    repo: &String,
    id: &String,
    no_comments: bool,
    no_labels: bool,
    no_subtasks: bool,
    task_statuses: &Vec<String>,
) -> bool {
    let fetch_subtasks = !no_subtasks && connector.supports_subtasks();
    let mut success = true;
    match connector.get_remote_task(user, repo, id, !no_comments, !no_labels, task_statuses) {
        Ok(Some(mut task)) => {
            if fetch_subtasks {
                match connector.list_remote_subtasks(user, repo, id, task_statuses) {
                    Ok(subtasks) => task.set_subtasks(subtasks),
                    Err(e) => {
                        eprintln!("Task ID {id}: {e}");
                        success = false;
                    }
                }
            }
            match import_remote_task(task, no_comments, fetch_subtasks) {
                Ok(Some(id)) => println!("Task ID {id} updated"),
                Ok(None) => println!("Task ID {id} skipped, nothing to update"),
                Err(e) => {
                    eprintln!("ERROR: {e}");
                    success = false;
                }
            }
        },
        Ok(None) => {
            eprintln!("Task ID {id}: not found on the remote");
            success = false;
        },
        Err(e) => {
            eprintln!("Task ID {id}: {e}");
            success = false;
        }
    }
    success
}

fn import_remote_task(remote_task: Task, no_comments: bool, with_subtasks: bool) -> Result<Option<String>, String> {
    match gittask::find_task(&remote_task.get_id().unwrap()) {
        Ok(Some(mut local_task)) => {
            let subtasks_equal = !with_subtasks
                || subtasks_are_equal(local_task.get_subtasks(), remote_task.get_subtasks());

            if local_task.get_property("name") == remote_task.get_property("name")
                && local_task.get_property("description") == remote_task.get_property("description")
                && local_task.get_property("status") == remote_task.get_property("status")
                && (no_comments || comments_are_equal(local_task.get_comments(), remote_task.get_comments()))
                && subtasks_equal {
                Ok(None)
            } else {
                local_task.set_property("name", remote_task.get_property("name").map(String::as_str).unwrap_or(""));
                local_task.set_property("description", remote_task.get_property("description").map(String::as_str).unwrap_or(""));
                local_task.set_property("status", remote_task.get_property("status").map(String::as_str).unwrap_or(""));
                if !no_comments {
                    if let Some(comments) = remote_task.get_comments() {
                        local_task.set_comments(comments.to_vec());
                    }
                }
                if with_subtasks {
                    if let Some(subtasks) = remote_task.get_subtasks() {
                        local_task.set_subtasks(subtasks.to_vec());
                    }
                }

                match gittask::update_task(local_task) {
                    Ok(id) => Ok(Some(id)),
                    Err(e) => Err(e),
                }
            }
        },
        Ok(None) => match gittask::create_task(remote_task) {
            Ok(local_task) => Ok(Some(local_task.get_id().unwrap())),
            Err(e) => Err(e),
        },
        Err(e) => Err(e)
    }
}

fn subtasks_are_equal(local: &Option<Vec<Subtask>>, remote: &Option<Vec<Subtask>>) -> bool {
    match (local, remote) {
        (None, None) => true,
        (Some(l), Some(r)) => l == r,
        (None, Some(r)) => r.is_empty(),
        (Some(l), None) => l.is_empty(),
    }
}

fn comments_are_equal(local_comments: &Option<Vec<Comment>>, remote_comments: &Option<Vec<Comment>>) -> bool {
    match (local_comments, remote_comments) {
        (None, None) => true,
        (Some(l), Some(r)) => l == r,
        _ => false,
    }
}

/// The project to use for a task: an explicit `--project`, or the project key when the
/// task ID is a full Jira-style issue key (`ABC-42`), or nothing (the connector's default).
pub(crate) fn resolve_project(project: &Option<String>, id: &str) -> Option<String> {
    project.clone().or_else(|| project_from_id(id))
}

fn project_from_id(id: &str) -> Option<String> {
    let (project, number) = id.rsplit_once('-')?;
    let valid_project = project.chars().next().map_or(false, |c| c.is_ascii_alphabetic())
        && project.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let valid_number = !number.is_empty() && number.chars().all(|c| c.is_ascii_digit());
    if valid_project && valid_number {
        Some(project.to_string())
    } else {
        None
    }
}

fn no_passing_remotes(project: &Option<String>) -> String {
    match project {
        Some(project) => format!("No passing remotes for project {project}"),
        None => "No passing remotes".to_string(),
    }
}

const AMBIGUOUS_REMOTES: &str = "More than one passing remote found. Please specify with --remote, --connector and/or --project option.";

/// Exactly one remote target: the connector's default project unless `project` is given.
pub(crate) fn get_user_repo(remote: &Option<String>,
                            connector_type: &Option<String>,
                            project: &Option<String>,
) -> Result<RemoteTarget, String> {
    let remotes = gittask::list_remotes(remote)?;
    let mut targets = get_matching_remote_connectors(remotes, &get_connector(connector_type), project, false);
    if targets.is_empty() {
        return Err(no_passing_remotes(project));
    }

    if targets.len() > 1 {
        return Err(AMBIGUOUS_REMOTES.to_owned());
    }

    Ok(targets.remove(0))
}

/// Every remote target of a single connector: all projects a multi-project connector
/// (e.g. Jira) manages, or the single default target otherwise.
fn get_user_repos(remote: &Option<String>,
                  connector_type: &Option<String>,
                  project: &Option<String>,
) -> Result<Vec<RemoteTarget>, String> {
    let remotes = gittask::list_remotes(remote)?;
    let targets = get_matching_remote_connectors(remotes, &get_connector(connector_type), project, true);
    if targets.is_empty() {
        return Err(no_passing_remotes(project));
    }

    if targets.len() > 1 {
        let first = &targets[0].0;
        let same_connector = targets.iter().all(|(c, _, _)| c.type_name() == first.type_name());
        if !same_connector || !first.supports_multiple_projects() {
            return Err(AMBIGUOUS_REMOTES.to_owned());
        }
    }

    Ok(targets)
}

pub(crate) fn task_export(ids: Option<String>, status: Option<Vec<String>>, limit: Option<usize>, format: Option<String>, pretty: bool) -> bool {
    if let Some(format) = format {
        if format.to_lowercase() != "json" {
            return error_message("Only JSON format is supported".to_string());
        }
    }

    match gittask::list_tasks() {
        Ok(mut tasks) => {
            let mut result = vec![];
            tasks.sort_by(|a, b| gittask::compare_ids(&a.get_id().unwrap_or_default(), &b.get_id().unwrap_or_default()));

            let status_manager = StatusManager::new();
            let statuses = match status {
                Some(statuses) => Some(statuses.iter().map(|s| status_manager.get_full_status_name(s)).collect::<Vec<_>>()),
                None => None
            };

            let ids = ids.map(parse_ids);

            let mut count = 0;
            for task in tasks {
                if let Some(ids) = &ids {
                    if !ids.contains(&task.get_id().unwrap()) {
                        continue;
                    }
                }

                if let Some(ref statuses) = statuses {
                    match task.get_property("status") {
                        Some(task_status) if statuses.contains(task_status) => {},
                        _ => continue,
                    }
                }

                if let Some(limit) = limit {
                    if count >= limit {
                        break;
                    }
                }

                result.push(task);
                count += 1;
            }

            let func = if pretty { serde_json::to_string_pretty } else { serde_json::to_string };

            if let Ok(result) = func(&result) {
                success_message(result)
            } else {
                error_message("ERROR serializing task list".to_string())
            }
        },
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_push(
    ids: String,
    remote: &Option<String>,
    connector_type: &Option<String>,
    project: &Option<String>,
    no_comments: bool,
    no_labels: bool,
    no_subtasks: bool,
    no_color: bool
) -> bool {
    let ids = parse_ids(ids);

    let status_manager = StatusManager::new();
    let mut task_statuses = vec![
        status_manager.get_starting_status(),
        status_manager.get_final_status(),
    ];
    if let Some(status_in_progress) = status_manager.get_in_progress_status() {
        task_statuses.insert(1, status_in_progress);
    }

    let no_color = check_no_color(no_color);
    let mut success = true;
    for id in ids {
        println!("Sync: task ID {id}");
        let (connector, user, repo) = match get_user_repo(remote, connector_type, &resolve_project(project, &id)) {
            Ok(target) => target,
            Err(e) => {
                error_message(format!("ERROR: {e}"));
                success = false;
                continue;
            }
        };
        success &= push_task(&connector, &user, &repo, &id, no_comments, no_labels, no_subtasks, &status_manager, &task_statuses, no_color);
    }
    success
}

fn push_task(
    connector: &Box<&'static dyn RemoteConnector>,
    user: &String,
    repo: &String,
    id: &String,
    no_comments: bool,
    no_labels: bool,
    no_subtasks: bool,
    status_manager: &StatusManager,
    task_statuses: &Vec<String>,
    no_color: bool,
) -> bool {
    let empty_string = String::new();
    let mut success = true;
    let sync_subtasks = !no_subtasks && connector.supports_subtasks();
    let local_task = match gittask::find_task(id) {
        Ok(Some(local_task)) => local_task,
        Ok(None) => {
            eprintln!("Sync: LOCAL task ID {id} NOT found");
            return false;
        }
        Err(e) => {
            eprintln!("ERROR: {e}");
            return false;
        }
    };
    println!("Sync: LOCAL task ID {id} found");
    let remote_task = match connector.get_remote_task(user, repo, id, !no_comments, !no_labels, task_statuses) {
        Ok(remote_task) => remote_task,
        Err(e) => {
            eprintln!("ERROR: could not fetch REMOTE task ID {id}: {e}");
            return false;
        }
    };
    if let Some(remote_task) = remote_task {
        println!("Sync: REMOTE task ID {id} found");

        let local_status = local_task.get_property("status").unwrap_or(&empty_string);
        let local_name = local_task.get_property("name").unwrap_or(&empty_string);
        let local_text = local_task.get_property("description").unwrap_or(&empty_string);

        let remote_status = remote_task.get_property("status").unwrap_or(&empty_string);
        let remote_name = remote_task.get_property("name").unwrap_or(&empty_string);
        let remote_text = remote_task.get_property("description").unwrap_or(&empty_string);

        let mut anything_synced = false;
        let status_changed = !status_names_match(local_status, remote_status);
        if local_name != remote_name || local_text != remote_text || status_changed {
            anything_synced = true;
            if status_changed {
                println!("{}: {} -> {}", id, status_manager.format_status(remote_status, no_color), status_manager.format_status(local_status, no_color));
            }
            let state = if status_manager.is_done(local_status) {
                RemoteTaskState::Closed(local_status.to_string(), remote_status.to_string())
            } else {
                RemoteTaskState::Open(local_status.to_string(), remote_status.to_string())
            };

            match connector.update_remote_task(
                &user,
                &repo,
                &local_task,
                if !no_labels { local_task.get_labels().into() } else { None },
                state
            ) {
                Ok(_) => {
                    println!("Sync: REMOTE task ID {id} has been updated");
                },
                Err(e) => {
                    eprintln!("ERROR: {e}");
                    success = false;
                }
            }
        }
        if !no_comments {
            let remote_comment_ids: Vec<String> = remote_task.get_comments().as_ref().unwrap_or(&vec![]).iter().filter_map(|comment| comment.get_id()).collect();
            anything_synced |= sync_local_comments(&connector, &user, &repo, &id, &local_task, &remote_comment_ids, &mut success);
        }
        if sync_subtasks {
            anything_synced |= sync_local_subtasks(&connector, &user, &repo, &id, &local_task, &status_manager, &task_statuses, &mut success);
        }
        if !anything_synced {
            println!("Nothing to sync");
        }
    } else {
        eprintln!("Sync: REMOTE task ID {id} NOT found");

        let local_task = match no_labels {
            true => {
                let mut local_task = local_task;
                local_task.set_labels(vec![]);
                local_task
            },
            false => local_task
        };

        match connector.create_remote_task(&user, &repo, &local_task) {
            Ok(id) => {
                println!("Sync: Created REMOTE task ID {id}");
                if local_task.get_id().unwrap() != id {
                    match gittask::update_task_id(&local_task.get_id().unwrap(), &id) {
                        Ok(_) => println!("Task ID {} -> {} updated", local_task.get_id().unwrap(), id),
                        Err(e) => {
                            eprintln!("ERROR: {e}");
                            success = false;
                        }
                    }
                }

                if !no_comments {
                    sync_local_comments(&connector, &user, &repo, &id, &local_task, &[], &mut success);
                }

                if sync_subtasks {
                    sync_local_subtasks(&connector, &user, &repo, &id, &local_task, &status_manager, &task_statuses, &mut success);
                }
            },
            Err(e) => {
                eprintln!("ERROR: {e}");
                success = false;
            }
        }
    }
    success
}

fn sync_local_subtasks(
    connector: &Box<&'static dyn RemoteConnector>,
    user: &String,
    repo: &String,
    task_id: &String,
    local_task: &Task,
    status_manager: &StatusManager,
    task_statuses: &Vec<String>,
    success: &mut bool,
) -> bool {
    let local_subtasks = match local_task.get_subtasks() {
        Some(subtasks) if !subtasks.is_empty() => subtasks.clone(),
        _ => return false,
    };

    let remote_subtasks = match connector.list_remote_subtasks(user, repo, task_id, task_statuses) {
        Ok(subtasks) => subtasks,
        Err(e) => {
            eprintln!("ERROR listing REMOTE subtasks of task ID {task_id}: {e}");
            *success = false;
            return false;
        }
    };

    let remote_ids: Vec<String> = remote_subtasks.iter().filter_map(|s| s.get_id()).collect();
    let mut synced = false;
    let mut id_updates = vec![];
    for subtask in &local_subtasks {
        let local_id = match subtask.get_id() {
            Some(id) => id,
            None => continue,
        };
        let is_done = status_manager.is_done(subtask.get_status());
        if remote_ids.contains(&local_id) {
            if let Err(e) = connector.update_remote_subtask(user, repo, task_id, subtask, is_done) {
                eprintln!("ERROR syncing REMOTE subtask {local_id}: {e}");
                *success = false;
            } else {
                println!("Sync: REMOTE subtask ID {local_id} updated");
                synced = true;
            }
        } else {
            match connector.create_remote_subtask(user, repo, task_id, subtask, is_done) {
                Ok(remote_id) => {
                    println!("Created REMOTE subtask ID {remote_id}");
                    if local_id != remote_id {
                        id_updates.push((local_id, remote_id));
                    }
                    synced = true;
                },
                Err(e) => {
                    eprintln!("ERROR creating REMOTE subtask: {e}");
                    *success = false;
                }
            }
        }
    }
    match gittask::update_subtask_ids(task_id, &id_updates) {
        Ok(_) => for (local_id, remote_id) in &id_updates {
            println!("Subtask ID {local_id} -> {remote_id} updated");
        },
        Err(e) => {
            eprintln!("ERROR: {e}");
            *success = false;
        }
    }
    synced
}

fn sync_local_comments(
    connector: &Box<&'static dyn RemoteConnector>,
    user: &String,
    repo: &String,
    task_id: &String,
    local_task: &Task,
    remote_comment_ids: &[String],
    success: &mut bool,
) -> bool {
    let mut synced = false;
    let mut id_updates = vec![];
    for comment in local_task.get_comments().as_ref().unwrap_or(&vec![]) {
        let local_id = match comment.get_id() {
            Some(id) => id,
            None => {
                eprintln!("ERROR: comment has no ID, skipping");
                *success = false;
                continue;
            }
        };
        if remote_comment_ids.contains(&local_id) {
            continue;
        }
        match connector.create_remote_comment(user, repo, task_id, comment) {
            Ok(remote_id) => {
                println!("Created REMOTE comment ID {remote_id}");
                if local_id != remote_id {
                    id_updates.push((local_id, remote_id));
                }
                synced = true;
            },
            Err(e) => {
                eprintln!("ERROR creating REMOTE comment: {e}");
                *success = false;
            }
        }
    }
    match gittask::update_comment_ids(task_id, &id_updates) {
        Ok(_) => for (local_id, remote_id) in &id_updates {
            println!("Comment ID {local_id} -> {remote_id} updated");
        },
        Err(e) => {
            eprintln!("ERROR: {e}");
            *success = false;
        }
    }
    synced
}

pub(crate) fn task_delete(
    ids: Option<String>,
    status: Option<Vec<String>>,
    push: bool,
    remote: &Option<String>,
    connector_type: &Option<String>,
    project: &Option<String>,
) -> bool {
    let ids = match status {
        Some(statuses) => {
            match gittask::list_tasks() {
                Ok(tasks) => {
                    let status_manager = StatusManager::new();
                    let statuses = statuses.iter().map(|s| status_manager.get_full_status_name(s)).collect::<Vec<_>>();
                    let ids = tasks.iter().filter(|task| task.get_property("status").map_or(false, |s| statuses.contains(s))).map(|task| task.get_id().unwrap()).collect::<Vec<_>>();
                    Ok(ids)
                },
                Err(e) => Err(e)
            }
        },
        None => {
            let ids = parse_ids(ids.unwrap());
            Ok(ids)
        }
    };

    if let Err(e) = ids {
        return error_message(e);
    }

    let ids = ids.unwrap();
    let ids = ids.iter().map(|id| id.as_str()).collect::<Vec<_>>();

    match gittask::delete_tasks(&ids) {
        Ok(deleted) => {
            let missing = ids.iter().filter(|id| !deleted.iter().any(|d| d == *id)).map(|id| id.to_string()).collect::<Vec<_>>();
            if deleted.is_empty() {
                return error_message(match missing.is_empty() {
                    true => "No tasks to delete".to_string(),
                    false => format!("Task(s) {} not found", missing.join(", ")),
                });
            }
            if !missing.is_empty() {
                eprintln!("Task(s) {} not found", missing.join(", "));
            }
            println!("Task(s) {} deleted", deleted.join(", "));
            let mut success = true;
            if push {
                for id in &deleted {
                    match get_user_repo(remote, connector_type, &resolve_project(project, id)) {
                        Ok((connector, user, repo)) => {
                            match connector.delete_remote_task(&user, &repo, &id.to_string()) {
                                Ok(_) => println!("Sync: REMOTE task ID {id} has been deleted"),
                                Err(e) => {
                                    eprintln!("ERROR: {e}");
                                    success = false;
                                }
                            }
                        },
                        Err(e) => {
                            eprintln!("ERROR: {e}");
                            success = false;
                        }
                    }
                }
            }

            success
        },
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

pub(crate) fn task_clear() -> bool {
    match gittask::clear_tasks() {
        Ok(task_count) => success_message(format!("{task_count} task(s) deleted")),
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

pub(crate) fn task_show(id: String, no_color: bool) -> bool {
    match gittask::find_task(&id) {
        Ok(Some(task)) => {
            let no_color = check_no_color(no_color);
            print_task(task, no_color);
            true
        },
        Ok(None) => error_message(format!("Task ID {id} not found")),
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

fn print_task(task: Task, no_color: bool) {
    let prop_manager = PropertyManager::new();
    let properties = prop_manager.get_properties();
    let context = extract_task_context(&task);

    let id_title = colorize_string("ID", DarkGray, no_color);
    println!("{}: {}", id_title, task.get_id().unwrap_or("---".to_owned()));

    let empty_string = String::new();

    let created = task.get_property("created").unwrap_or(&empty_string);
    if !created.is_empty() {
        let created_title = colorize_string("Created", DarkGray, no_color);
        println!("{}: {}", created_title, prop_manager.format_value("created", created, &context, properties, true));
    }

    let author = task.get_property("author").unwrap_or(&empty_string);
    if !author.is_empty() {
        let author_title = colorize_string("Author", DarkGray, no_color);
        println!("{}: {}", author_title, prop_manager.format_value("author", author, &context, properties, no_color));
    }

    let name_title = colorize_string("Name", DarkGray, no_color);
    println!("{}: {}", name_title, prop_manager.format_value("name", task.get_property("name").unwrap_or(&empty_string), &context, properties, no_color));

    if let Some(labels) = task.get_labels() {
        if !labels.is_empty() {
            let labels_title = colorize_string("Labels", DarkGray, no_color);
            print!("{labels_title}: ");

            for label in labels {
                print_label(label, no_color);
            }

            println!();
        }
    }

    let status_manager = StatusManager::new();
    let status_title = colorize_string("Status", DarkGray, no_color);
    println!("{}: {}", status_title, status_manager.format_status(task.get_property("status").unwrap_or(&empty_string), no_color));

    task.get_all_properties().iter().filter(|entry| {
        entry.0 != "name" && entry.0 != "status" && entry.0 != "description" && entry.0 != "created" && entry.0 != "author"
    }).for_each(|entry| {
        let title = colorize_string(&capitalize(entry.0), DarkGray, no_color);
        println!("{}: {}", title, prop_manager.format_value(entry.0, entry.1, &context, properties, no_color));
    });

    let description = task.get_property("description").unwrap_or(&empty_string);
    if !description.is_empty() {
        let description_title = colorize_string("Description", DarkGray, no_color);
        println!("{}: {}", description_title, prop_manager.format_value("description", description, &context, properties, no_color));
    }

    if let Some(subtasks) = task.get_subtasks() {
        if !subtasks.is_empty() {
            let status_manager = StatusManager::new();
            let subtasks_title = colorize_string("Subtasks", DarkGray, no_color);
            println!("{}:", subtasks_title);
            for subtask in subtasks {
                print_subtask(subtask, &status_manager, no_color);
            }
        }
    }

    if let Some(comments) = task.get_comments() {
        for comment in comments {
            print_comment(comment, &prop_manager, no_color);
        }
    }
}

fn print_subtask(subtask: &Subtask, status_manager: &StatusManager, no_color: bool) {
    let id = subtask.get_id().unwrap_or_else(|| "---".to_string());
    let id_title = colorize_string("  ID", DarkGray, no_color);
    let status = status_manager.format_status(subtask.get_status(), no_color);
    println!("{}: {} | {} | {}", id_title, id, status, subtask.get_name());
}

fn print_comment(comment: &Comment, prop_manager: &PropertyManager, no_color: bool) {
    let separator = colorize_string("---------------", DarkGray, no_color);
    println!("{}", separator);

    if let Some(id) = comment.get_id() {
        let id_title = colorize_string("Comment ID", DarkGray, no_color);
        println!("{}: {}", id_title, id);
    }

    let empty_string = String::new();
    let comment_properties = comment.get_all_properties();

    let created = comment_properties.get("created").unwrap_or(&empty_string);
    if !created.is_empty() {
        let created_title = colorize_string("Created", DarkGray, no_color);
        println!("{}: {}", created_title, prop_manager.format_value("created", created, comment_properties, prop_manager.get_properties(), true));
    }

    let author = comment_properties.get("author").unwrap_or(&empty_string);
    if !author.is_empty() {
        let author_title = colorize_string("Author", DarkGray, no_color);
        println!("{}: {}", author_title, prop_manager.format_value("author", author, comment_properties, prop_manager.get_properties(), no_color));
    }

    println!("{}", comment.get_text());
}

fn print_label(label: &Label, no_color: bool) {
    match no_color {
        true => print!("{} ", label.get_name()),
        false => {
            let color = str_to_color(label.get_color().as_str(), &None);
            print!("{} ", color.paint(label.get_name()));
        }
    }
}

fn make_comparison(first: &Task, second: &Task, prop: &str, value_type: &str) -> Ordering {
    match prop {
        "id" => {
            gittask::compare_ids(&first.get_id().unwrap_or_default(), &second.get_id().unwrap_or_default())
        },
        _ => {
            match value_type {
                "integer" | "datetime" => {
                    let first_value = match first.get_property(prop) {
                        Some(value) => value.parse::<u64>().unwrap_or(0),
                        _ => 0,
                    };
                    let second_value = match second.get_property(prop) {
                        Some(value) => value.parse::<u64>().unwrap_or(0),
                        _ => 0,
                    };

                    first_value.cmp(&second_value)
                },
                _ => {
                    let first_value = match first.get_property(prop) {
                        Some(value) => value.to_lowercase(),
                        _ => String::new(),
                    };
                    let second_value = match second.get_property(prop) {
                        Some(value) => value.to_lowercase(),
                        _ => String::new(),
                    };

                    first_value.cmp(&second_value)
                }
            }
        }
    }
}

pub(crate) fn task_list(status: Option<Vec<String>>,
             keyword: Option<String>,
             filter: Option<String>,
             from: Option<String>,
             until: Option<String>,
             author: Option<String>,
             columns: Option<Vec<String>>,
             headers: bool,
             sort: Option<Vec<String>>,
             limit: Option<usize>,
             no_color: bool) -> bool {
    match gittask::list_tasks() {
        Ok(tasks) => {
            if tasks.is_empty() {
                return true;
            }

            let prop_manager = PropertyManager::new();

            let from = parse_date(from);
            let until = parse_date(until);

            let status_manager = StatusManager::new();
            let statuses = match status {
                Some(statuses) => Some(statuses.iter().map(|s| status_manager.get_full_status_name(s)).collect::<Vec<_>>()),
                None => None
            };

            let mut filtered_tasks: Vec<Task> = tasks.into_iter().filter(|task| {
                if let Some(ref statuses) = statuses {
                    match task.get_property("status") {
                        Some(task_status) if statuses.contains(task_status) => {},
                        _ => return false,
                    }
                }

                if keyword.as_ref().is_some() {
                    let keyword = keyword.as_ref().unwrap().as_str();
                    let props = task.get_all_properties();
                    if !props.iter().any(|entry| entry.1.contains(keyword)) {
                        return false;
                    }
                }

                if let Some(ref filter) = filter {
                    let mut eval_context = HashMapContext::new();
                    let context = extract_task_context(&task);

                    for (k, v) in context {
                        let property = prop_manager.get_properties().iter().find(|p| p.get_name() == k);
                        let is_integer = match property {
                            Some(property) => matches!(property.get_value_type(), PropertyValueType::Integer | PropertyValueType::DateTime),
                            None => false,
                        };

                        if is_integer {
                            if let Ok(i) = v.parse::<i64>() {
                                eval_context.set_value(k.into(), i.into()).unwrap();
                            } else {
                                eval_context.set_value(k.into(), v.into()).unwrap();
                            }
                        } else {
                            eval_context.set_value(k.into(), v.into()).unwrap();
                        }
                    }

                    if let Ok(result) = evalexpr::eval_boolean_with_context(filter, &eval_context) {
                        if !result {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }

                if from.is_some() || until.is_some() {
                    let created = task.get_property("created");
                    if let Some(created) = created {
                        let created = match created.parse().ok().and_then(|ts| Local.timestamp_opt(ts, 0).single()) {
                            Some(dt) => dt,
                            None => return true,
                        };

                        if from.is_some() {
                            if created < from.unwrap().earliest().unwrap() {
                                return false;
                            }
                        }

                        if until.is_some() {
                            if created > until.unwrap().latest().unwrap() {
                                return false;
                            }
                        }
                    }
                }

                if author.as_ref().is_some() {
                    match task.get_property("author") {
                        Some(task_author) => {
                            if author.as_ref().unwrap().to_lowercase() != task_author.to_lowercase() {
                                return false;
                            }
                        },
                        None => return false,
                    }
                }

                true
            }).collect();

            let sort = match sort {
                Some(sort) => Some(sort),
                None => match gittask::get_config_value("task.list.sort") {
                    Ok(sort) => {
                        Some(sort.split(",").map(|s| s.trim().to_string()).collect())
                    },
                    _ => None
                }
            };

            filtered_tasks.sort_by(|a, b| {
                match &sort {
                    Some(sort) if !sort.is_empty() => {
                        let mut ordering = None;
                        for s in sort {
                            let mut s = s.trim();
                            let comparison;
                            if s.to_lowercase().ends_with(" desc") {
                                s = s[..(s.len() - "desc".len())].trim();
                                comparison = make_comparison(b, a, s, &prop_manager.get_parameter(&s, "value_type").unwrap_or_else(|| String::from("")));
                            } else {
                                if s.to_lowercase().ends_with(" asc") {
                                    s = s[..(s.len() - "asc".len())].trim();
                                }
                                comparison = make_comparison(a, b, s, &prop_manager.get_parameter(&s, "value_type").unwrap_or_else(|| String::from("")));
                            }

                            if ordering.is_none() {
                                ordering = Some(comparison);
                            } else {
                                ordering = Some(ordering.unwrap().then(comparison));
                            }
                        }

                        ordering.unwrap()
                    },
                    _ => make_comparison(b, a, "id", "")
                }
            });

            let filtered_tasks: Vec<Task> = match limit {
                Some(limit) => filtered_tasks.into_iter().take(limit).collect(),
                None => filtered_tasks
            };

            if !filtered_tasks.is_empty() {
                let no_color = check_no_color(no_color);

                let columns = columns.unwrap_or_else(|| {
                    let list_columns = gittask::get_config_value("task.list.columns").unwrap_or_else(|_| String::from(DEFAULT_LIST_COLUMNS));
                    list_columns.split(",").map(|s| s.trim().to_string()).collect()
                });

                let show_headers = headers || match gittask::get_config_value("task.list.show.headers") {
                    Ok(show_headers) => parse_bool(&show_headers).unwrap_or(false),
                    _ => false
                };

                if show_headers {
                    let header = columns.join(" | ");
                    println!("{}", header);
                }

                for task in filtered_tasks {
                    print_task_line(task, &columns, no_color, &prop_manager, &status_manager);
                }
            }

            true
        },
        Err(e) => {
            error_message(format!("ERROR: {e}"))
        }
    }
}

fn print_task_line(task: Task, columns: &Vec<String>, no_color: bool, prop_manager: &PropertyManager, status_manager: &StatusManager) {
    let context = extract_task_context(&task);

    columns.iter().for_each(|column| {
        print_column(&task, column, &context, no_color, prop_manager, status_manager);
    });
    println!();
}

fn print_column(
    task: &Task,
    column: &String,
    context: &HashMap<String, String>,
    no_color: bool,
    prop_manager: &PropertyManager,
    status_manager: &StatusManager
) {
    let empty_string = String::new();
    match column.as_str() {
        "status" => {
            print!("{} ", status_manager.format_status(task.get_property(column).unwrap_or(&empty_string), no_color))
        },
        "labels" => if let Some(labels) = task.get_labels() {
            for label in labels {
                print_label(label, no_color);
            }
        },
        column => {
            let value = if column == "id" {
                &task.get_id().unwrap()
            } else {
                task.get_property(column).unwrap_or_else(|| {
                    &empty_string
                })
            };
            print!("{} ", prop_manager.format_value(column, value, context, prop_manager.get_properties(), no_color))
        },
    }
}

pub(crate) fn task_stats(no_color: bool) -> bool {
    match gittask::list_tasks() {
        Ok(tasks) => {
            let mut total = 0;
            let mut status_stats = HashMap::<String, i32>::new();
            let mut author_stats = HashMap::<String, i32>::new();
            let no_color = check_no_color(no_color);

            for task in tasks {
                total += 1;

                if let Some(status) = task.get_property("status") {
                    status_stats.entry(status.to_owned()).and_modify(|count| *count += 1).or_insert(1);
                }

                if let Some(author) = task.get_property("author") {
                    author_stats.entry(author.to_owned()).and_modify(|count| *count += 1).or_insert(1);
                }
            }

            println!("Total tasks: {total}");
            println!();

            let status_manager = StatusManager::new();
            for status in status_manager.get_statuses() {
                if let Some(count) = status_stats.get(status.get_name()) {
                    println!("{}: {}", status_manager.format_status(status.get_name(), no_color), count);
                }
            }

            if !author_stats.is_empty() {
                println!();
                println!("Top 10 authors:");

                let prop_manager = PropertyManager::new();
                let empty_context = HashMap::new();

                let mut author_stats = author_stats.iter().collect::<Vec<_>>();
                author_stats.sort_by(|a, b| b.1.cmp(a.1));

                for author in author_stats.iter().take(10) {
                    println!("{}: {}", prop_manager.format_value("author", &author.0, &empty_context, &vec![], no_color), author.1);
                }
            }
            true
        },
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn check_no_color(no_color: bool) -> bool {
    no_color
        || gittask::get_config_value("color.ui").unwrap_or_else(|_| "true".to_string()) == "false"
        || std::env::var("NO_COLOR").is_ok()
}

fn extract_task_context(task: &Task) -> HashMap<String, String> {
    let mut context = task.get_all_properties().to_owned();
    context.insert("id".to_string(), task.get_id().unwrap());
    context
}

fn get_connector<'a>(connector_type: &Option<String>) -> Option<String> {
    match connector_type {
        Some(connector_type) => {
            Some(connector_type.to_string())
        },
        None => match gittask::get_config_value("task.default.connector") {
            Ok(default_connector) => {
                Some(default_connector)
            },
            _ => None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_from_id() {
        assert_eq!(project_from_id("ABC-42"), Some("ABC".to_string()));
        assert_eq!(project_from_id("a_1-7"), Some("a_1".to_string()));
        assert_eq!(project_from_id("42"), None);
        assert_eq!(project_from_id("1-2"), None, "project keys start with a letter");
        assert_eq!(project_from_id("ABC-"), None);
        assert_eq!(project_from_id("ABC-4x"), None);
    }

    #[test]
    fn test_resolve_project_prefers_explicit_project() {
        assert_eq!(resolve_project(&Some("XYZ".to_string()), "ABC-42"), Some("XYZ".to_string()));
        assert_eq!(resolve_project(&None, "ABC-42"), Some("ABC".to_string()));
        assert_eq!(resolve_project(&None, "42"), None);
    }

    #[test]
    fn test_task_create_rejects_blank_name_without_panic() {
        let _repo = gittask::testing::TestRepo::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            task_create("".to_string(), None, true, false, &None, &None, &None)
        }));
        assert!(result.is_ok(), "task_create must not panic on an empty name");
        assert!(!result.unwrap(), "task_create must fail on an empty name");

        let result = task_create("   ".to_string(), None, true, false, &None, &None, &None);
        assert!(!result, "task_create must fail on a whitespace-only name");
    }

    #[test]
    fn test_task_set_rejects_blank_name_and_status() {
        let _repo = gittask::testing::TestRepo::new();
        let task = gittask::Task::new("Keep me".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        let task = gittask::create_task(task).unwrap();
        let id = task.get_id().unwrap();

        assert!(!task_set(id.clone(), "name".to_string(), "  ".to_string(), false, &None, &None, &None, true));
        assert!(!task_set(id.clone(), "status".to_string(), "".to_string(), false, &None, &None, &None, true));
        let task = gittask::find_task(&id).unwrap().unwrap();
        assert_eq!(task.get_property("name").unwrap(), "Keep me");
        assert_eq!(task.get_property("status").unwrap(), "OPEN");

        let _ = gittask::delete_tasks(&[&id]);
    }

    #[test]
    fn test_task_set_status_validates_against_configured_statuses() {
        let _repo = gittask::testing::TestRepo::new();
        let task = gittask::Task::new("Status check".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        let task = gittask::create_task(task).unwrap();
        let id = task.get_id().unwrap();
        let status_manager = StatusManager::new();
        let final_status = status_manager.get_final_status();
        let shortcut = status_manager.get_property(&final_status, "shortcut").unwrap();

        assert!(!task_status(id.clone(), "BOGUS".to_string(), false, false, &None, &None, &None, true), "unknown status must be rejected");
        assert_eq!(gittask::find_task(&id).unwrap().unwrap().get_property("status").unwrap(), "OPEN");

        assert!(task_status(id.clone(), shortcut, false, false, &None, &None, &None, true), "a shortcut must be accepted");
        assert_eq!(gittask::find_task(&id).unwrap().unwrap().get_property("status").unwrap(), &final_status);

        let _ = gittask::delete_tasks(&[&id]);
    }

    #[test]
    fn test_task_status_create_adds_unknown_status() {
        let _repo = gittask::testing::TestRepo::new();
        let task = gittask::Task::new("Status create".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        let task = gittask::create_task(task).unwrap();
        let id = task.get_id().unwrap();
        let name = "TEST_CREATED_STATUS";

        let result = task_status(id.clone(), name.to_string(), true, false, &None, &None, &None, true);
        let task_status_value = gittask::find_task(&id).unwrap().unwrap().get_property("status").cloned();
        let created = StatusManager::new().resolve_status(name);
        let _ = StatusManager::new().delete_status(name.to_string());
        let _ = gittask::delete_tasks(&[&id]);

        assert!(result, "--create must add the status and set it");
        assert_eq!(task_status_value.as_deref(), Some(name));
        assert_eq!(created, Ok(name.to_string()), "the status must be in the configuration afterwards");
    }

    #[test]
    fn test_task_set_returns_false_on_nonexistent_task() {
        let _repo = gittask::testing::TestRepo::new();
        let result = task_set("99999".to_string(), "name".to_string(), "test".to_string(), false, &None, &None, &None, true);
        assert!(!result, "task_set should return false when task is not found");
    }

    #[test]
    fn test_task_unset_returns_false_on_nonexistent_task() {
        let _repo = gittask::testing::TestRepo::new();
        let result = task_unset("99999".to_string(), "name".to_string());
        assert!(!result, "task_unset should return false when task is not found");
    }

    #[test]
    fn test_task_unset_protected_properties() {
        let _repo = gittask::testing::TestRepo::new();
        let task = gittask::Task::new("T".to_string(), "d".to_string(), "OPEN".to_string()).unwrap();
        let task = gittask::create_task(task).unwrap();
        let id = task.get_id().unwrap();

        assert!(!task_unset(id.clone(), "status".to_string()), "unsetting status must be rejected");
        assert!(!task_unset(id.clone(), "name".to_string()), "unsetting name must be rejected");
        let task = gittask::find_task(&id).unwrap().unwrap();
        assert!(task.has_property("status"));
        assert!(task.has_property("name"));

        let _ = gittask::delete_tasks(&[&id]);
    }

    #[test]
    fn test_task_list_with_statusless_task() {
        let _repo = gittask::testing::TestRepo::new();
        let json = r#"[{"id":"999888","props":{"name":"nostatus","description":"","created":"123"},"comments":null,"labels":null}]"#;
        let tasks: Vec<gittask::Task> = serde_json::from_str(json).unwrap();
        let _ = gittask::create_task(tasks.into_iter().next().unwrap());

        let list_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            task_list(Some(vec!["OPEN".to_string()]), None, None, None, None, None, None, false, None, None, true)
        }));
        let export_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            task_export(None, Some(vec!["OPEN".to_string()]), None, None, false)
        }));
        let show_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            task_show("999888".to_string(), true)
        }));

        let _ = gittask::delete_tasks(&["999888"]);
        assert!(list_result.is_ok(), "task_list must not panic on a task without status");
        assert!(export_result.is_ok(), "task_export must not panic on a task without status");
        assert!(show_result.is_ok(), "task_show must not panic on a task without status");
    }

    #[test]
    fn test_task_replace_returns_false_on_nonexistent_task() {
        let _repo = gittask::testing::TestRepo::new();
        let result = task_replace("99999".to_string(), "name".to_string(), "old".to_string(), "new".to_string(), false, false, &None, &None, &None, true);
        assert!(!result, "task_replace should return false when task is not found");
    }

    #[test]
    fn test_make_comparison_datetime_numeric() {
        let _repo = gittask::testing::TestRepo::new();
        let mut task_a = gittask::Task::new("A".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        task_a.set_property("created", "9");
        let mut task_b = gittask::Task::new("B".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        task_b.set_property("created", "10");
        let result = make_comparison(&task_a, &task_b, "created", "datetime");
        assert_eq!(result, std::cmp::Ordering::Less, "datetime comparison should be numeric, not lexicographic (9 < 10)");
    }

    #[test]
    fn test_filter_context_datetime_is_numeric() {
        let mut eval_context = evalexpr::HashMapContext::new();
        let created_value = "1000000";

        let is_numeric = matches!(PropertyValueType::DateTime, PropertyValueType::Integer | PropertyValueType::DateTime);
        assert!(is_numeric, "DateTime should be treated as numeric type");

        if let Ok(i) = created_value.parse::<i64>() {
            eval_context.set_value("created".into(), evalexpr::Value::Int(i)).unwrap();
        }

        let result = evalexpr::eval_boolean_with_context("created > 100", &eval_context);
        assert!(result.is_ok(), "datetime filter should evaluate without error, got: {:?}", result);
        assert!(result.unwrap(), "created (1000000) should be > 100 numerically");
    }

    struct FakeConnector {
        next_id: std::sync::atomic::AtomicU64,
        lookup: std::sync::Mutex<Result<Option<Task>, String>>,
        created_tasks: std::sync::atomic::AtomicU64,
    }

    impl FakeConnector {
        fn leaked(first_id: u64) -> Box<&'static dyn RemoteConnector> {
            Box::new(Self::leaked_with_lookup(first_id, Err("not used".to_string())))
        }

        fn leaked_with_lookup(first_id: u64, lookup: Result<Option<Task>, String>) -> &'static FakeConnector {
            Box::leak(Box::new(FakeConnector {
                next_id: std::sync::atomic::AtomicU64::new(first_id),
                lookup: std::sync::Mutex::new(lookup),
                created_tasks: std::sync::atomic::AtomicU64::new(0),
            }))
        }

        fn next(&self) -> String {
            self.next_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst).to_string()
        }

        fn created_tasks(&self) -> u64 {
            self.created_tasks.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl RemoteConnector for FakeConnector {
        fn type_name(&self) -> &str { "fake" }
        fn supports_remote(&self, _url: &str) -> Option<(String, String)> { None }
        fn list_remote_tasks(&self, _: &String, _: &String, _: bool, _: bool, _: Option<usize>, _: RemoteTaskState, _: &Vec<String>) -> Result<Vec<Task>, String> { Err("not used".to_string()) }
        fn get_remote_task(&self, _: &String, _: &String, _: &String, _: bool, _: bool, _: &Vec<String>) -> Result<Option<Task>, String> { self.lookup.lock().unwrap().clone() }
        fn create_remote_task(&self, _: &String, _: &String, _: &Task) -> Result<String, String> { self.created_tasks.fetch_add(1, std::sync::atomic::Ordering::SeqCst); Ok("77".to_string()) }
        fn create_remote_comment(&self, _: &String, _: &String, _: &String, _: &Comment) -> Result<String, String> { Ok(self.next()) }
        fn create_remote_label(&self, _: &String, _: &String, _: &String, _: &Label) -> Result<(), String> { Err("not used".to_string()) }
        fn update_remote_task(&self, _: &String, _: &String, _: &Task, _: Option<&Vec<Label>>, _: RemoteTaskState) -> Result<(), String> { Err("not used".to_string()) }
        fn update_remote_comment(&self, _: &String, _: &String, _: &String, _: &String, _: &String) -> Result<(), String> { Err("not used".to_string()) }
        fn delete_remote_task(&self, _: &String, _: &String, _: &String) -> Result<(), String> { Err("not used".to_string()) }
        fn delete_remote_comment(&self, _: &String, _: &String, _: &String, _: &String) -> Result<(), String> { Err("not used".to_string()) }
        fn delete_remote_label(&self, _: &String, _: &String, _: &String, _: &String) -> Result<(), String> { Err("not used".to_string()) }
        fn supports_subtasks(&self) -> bool { true }
        fn list_remote_subtasks(&self, _: &String, _: &String, _: &String, _: &Vec<String>) -> Result<Vec<Subtask>, String> { Ok(vec![]) }
        fn create_remote_subtask(&self, _: &String, _: &String, _: &String, _: &Subtask, _: bool) -> Result<String, String> { Ok(self.next()) }
    }

    fn push_with_lookup(lookup: Result<Option<Task>, String>) -> (bool, &'static FakeConnector, String) {
        let task = gittask::create_task(gittask::Task::new("Local".to_string(), "".to_string(), "OPEN".to_string()).unwrap()).unwrap();
        let id = task.get_id().unwrap();
        let fake = FakeConnector::leaked_with_lookup(2, lookup);
        let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);
        let statuses = vec!["OPEN".to_string(), "CLOSED".to_string()];
        let result = push_task(&connector, &"u".to_string(), &"r".to_string(), &id, false, false, true, &StatusManager::new(), &statuses, true);
        (result, fake, id)
    }

    #[test]
    fn test_push_task_does_not_create_a_remote_task_when_the_lookup_fails() {
        let _repo = gittask::testing::TestRepo::new();

        let (result, fake, id) = push_with_lookup(Err("network down".to_string()));

        assert!(!result, "a failed lookup must fail the push");
        assert_eq!(fake.created_tasks(), 0, "a failed lookup must not be mistaken for a missing remote task");
        assert!(gittask::find_task(&id).unwrap().is_some(), "the local task must keep its id");
    }

    #[test]
    fn test_push_task_creates_the_remote_task_when_it_is_not_found() {
        let _repo = gittask::testing::TestRepo::new();

        let (result, fake, id) = push_with_lookup(Ok(None));

        assert!(result, "a missing remote task must be created");
        assert_eq!(fake.created_tasks(), 1);
        assert!(gittask::find_task(&id).unwrap().is_none(), "the local task must adopt the new remote id");
        assert!(gittask::find_task("77").unwrap().is_some());
    }

    #[test]
    fn test_pull_task_keeps_the_local_task_when_the_lookup_fails() {
        let _repo = gittask::testing::TestRepo::new();
        let mut task = gittask::Task::new("Local".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        task.add_comment(None, HashMap::new(), "keep me".to_string());
        let id = gittask::create_task(task).unwrap().get_id().unwrap();
        let fake = FakeConnector::leaked_with_lookup(2, Err("network down".to_string()));
        let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);

        let result = pull_task(&connector, &"u".to_string(), &"r".to_string(), &id, false, false, true, &vec!["OPEN".to_string(), "CLOSED".to_string()]);

        assert!(!result);
        let comments = gittask::find_task(&id).unwrap().unwrap().get_comments().clone().unwrap_or_default();
        assert_eq!(comments.len(), 1, "a failed lookup must leave local comments alone");
    }

    #[test]
    fn test_sync_local_subtasks_survives_remote_ids_that_collide_with_local_ones() {
        let _repo = gittask::testing::TestRepo::new();
        let mut task = gittask::Task::new("Parent".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        task.add_subtask(None, "A".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        task.add_subtask(None, "B".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        let task = gittask::create_task(task).unwrap();
        let id = task.get_id().unwrap();
        let connector = FakeConnector::leaked(2);
        let status_manager = StatusManager::new();
        let mut success = true;

        sync_local_subtasks(&connector, &"u".to_string(), &"r".to_string(), &id, &task, &status_manager, &vec!["OPEN".to_string(), "CLOSED".to_string()], &mut success);

        assert!(success);
        let ids = gittask::find_task(&id).unwrap().unwrap().get_subtasks().clone().unwrap()
            .iter().map(|s| (s.get_name().to_string(), s.get_id().unwrap())).collect::<Vec<_>>();
        assert_eq!(ids, vec![("A".to_string(), "2".to_string()), ("B".to_string(), "3".to_string())], "each subtask must end up with the id the remote returned for it");
    }

    #[test]
    fn test_sync_local_comments_survives_remote_ids_that_collide_with_local_ones() {
        let _repo = gittask::testing::TestRepo::new();
        let mut task = gittask::Task::new("Parent".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        task.add_comment(None, HashMap::new(), "first".to_string());
        task.add_comment(None, HashMap::new(), "second".to_string());
        let task = gittask::create_task(task).unwrap();
        let id = task.get_id().unwrap();
        let connector = FakeConnector::leaked(2);
        let mut success = true;

        sync_local_comments(&connector, &"u".to_string(), &"r".to_string(), &id, &task, &[], &mut success);

        assert!(success);
        let ids = gittask::find_task(&id).unwrap().unwrap().get_comments().clone().unwrap()
            .iter().map(|c| (c.get_text().to_string(), c.get_id().unwrap())).collect::<Vec<_>>();
        assert_eq!(ids, vec![("first".to_string(), "2".to_string()), ("second".to_string(), "3".to_string())], "each comment must end up with the id the remote returned for it");
    }

    fn tasks_ref_target(repo: &gittask::testing::TestRepo) -> git2::Oid {
        git2::Repository::open(repo.path()).unwrap().find_reference(&gittask::get_ref_path()).unwrap().target().unwrap()
    }

    #[test]
    fn test_task_delete_continues_past_absent_ids() {
        let _repo = gittask::testing::TestRepo::new();
        let a = gittask::create_task(gittask::Task::new("A".to_string(), "".to_string(), "OPEN".to_string()).unwrap()).unwrap().get_id().unwrap();
        let b = gittask::create_task(gittask::Task::new("B".to_string(), "".to_string(), "OPEN".to_string()).unwrap()).unwrap().get_id().unwrap();
        let c = gittask::create_task(gittask::Task::new("C".to_string(), "".to_string(), "OPEN".to_string()).unwrap()).unwrap().get_id().unwrap();
        assert!(task_delete(Some(b.clone()), None, false, &None, &None, &None));

        let result = task_delete(Some(format!("{a},{b},{c}")), None, false, &None, &None, &None);

        assert!(result, "a batch with an already deleted id must still delete the others");
        assert!(gittask::find_task(&a).unwrap().is_none());
        assert!(gittask::find_task(&c).unwrap().is_none());
    }

    #[test]
    fn test_task_delete_fails_when_nothing_matches() {
        let repo = gittask::testing::TestRepo::new();
        let a = gittask::create_task(gittask::Task::new("A".to_string(), "".to_string(), "OPEN".to_string()).unwrap()).unwrap().get_id().unwrap();
        let before = tasks_ref_target(&repo);

        assert!(!task_delete(Some("999999".to_string()), None, false, &None, &None, &None), "deleting an unknown id must fail");
        assert!(!task_delete(None, Some(vec!["CLOSED".to_string()]), false, &None, &None, &None), "deleting by a status no task has must fail");

        assert_eq!(tasks_ref_target(&repo), before, "no commit must be written when nothing is deleted");
        assert!(gittask::find_task(&a).unwrap().is_some());
    }

    #[test]
    fn test_task_replace_rejects_a_blank_name() {
        let _repo = gittask::testing::TestRepo::new();
        let task = gittask::create_task(gittask::Task::new("Old name".to_string(), "".to_string(), "OPEN".to_string()).unwrap()).unwrap();
        let id = task.get_id().unwrap();

        let result = task_replace(id.clone(), "name".to_string(), "Old name".to_string(), "  ".to_string(), false, false, &None, &None, &None, true);

        assert!(!result, "task_replace must fail when the replacement blanks out the name");
        let task = gittask::find_task(&id).unwrap().unwrap();
        assert_eq!(task.get_property("name").unwrap(), "Old name", "the name must stay untouched");
    }

    #[test]
    fn test_task_replace_invalid_regex_no_panic() {
        let _repo = gittask::testing::TestRepo::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            task_replace("1".to_string(), "name".to_string(), "[invalid".to_string(), "".to_string(), true, false, &None, &None, &None, true)
        }));
        assert!(result.is_ok(), "task_replace should not panic on invalid regex");
        assert!(!result.unwrap(), "task_replace should return false on invalid regex");
    }

    #[test]
    fn test_date_filter_with_non_numeric_created() {
        let _repo = gittask::testing::TestRepo::new();
        let mut task = gittask::Task::new("Test".to_string(), "desc".to_string(), "OPEN".to_string()).unwrap();
        task.set_property("created", "not-a-number");
        let create_result = gittask::create_task(task);
        assert!(create_result.is_ok());
        let task = create_result.unwrap();
        let task_id = task.get_id().unwrap();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            task_list(None, None, None, Some("2020-01-01".to_string()), None, None, None, false, None, None, true)
        }));
        let _ = gittask::delete_tasks(&[&task_id]);
        assert!(result.is_ok(), "task_list with --from should not panic when a task has non-numeric created value");
    }

    #[test]
    fn test_import_task_without_id() {
        let _repo = gittask::testing::TestRepo::new();
        let input = r#"[{"props":{"name":"test","status":"OPEN","description":"d","created":"123"},"comments":null,"labels":null}]"#.to_string();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            import_from_input(None, &input)
        }));
        assert!(result.is_ok(), "import_from_input should not panic when task has no id");
    }

    #[test]
    fn test_task_edit_id_same_value() {
        let _repo = gittask::testing::TestRepo::new();
        let task = gittask::Task::new("Test".to_string(), "desc".to_string(), "OPEN".to_string()).unwrap();
        let create_result = gittask::create_task(task);
        assert!(create_result.is_ok());
        let task = create_result.unwrap();
        let task_id = task.get_id().unwrap();

        // Simulate what task_edit does for "id" when user doesn't change the id
        let mut task = gittask::find_task(&task_id).unwrap().unwrap();
        let text = task_id.clone(); // same id
        task.set_id(text.clone());
        let update_result = gittask::update_task(task);
        assert!(update_result.is_ok());
        if text != task_id {
            let _ = gittask::delete_tasks(&[&task_id]);
        }

        let find_result = gittask::find_task(&task_id);
        assert!(find_result.is_ok());
        assert!(find_result.unwrap().is_some(), "Task should still exist after editing ID to the same value");

        let _ = gittask::delete_tasks(&[&task_id]);
    }

    #[test]
    fn test_subtask_add_and_delete_flow() {
        let _repo = gittask::testing::TestRepo::new();
        use crate::operations::subtask::{task_subtask_add, task_subtask_delete, task_subtask_set};
        let task = gittask::Task::new("Parent".to_string(), "d".to_string(), "OPEN".to_string()).unwrap();
        let created = gittask::create_task(task).unwrap();
        let task_id = created.get_id().unwrap();

        assert!(task_subtask_add(task_id.clone(), Some("Sub1".to_string()), None, false, &None, &None, &None));
        assert!(task_subtask_add(task_id.clone(), Some("Sub2".to_string()), Some("CLOSED".to_string()), false, &None, &None, &None));

        let task = gittask::find_task(&task_id).unwrap().unwrap();
        let subtasks = task.get_subtasks().as_ref().unwrap();
        assert_eq!(subtasks.len(), 2);
        assert_eq!(subtasks[1].get_status(), "CLOSED");

        assert!(task_subtask_set(task_id.clone(), "1".to_string(), "name".to_string(), "SubRenamed".to_string(), false, &None, &None, &None));
        let task = gittask::find_task(&task_id).unwrap().unwrap();
        assert_eq!(task.get_subtask("1").unwrap().get_name(), "SubRenamed");

        assert!(task_subtask_delete(task_id.clone(), "1".to_string(), false, &None, &None, &None));
        let task = gittask::find_task(&task_id).unwrap().unwrap();
        assert_eq!(task.get_subtasks().as_ref().unwrap().len(), 1);

        let _ = gittask::delete_tasks(&[&task_id]);
    }

    #[test]
    fn test_subtask_add_rejects_missing_name_without_pipe() {
        let _repo = gittask::testing::TestRepo::new();
        use crate::operations::subtask::task_subtask_add;
        let task = gittask::Task::new("P".to_string(), "d".to_string(), "OPEN".to_string()).unwrap();
        let created = gittask::create_task(task).unwrap();
        let id = created.get_id().unwrap();

        let ok = task_subtask_add(id.clone(), Some("  ".to_string()), None, false, &None, &None, &None);
        assert!(!ok, "whitespace-only name should be rejected");

        let _ = gittask::delete_tasks(&[&id]);
    }

    #[test]
    fn test_subtask_export_roundtrip() {
        let _repo = gittask::testing::TestRepo::new();
        let task = gittask::Task::new("Parent".to_string(), "d".to_string(), "OPEN".to_string()).unwrap();
        let mut created = gittask::create_task(task).unwrap();
        let id = created.get_id().unwrap();
        created.add_subtask(Some("10".to_string()), "Sub".to_string(), "OPEN".to_string(), std::collections::HashMap::new()).unwrap();
        gittask::update_task(created).unwrap();

        let tasks = gittask::list_tasks().unwrap();
        let json = serde_json::to_string(&tasks).unwrap();
        let input: Vec<gittask::Task> = serde_json::from_str(&json).unwrap();
        let reimported = input.iter().find(|t| t.get_id().unwrap() == id).unwrap();
        assert!(reimported.get_subtasks().is_some());
        assert_eq!(reimported.get_subtasks().as_ref().unwrap()[0].get_name(), "Sub");

        let _ = gittask::delete_tasks(&[&id]);
    }

    #[test]
    fn test_check_no_color_empty_value() {
        unsafe { std::env::set_var("NO_COLOR", ""); }
        let result = check_no_color(false);
        unsafe { std::env::remove_var("NO_COLOR"); }
        assert!(result, "NO_COLOR='' should disable color per the NO_COLOR spec");
    }
}
