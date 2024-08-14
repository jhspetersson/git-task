mod util;
mod github;

extern crate gittask;

use std::collections::HashMap;

use chrono::{Local, TimeZone};
use clap::{Parser, Subcommand};
use nu_ansi_term::AnsiString;
use nu_ansi_term::Color::{Cyan, DarkGray, Fixed, Green, Red, Yellow};
use octocrab::models::IssueState::{Open, Closed};
use regex::Regex;

use gittask::{Comment, Task};
use crate::github::{get_github_issue, get_runtime, list_github_issues, update_github_issue_status};
use crate::util::{capitalize, format_datetime, parse_date, read_from_pipe};

#[derive(Parser)]
#[command(arg_required_else_help(true))]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// List all tasks
    List {
        /// filter by status (o - OPEN, i - IN_PROGRESS, c - CLOSED)
        #[arg(short, long)]
        status: Option<String>,
        /// filter by keyword
        #[arg(short, long)]
        keyword: Option<String>,
        /// newer than date, YYYY-MM-DD, inclusive
        #[arg(short, long)]
        from: Option<String>,
        /// older than date, YYYY-MM-DD, inclusive
        #[arg(short, long)]
        until: Option<String>,
        /// comma-separated list of columns
        #[arg(short, long, value_delimiter = ',')]
        columns: Option<Vec<String>>,
    },
    /// Show a task with all properties
    Show {
        /// task ID
        id: String,
    },
    /// Create a new task
    Create {
        name: String,
    },
    /// Update task status
    Status {
        /// task ID
        id: String,
        /// status (o - OPEN, i - IN_PROGRESS, c - CLOSED)
        status: String,
    },
    /// Get a property
    Get {
        /// task ID
        id: String,
        /// property name
        prop_name: String,
    },
    /// Set a property
    Set {
        /// task ID
        id: String,
        /// property name
        prop_name: String,
        /// property value
        value: String,
    },
    /// Add or delete comments
    Comment {
        #[command(subcommand)]
        subcommand: CommentCommand,
    },
    /// Import tasks from a source
    Import {
        /// space separated task IDs
        ids: Option<Vec<String>>,
        /// Input format (only JSON is currently supported)
        #[arg(short, long)]
        format: Option<String>,
    },
    /// Export tasks
    Export {
        /// space separated task IDs
        ids: Option<Vec<String>>,
        /// Output format (only JSON is currently supported)
        #[arg(short, long)]
        format: Option<String>,
        /// Prettify output
        #[arg(short, long)]
        pretty: bool,
    },
    /// Push task status to the remote source (e.g., GitHub)
    Push {
        /// space separated task IDs
        ids: Vec<String>,
    },
    /// Import tasks from a remote source (e.g., GitHub)
    Pull {
        /// space separated task IDs
        ids: Option<Vec<String>>,
        /// Don't import task comments
        #[arg(short, long)]
        no_comments: bool,
    },
    /// Show total task count and count by status
    Stats,
    /// Delete one or several tasks at once
    #[clap(visible_aliases(["del", "remove", "rem"]))]
    Delete {
        /// space separated task IDs
        ids: Vec<String>,
    },
    /// Delete all tasks
    Clear,
}

#[derive(Subcommand)]
enum CommentCommand {
    /// Add a comment
    Add {
        /// task ID
        task_id: String,
        /// comment text
        text: String,
    },
    /// Delete a comment
    #[clap(visible_aliases(["del", "remove", "rem"]))]
    Delete {
        /// task ID
        task_id: String,
        /// comment ID
        comment_id: String,
    },
}

fn main() {
    let args = Args::parse();
    match args.command {
        Some(Command::List { status, keyword, from, until, columns }) => task_list(status, keyword, from, until, columns),
        Some(Command::Show { id }) => task_show(id),
        Some(Command::Create { name }) => task_create(name),
        Some(Command::Status { id, status }) => task_status(id, status),
        Some(Command::Get { id, prop_name }) => task_get(id, prop_name),
        Some(Command::Set { id, prop_name, value }) => task_set(id, prop_name, value),
        Some(Command::Comment { subcommand }) => task_comment(subcommand),
        Some(Command::Import { ids, format }) => task_import(ids, format),
        Some(Command::Export { ids, format, pretty }) => task_export(ids, format, pretty),
        Some(Command::Push { ids }) => task_push(ids),
        Some(Command::Pull { ids, no_comments }) => task_pull(ids, no_comments),
        Some(Command::Stats) => task_stats(),
        Some(Command::Delete { ids }) => task_delete(ids),
        Some(Command::Clear) => task_clear(),
        None => { }
    }
}

fn task_create(name: String) {
    let task = Task::new(name, String::from(""), "OPEN".to_owned());

    match gittask::create_task(task.unwrap()) {
        Ok(id) => println!("Task ID {id} created"),
        Err(e) => eprintln!("ERROR: {e}"),
    };
}

fn task_status(id: String, status: String) {
    let status = get_full_status(&status);
    task_set(id, "status".to_string(), status);
}

fn get_full_status(status: &String) -> String {
    match status.as_str() {
        "o" => String::from("OPEN"),
        "i" => String::from("IN_PROGRESS"),
        "c" => String::from("CLOSED"),
        status => String::from(status)
    }
}

fn task_get(id: String, prop_name: String) {
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

fn task_set(id: String, prop_name: String, value: String) {
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

fn task_comment(subcommand: CommentCommand) {
    match subcommand {
        CommentCommand::Add { task_id, text } => task_comment_add(task_id, text),
        CommentCommand::Delete { task_id, comment_id } => task_comment_delete(task_id, comment_id),
    }
}

fn task_comment_add(task_id: String, text: String) {
    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            task.add_comment(None, HashMap::new(), text);
            match gittask::update_task(task) {
                Ok(_) => println!("Task ID {task_id} updated"),
                Err(e) => eprintln!("ERROR: {e}"),
            }
        },
        Ok(None) => eprintln!("Task ID {task_id} not found"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

fn task_comment_delete(task_id: String, comment_id: String) {
    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            match task.delete_comment(comment_id) {
                Ok(_) => {
                    match gittask::update_task(task) {
                        Ok(_) => println!("Task ID {task_id} updated"),
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

fn task_import(ids: Option<Vec<String>>, format: Option<String>) {
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
                Ok(id) => println!("Task ID {id} imported"),
                Err(e) => eprintln!("ERROR: {e}"),
            }
        }
    } else {
        eprintln!("Can't deserialize input");
    }
}

fn task_pull(ids: Option<Vec<String>>, no_comments: bool) {
    match get_user_repo() {
        Ok((user, repo)) => {
            println!("Importing tasks from {user}/{repo}...");

            if ids.is_some() {
                let runtime = get_runtime();
                for id in ids.unwrap() {
                    match get_github_issue(&runtime, &user, &repo, id.parse().unwrap(), !no_comments) {
                        Some(task) => {
                            match gittask::create_task(task) {
                                Ok(id) => println!("Task ID {id} imported"),
                                Err(e) => eprintln!("ERROR: {e}"),
                            }
                        },
                        None => eprintln!("Task ID {id} not found")
                    }
                }
            } else {
                let tasks = list_github_issues(user.to_string(), repo.to_string(), !no_comments);

                if tasks.is_empty() {
                    println!("No tasks found");
                } else {
                    for task in tasks {
                        match gittask::create_task(task) {
                            Ok(id) => println!("Task ID {id} imported"),
                            Err(e) => eprintln!("ERROR: {e}"),
                        }
                    }
                }
            }
        },
        Err(e) => eprintln!("ERROR: {e}")
    }
}

fn get_user_repo() -> Result<(String, String), String> {
    match gittask::list_remotes() {
        Ok(remotes) => {
            let user_repo = remotes.into_iter().map(|ref remote| {
                match Regex::new("https://github.com/([a-z0-9-]+)/([a-z0-9-]+)\\.?").unwrap().captures(&remote.to_lowercase()) {
                    Some(caps) if caps.len() == 3 => {
                        let user = caps.get(1).unwrap().as_str().to_string();
                        let repo = caps.get(2).unwrap().as_str().to_string();
                        Some((user, repo))
                    },
                    _ => None,
                }
            }).filter(|s| s.is_some()).collect::<Vec<_>>();

            if user_repo.is_empty() {
                return Err("No GitHub remotes".to_string());
            }

            if user_repo.len() > 1 {
                return Err("More than one GitHub remote found".to_owned());
            }

            Ok(user_repo.first().unwrap().to_owned().unwrap())
        },
        Err(e) => Err(e)
    }
}

fn task_export(ids: Option<Vec<String>>, format: Option<String>, pretty: bool) {
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

fn task_push(ids: Vec<String>) {
    if ids.is_empty() {
        eprintln!("Select one or more task IDs");
        return;
    }

    match get_user_repo() {
        Ok((user, repo)) => {
            let runtime = get_runtime();
            for id in ids {
                println!("Sync: task ID {id}");
                if let Ok(Some(local_task)) = gittask::find_task(&id) {
                    println!("Sync: LOCAL task ID {id} found");
                    let remote_task = get_github_issue(&runtime, &user, &repo, id.parse().unwrap(), false);
                    if let Some(remote_task) = remote_task {
                        println!("Sync: REMOTE task ID {id} found");
                        let local_status = local_task.get_property("status").unwrap();
                        let remote_status = remote_task.get_property("status").unwrap();
                        if local_status != remote_status {
                            println!("{}: {} -> {}", id, format_status(remote_status), format_status(local_status));
                            let state = if local_status == "CLOSED" { Closed } else { Open };
                            let result = update_github_issue_status(&runtime, &user, &repo, id.parse().unwrap(), state);
                            if result {
                                println!("Sync: REMOTE task ID {id} has been updated");
                            }
                        } else {
                            eprintln!("Nothing to sync");
                        }
                    } else {
                        eprintln!("Sync: REMOTE task ID {id} NOT found");
                    }
                } else {
                    eprintln!("Sync: LOCAL task ID {id} NOT found");
                }
            }
        },
        Err(e) => eprintln!("ERROR: {e}")
    }
}

fn task_delete(ids: Vec<String>) {
    let ids = ids.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    match gittask::delete_tasks(&ids) {
        Ok(_) => println!("Task(s) {} deleted", ids.join(", ")),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

fn task_clear() {
    match gittask::clear_tasks() {
        Ok(task_count) => println!("{task_count} task(s) deleted"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

fn task_show(id: String) {
    match gittask::find_task(&id) {
        Ok(Some(task)) => print_task(task),
        Ok(None) => eprintln!("Task ID {id} not found"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

fn print_task(task: Task) {
    let id_title = DarkGray.paint("ID");
    println!("{}: {}", id_title, task.get_id().unwrap_or("---".to_owned()));

    let empty_string = String::new();

    let created = task.get_property("created").unwrap_or(&empty_string);
    if !created.is_empty() {
        let created_title = DarkGray.paint("Created");
        println!("{}: {}", created_title, format_datetime(created.parse().unwrap()));
    }

    let author = task.get_property("author").unwrap_or(&empty_string);
    if !author.is_empty() {
        let author_title = DarkGray.paint("Author");
        println!("{}: {}", author_title, format_author(author));
    }

    let name_title = DarkGray.paint("Name");
    println!("{}: {}", name_title, task.get_property("name").unwrap());

    let status_title = DarkGray.paint("Status");
    println!("{}: {}", status_title, format_status(task.get_property("status").unwrap()));

    task.get_all_properties().iter().filter(|entry| {
        entry.0 != "name" && entry.0 != "status" && entry.0 != "description" && entry.0 != "created" && entry.0 != "author"
    }).for_each(|entry| {
        let title = DarkGray.paint(capitalize(entry.0));
        println!("{}: {}", title, entry.1);
    });

    let description = task.get_property("description").unwrap_or(&empty_string);
    if !description.is_empty() {
        let description_title = DarkGray.paint("Description");
        println!("{}: {}", description_title, description);
    }

    if let Some(comments) = task.get_comments() {
        for comment in comments {
            print_comment(comment);
        }
    }
}

fn print_comment(comment: &Comment) {
    println!("{}", DarkGray.paint("---------------"));

    if let Some(id) = comment.get_id() {
        let id_title = DarkGray.paint("Comment ID");
        println!("{}: {}", id_title, id);
    }

    let empty_string = String::new();
    let comment_properties = comment.get_all_properties();

    let created = comment_properties.get("created").unwrap_or(&empty_string);
    if !created.is_empty() {
        let created_title = DarkGray.paint("Created");
        println!("{}: {}", created_title, format_datetime(created.parse().unwrap()));
    }

    let author = comment_properties.get("author").unwrap_or(&empty_string);
    if !author.is_empty() {
        let author_title = DarkGray.paint("Author");
        println!("{}: {}", author_title, format_author(author));
    }

    println!("{}", comment.get_text());
}

fn format_status(status: &str) -> AnsiString {
    match status {
        "OPEN" => Red.paint("OPEN"),
        "IN_PROGRESS" => Yellow.paint("IN_PROGRESS"),
        "CLOSED" => Green.paint("CLOSED"),
        s => s.into()
    }
}

fn format_author(author: &str) -> AnsiString {
    Cyan.paint(author)
}

fn task_list(status: Option<String>, keyword: Option<String>, from: Option<String>, until: Option<String>, columns: Option<Vec<String>>) {
    match gittask::list_tasks() {
        Ok(mut tasks) => {
            tasks.sort_by_key(|task| std::cmp::Reverse(task.get_id().unwrap().parse::<u64>().unwrap_or(0)));

            let from = parse_date(from);
            let until = parse_date(until);

            for task in tasks {
                if status.as_ref().is_some() {
                    let task_status = task.get_property("status").unwrap();
                    if get_full_status(status.as_ref().unwrap()).as_str() != task_status {
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

                print_task_line(task, &columns);
            }
        },
        Err(e) => {
            eprintln!("ERROR: {e}");
        }
    }
}

fn print_task_line(task: Task, columns: &Option<Vec<String>>) {
    let columns = match columns {
        Some(columns) => columns,
        _ => &vec![String::from("id"), String::from("created"), String::from("status"), String::from("name")]
    };

    let empty_string = String::new();

    columns.iter().for_each(|column| {
        let value = if column == "id" { &task.get_id().unwrap() } else { task.get_property(column).unwrap_or(&empty_string) };
        print_column(column, &value);
    });
    println!();
}

fn print_column(column: &String, value: &String) {
    match column.as_str() {
        "id" => print!("{} ", DarkGray.paint(value)),
        "created" => print!("{} ", Fixed(239).paint(format_datetime(value.parse().unwrap_or(0)))),
        "status" => print!("{} ", format_status(value)),
        "author" => print!("{} ", format_author(value)),
        _ => print!("{} ", value),
    }
}

fn task_stats() {
    match gittask::list_tasks() {
        Ok(tasks) => {
            let mut total = 0;
            let mut status_stats = HashMap::<String, i32>::new();
            for task in tasks {
                total += 1;
                match task.get_property("status") {
                    Some(status) => {
                        let current_value = match status_stats.get(status) {
                            Some(&value) => value,
                            _ => 0
                        };
                        let key = status.clone();
                        status_stats.insert(key, current_value + 1);
                    },
                    _ => {}
                }
            }

            println!("Total tasks: {total}");
            println!();
            match status_stats.get("OPEN") {
                Some(count) => println!("{}: {}", format_status("OPEN"), count),
                _ => {}
            };
            match status_stats.get("IN_PROGRESS") {
                Some(count) => println!("{}: {}", format_status("IN_PROGRESS"), count),
                _ => {}
            };
            match status_stats.get("CLOSED") {
                Some(count) => println!("{}: {}", format_status("CLOSED"), count),
                _ => {}
            };
        },
        Err(e) => eprintln!("ERROR: {e}")
    }
}