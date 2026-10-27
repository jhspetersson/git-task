pub(crate) mod comment;
pub(crate) mod config;
pub(crate) mod label;
pub(crate) mod subtask;

use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{DateTime, Local, NaiveDate, TimeZone};
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
                                match gittask::update_task_id(&task.get_id().unwrap(), &id, false) {
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

    task_set(ids, "status".to_string(), status, push, false, remote, connector_type, project, no_color)
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
    force: bool,
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
                match gittask::update_task_id(&id, &value, force) {
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
                    let new_value = if prop_name == "status" {
                        match StatusManager::new().resolve_status(&new_value) {
                            Ok(canonical) => canonical,
                            Err(e) => {
                                eprintln!("Task ID {id}: {e}");
                                success = false;
                                continue;
                            }
                        }
                    } else {
                        new_value
                    };
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

pub(crate) fn task_edit(id: String, prop_name: String, force: bool) -> bool {
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
                            match gittask::update_task_id(&id, &text, force) {
                                Ok(_) => success_message(format!("Task ID {id} -> {text} updated")),
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
        let mut success = true;
        let mut to_import = vec![];

        for task in tasks {
            let id = match task.get_id() {
                Some(id) => id,
                None => {
                    eprintln!("ERROR: task has no id, skipping");
                    success = false;
                    continue;
                }
            };

            if let Some(ids) = &ids {
                if !ids.contains(&id) {
                    continue;
                }
            }

            to_import.push(task);
        }

        match gittask::import_tasks(to_import) {
            Ok(imported) => for task in imported {
                println!("Task ID {} imported", task.get_id().unwrap());
            },
            Err(e) => {
                eprintln!("ERROR: {e}");
                success = false;
            }
        }
        success
    } else {
        error_message("Can't deserialize input".to_string())
    }
}

pub(crate) fn task_pull(
    ids: Option<String>,
    limit: Option<usize>,
    status: Option<String>,
    open: bool,
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

    let state = pull_state(status, open, &status_manager);

    pull_targets(targets, limit, &state, &task_statuses, no_comments, no_labels, no_subtasks)
}

/// Pulls tasks from every target in turn. `limit` caps the total number of tasks
/// imported across all targets, not the number per target: each target is asked
/// for whatever budget is left, and targets after the budget runs out are skipped.
fn pull_targets(
    targets: Vec<RemoteTarget>,
    limit: Option<usize>,
    state: &RemoteTaskState,
    task_statuses: &Vec<String>,
    no_comments: bool,
    no_labels: bool,
    no_subtasks: bool,
) -> bool {
    let mut success = true;
    let mut remaining = limit;
    for (connector, user, repo) in targets {
        if remaining == Some(0) {
            break;
        }
        println!("Pulling tasks from {user}/{repo}...");
        let fetch_subtasks = !no_subtasks && connector.supports_subtasks();
        let fetch_labels = !no_labels && connector.supports_labels();

        match connector.list_remote_tasks(&user, &repo, !no_comments, fetch_labels, remaining, state.clone(), task_statuses) {
            Ok(tasks) => {
                let tasks: Vec<Task> = match remaining {
                    Some(budget) => tasks.into_iter().take(budget).collect(),
                    None => tasks,
                };
                if let Some(budget) = remaining.as_mut() {
                    *budget -= tasks.len();
                }
                if tasks.is_empty() {
                    success_message("No tasks found".to_string());
                } else {
                    let mut to_write = vec![];
                    for mut task in tasks {
                        let task_id = task.get_id().unwrap();
                        if fetch_subtasks {
                            match connector.list_remote_subtasks(&user, &repo, &task_id, task_statuses) {
                                Ok(subtasks) => task.set_subtasks(subtasks),
                                Err(e) => {
                                    eprintln!("Task ID {task_id}: {e}");
                                    success = false;
                                }
                            }
                        }
                        match import_remote_task(task, no_comments, fetch_subtasks, fetch_labels) {
                            Ok(Some(task)) => to_write.push(task),
                            Ok(None) => println!("Task ID {task_id} skipped, nothing to update"),
                            Err(e) => {
                                eprintln!("ERROR: {e}");
                                success = false;
                            }
                        }
                    }
                    success &= write_imported_tasks(to_write);
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

fn task_matches(
    task: &Task,
    statuses: &Option<Vec<String>>,
    keyword: &Option<String>,
    filter: &Option<String>,
    from: &Option<NaiveDate>,
    until: &Option<NaiveDate>,
    author: &Option<String>,
    prop_manager: &PropertyManager,
) -> bool {
    if let Some(statuses) = statuses {
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

    if let Some(filter) = filter {
        let mut eval_context: HashMapContext = HashMapContext::new();
        let context = extract_task_context(&task);

        for (k, v) in context {
            let property = prop_manager.get_properties().iter().find(|p| p.get_name() == k);
            let is_integer = match property {
                Some(property) => matches!(property.get_value_type(), PropertyValueType::Integer | PropertyValueType::DateTime),
                None => false,
            };

            if is_integer {
                if let Ok(i) = v.parse::<i64>() {
                    eval_context.set_value(k.into(), evalexpr::Value::from_int(i)).unwrap();
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
        let created = task.get_property("created")
            .and_then(|created| created.parse().ok())
            .and_then(|ts| Local.timestamp_opt(ts, 0).single());
        if let Some(created) = created {
            if !created_within(&created, from, until) {
                return false;
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
}

fn created_within(created: &DateTime<Local>, from: &Option<NaiveDate>, until: &Option<NaiveDate>) -> bool {
    let day = created.date_naive();
    from.is_none_or(|from| day >= from) && until.is_none_or(|until| day <= until)
}

fn pull_state(status: Option<String>, open: bool, status_manager: &StatusManager) -> RemoteTaskState {
    if open {
        return RemoteTaskState::Open(String::new(), String::new());
    }
    match status {
        Some(s) => {
            let status = status_manager.get_full_status_name(&s);
            let is_done = status_manager.get_property(&status, "is_done").and_then(|v| parse_bool(&v).ok()).unwrap_or(false);
            if is_done { RemoteTaskState::Closed(status.clone(), status) } else { RemoteTaskState::Open(status.clone(), status) }
        },
        None => RemoteTaskState::All
    }
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
    let fetch_labels = !no_labels && connector.supports_labels();
    let mut success = true;
    match connector.get_remote_task(user, repo, id, !no_comments, fetch_labels, task_statuses) {
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
            match import_remote_task(task, no_comments, fetch_subtasks, fetch_labels) {
                Ok(Some(task)) => success &= write_imported_tasks(vec![task]),
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

fn write_imported_tasks(tasks: Vec<Task>) -> bool {
    match gittask::import_tasks(tasks) {
        Ok(written) => {
            for task in written {
                println!("Task ID {} updated", task.get_id().unwrap());
            }
            true
        },
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

fn import_remote_task(remote_task: Task, no_comments: bool, with_subtasks: bool, with_labels: bool) -> Result<Option<Task>, String> {
    match gittask::find_task(&remote_task.get_id().unwrap()) {
        Ok(Some(mut local_task)) => {
            let merged_subtasks = match (with_subtasks, remote_task.get_subtasks()) {
                (true, Some(remote_subtasks)) => Some(merge_subtasks(local_task.get_subtasks(), remote_subtasks)),
                _ => None,
            };
            let subtasks_equal = !with_subtasks
                || subtasks_are_equal(local_task.get_subtasks(), &merged_subtasks);
            let labels_equal = !with_labels
                || label_names(local_task.get_labels()) == label_names(remote_task.get_labels());
            let project_equal = match remote_task.get_property("project_id") {
                Some(remote_project) => local_task.get_property("project_id") == Some(remote_project),
                None => true,
            };

            if local_task.get_property("name") == remote_task.get_property("name")
                && local_task.get_property("description") == remote_task.get_property("description")
                && local_task.get_property("status") == remote_task.get_property("status")
                && (no_comments || comments_are_equal(local_task.get_comments(), remote_task.get_comments()))
                && subtasks_equal
                && labels_equal
                && project_equal {
                Ok(None)
            } else {
                local_task.set_property("name", remote_task.get_property("name").map(String::as_str).unwrap_or(""));
                local_task.set_property("description", remote_task.get_property("description").map(String::as_str).unwrap_or(""));
                local_task.set_property("status", remote_task.get_property("status").map(String::as_str).unwrap_or(""));
                if let Some(remote_project) = remote_task.get_property("project_id") {
                    local_task.set_property("project_id", remote_project);
                }
                if !no_comments {
                    if let Some(comments) = remote_task.get_comments() {
                        local_task.set_comments(comments.to_vec());
                    }
                }
                if let Some(subtasks) = merged_subtasks {
                    local_task.set_subtasks(subtasks);
                }
                if with_labels {
                    local_task.set_labels(remote_task.get_labels().clone().unwrap_or_default());
                }

                Ok(Some(local_task))
            }
        },
        Ok(None) => Ok(Some(remote_task)),
        Err(e) => Err(e)
    }
}

fn label_names(labels: &Option<Vec<Label>>) -> Vec<String> {
    let mut names = labels.as_ref().map(|labels| labels.iter().map(|l| l.get_name()).collect::<Vec<_>>()).unwrap_or_default();
    names.sort();
    names
}

fn merge_subtasks(local: &Option<Vec<Subtask>>, remote: &[Subtask]) -> Vec<Subtask> {
    let local_by_id: HashMap<String, &Subtask> = local.iter().flatten()
        .filter_map(|subtask| subtask.get_id().map(|id| (id, subtask)))
        .collect();
    remote.iter().map(|remote_subtask| {
        match remote_subtask.get_id().and_then(|id| local_by_id.get(&id)) {
            Some(local_subtask) => {
                let mut merged = (*local_subtask).clone();
                merged.set_name(remote_subtask.get_name().to_string());
                merged.set_status(remote_subtask.get_status().to_string());
                for (key, value) in remote_subtask.get_all_properties() {
                    merged.set_property(key, value);
                }
                merged
            },
            None => remote_subtask.clone(),
        }
    }).collect()
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
        (None, Some(r)) => r.is_empty(),
        (Some(l), None) => l.is_empty(),
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
    let sync_labels = !no_labels && connector.supports_labels();
    let remote_task = match connector.get_remote_task(user, repo, id, !no_comments, sync_labels, task_statuses) {
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
        let labels_changed = sync_labels && label_names(local_task.get_labels()) != label_names(remote_task.get_labels());
        let project_changed = match (local_task.get_property("project_id"), remote_task.get_property("project_id")) {
            (Some(local_project), Some(remote_project)) => local_project != remote_project,
            _ => false,
        };
        if local_name != remote_name || local_text != remote_text || status_changed || labels_changed || project_changed {
            anything_synced = true;
            if status_changed {
                println!("{}: {} -> {}", id, status_manager.format_status(remote_status, no_color), status_manager.format_status(local_status, no_color));
            }
            if project_changed {
                println!("{}: project {} -> {}", id, remote_task.get_property("project_id").unwrap(), local_task.get_property("project_id").unwrap());
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
                if sync_labels { local_task.get_labels().into() } else { None },
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
            let remote_comments = remote_task.get_comments().as_ref().map(Vec::as_slice).unwrap_or(&[]);
            anything_synced |= sync_local_comments(&connector, &user, &repo, &id, &local_task, remote_comments, &mut success);
        }
        if sync_subtasks {
            anything_synced |= sync_local_subtasks(&connector, &user, &repo, &id, &local_task, &status_manager, &task_statuses, &mut success);
        }
        if !anything_synced {
            println!("Nothing to sync");
        }
    } else {
        eprintln!("Sync: REMOTE task ID {id} NOT found");

        let local_task = match sync_labels {
            false => {
                let mut local_task = local_task;
                local_task.set_labels(vec![]);
                local_task
            },
            true => local_task
        };

        match connector.create_remote_task(&user, &repo, &local_task) {
            Ok(id) => {
                println!("Sync: Created REMOTE task ID {id}");
                let local_id = local_task.get_id().unwrap();
                if local_id != id {
                    match gittask::update_task_id(&local_id, &id, false) {
                        Ok(_) => println!("Task ID {local_id} -> {id} updated"),
                        Err(e) => {
                            eprintln!("ERROR: could not rename LOCAL task ID {local_id} to {id}: {e}");
                            eprintln!("REMOTE task ID {id} has been created but LOCAL task ID {local_id} still points nowhere, resolve the conflict manually");
                            return false;
                        }
                    }
                }

                let local_status = local_task.get_property("status").unwrap_or(&empty_string);
                let starting_status = &task_statuses[0];
                if local_status != starting_status {
                    let mut created_task = local_task.clone();
                    created_task.set_id(id.clone());
                    let state = if status_manager.is_done(local_status) {
                        RemoteTaskState::Closed(local_status.to_string(), starting_status.to_string())
                    } else {
                        RemoteTaskState::Open(local_status.to_string(), starting_status.to_string())
                    };
                    match connector.update_remote_task(&user, &repo, &created_task, None, state) {
                        Ok(_) => println!("{}: {} -> {}", id, status_manager.format_status(starting_status, no_color), status_manager.format_status(local_status, no_color)),
                        Err(e) => {
                            eprintln!("ERROR: could not set status of REMOTE task ID {id}: {e}");
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
                        id_updates.push((local_id, remote_id.clone()));
                    }
                    synced = true;
                    if let Err(e) = push_subtask_status(connector, user, repo, task_id, subtask, &remote_id, status_manager) {
                        eprintln!("ERROR: {e}");
                        *success = false;
                    }
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

pub(crate) fn push_subtask_status(
    connector: &Box<&'static dyn RemoteConnector>,
    user: &String,
    repo: &String,
    task_id: &String,
    subtask: &Subtask,
    remote_id: &str,
    status_manager: &StatusManager,
) -> Result<(), String> {
    if subtask.get_status() == status_manager.get_starting_status() {
        return Ok(());
    }
    let mut created = subtask.clone();
    created.set_id(remote_id.to_string());
    let is_done = status_manager.is_done(subtask.get_status());
    connector.update_remote_subtask(user, repo, task_id, &created, is_done)
        .map_err(|e| format!("REMOTE subtask ID {remote_id} was created but its status could not be set: {e}"))
}

fn sync_local_comments(
    connector: &Box<&'static dyn RemoteConnector>,
    user: &String,
    repo: &String,
    task_id: &String,
    local_task: &Task,
    remote_comments: &[Comment],
    success: &mut bool,
) -> bool {
    let remote_texts: HashMap<String, String> = remote_comments.iter()
        .filter_map(|comment| comment.get_id().map(|id| (id, comment.get_text())))
        .collect();
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
        if let Some(remote_text) = remote_texts.get(&local_id) {
            let local_text = comment.get_text();
            if local_text != *remote_text {
                match connector.update_remote_comment(user, repo, task_id, &local_id, &local_text) {
                    Ok(_) => {
                        println!("Updated REMOTE comment ID {local_id}");
                        synced = true;
                    },
                    Err(e) => {
                        eprintln!("ERROR updating REMOTE comment ID {local_id}: {e}");
                        *success = false;
                    }
                }
            }
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
                        Some(value) => value.parse::<i64>().unwrap_or(0),
                        _ => 0,
                    };
                    let second_value = match second.get_property(prop) {
                        Some(value) => value.parse::<i64>().unwrap_or(0),
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

            let mut filtered_tasks: Vec<Task> = tasks.into_iter().filter(|task| task_matches(task, &statuses, &keyword, &filter, &from, &until, &author, &prop_manager)).collect();

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

        assert!(!task_set(id.clone(), "name".to_string(), "  ".to_string(), false, false, &None, &None, &None, true));
        assert!(!task_set(id.clone(), "status".to_string(), "".to_string(), false, false, &None, &None, &None, true));
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
        let result = task_set("99999".to_string(), "name".to_string(), "test".to_string(), false, false, &None, &None, &None, true);
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
    fn test_make_comparison_integer_handles_negative_values() {
        let _repo = gittask::testing::TestRepo::new();
        let with_priority = |value: &str| {
            let mut task = gittask::Task::new(value.to_string(), "".to_string(), "OPEN".to_string()).unwrap();
            task.set_property("priority", value);
            task
        };
        let zero = with_priority("0");
        let minus_one = with_priority("-1");
        let minus_ten = with_priority("-10");

        assert_eq!(make_comparison(&minus_ten, &minus_one, "priority", "integer"), std::cmp::Ordering::Less, "-10 < -1");
        assert_eq!(make_comparison(&minus_one, &zero, "priority", "integer"), std::cmp::Ordering::Less, "-1 < 0");
        assert_eq!(make_comparison(&zero, &minus_ten, "priority", "integer"), std::cmp::Ordering::Greater, "0 > -10");
        assert_eq!(make_comparison(&minus_one, &minus_one, "priority", "integer"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn test_filter_context_datetime_is_numeric() {
        let mut eval_context: evalexpr::HashMapContext = evalexpr::HashMapContext::new();
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
        updates: std::sync::Mutex<Vec<Option<Vec<Label>>>>,
        remote_tasks: std::sync::Mutex<HashMap<String, Vec<Task>>>,
        list_limits: std::sync::Mutex<Vec<(String, Option<usize>)>>,
        subtask_updates: std::sync::Mutex<Vec<String>>,
        labels_supported: std::sync::atomic::AtomicBool,
        failing_subtask: std::sync::Mutex<Option<String>>,
        created_subtasks: std::sync::Mutex<Vec<Subtask>>,
        reject_subtask_updates: std::sync::atomic::AtomicBool,
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
                updates: std::sync::Mutex::new(vec![]),
                remote_tasks: std::sync::Mutex::new(HashMap::new()),
                list_limits: std::sync::Mutex::new(vec![]),
                subtask_updates: std::sync::Mutex::new(vec![]),
                labels_supported: std::sync::atomic::AtomicBool::new(true),
                failing_subtask: std::sync::Mutex::new(None),
                created_subtasks: std::sync::Mutex::new(vec![]),
                reject_subtask_updates: std::sync::atomic::AtomicBool::new(false),
            }))
        }

        /// A connector whose `list_remote_tasks` serves the given tasks per repo name.
        fn leaked_with_remote_tasks(remote_tasks: HashMap<String, Vec<Task>>) -> &'static FakeConnector {
            let fake = Self::leaked_with_lookup(1, Err("not used".to_string()));
            *fake.remote_tasks.lock().unwrap() = remote_tasks;
            fake
        }

        /// The (repo, limit) pairs passed to `list_remote_tasks`, in call order.
        fn list_limits(&self) -> Vec<(String, Option<usize>)> {
            self.list_limits.lock().unwrap().clone()
        }

        fn next(&self) -> String {
            self.next_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst).to_string()
        }

        fn created_tasks(&self) -> u64 {
            self.created_tasks.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn updates(&self) -> Vec<Option<Vec<Label>>> {
            self.updates.lock().unwrap().clone()
        }
    }

    impl RemoteConnector for FakeConnector {
        fn type_name(&self) -> &str { "fake" }
        fn supports_remote(&self, _url: &str) -> Option<(String, String)> { None }
        fn list_remote_tasks(&self, _: &String, repo: &String, _: bool, _: bool, limit: Option<usize>, _: RemoteTaskState, _: &Vec<String>) -> Result<Vec<Task>, String> {
            self.list_limits.lock().unwrap().push((repo.clone(), limit));
            let tasks = self.remote_tasks.lock().unwrap().get(repo).cloned().unwrap_or_default();
            Ok(match limit {
                Some(n) => tasks.into_iter().take(n).collect(),
                None => tasks,
            })
        }
        fn get_remote_task(&self, _: &String, _: &String, _: &String, _: bool, _: bool, _: &Vec<String>) -> Result<Option<Task>, String> { self.lookup.lock().unwrap().clone() }
        fn create_remote_task(&self, _: &String, _: &String, _: &Task) -> Result<String, String> { self.created_tasks.fetch_add(1, std::sync::atomic::Ordering::SeqCst); Ok("77".to_string()) }
        fn create_remote_comment(&self, _: &String, _: &String, _: &String, _: &Comment) -> Result<String, String> { Ok(self.next()) }
        fn create_remote_label(&self, _: &String, _: &String, _: &String, _: &Label) -> Result<(), String> { Err("not used".to_string()) }
        fn update_remote_task(&self, _: &String, _: &String, _: &Task, labels: Option<&Vec<Label>>, _: RemoteTaskState) -> Result<(), String> { self.updates.lock().unwrap().push(labels.cloned()); Ok(()) }
        fn update_remote_comment(&self, _: &String, _: &String, _: &String, _: &String, _: &String) -> Result<(), String> { Err("not used".to_string()) }
        fn delete_remote_task(&self, _: &String, _: &String, _: &String) -> Result<(), String> { Err("not used".to_string()) }
        fn delete_remote_comment(&self, _: &String, _: &String, _: &String, _: &String) -> Result<(), String> { Err("not used".to_string()) }
        fn delete_remote_label(&self, _: &String, _: &String, _: &String, _: &String) -> Result<(), String> { Err("not used".to_string()) }
        fn supports_subtasks(&self) -> bool { true }
        fn supports_labels(&self) -> bool { self.labels_supported.load(std::sync::atomic::Ordering::SeqCst) }
        fn list_remote_subtasks(&self, _: &String, _: &String, _: &String, _: &Vec<String>) -> Result<Vec<Subtask>, String> { Ok(self.created_subtasks.lock().unwrap().clone()) }
        fn create_remote_subtask(&self, _: &String, _: &String, _: &String, subtask: &Subtask, _: bool) -> Result<String, String> {
            if self.failing_subtask.lock().unwrap().as_deref() == Some(subtask.get_name()) {
                return Err("remote refused".to_string());
            }
            let remote_id = self.next();
            let mut created = subtask.clone();
            created.set_id(remote_id.clone());
            self.created_subtasks.lock().unwrap().push(created);
            Ok(remote_id)
        }
        fn update_remote_subtask(&self, _: &String, _: &String, _: &String, subtask: &Subtask, _: bool) -> Result<(), String> {
            self.subtask_updates.lock().unwrap().push(subtask.get_id().unwrap_or_default());
            if self.reject_subtask_updates.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("transition rejected".to_string());
            }
            Ok(())
        }
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
    fn test_list_filter_applies_the_author_when_created_is_unparseable() {
        let _repo = gittask::testing::TestRepo::new();
        let mut task = gittask::Task::new("Test".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        task.set_property("created", "not-a-number");
        task.set_property("author", "alice");
        let from = parse_date(Some("2020-01-01".to_string()));
        let prop_manager = PropertyManager::new();

        assert!(!task_matches(&task, &None, &None, &None, &from, &None, &Some("bob".to_string()), &prop_manager), "a bad created value must not switch off the author filter");
        assert!(task_matches(&task, &None, &None, &None, &from, &None, &Some("alice".to_string()), &prop_manager));
        assert!(task_matches(&task, &None, &None, &None, &from, &None, &None, &prop_manager), "a bad created value is tolerated by the date filter");
    }

    #[test]
    fn test_until_includes_the_whole_day() {
        let from = parse_date(Some("2024-03-10".to_string()));
        let until = parse_date(Some("2024-03-12".to_string()));

        let noon_on_the_until_day = Local.with_ymd_and_hms(2024, 3, 12, 12, 0, 0).unwrap();
        assert!(created_within(&noon_on_the_until_day, &from, &until), "--until is documented as inclusive");
        let last_second_of_the_until_day = Local.with_ymd_and_hms(2024, 3, 12, 23, 59, 59).unwrap();
        assert!(created_within(&last_second_of_the_until_day, &from, &until));
        let next_day = Local.with_ymd_and_hms(2024, 3, 13, 0, 0, 0).unwrap();
        assert!(!created_within(&next_day, &from, &until));

        let noon_on_the_from_day = Local.with_ymd_and_hms(2024, 3, 10, 12, 0, 0).unwrap();
        assert!(created_within(&noon_on_the_from_day, &from, &until));
        let before_the_from_day = Local.with_ymd_and_hms(2024, 3, 9, 23, 59, 59).unwrap();
        assert!(!created_within(&before_the_from_day, &from, &until));

        assert!(created_within(&next_day, &from, &None), "no upper bound");
        assert!(created_within(&before_the_from_day, &None, &until), "no lower bound");
    }

    #[test]
    fn test_pull_state_open_requests_every_open_issue() {
        let _repo = gittask::testing::TestRepo::new();
        let manager = StatusManager::new();

        assert_eq!(pull_state(None, false, &manager), RemoteTaskState::All);
        assert_eq!(pull_state(Some("o".to_string()), false, &manager), RemoteTaskState::Open("OPEN".to_string(), "OPEN".to_string()));
        assert_eq!(pull_state(Some("c".to_string()), false, &manager), RemoteTaskState::Closed("CLOSED".to_string(), "CLOSED".to_string()));
        assert_eq!(pull_state(None, true, &manager), RemoteTaskState::Open(String::new(), String::new()), "--open must not pin a single status name, so connectors fall back to their open group");
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
    fn test_push_task_stops_when_the_remote_id_collides_with_another_local_task() {
        let _repo = gittask::testing::TestRepo::new();
        let mut other = gittask::Task::new("Other".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        other.set_id("77".to_string());
        other.add_comment(Some("1".to_string()), HashMap::new(), "unrelated comment".to_string());
        gittask::create_task(other).unwrap();
        let mut local = gittask::Task::new("Local".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        local.add_comment(Some("1".to_string()), HashMap::new(), "local comment".to_string());
        let id = gittask::create_task(local).unwrap().get_id().unwrap();
        let fake = FakeConnector::leaked_with_lookup(2, Ok(None));
        let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);
        let statuses = vec!["OPEN".to_string(), "CLOSED".to_string()];

        let result = push_task(&connector, &"u".to_string(), &"r".to_string(), &id, false, false, true, &StatusManager::new(), &statuses, true);

        assert!(!result, "a push that cannot adopt the remote id must fail");
        assert_eq!(fake.created_tasks(), 1);
        let other = gittask::find_task("77").unwrap().unwrap();
        assert_eq!(comment_ids(&other), vec!["1".to_string()], "the unrelated task that owns the colliding id must not be touched");
        assert_eq!(other.get_property("name").unwrap(), "Other");
        let local = gittask::find_task(&id).unwrap().expect("the local task must keep its id");
        assert_eq!(comment_ids(&local), vec!["1".to_string()], "nothing must be synced against the wrong task");
    }

    #[test]
    fn test_update_task_id_refuses_to_overwrite_another_task() {
        let _repo = gittask::testing::TestRepo::new();
        let first = gittask::create_task(gittask::Task::new("First".to_string(), "".to_string(), "OPEN".to_string()).unwrap()).unwrap().get_id().unwrap();
        let second = gittask::create_task(gittask::Task::new("Second".to_string(), "".to_string(), "OPEN".to_string()).unwrap()).unwrap().get_id().unwrap();

        let result = gittask::update_task_id(&first, &second, false);

        assert_eq!(result, Err(format!("Task ID {second} already exists")));
        assert_eq!(gittask::find_task(&first).unwrap().unwrap().get_property("name").unwrap(), "First", "the renamed task must survive");
        assert_eq!(gittask::find_task(&second).unwrap().unwrap().get_property("name").unwrap(), "Second", "the task owning the id must not be overwritten");
    }

    fn comment_ids(task: &Task) -> Vec<String> {
        task.get_comments().clone().unwrap_or_default().iter().filter_map(|c| c.get_id()).collect()
    }

    fn label_names(task: &Task) -> Vec<String> {
        task.get_labels().clone().unwrap_or_default().iter().map(|l| l.get_name()).collect()
    }

    #[test]
    fn test_comments_are_equal_treats_missing_and_empty_alike() {
        assert!(comments_are_equal(&None, &Some(vec![])), "a task created locally has no comment list, a fetched task has an empty one");
        assert!(comments_are_equal(&Some(vec![]), &None));
        assert!(comments_are_equal(&None, &None));
        let comment = Comment::new("1".to_string(), HashMap::new(), "text".to_string());
        assert!(!comments_are_equal(&None, &Some(vec![comment.clone()])));
        assert!(!comments_are_equal(&Some(vec![comment]), &Some(vec![])));
    }

    fn remote_task(id: &str) -> Task {
        let mut task = gittask::Task::new(format!("Remote {id}"), "".to_string(), "OPEN".to_string()).unwrap();
        task.set_id(id.to_string());
        task
    }

    fn pull_targets_for(fake: &'static FakeConnector, repos: &[&str], limit: Option<usize>) -> bool {
        let targets: Vec<RemoteTarget> = repos.iter().map(|repo| {
            let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);
            (connector, "u".to_string(), repo.to_string())
        }).collect();
        let statuses = vec!["OPEN".to_string(), "CLOSED".to_string()];
        pull_targets(targets, limit, &RemoteTaskState::All, &statuses, true, true, true)
    }

    #[test]
    fn test_pull_targets_applies_the_limit_across_projects() {
        let _repo = gittask::testing::TestRepo::new();
        let fake = FakeConnector::leaked_with_remote_tasks(HashMap::from([
            ("a".to_string(), vec![remote_task("1"), remote_task("2"), remote_task("3")]),
            ("b".to_string(), vec![remote_task("4"), remote_task("5"), remote_task("6")]),
            ("c".to_string(), vec![remote_task("7")]),
        ]));

        let result = pull_targets_for(fake, &["a", "b", "c"], Some(4));

        assert!(result);
        assert_eq!(gittask::list_tasks().unwrap().len(), 4, "--limit caps the total across all projects");
        assert_eq!(
            fake.list_limits(),
            vec![("a".to_string(), Some(4)), ("b".to_string(), Some(1))],
            "each project is asked for the remaining budget and projects after it runs out are skipped"
        );
    }

    #[test]
    fn test_pull_targets_without_a_limit_pulls_every_project() {
        let _repo = gittask::testing::TestRepo::new();
        let fake = FakeConnector::leaked_with_remote_tasks(HashMap::from([
            ("a".to_string(), vec![remote_task("1"), remote_task("2")]),
            ("b".to_string(), vec![remote_task("3")]),
        ]));

        let result = pull_targets_for(fake, &["a", "b"], None);

        assert!(result);
        assert_eq!(gittask::list_tasks().unwrap().len(), 3);
        assert_eq!(fake.list_limits(), vec![("a".to_string(), None), ("b".to_string(), None)]);
    }

    #[test]
    fn test_pull_task_skips_a_local_task_without_comments_when_the_remote_has_none() {
        let _repo = gittask::testing::TestRepo::new();
        let local = gittask::create_task(gittask::Task::new("Same".to_string(), "same".to_string(), "OPEN".to_string()).unwrap()).unwrap();
        let id = local.get_id().unwrap();
        let before = tasks_ref_target(&_repo);
        let mut remote = local.clone();
        remote.set_comments(vec![]);
        remote.set_labels(vec![]);
        let fake = FakeConnector::leaked_with_lookup(2, Ok(Some(remote)));
        let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);

        assert!(pull_task(&connector, &"u".to_string(), &"r".to_string(), &id, false, false, true, &vec!["OPEN".to_string(), "CLOSED".to_string()]));

        assert_eq!(tasks_ref_target(&_repo), before, "nothing changed, so nothing must be written");
    }

    #[test]
    fn test_pull_task_updates_the_labels_of_an_existing_task() {
        let _repo = gittask::testing::TestRepo::new();
        let local = gittask::create_task(gittask::Task::new("Same".to_string(), "same".to_string(), "OPEN".to_string()).unwrap()).unwrap();
        let id = local.get_id().unwrap();
        let mut remote = local.clone();
        remote.set_labels(vec![Label::new("bug".to_string(), Some("ff0000".to_string()), None)]);
        let fake = FakeConnector::leaked_with_lookup(2, Ok(Some(remote)));
        let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);
        let statuses = vec!["OPEN".to_string(), "CLOSED".to_string()];

        assert!(pull_task(&connector, &"u".to_string(), &"r".to_string(), &id, false, false, true, &statuses));
        assert_eq!(label_names(&gittask::find_task(&id).unwrap().unwrap()), vec!["bug".to_string()], "a label added on the remote must reach the existing local task");

        let mut remote = local.clone();
        remote.set_labels(vec![]);
        *fake.lookup.lock().unwrap() = Ok(Some(remote));
        assert!(pull_task(&connector, &"u".to_string(), &"r".to_string(), &id, false, false, true, &statuses));
        assert!(label_names(&gittask::find_task(&id).unwrap().unwrap()).is_empty(), "a label removed on the remote must disappear locally");
    }

    #[test]
    fn test_push_task_sends_label_only_changes() {
        let _repo = gittask::testing::TestRepo::new();
        let mut local = gittask::Task::new("Same".to_string(), "same".to_string(), "OPEN".to_string()).unwrap();
        local.set_labels(vec![Label::new("bug".to_string(), None, None)]);
        let local = gittask::create_task(local).unwrap();
        let id = local.get_id().unwrap();
        let mut remote = local.clone();
        remote.set_labels(vec![]);
        let fake = FakeConnector::leaked_with_lookup(2, Ok(Some(remote)));
        let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);
        let statuses = vec!["OPEN".to_string(), "CLOSED".to_string()];

        assert!(push_task(&connector, &"u".to_string(), &"r".to_string(), &id, false, false, true, &StatusManager::new(), &statuses, true));

        let updates = fake.updates();
        assert_eq!(updates.len(), 1, "a label-only change must still update the remote task");
        let sent = updates[0].clone().unwrap_or_default().iter().map(|l| l.get_name()).collect::<Vec<_>>();
        assert_eq!(sent, vec!["bug".to_string()]);
    }

    #[test]
    fn test_push_task_sends_project_only_changes() {
        let _repo = gittask::testing::TestRepo::new();
        let mut local = gittask::Task::new("Same".to_string(), "same".to_string(), "OPEN".to_string()).unwrap();
        local.set_property("project_id", "2");
        let local = gittask::create_task(local).unwrap();
        let id = local.get_id().unwrap();
        let mut remote = local.clone();
        remote.set_property("project_id", "1");
        let fake = FakeConnector::leaked_with_lookup(2, Ok(Some(remote)));
        let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);
        let statuses = vec!["OPEN".to_string(), "CLOSED".to_string()];

        assert!(push_task(&connector, &"u".to_string(), &"r".to_string(), &id, true, true, true, &StatusManager::new(), &statuses, true));

        assert_eq!(fake.updates().len(), 1, "a project-only change must still update the remote task");
    }

    #[test]
    fn test_push_task_ignores_a_local_project_the_connector_does_not_report() {
        let _repo = gittask::testing::TestRepo::new();
        let mut local = gittask::Task::new("Same".to_string(), "same".to_string(), "OPEN".to_string()).unwrap();
        local.set_property("project_id", "2");
        let local = gittask::create_task(local).unwrap();
        let id = local.get_id().unwrap();
        let mut remote = local.clone();
        remote.delete_property("project_id");
        let fake = FakeConnector::leaked_with_lookup(2, Ok(Some(remote)));
        let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);
        let statuses = vec!["OPEN".to_string(), "CLOSED".to_string()];

        assert!(push_task(&connector, &"u".to_string(), &"r".to_string(), &id, true, true, true, &StatusManager::new(), &statuses, true));

        assert!(fake.updates().is_empty(), "a connector that does not manage projects must not see a change");
    }

    #[test]
    fn test_pull_takes_the_remote_project() {
        let _repo = gittask::testing::TestRepo::new();
        let mut local = gittask::Task::new("Same".to_string(), "same".to_string(), "OPEN".to_string()).unwrap();
        local.set_property("project_id", "1");
        let id = gittask::create_task(local).unwrap().get_id().unwrap();
        let mut remote = gittask::find_task(&id).unwrap().unwrap();
        remote.set_property("project_id", "2");
        let fake = FakeConnector::leaked_with_lookup(2, Ok(Some(remote)));
        let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);

        assert!(pull_task(&connector, &"u".to_string(), &"r".to_string(), &id, true, true, true, &vec!["OPEN".to_string(), "CLOSED".to_string()]));

        assert_eq!(gittask::find_task(&id).unwrap().unwrap().get_property("project_id"), Some(&"2".to_string()), "a project change on the remote side must be pulled");
    }

    #[test]
    fn test_pull_keeps_the_local_project_when_the_connector_does_not_report_one() {
        let _repo = gittask::testing::TestRepo::new();
        let mut local = gittask::Task::new("Local".to_string(), "same".to_string(), "OPEN".to_string()).unwrap();
        local.set_property("project_id", "1");
        let id = gittask::create_task(local).unwrap().get_id().unwrap();
        let mut remote = gittask::find_task(&id).unwrap().unwrap();
        remote.set_property("name", "Renamed remotely");
        remote.delete_property("project_id");
        let fake = FakeConnector::leaked_with_lookup(2, Ok(Some(remote)));
        let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);

        assert!(pull_task(&connector, &"u".to_string(), &"r".to_string(), &id, true, true, true, &vec!["OPEN".to_string(), "CLOSED".to_string()]));

        let pulled = gittask::find_task(&id).unwrap().unwrap();
        assert_eq!(pulled.get_property("name"), Some(&"Renamed remotely".to_string()));
        assert_eq!(pulled.get_property("project_id"), Some(&"1".to_string()), "a connector without projects must not wipe the local project");
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
    fn test_sync_local_subtasks_saves_the_mapping_when_only_part_of_the_batch_succeeds() {
        let _repo = gittask::testing::TestRepo::new();
        let mut task = gittask::Task::new("Parent".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        task.add_subtask(None, "A".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        task.add_subtask(None, "B".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        let task = gittask::create_task(task).unwrap();
        let id = task.get_id().unwrap();
        let fake = FakeConnector::leaked_with_lookup(2, Err("not used".to_string()));
        *fake.failing_subtask.lock().unwrap() = Some("B".to_string());
        let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);
        let statuses = vec!["OPEN".to_string(), "CLOSED".to_string()];
        let subtask_ids = |id: &str| gittask::find_task(id).unwrap().unwrap().get_subtasks().clone().unwrap()
            .iter().map(|s| (s.get_name().to_string(), s.get_id().unwrap())).collect::<Vec<_>>();

        let mut success = true;
        sync_local_subtasks(&connector, &"u".to_string(), &"r".to_string(), &id, &task, &StatusManager::new(), &statuses, &mut success);

        assert!(!success, "the failed creation must be reported");
        assert_eq!(subtask_ids(&id), vec![("A".to_string(), "2".to_string()), ("B".to_string(), "3".to_string())], "A must adopt the remote id even though B failed, and B must move off that id");

        *fake.failing_subtask.lock().unwrap() = None;
        let task = gittask::find_task(&id).unwrap().unwrap();
        let mut success = true;
        sync_local_subtasks(&connector, &"u".to_string(), &"r".to_string(), &id, &task, &StatusManager::new(), &statuses, &mut success);

        assert!(success);
        assert_eq!(subtask_ids(&id), vec![("A".to_string(), "2".to_string()), ("B".to_string(), "3".to_string())], "the retry must only create B, never A again");
    }

    #[test]
    fn test_sync_local_subtasks_keeps_the_remote_id_when_the_status_cannot_be_applied() {
        let _repo = gittask::testing::TestRepo::new();
        let mut task = gittask::Task::new("Parent".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        task.add_subtask(None, "Open child".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        task.add_subtask(None, "Closed child".to_string(), "CLOSED".to_string(), HashMap::new()).unwrap();
        let task = gittask::create_task(task).unwrap();
        let id = task.get_id().unwrap();
        let fake = FakeConnector::leaked_with_lookup(10, Err("not used".to_string()));
        fake.reject_subtask_updates.store(true, std::sync::atomic::Ordering::SeqCst);
        let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);
        let mut success = true;

        sync_local_subtasks(&connector, &"u".to_string(), &"r".to_string(), &id, &task, &StatusManager::new(), &vec!["OPEN".to_string(), "CLOSED".to_string()], &mut success);

        assert!(!success, "a rejected status change must fail the push");
        assert_eq!(*fake.subtask_updates.lock().unwrap(), vec!["11".to_string()], "only the subtask whose status differs from the starting one needs a status update, addressed by its remote id");
        let ids = gittask::find_task(&id).unwrap().unwrap().get_subtasks().clone().unwrap()
            .iter().map(|s| s.get_id().unwrap()).collect::<Vec<_>>();
        assert_eq!(ids, vec!["10".to_string(), "11".to_string()], "the remote ids must be recorded even though the status update failed, so a retry does not create duplicates");
    }

    #[test]
    fn test_pull_keeps_local_labels_when_the_connector_cannot_sync_them() {
        let _repo = gittask::testing::TestRepo::new();
        let mut local = gittask::Task::new("Parent".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        local.set_labels(vec![Label::new("urgent".to_string(), None, None)]);
        let id = gittask::create_task(local).unwrap().get_id().unwrap();
        let mut remote = remote_task(&id);
        remote.set_property("name", "Renamed remotely");
        let fake = FakeConnector::leaked_with_lookup(2, Ok(Some(remote)));
        fake.labels_supported.store(false, std::sync::atomic::Ordering::SeqCst);
        let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);

        assert!(pull_task(&connector, &"u".to_string(), &"r".to_string(), &id, true, false, true, &vec!["OPEN".to_string(), "CLOSED".to_string()]));

        let pulled = gittask::find_task(&id).unwrap().unwrap();
        assert_eq!(pulled.get_property("name").unwrap(), "Renamed remotely", "the rest of the task must still be pulled");
        assert_eq!(label_names(&pulled), vec!["urgent".to_string()], "a connector without label support must not wipe local labels");
    }

    #[test]
    fn test_pull_takes_remote_labels_when_the_connector_syncs_them() {
        let _repo = gittask::testing::TestRepo::new();
        let mut local = gittask::Task::new("Parent".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        local.set_labels(vec![Label::new("urgent".to_string(), None, None)]);
        let id = gittask::create_task(local).unwrap().get_id().unwrap();
        let mut remote = remote_task(&id);
        remote.set_property("name", "Parent");
        remote.set_labels(vec![]);
        let fake = FakeConnector::leaked_with_lookup(2, Ok(Some(remote)));
        let connector: Box<&'static dyn RemoteConnector> = Box::new(fake);

        assert!(pull_task(&connector, &"u".to_string(), &"r".to_string(), &id, true, false, true, &vec!["OPEN".to_string(), "CLOSED".to_string()]));

        assert!(label_names(&gittask::find_task(&id).unwrap().unwrap()).is_empty(), "a label removed remotely must be removed locally when the connector syncs labels");
    }

    #[test]
    fn test_import_remote_task_keeps_local_subtask_properties() {
        let _repo = gittask::testing::TestRepo::new();
        let mut local = gittask::Task::new("Parent".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        local.add_subtask(Some("5".to_string()), "Child".to_string(), "OPEN".to_string(), HashMap::from([
            ("estimate".to_string(), "3d".to_string()),
            ("owner".to_string(), "alice".to_string()),
        ])).unwrap();
        local.add_subtask(Some("6".to_string()), "Gone remotely".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        let local = gittask::create_task(local).unwrap();
        let mut remote = remote_task(&local.get_id().unwrap());
        remote.set_property("name", "Parent");
        remote.set_subtasks(vec![
            gittask::Subtask::new(Some("5".to_string()), "Child renamed".to_string(), "CLOSED".to_string(), HashMap::from([
                ("created".to_string(), "123".to_string()),
            ])),
            gittask::Subtask::new(Some("7".to_string()), "New child".to_string(), "OPEN".to_string(), HashMap::new()),
        ]);

        let merged = import_remote_task(remote, true, true, false).unwrap().expect("the task has changed");

        let subtasks = merged.get_subtasks().clone().unwrap();
        let summary: Vec<(String, String, String)> = subtasks.iter().map(|s| (s.get_id().unwrap(), s.get_name().to_string(), s.get_status().to_string())).collect();
        assert_eq!(summary, vec![
            ("5".to_string(), "Child renamed".to_string(), "CLOSED".to_string()),
            ("7".to_string(), "New child".to_string(), "OPEN".to_string()),
        ], "the remote decides which subtasks exist and what they are called");
        assert_eq!(subtasks[0].get_property("estimate").map(String::as_str), Some("3d"), "a property the connector does not manage must survive the pull");
        assert_eq!(subtasks[0].get_property("owner").map(String::as_str), Some("alice"));
        assert_eq!(subtasks[0].get_property("created").map(String::as_str), Some("123"), "properties the remote provides are taken from it");
    }

    #[test]
    fn test_import_remote_task_reports_nothing_to_do_when_only_local_subtask_properties_differ() {
        let _repo = gittask::testing::TestRepo::new();
        let mut local = gittask::Task::new("Parent".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        local.add_subtask(Some("5".to_string()), "Child".to_string(), "OPEN".to_string(), HashMap::from([("owner".to_string(), "alice".to_string())])).unwrap();
        let local = gittask::create_task(local).unwrap();
        let mut remote = remote_task(&local.get_id().unwrap());
        remote.set_property("name", "Parent");
        remote.set_subtasks(vec![gittask::Subtask::new(Some("5".to_string()), "Child".to_string(), "OPEN".to_string(), HashMap::new())]);

        assert!(import_remote_task(remote, true, true, false).unwrap().is_none(), "local-only properties must not look like a remote change");
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
