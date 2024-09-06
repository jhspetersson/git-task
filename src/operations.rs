use std::collections::HashMap;

use chrono::{Local, TimeZone};
use nu_ansi_term::AnsiString;
use nu_ansi_term::Color::{Cyan, DarkGray, Fixed};
use octocrab::models::IssueState::{Closed, Open};
use octocrab::params::State;
use gittask::{Comment, Task};
use crate::github::{create_github_comment, create_github_issue, delete_github_comment, delete_github_issue, get_github_issue, get_runtime, list_github_issues, list_github_origins, update_github_issue_status};
use crate::status;
use crate::status::StatusManager;
use crate::util::{capitalize, colorize_string, format_datetime, get_text_from_editor, parse_date, read_from_pipe};

pub(crate) fn task_create(name: String, description: Option<String>, no_desc: bool, push: bool, remote: Option<String>) {
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

            if push {
                match get_user_repo(remote) {
                    Ok((user, repo)) => {
                        let runtime = get_runtime();
                        match create_github_issue(&runtime, &user, &repo, &task) {
                            Ok(id) => {
                                println!("Sync: Created REMOTE task ID {id}");
                                match gittask::update_task_id(&task.get_id().unwrap(), &id) {
                                    Ok(_) => println!("Task ID {} -> {} updated", task.get_id().unwrap(), id),
                                    Err(e) => eprintln!("ERROR: {e}")
                                }
                            },
                            Err(e) => eprintln!("ERROR: {e}")
                        }
                    },
                    Err(e) => eprintln!("ERROR: {e}")
                }
            }
        },
        Err(e) => eprintln!("ERROR: {e}")
    };
}

pub(crate) fn task_status(id: String, status: String) {
    let status_manager = StatusManager::new();
    let status = status_manager.get_full_status_name(&status);
    task_set(id, "status".to_string(), status);
}

pub(crate) fn task_get(id: String, prop_name: String) {
    match gittask::find_task(&id) {
        Ok(Some(task)) => {
            match task.get_property(&prop_name) {
                Some(value) => println!("{value}"),
                None => eprintln!("Task property {prop_name} not found")
            }
        },
        Ok(None) => eprintln!("Task ID {id} not found"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

pub(crate) fn task_set(id: String, prop_name: String, value: String) {
    match prop_name.as_str() {
        "id" => {
            match gittask::update_task_id(&id, &value) {
                Ok(_) => {
                    println!("Task ID {id} -> {value} updated");
                    if let Err(e) = gittask::delete_tasks(&[&id]) {
                        eprintln!("ERROR: {e}");
                    }
                },
                Err(e) => eprintln!("ERROR: {e}"),
            }
        },
        _ => {
            match gittask::find_task(&id) {
                Ok(Some(mut task)) => {
                    task.set_property(prop_name, value);

                    match gittask::update_task(task) {
                        Ok(_) => println!("Task ID {id} updated"),
                        Err(e) => eprintln!("ERROR: {e}"),
                    }
                },
                Ok(None) => eprintln!("Task ID {id} not found"),
                Err(e) => eprintln!("ERROR: {e}"),
            }
        }
    }
}

pub(crate) fn task_edit(id: String, prop_name: String) {
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
                                },
                                Err(e) => eprintln!("ERROR: {e}"),
                            }
                        },
                        None => eprintln!("Editing failed"),
                    }
                },
                _ => {
                    match task.get_property(&prop_name) {
                        Some(value) => {
                            match get_text_from_editor(Some(value)) {
                                Some(text) => {
                                    task.set_property(prop_name, text);
                                    match gittask::update_task(task) {
                                        Ok(_) => println!("Task ID {id} updated"),
                                        Err(e) => eprintln!("ERROR: {e}"),
                                    }
                                },
                                None => eprintln!("Editing failed"),
                            }
                        },
                        None => eprintln!("Task property {prop_name} not found")
                    }
                }
            }
        },
        Ok(None) => eprintln!("Task ID {id} not found"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

pub(crate) fn task_comment_add(task_id: String, text: String, push: bool, remote: Option<String>) {
    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            let comment = task.add_comment(None, HashMap::new(), text);
            match gittask::update_task(task) {
                Ok(_) => {
                    println!("Task ID {task_id} updated");

                    if push {
                        match get_user_repo(remote) {
                            Ok((user, repo)) => {
                                let runtime = get_runtime();
                                match create_github_comment(&runtime, &user, &repo, &task_id, &comment) {
                                    Ok(remote_comment_id) => {
                                        println!("Created REMOTE comment ID {}", remote_comment_id);
                                        match gittask::update_comment_id(&task_id, &comment.get_id().unwrap(), &remote_comment_id) {
                                            Ok(_) => println!("Comment ID {} -> {} updated", &comment.get_id().unwrap(), remote_comment_id),
                                            Err(e) => eprintln!("ERROR: {e}"),
                                        }
                                    },
                                    Err(e) => eprintln!("ERROR creating REMOTE comment: {}", e)
                                }
                            },
                            Err(e) => eprintln!("ERROR: {e}"),
                        }
                    }
                },
                Err(e) => eprintln!("ERROR: {e}"),
            }
        },
        Ok(None) => eprintln!("Task ID {task_id} not found"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

pub(crate) fn task_comment_delete(task_id: String, comment_id: String, push: bool, remote: Option<String>) {
    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            match task.delete_comment(&comment_id) {
                Ok(_) => {
                    match gittask::update_task(task) {
                        Ok(_) => {
                            println!("Task ID {task_id} updated");

                            if push {
                                match get_user_repo(remote) {
                                    Ok((user, repo)) => {
                                        let comment_id = comment_id.clone().parse().unwrap();
                                        match delete_github_comment(&user, &repo, comment_id) {
                                            Ok(_) => println!("Sync: REMOTE comment ID {comment_id} has been deleted"),
                                            Err(e) => eprintln!("ERROR: {e}")
                                        }
                                    },
                                    Err(e) => eprintln!("ERROR: {e}"),
                                }
                            }
                        },
                        Err(e) => eprintln!("ERROR: {e}"),
                    }
                },
                Err(e) => eprintln!("ERROR: {e}"),
            }
        },
        Ok(None) => eprintln!("Task ID {task_id} not found"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

pub(crate) fn task_import(ids: Option<Vec<String>>, format: Option<String>) {
    if let Some(format) = format {
        if format.to_lowercase() != "json" {
            eprintln!("Only JSON format is supported");
            return;
        }
    }

    if let Some(input) = read_from_pipe() {
        import_from_input(ids, &input);
    } else {
        eprintln!("Can't read from pipe");
    }
}

fn import_from_input(ids: Option<Vec<String>>, input: &String) {
    if let Ok(tasks) = serde_json::from_str::<Vec<Task>>(input) {
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
    } else {
        eprintln!("Can't deserialize input");
    }
}

pub(crate) fn task_pull(ids: Option<Vec<String>>, limit: Option<usize>, status: Option<String>, remote: Option<String>, no_comments: bool) {
    match get_user_repo(remote) {
        Ok((user, repo)) => {
            println!("Importing tasks from {user}/{repo}...");

            if ids.is_some() {
                let runtime = get_runtime();
                for id in ids.unwrap() {
                    match get_github_issue(&runtime, &user, &repo, id.parse().unwrap(), !no_comments) {
                        Some(task) => {
                            match gittask::create_task(task) {
                                Ok(_) => println!("Task ID {id} imported"),
                                Err(e) => eprintln!("ERROR: {e}"),
                            }
                        },
                        None => eprintln!("Task ID {id} not found")
                    }
                }
            } else {
                let status_manager = StatusManager::new();
                let state = match status {
                    Some(s) => {
                        let status = status_manager.get_full_status_name(&s);
                        let is_done = status_manager.get_property(&status, "is_done").unwrap().parse::<bool>().unwrap();
                        if is_done { State::Closed } else { State::Open }
                    },
                    None => State::All
                };

                let tasks = list_github_issues(user.to_string(), repo.to_string(), !no_comments, limit, state);

                if tasks.is_empty() {
                    println!("No tasks found");
                } else {
                    for task in tasks {
                        match gittask::create_task(task) {
                            Ok(task) => println!("Task ID {} imported", task.get_id().unwrap()),
                            Err(e) => eprintln!("ERROR: {e}"),
                        }
                    }
                }
            }
        },
        Err(e) => eprintln!("ERROR: {e}")
    }
}

fn get_user_repo(remote: Option<String>) -> Result<(String, String), String> {
    match gittask::list_remotes(remote) {
        Ok(remotes) => {
            match list_github_origins(remotes) {
                Ok(user_repo) => {
                    if user_repo.is_empty() {
                        return Err("No GitHub remotes".to_string());
                    }

                    if user_repo.len() > 1 {
                        return Err("More than one GitHub remote found. Please specify with --remote option.".to_owned());
                    }

                    Ok(user_repo.first().unwrap().clone())
                },
                Err(e) => Err(e)
            }
        },
        Err(e) => Err(e)
    }
}

pub(crate) fn task_export(ids: Option<Vec<String>>, format: Option<String>, pretty: bool) {
    if let Some(format) = format {
        if format.to_lowercase() != "json" {
            eprintln!("Only JSON format is supported");
            return;
        }
    }

    match gittask::list_tasks() {
        Ok(mut tasks) => {
            let mut result = vec![];
            tasks.sort_by_key(|task| task.get_id().unwrap().parse::<u64>().unwrap_or(0));

            for task in tasks {
                if let Some(ids) = &ids {
                    if !ids.contains(&task.get_id().unwrap()) {
                        continue;
                    }
                }

                result.push(task);
            }

            let func = if pretty { serde_json::to_string_pretty } else { serde_json::to_string };

            if let Ok(result) = func(&result) {
                println!("{}", result);
            } else {
                eprintln!("ERROR serializing task list");
            }
        },
        Err(e) => eprintln!("ERROR: {e}")
    }
}

pub(crate) fn task_push(ids: Vec<String>, remote: Option<String>, no_comments: bool, no_color: bool) {
    if ids.is_empty() {
        eprintln!("Select one or more task IDs");
        return;
    }

    match get_user_repo(remote) {
        Ok((user, repo)) => {
            let runtime = get_runtime();
            let status_manager = StatusManager::new();
            for id in ids {
                println!("Sync: task ID {id}");
                if let Ok(Some(local_task)) = gittask::find_task(&id) {
                    println!("Sync: LOCAL task ID {id} found");
                    let remote_task = get_github_issue(&runtime, &user, &repo, id.parse().unwrap(), !no_comments);
                    if let Some(remote_task) = remote_task {
                        println!("Sync: REMOTE task ID {id} found");
                        let local_status = local_task.get_property("status").unwrap();
                        let remote_status = remote_task.get_property("status").unwrap();
                        if local_status != remote_status {
                            println!("{}: {} -> {}", id, status_manager.format_status(remote_status, no_color), status_manager.format_status(local_status, no_color));
                            let state = if local_status == "CLOSED" { Closed } else { Open };
                            let result = update_github_issue_status(&runtime, &user, &repo, id.parse().unwrap(), state);
                            if result {
                                println!("Sync: REMOTE task ID {id} has been updated");

                                if !no_comments {
                                    let remote_comment_ids: Vec<String> = remote_task.get_comments().as_ref().unwrap_or(&vec![]).iter().map(|comment| comment.get_id().unwrap()).collect();
                                    for comment in local_task.get_comments().as_ref().unwrap_or(&vec![]) {
                                        let local_comment_id = comment.get_id().unwrap();
                                        if !remote_comment_ids.contains(&local_comment_id) {
                                            create_remote_comment(&runtime, &user, &repo, &id, &comment);
                                        }
                                    }
                                }
                            }
                        } else {
                            eprintln!("Nothing to sync");
                        }
                    } else {
                        eprintln!("Sync: REMOTE task ID {id} NOT found");
                        match create_github_issue(&runtime, &user, &repo, &local_task) {
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
                                                create_remote_comment(&runtime, &user, &repo, &id, &comment);
                                            }
                                        }
                                    }
                                }
                            },
                            Err(e) => eprintln!("ERROR: {e}")
                        }
                    }
                } else {
                    eprintln!("Sync: LOCAL task ID {id} NOT found");
                }
            }
        },
        Err(e) => eprintln!("ERROR: {e}")
    }
}

fn create_remote_comment(runtime: &tokio::runtime::Runtime, user: &String, repo: &String, id: &String, comment: &Comment) {
    let local_comment_id = comment.get_id().unwrap();
    match create_github_comment(&runtime, user, repo, id, comment) {
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

pub(crate) fn task_delete(ids: Vec<String>, push: bool, remote: Option<String>) {
    let ids = ids.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    match gittask::delete_tasks(&ids) {
        Ok(_) => println!("Task(s) {} deleted", ids.join(", ")),
        Err(e) => eprintln!("ERROR: {e}"),
    }

    if push {
        match get_user_repo(remote) {
            Ok((user, repo)) => {
                for id in ids {
                    match delete_github_issue(&user, &repo, id.parse().unwrap()) {
                        Ok(_) => println!("Sync: REMOTE task ID {id} has been deleted"),
                        Err(e) => eprintln!("ERROR: {e}")
                    }
                }
            },
            Err(e) => eprintln!("ERROR: {e}"),
        }
    }
}

pub(crate) fn task_clear() {
    match gittask::clear_tasks() {
        Ok(task_count) => println!("{task_count} task(s) deleted"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

pub(crate) fn task_show(id: String, no_color: bool) {
    match gittask::find_task(&id) {
        Ok(Some(task)) => print_task(task, no_color),
        Ok(None) => eprintln!("Task ID {id} not found"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

fn print_task(task: Task, no_color: bool) {
    let id_title = colorize_string("ID", DarkGray, no_color);
    println!("{}: {}", id_title, task.get_id().unwrap_or("---".to_owned()));

    let empty_string = String::new();

    let created = task.get_property("created").unwrap_or(&empty_string);
    if !created.is_empty() {
        let created_title = colorize_string("Created", DarkGray, no_color);
        println!("{}: {}", created_title, format_datetime(created.parse().unwrap()));
    }

    let author = task.get_property("author").unwrap_or(&empty_string);
    if !author.is_empty() {
        let author_title = colorize_string("Author", DarkGray, no_color);
        println!("{}: {}", author_title, format_author(author, no_color));
    }

    let name_title = colorize_string("Name", DarkGray, no_color);
    println!("{}: {}", name_title, task.get_property("name").unwrap());

    let status_manager = StatusManager::new();
    let status_title = colorize_string("Status", DarkGray, no_color);
    println!("{}: {}", status_title, status_manager.format_status(task.get_property("status").unwrap(), no_color));

    task.get_all_properties().iter().filter(|entry| {
        entry.0 != "name" && entry.0 != "status" && entry.0 != "description" && entry.0 != "created" && entry.0 != "author"
    }).for_each(|entry| {
        let title = colorize_string(&capitalize(entry.0), DarkGray, no_color);
        println!("{}: {}", title, entry.1);
    });

    let description = task.get_property("description").unwrap_or(&empty_string);
    if !description.is_empty() {
        let description_title = colorize_string("Description", DarkGray, no_color);
        println!("{}: {}", description_title, description);
    }

    if let Some(comments) = task.get_comments() {
        for comment in comments {
            print_comment(comment, no_color);
        }
    }
}

fn print_comment(comment: &Comment, no_color: bool) {
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
        println!("{}: {}", created_title, format_datetime(created.parse().unwrap()));
    }

    let author = comment_properties.get("author").unwrap_or(&empty_string);
    if !author.is_empty() {
        let author_title = colorize_string("Author", DarkGray, no_color);
        println!("{}: {}", author_title, format_author(author, no_color));
    }

    println!("{}", comment.get_text());
}

fn format_author(author: &str, no_color: bool) -> AnsiString {
    if no_color { author.into() } else { Cyan.paint(author) }
}

pub(crate) fn task_list(status: Option<String>,
             keyword: Option<String>,
             from: Option<String>,
             until: Option<String>,
             author: Option<String>,
             columns: Option<Vec<String>>,
             sort: Option<Vec<String>>,
             limit: Option<usize>,
             no_color: bool) {
    match gittask::list_tasks() {
        Ok(mut tasks) => {
            tasks.sort_by(|a, b| {
                match &sort {
                    Some(sort) if !sort.is_empty() => {
                        let mut ordering = None;
                        for s in sort {
                            let mut s = s.trim();
                            let comparison;
                            if s.to_lowercase().ends_with(" desc") {
                                s = s[..(s.len() - "desc".len())].trim();
                                comparison = b.get_property(&s).unwrap_or(&String::new()).to_lowercase().cmp(&a.get_property(&s).unwrap_or(&String::new()).to_lowercase());
                            } else {
                                if s.to_lowercase().ends_with(" asc") {
                                    s = s[..(s.len() - "asc".len())].trim();
                                }
                                comparison = a.get_property(&s).unwrap_or(&String::new()).to_lowercase().cmp(&b.get_property(&s).unwrap_or(&String::new()).to_lowercase());
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

            let mut count = 0;
            for task in tasks {
                if status.as_ref().is_some() {
                    let task_status = task.get_property("status").unwrap();
                    if status_manager.get_full_status_name(status.as_ref().unwrap()).as_str() != task_status {
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

                print_task_line(task, &columns, no_color, &status_manager);

                count += 1;
            }
        },
        Err(e) => {
            eprintln!("ERROR: {e}");
        }
    }
}

fn print_task_line(task: Task, columns: &Option<Vec<String>>, no_color: bool, status_manager: &StatusManager) {
    let columns = match columns {
        Some(columns) => columns,
        _ => &vec![String::from("id"), String::from("created"), String::from("status"), String::from("name")]
    };

    let empty_string = String::new();

    columns.iter().for_each(|column| {
        let value = if column == "id" { &task.get_id().unwrap() } else { task.get_property(column).unwrap_or(&empty_string) };
        print_column(column, &value, no_color, status_manager);
    });
    println!();
}

fn print_column(column: &String, value: &String, no_color: bool, status_manager: &StatusManager) {
    match no_color {
        false => {
            match column.as_str() {
                "id" => print!("{} ", DarkGray.paint(value)),
                "created" => print!("{} ", Fixed(239).paint(format_datetime(value.parse().unwrap_or(0)))),
                "status" => print!("{} ", status_manager.format_status(value, no_color)),
                "author" => print!("{} ", format_author(value, no_color)),
                _ => print!("{} ", value),
            }
        },
        true => {
            match column.as_str() {
                "created" => print!("{} ", format_datetime(value.parse().unwrap_or(0))),
                _ => print!("{} ", value),
            }
        }
    }
}

pub(crate) fn task_stats(no_color: bool) {
    match gittask::list_tasks() {
        Ok(tasks) => {
            let mut total = 0;
            let mut status_stats = HashMap::<String, i32>::new();
            let mut author_stats = HashMap::<String, i32>::new();

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

                let mut author_stats = author_stats.iter().collect::<Vec<_>>();
                author_stats.sort_by(|a, b| b.1.cmp(a.1));

                for author in author_stats.iter().take(10) {
                    println!("{}: {}", format_author(&author.0, no_color), author.1);
                }
            }
        },
        Err(e) => eprintln!("ERROR: {e}")
    }
}

pub(crate) fn task_config_get(param: String) {
    match param.as_str() {
        "task.ref" => println!("{}", gittask::get_ref_path()),
        _ => eprintln!("Unknown parameter: {}", param)
    }
}

pub(crate) fn task_config_set(param: String, value: String, move_ref: bool) {
    match param.as_str() {
        "task.ref" => {
            match gittask::set_ref_path(&value, move_ref) {
                Ok(_) => println!("{param} has been updated"),
                Err(e) => eprintln!("ERROR: {e}")
            }
        },
        _ => eprintln!("Unknown parameter: {}", param)
    }
}

pub(crate) fn task_config_list() {
    println!("task.ref");
}

pub(crate) fn task_config_status_add(name: String, shortcut: String, color: String, is_done: Option<bool>) {
    let mut status_manager = StatusManager::new();
    match status_manager.add_status(name, shortcut, color, is_done.unwrap_or(false)) {
        Ok(_) => println!("Status has been added"),
        Err(e) => eprintln!("ERROR: {e}")
    }
}

pub(crate) fn task_config_status_delete(name: String, force: bool) {
    let mut status_manager = StatusManager::new();
    let name = status_manager.get_full_status_name(&name);

    if !force {
        if let Ok(tasks) = gittask::list_tasks() {
            let task_exist = tasks.iter().any(|task| task.get_property("status").unwrap() == name.as_str());
            if task_exist {
                eprintln!("Can't delete a status, some tasks still have it. Use --force option to override.");
                return;
            }
        }
    }

    match status_manager.delete_status(name) {
        Ok(_) => println!("Status has been deleted"),
        Err(e) => eprintln!("ERROR: {e}")
    }
}

pub(crate) fn task_config_status_get(name: String, param: String) {
    let status_manager = StatusManager::new();
    match status_manager.get_property(&name, &param) {
        Some(value) => println!("{}", value),
        None => eprintln!("Unknown status {} or property: {}", name, param)
    }
}

pub(crate) fn task_config_status_set(name: String, param: String, value: String) {
    let mut status_manager = StatusManager::new();
    match status_manager.set_property(&name, &param, &value) {
        Ok(prev_value) => {
            println!("{name} {param} has been updated");

            if param.as_str() == "name" && prev_value.is_some() {
                let prev_status = prev_value.unwrap();
                match gittask::list_tasks() {
                    Ok(tasks) => {
                        for mut task in tasks {
                            if task.get_property("status").unwrap() == prev_status.as_str() {
                                task.set_property("status".to_string(), value.clone());
                                if let Err(e) = gittask::update_task(task) {
                                    eprintln!("ERROR: {e}");
                                }
                            }
                        }
                    },
                    Err(e) => eprintln!("ERROR: {e}")
                }
            }
        },
        Err(e) => eprintln!("ERROR: {e}")
    }
}

pub(crate) fn task_config_status_list() {
    let status_manager = StatusManager::new();
    println!("Name\tShortcut\tColor\tIs DONE");
    status_manager.get_statuses().iter().for_each(|status| {
        println!("{}\t{}\t{}\t{}", status.get_name(), status.get_shortcut(), status.get_color(), status.is_done());
    })
}

pub(crate) fn task_config_status_import() {
    if let Some(input) = read_from_pipe() {
        match status::parse_statuses(input) {
            Ok(statuses) => {
                let mut status_manager = StatusManager::new();
                match status_manager.set_statuses(statuses) {
                    Ok(_) => println!("Import successful"),
                    Err(e) => eprintln!("ERROR: {e}")
                }
            },
            Err(e) => eprintln!("{e}")
        }
    } else {
        eprintln!("Can't read from pipe");
    }
}

pub(crate) fn task_config_status_export(pretty: bool) {
    let status_manager = StatusManager::new();
    let func = if pretty { serde_json::to_string_pretty } else { serde_json::to_string };

    if let Ok(result) = func(&status_manager.get_statuses()) {
        println!("{}", result);
    } else {
        eprintln!("ERROR serializing status list");
    }
}

pub(crate) fn task_config_status_reset() {
    let mut status_manager = StatusManager::new();
    match status_manager.set_defaults() {
        Ok(_) => println!("Statuses have been reset"),
        Err(e) => eprintln!("ERROR: {e}")
    }
}