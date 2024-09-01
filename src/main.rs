mod util;
mod github;

extern crate gittask;

use std::collections::HashMap;

use chrono::{Local, TimeZone};
use clap::{Parser, Subcommand};
use nu_ansi_term::AnsiString;
use nu_ansi_term::Color::{Cyan, DarkGray, Fixed, Green, Red, Yellow};
use octocrab::models::IssueState::{Open, Closed};

use gittask::{Comment, Task};
use crate::github::{create_github_issue, delete_github_issue, get_github_issue, get_runtime, list_github_issues, list_github_origins, update_github_issue_status};
use crate::util::{capitalize, colorize_string, format_datetime, parse_date, read_from_pipe};

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
        /// filter by author
        #[arg(long)]
        author: Option<String>,
        /// comma-separated list of columns
        #[arg(short, long, value_delimiter = ',')]
        columns: Option<Vec<String>>,
        /// soring by one or more task properties, e.g. --sort "author, created desc"
        #[arg(long, value_delimiter = ',')]
        sort: Option<Vec<String>>,
        /// limit displayed task count
        #[arg(short, long)]
        limit: Option<usize>,
        /// disable colors
        #[arg(long)]
        no_color: bool,
    },
    /// Show a task with all properties
    Show {
        /// task ID
        id: String,
        /// disable colors
        #[arg(long)]
        no_color: bool,
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
        /// Use this remote if there are several of them
        #[arg(short, long)]
        remote: Option<String>,
        /// disable colors
        #[arg(long)]
        no_color: bool,
    },
    /// Import tasks from a remote source (e.g., GitHub)
    Pull {
        /// space separated task IDs
        ids: Option<Vec<String>>,
        /// Use this remote if there are several of them
        #[arg(short, long)]
        remote: Option<String>,
        /// Don't import task comments
        #[arg(short, long)]
        no_comments: bool,
    },
    /// Show total task count and count by status
    Stats {
        /// disable colors
        #[arg(long)]
        no_color: bool,
    },
    /// Delete one or several tasks at once
    #[clap(visible_aliases(["del", "remove", "rem"]))]
    Delete {
        /// space separated task IDs
        ids: Vec<String>,
        /// Also delete task from the remote source (e.g., GitHub)
        #[arg(short, long)]
        push: bool,
        /// Use this remote if there are several of them
        #[arg(short, long)]
        remote: Option<String>,
    },
    /// Delete all tasks
    Clear,
    /// Set configuration parameters
    Config {
        #[command(subcommand)]
        subcommand: ConfigCommand,
    },
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

#[derive(Subcommand)]
enum ConfigCommand {
    /// Get configuration parameter
    Get {
        /// parameter name
        param: String,
    },
    /// Set configuration parameter
    Set {
        /// parameter name
        param: String,
        /// parameter value
        value: String,
    },
    /// List configuration parameters
    List,
}

fn main() {
    let _ = enable_ansi_support::enable_ansi_support();
    let args = Args::parse();
    match args.command {
        Some(Command::List { status, keyword, from, until, author, columns, sort, limit, no_color }) => task_list(status, keyword, from, until, author, columns, sort, limit, no_color),
        Some(Command::Show { id, no_color }) => task_show(id, no_color),
        Some(Command::Create { name }) => task_create(name),
        Some(Command::Status { id, status }) => task_status(id, status),
        Some(Command::Get { id, prop_name }) => task_get(id, prop_name),
        Some(Command::Set { id, prop_name, value }) => task_set(id, prop_name, value),
        Some(Command::Comment { subcommand }) => task_comment(subcommand),
        Some(Command::Import { ids, format }) => task_import(ids, format),
        Some(Command::Export { ids, format, pretty }) => task_export(ids, format, pretty),
        Some(Command::Push { ids, remote, no_color }) => task_push(ids, remote, no_color),
        Some(Command::Pull { ids, remote, no_comments }) => task_pull(ids, remote, no_comments),
        Some(Command::Stats { no_color }) => task_stats(no_color),
        Some(Command::Delete { ids, push, remote }) => task_delete(ids, push, remote),
        Some(Command::Clear) => task_clear(),
        Some(Command::Config { subcommand }) => task_config(subcommand),
        None => { }
    }
}

fn task_create(name: String) {
    let task = Task::new(name, String::from(""), "OPEN".to_owned());

    match gittask::create_task(task.unwrap()) {
        Ok(task) => println!("Task ID {} created", task.get_id().unwrap()),
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
    match prop_name.as_str() {
        "id" => {
            match gittask::update_task_id(&id, &value) {
                Ok(_) => println!("Task ID {id} -> {value} updated"),
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
                Ok(_) => println!("Task ID {id} imported"),
                Err(e) => eprintln!("ERROR: {e}"),
            }
        }
    } else {
        eprintln!("Can't deserialize input");
    }
}

fn task_pull(ids: Option<Vec<String>>, remote: Option<String>, no_comments: bool) {
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
                let tasks = list_github_issues(user.to_string(), repo.to_string(), !no_comments);

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

fn task_push(ids: Vec<String>, remote: Option<String>, no_color: bool) {
    if ids.is_empty() {
        eprintln!("Select one or more task IDs");
        return;
    }

    match get_user_repo(remote) {
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
                            println!("{}: {} -> {}", id, format_status(remote_status, no_color), format_status(local_status, no_color));
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
                        match create_github_issue(&runtime, &user, &repo, &local_task) {
                            Ok(id) => {
                                println!("Sync: Created REMOTE task ID {id}");
                                if local_task.get_id().unwrap() != id {
                                    match gittask::update_task_id(&local_task.get_id().unwrap(), &id) {
                                        Ok(_) => println!("Task ID {} -> {} updated", local_task.get_id().unwrap(), id),
                                        Err(e) => eprintln!("ERROR: {e}"),
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

fn task_delete(ids: Vec<String>, push: bool, remote: Option<String>) {
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

fn task_clear() {
    match gittask::clear_tasks() {
        Ok(task_count) => println!("{task_count} task(s) deleted"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

fn task_show(id: String, no_color: bool) {
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

    let status_title = colorize_string("Status", DarkGray, no_color);
    println!("{}: {}", status_title, format_status(task.get_property("status").unwrap(), no_color));

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

fn format_status(status: &str, no_color: bool) -> AnsiString {
    match no_color {
        false => {
            match status {
                "OPEN" => Red.paint("OPEN"),
                "IN_PROGRESS" => Yellow.paint("IN_PROGRESS"),
                "CLOSED" => Green.paint("CLOSED"),
                s => s.into()
            }
        },
        true => status.into()
    }
}

fn format_author(author: &str, no_color: bool) -> AnsiString {
    if no_color { author.into() } else { Cyan.paint(author) }
}

fn task_list(status: Option<String>,
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
                    _ => a.get_id().unwrap().parse::<u64>().unwrap_or(0).cmp(&b.get_id().unwrap().parse::<u64>().unwrap_or(0))
                }
            });

            let from = parse_date(from);
            let until = parse_date(until);

            let mut count = 0;
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

                print_task_line(task, &columns, no_color);

                count += 1;
            }
        },
        Err(e) => {
            eprintln!("ERROR: {e}");
        }
    }
}

fn print_task_line(task: Task, columns: &Option<Vec<String>>, no_color: bool) {
    let columns = match columns {
        Some(columns) => columns,
        _ => &vec![String::from("id"), String::from("created"), String::from("status"), String::from("name")]
    };

    let empty_string = String::new();

    columns.iter().for_each(|column| {
        let value = if column == "id" { &task.get_id().unwrap() } else { task.get_property(column).unwrap_or(&empty_string) };
        print_column(column, &value, no_color);
    });
    println!();
}

fn print_column(column: &String, value: &String, no_color: bool) {
    match no_color {
        false => {
            match column.as_str() {
                "id" => print!("{} ", DarkGray.paint(value)),
                "created" => print!("{} ", Fixed(239).paint(format_datetime(value.parse().unwrap_or(0)))),
                "status" => print!("{} ", format_status(value, no_color)),
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

fn task_stats(no_color: bool) {
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

            if let Some(count) = status_stats.get("OPEN") {
                println!("{}: {}", format_status("OPEN", no_color), count);
            }

            if let Some(count) = status_stats.get("IN_PROGRESS") {
                println!("{}: {}", format_status("IN_PROGRESS", no_color), count);
            }

            if let Some(count) = status_stats.get("CLOSED") {
                println!("{}: {}", format_status("CLOSED", no_color), count);
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

fn task_config(subcommand: ConfigCommand) {
    match subcommand {
        ConfigCommand::Get { param } => task_config_get(param),
        ConfigCommand::Set { param, value } => task_config_set(param, value),
        ConfigCommand::List => task_config_list(),
    }
}

fn task_config_get(param: String) {
    match param.as_str() {
        "task.ref" => {
            match gittask::get_ref_path() {
                Ok(ref_path) => println!("{ref_path}"),
                Err(e) => eprintln!("ERROR: {e}")
            }
        },
        _ => eprintln!("Unknown parameter: {}", param)
    }
}

fn task_config_set(param: String, value: String) {
    match param.as_str() {
        "task.ref" => {
            match gittask::set_ref_path(&value) {
                Ok(_) => println!("{param} has been updated"),
                Err(e) => eprintln!("ERROR: {e}")
            }
        },
        _ => eprintln!("Unknown parameter: {}", param)
    }
}

fn task_config_list() {
    println!("task.ref");
}