pub(crate) mod comment;
pub(crate) mod config;
pub(crate) mod label;

use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{Local, TimeZone};
use nu_ansi_term::Color::DarkGray;
use regex::Regex;

use gittask::{Comment, Label, Task};

use crate::connectors::{get_matching_remote_connectors, RemoteConnector, RemoteTaskState};
use crate::property::PropertyManager;
use crate::status::StatusManager;
use crate::util::{capitalize, colorize_string, error_message, get_text_from_editor, parse_date, parse_ids, read_from_pipe, str_to_color, success_message};

pub(crate) fn task_create(name: String, description: Option<String>, no_desc: bool, push: bool, remote: &Option<String>) -> bool {
    let description = match description {
        Some(description) => description,
        None => match no_desc {
            true => String::from(""),
            false => get_text_from_editor(None).unwrap_or_else(|| String::from(""))
        }
    };

    let status_manager = StatusManager::new();
    let task = Task::new(name, description, status_manager.get_starting_status());

    match gittask::create_task(task.unwrap()) {
        Ok(task) => {
            println!("Task ID {} created", task.get_id().unwrap());
            let mut success = false;
            if push {
                match get_user_repo(remote) {
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

pub(crate) fn task_status(ids: String, status: String, push: bool, remote: &Option<String>, no_color: bool) -> bool {
    let status_manager = StatusManager::new();
    let status = status_manager.get_full_status_name(&status);

    task_set(ids, "status".to_string(), status.clone(), push, remote, no_color)
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

pub(crate) fn task_set(ids: String, prop_name: String, value: String, push: bool, remote: &Option<String>, no_color: bool) -> bool {
    let ids = parse_ids(ids);
    match prop_name.as_str() {
        "id" => {
            for id in &ids {
                match gittask::update_task_id(&id, &value) {
                    Ok(_) => {
                        println!("Task ID {id} -> {value} updated");

                        if push {
                            task_push(value.clone(), remote, false, false, no_color);
                        }
                    },
                    Err(e) => {
                        error_message(format!("ERROR: {e}"));
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
                                    task_push(id.to_string(), remote, false, false, no_color);
                                }
                            },
                            Err(e) => {
                                error_message(format!("ERROR: {e}"));
                            },
                        }
                    },
                    Ok(None) => {
                        error_message(format!("Task ID {id} not found"));
                    },
                    Err(e) =>{
                        error_message(format!("ERROR: {e}"));
                    }
                }
            }
        }
    }

    true
}

pub(crate) fn task_replace(ids: String, prop_name: String, search: String, replace: String, regex: bool, push: bool, remote: &Option<String>, no_color: bool) -> bool {
    let ids = parse_ids(ids);
    let regex = match regex {
        true => Some(Box::new(Regex::new(search.as_str()).unwrap())),
        false => None
    };
    for id in ids {
        match gittask::find_task(&id) {
            Ok(Some(mut task)) => {
                if let Some(value) = task.get_property(&prop_name) {
                    let new_value = match regex {
                        Some(ref regex) => regex.replace_all(value.as_str(), search.as_str()).to_string(),
                        None => value.replace(&search, &replace)
                    };
                    task.set_property(&prop_name, &new_value);
                    match gittask::update_task(task) {
                        Ok(_) => {
                            println!("Task ID {id} updated");
                            if push {
                                task_push(id.to_string(), remote, false, false, no_color);
                            }
                        },
                        Err(e) => eprintln!("ERROR: {e}")
                    }
                } else {
                    eprintln!("Task ID {id}: property not found")
                }
            },
            _ => {}
        }
    }

    true
}

pub(crate) fn task_unset(ids: String, prop_name: String) -> bool {
    let ids = parse_ids(ids);
    for id in ids {
        match gittask::find_task(&id) {
            Ok(Some(mut task)) => {
                if task.delete_property(&prop_name) {
                    match gittask::update_task(task) {
                        Ok(_) => println!("Task ID {id} updated"),
                        Err(e) => eprintln!("ERROR: {e}")
                    }
                } else {
                    eprintln!("Task ID {id}: property not found")
                }
            },
            Ok(None) => eprintln!("Task ID {id} not found"),
            Err(e) => eprintln!("ERROR: {e}")
        }
    };

    true
}

pub(crate) fn task_edit(id: String, prop_name: String) -> bool {
    match gittask::find_task(&id) {
        Ok(Some(mut task)) => {
            match prop_name.as_str() {
                "id" => {
                    match get_text_from_editor(Some(&task.get_id().unwrap())) {
                        Some(text) => {
                            task.set_id(text.clone());
                            match gittask::update_task(task) {
                                Ok(_) => {
                                    println!("Task ID {id} -> {text} updated");
                                    if let Err(e) = gittask::delete_tasks(&[&id]) {
                                        eprintln!("ERROR: {e}");
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
            let id = task.get_id().unwrap().to_string();

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
    no_comments: bool,
    no_labels: bool,
) -> bool {
    match get_user_repo(remote) {
        Ok((connector, user, repo)) => {
            println!("Pulling tasks from {user}/{repo}...");

            let ids = ids.map(parse_ids);

            let status_manager = StatusManager::new();
            let task_statuses = vec![
                status_manager.get_starting_status(),
                status_manager.get_final_status(),
            ];

            if ids.is_some() {
                for id in ids.unwrap() {
                    match connector.get_remote_task(&user, &repo, &id, !no_comments, !no_labels, &task_statuses) {
                        Some(task) => {
                            match import_remote_task(task, no_comments) {
                                Ok(Some(id)) => println!("Task ID {id} updated"),
                                Ok(None) => println!("Task ID {id} skipped, nothing to update"),
                                Err(e) => eprintln!("ERROR: {e}"),
                            }
                        },
                        None => eprintln!("Task ID {id} not found")
                    }
                }
                true
            } else {
                let state = match status {
                    Some(s) => {
                        let status = status_manager.get_full_status_name(&s);
                        let is_done = status_manager.get_property(&status, "is_done").unwrap().parse::<bool>().unwrap();
                        if is_done { RemoteTaskState::Closed } else { RemoteTaskState::Open }
                    },
                    None => RemoteTaskState::All
                };

                let tasks = connector.list_remote_tasks(&user, &repo, !no_comments, limit, state, &task_statuses);

                if tasks.is_empty() {
                    success_message("No tasks found".to_string())
                } else {
                    for task in tasks {
                        let task_id = task.get_id().unwrap();
                        match import_remote_task(task, no_comments) {
                            Ok(Some(id)) => println!("Task ID {id} updated"),
                            Ok(None) => println!("Task ID {task_id} skipped, nothing to update"),
                            Err(e) => eprintln!("ERROR: {e}"),
                        }
                    }
                    true
                }
            }
        },
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

fn import_remote_task(remote_task: Task, no_comments: bool) -> Result<Option<String>, String> {
    match gittask::find_task(&remote_task.get_id().unwrap()) {
        Ok(Some(mut local_task)) => {
            if local_task.get_property("name") == remote_task.get_property("name")
                && local_task.get_property("description") == remote_task.get_property("description")
                && local_task.get_property("status") == remote_task.get_property("status")
                && (no_comments || comments_are_equal(local_task.get_comments(), remote_task.get_comments())) {
                Ok(None)
            } else {
                local_task.set_property("name", remote_task.get_property("name").unwrap());
                local_task.set_property("description", remote_task.get_property("description").unwrap());
                local_task.set_property("status", remote_task.get_property("status").unwrap());
                if !no_comments {
                    if let Some(comments) = remote_task.get_comments() {
                        local_task.set_comments(comments.to_vec());
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

fn comments_are_equal(local_comments: &Option<Vec<Comment>>, remote_comments: &Option<Vec<Comment>>) -> bool {
    (local_comments.is_none() && remote_comments.is_none())
    || (local_comments.is_some() && remote_comments.is_some()
        && local_comments.clone().unwrap() == remote_comments.clone().unwrap()
    )
}

fn get_user_repo(remote: &Option<String>) -> Result<(Box<&'static dyn RemoteConnector>, String, String), String> {
    match gittask::list_remotes(remote) {
        Ok(remotes) => {
            let user_repo = get_matching_remote_connectors(remotes);
            if user_repo.is_empty() {
                return Err("No passing remotes".to_string());
            }

            if user_repo.len() > 1 {
                return Err("More than one passing remote found. Please specify with --remote option.".to_owned());
            }

            Ok(user_repo.first().unwrap().clone())
        },
        Err(e) => Err(e)
    }
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
            tasks.sort_by_key(|task| task.get_id().unwrap().parse::<u64>().unwrap_or(0));

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
                    let task_status = task.get_property("status").unwrap();
                    if !statuses.contains(&task_status) {
                        continue;
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

pub(crate) fn task_push(ids: String, remote: &Option<String>, no_comments: bool, no_labels: bool, no_color: bool) -> bool {
    let ids = parse_ids(ids);

    match get_user_repo(remote) {
        Ok((connector, user, repo)) => {
            let status_manager = StatusManager::new();
            let task_statuses = vec![
                status_manager.get_starting_status(),
                status_manager.get_final_status(),
            ];
            let no_color = check_no_color(no_color);
            for id in ids {
                println!("Sync: task ID {id}");
                if let Ok(Some(local_task)) = gittask::find_task(&id) {
                    println!("Sync: LOCAL task ID {id} found");
                    let remote_task = connector.get_remote_task(&user, &repo, &id, !no_comments, !no_labels, &task_statuses);
                    if let Some(remote_task) = remote_task {
                        println!("Sync: REMOTE task ID {id} found");

                        let local_status = local_task.get_property("status").unwrap();
                        let local_name = local_task.get_property("name").unwrap();
                        let local_text = local_task.get_property("description").unwrap();

                        let remote_status = remote_task.get_property("status").unwrap();
                        let remote_name = remote_task.get_property("name").unwrap();
                        let remote_text = remote_task.get_property("description").unwrap();

                        if local_name != remote_name || local_text != remote_text || local_status != remote_status {
                            if local_status != remote_status {
                                println!("{}: {} -> {}", id, status_manager.format_status(remote_status, no_color), status_manager.format_status(local_status, no_color));
                            }
                            let state = if status_manager.is_done(local_status) { RemoteTaskState::Closed } else { RemoteTaskState::Open };

                            match connector.update_remote_task(&user, &repo, &id, local_name, local_text, state) {
                                Ok(_) => {
                                    println!("Sync: REMOTE task ID {id} has been updated");
                                },
                                Err(e) => eprintln!("ERROR: {e}")
                            }
                        } else {
                            if !no_comments {
                                let mut comments_updated = false;
                                let remote_comment_ids: Vec<String> = remote_task.get_comments().as_ref().unwrap_or(&vec![]).iter().map(|comment| comment.get_id().unwrap()).collect();
                                for comment in local_task.get_comments().as_ref().unwrap_or(&vec![]) {
                                    let local_comment_id = comment.get_id().unwrap();
                                    if !remote_comment_ids.contains(&local_comment_id) {
                                        create_remote_comment(&connector, &user, &repo, &id, &comment);
                                        comments_updated = true;
                                    }
                                }
                                if !comments_updated {
                                    println!("Nothing to sync");
                                }
                            } else {
                                println!("Nothing to sync");
                            }
                        }
                    } else {
                        eprintln!("Sync: REMOTE task ID {id} NOT found");

                        match connector.create_remote_task(&user, &repo, &local_task) {
                            Ok(id) => {
                                println!("Sync: Created REMOTE task ID {id}");
                                if local_task.get_id().unwrap() != id {
                                    match gittask::update_task_id(&local_task.get_id().unwrap(), &id) {
                                        Ok(_) => println!("Task ID {} -> {} updated", local_task.get_id().unwrap(), id),
                                        Err(e) => eprintln!("ERROR: {e}"),
                                    }
                                }

                                if !no_comments {
                                    if let Some(comments) = local_task.get_comments() {
                                        if !comments.is_empty() {
                                            for comment in comments {
                                                create_remote_comment(&connector, &user, &repo, &id, &comment);
                                            }
                                        }
                                    }
                                }
                            },
                            Err(e) => eprintln!("ERROR: {e}")
                        }
                    }
                } else {
                    eprintln!("Sync: LOCAL task ID {id} NOT found")
                }
            }
            true
        },
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

fn create_remote_comment(connector: &Box<&'static dyn RemoteConnector>, user: &String, repo: &String, id: &String, comment: &Comment) {
    let local_comment_id = comment.get_id().unwrap();
    match connector.create_remote_comment(user, repo, id, comment) {
        Ok(remote_comment_id) => {
            println!("Created REMOTE comment ID {}", remote_comment_id);
            match gittask::update_comment_id(&id, &local_comment_id, &remote_comment_id) {
                Ok(_) => println!("Comment ID {} -> {} updated", local_comment_id, remote_comment_id),
                Err(e) => eprintln!("ERROR: {e}"),
            }
        },
        Err(e) => eprintln!("ERROR creating REMOTE comment: {}", e)
    }
}

pub(crate) fn task_delete(ids: Option<String>, status: Option<Vec<String>>, push: bool, remote: &Option<String>) -> bool {
    let ids = match status {
        Some(statuses) => {
            match gittask::list_tasks() {
                Ok(tasks) => {
                    let status_manager = StatusManager::new();
                    let statuses = statuses.iter().map(|s| status_manager.get_full_status_name(s)).collect::<Vec<_>>();
                    let ids = tasks.iter().filter(|task| statuses.contains(task.get_property("status").unwrap())).map(|task| task.get_id().unwrap()).collect::<Vec<_>>();
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
        Ok(_) => {
            println!("Task(s) {} deleted", ids.join(", "));
            let mut success = false;
            if push {
                match get_user_repo(remote) {
                    Ok((connector, user, repo)) => {
                        for id in ids {
                            match connector.delete_remote_task(&user, &repo, &id.to_string()) {
                                Ok(_) => println!("Sync: REMOTE task ID {id} has been deleted"),
                                Err(e) => eprintln!("ERROR: {e}")
                            }
                        }
                        success = true;
                    },
                    Err(e) => eprintln!("ERROR: {e}"),
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
    println!("{}: {}", name_title, prop_manager.format_value("name", task.get_property("name").unwrap(), &context, properties, no_color));

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
    println!("{}: {}", status_title, status_manager.format_status(task.get_property("status").unwrap(), no_color));

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

    if let Some(comments) = task.get_comments() {
        for comment in comments {
            print_comment(comment, &prop_manager, no_color);
        }
    }
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
        true => print!("{}", label.get_name()),
        false => {
            let color = str_to_color(label.get_color().as_str(), &None);
            print!("{} ", color.paint(label.get_name()));
        }
    }
}

fn make_comparison(first: &Task, second: &Task, prop: &str, value_type: &str) -> Ordering {
    match prop {
        "id" => {
            let first_value = match first.get_id() {
                Some(value) => value.parse::<u64>().unwrap_or(0),
                _ => 0,
            };
            let second_value = match second.get_id() {
                Some(value) => value.parse::<u64>().unwrap_or(0),
                _ => 0,
            };

            first_value.cmp(&second_value)
        },
        _ => {
            match value_type {
                "integer" => {
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
             from: Option<String>,
             until: Option<String>,
             author: Option<String>,
             columns: Option<Vec<String>>,
             sort: Option<Vec<String>>,
             limit: Option<usize>,
             no_color: bool) -> bool {
    match gittask::list_tasks() {
        Ok(mut tasks) => {
            let prop_manager = PropertyManager::new();
            let sort = match sort {
                Some(sort) => Some(sort),
                None => match gittask::get_config_value("task.list.sort") {
                    Ok(sort) => {
                        Some(sort.split(",").map(|s| s.trim().to_string()).collect())
                    },
                    _ => None
                }
            };
            tasks.sort_by(|a, b| {
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
                    _ => b.get_id().unwrap().parse::<u64>().unwrap_or(0).cmp(&a.get_id().unwrap().parse::<u64>().unwrap_or(0))
                }
            });

            let from = parse_date(from);
            let until = parse_date(until);

            let status_manager = StatusManager::new();
            let statuses = match status {
                Some(statuses) => Some(statuses.iter().map(|s| status_manager.get_full_status_name(s)).collect::<Vec<_>>()),
                None => None
            };
            let no_color = check_no_color(no_color);

            let columns = match columns {
                Some(columns) => Some(columns),
                None => match gittask::get_config_value("task.list.columns") {
                    Ok(list_columns) => {
                        Some(list_columns.split(",").map(|s| s.trim().to_string()).collect())
                    },
                    _ => None
                }
            };

            let mut count = 0;
            for task in tasks {
                if let Some(ref statuses) = statuses {
                    let task_status = task.get_property("status").unwrap();
                    if !statuses.contains(&task_status) {
                        continue;
                    }
                }

                if keyword.as_ref().is_some() {
                    let keyword = keyword.as_ref().unwrap().as_str();
                    let props = task.get_all_properties();
                    if !props.iter().any(|entry| entry.1.contains(keyword)) {
                        continue;
                    }
                }

                if from.is_some() || until.is_some() {
                    let created = task.get_property("created");
                    if let Some(created) = created {
                        let created = Local.timestamp_opt(created.parse().unwrap(), 0).unwrap();

                        if from.is_some() {
                            if created < from.unwrap().earliest().unwrap() {
                                continue;
                            }
                        }

                        if until.is_some() {
                            if created > until.unwrap().latest().unwrap() {
                                continue;
                            }
                        }
                    }
                }

                if author.as_ref().is_some() {
                    if let Some(task_author) = task.get_property("author") {
                        if author.as_ref().unwrap().to_lowercase() != task_author.to_lowercase() {
                            continue;
                        }
                    }
                }

                if let Some(limit) = limit {
                    if count >= limit {
                        break;
                    }
                }

                print_task_line(task, &columns, no_color, &prop_manager, &status_manager);

                count += 1;
            }

            true
        },
        Err(e) => {
            error_message(format!("ERROR: {e}"))
        }
    }
}

fn print_task_line(task: Task, columns: &Option<Vec<String>>, no_color: bool, prop_manager: &PropertyManager, status_manager: &StatusManager) {
    let columns = match columns {
        Some(columns) => columns,
        _ => &vec![String::from("id"), String::from("created"), String::from("status"), String::from("name")]
    };
    let context = extract_task_context(&task);
    let empty_string = String::new();

    columns.iter().for_each(|column| {
        let value = if column == "id" { &task.get_id().unwrap() } else { task.get_property(column).unwrap_or(&empty_string) };
        print_column(column, &value, &context, no_color, prop_manager, status_manager);
    });
    println!();
}

fn print_column(column: &String, value: &String, context: &HashMap<String, String>, no_color: bool, prop_manager: &PropertyManager, status_manager: &StatusManager) {
    match column.as_str() {
        "status" => print!("{} ", status_manager.format_status(value, no_color)),
        column => print!("{} ", prop_manager.format_value(column, value, context, prop_manager.get_properties(), no_color)),
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

fn check_no_color(no_color: bool) -> bool {
    no_color
        || gittask::get_config_value("color.ui").unwrap_or_else(|_| "true".to_string()) == "false"
        || std::env::var("NO_COLOR").unwrap_or_else(|_| "0".to_string()) == "1"
}

fn extract_task_context(task: &Task) -> HashMap<String, String> {
    let mut context = task.get_all_properties().to_owned();
    context.insert("id".to_string(), task.get_id().unwrap());
    context
}