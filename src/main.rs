extern crate gittask;

use std::collections::HashMap;
use std::io::{IsTerminal, Read};
use std::time::{Duration, UNIX_EPOCH};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use clap::{Parser, Subcommand};
use nu_ansi_term::AnsiString;
use nu_ansi_term::Color::{DarkGray, Green, Red, Yellow};
use octocrab::models::IssueState::Open;
use octocrab::params;
use regex::Regex;

use gittask::Task;

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
        /// task id
        id: String,
    },
    /// Create a new task
    Create {
        name: String,
    },
    /// Update task status
    Status {
        /// task id
        id: String,
        /// status (o - OPEN, i - IN_PROGRESS, c - CLOSED)
        status: String,
    },
    /// Get a property
    Get {
        /// task id
        id: String,
        /// property name
        prop_name: String,
    },
    /// Set a property
    Set {
        /// task id
        id: String,
        /// property name
        prop_name: String,
        /// property value
        value: String,
    },
    /// Import tasks from a source
    Import {
        source: Option<String>,
    },
    /// Export tasks
    Export {
        /// space separated task ids
        ids: Option<Vec<String>>,
        /// Output format (only JSON is currently supported)
        #[arg(short, long)]
        format: Option<String>,
        /// Prettify output
        #[arg(short, long)]
        pretty: bool,
    },
    /// Delete one or several tasks at once
    #[clap(visible_aliases(["del", "remove", "rem"]))]
    Delete {
        /// space separated task ids
        ids: Vec<String>,
    },
    /// Delete all tasks
    Clear,
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
        Some(Command::Import { source }) => task_import(source),
        Some(Command::Export { ids, format, pretty }) => task_export(ids, format, pretty),
        Some(Command::Delete { ids }) => task_delete(ids),
        Some(Command::Clear) => task_clear(),
        None => { }
    }
}

fn task_create(name: String) {
    let task = Task::new(name, String::from(""), "OPEN".to_owned());

    match gittask::create_task(task.unwrap()) {
        Ok(id) => println!("Task id {id} created"),
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
        Ok(None) => eprintln!("Task id {id} not found"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

fn task_set(id: String, prop_name: String, value: String) {
    match gittask::find_task(&id) {
        Ok(Some(mut task)) => {
            task.set_property(prop_name, value);

            match gittask::update_task(task) {
                Ok(_) => println!("Task id {id} updated"),
                Err(e) => eprintln!("ERROR: {e}"),
            }
        },
        Ok(None) => eprintln!("Task id {id} not found"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

fn task_import(source: Option<String>) {
    if let Some(input) = read_from_pipe() {
        import_from_input(&input);
    } else if source.is_none() || source.unwrap().to_lowercase() == "github" {
        import_from_remote();
    } else {
        eprintln!("Unknown import source");
    }
}

pub fn read_from_pipe() -> Option<String> {
    let mut buf = String::new();
    match std::io::stdin().is_terminal() {
        false => {
            std::io::stdin().read_to_string(&mut buf).ok()?;
            Some(buf)
        },
        true => None
    }
}

fn import_from_input(input: &String) {
    if let Ok(tasks) = serde_json::from_str::<Vec<HashMap<String, String>>>(input) {
        for mut task in tasks {
            let id = task.get("id").unwrap().to_string();
            task.remove("id");

            match Task::from_properties(id, task) {
                Ok(task) => {
                    match gittask::create_task(task) {
                        Ok(id) => println!("Task id {id} imported"),
                        Err(e) => eprintln!("ERROR: {e}"),
                    }
                },
                Err(e) => eprintln!("ERROR: {e}"),
            }
        }
    } else {
        eprintln!("Can't deserialize input");
    }
}

fn import_from_remote() {
    match gittask::list_remotes() {
        Ok(remotes) => {
            let user_repo = remotes.into_iter().map(|ref remote| {
                match Regex::new("https://github.com/([a-z0-9-]+)/([a-z0-9-]+)\\.git").unwrap().captures(&remote.to_lowercase()) {
                    Some(caps) if caps.len() == 3 => {
                        let user = caps.get(1).unwrap().as_str().to_string();
                        let repo = caps.get(2).unwrap().as_str().to_string();
                        Some((user, repo))
                    },
                    _ => None,
                }
            }).filter(|s| s.is_some()).collect::<Vec<_>>();
            if user_repo.is_empty() {
                eprintln!("No GitHub remotes");
                return;
            }
            if user_repo.len() > 1 {
                eprintln!("More than one GitHub remote found");
                return;
            }
            let user_repo = user_repo.first().unwrap();
            if let Some((user, repo)) = user_repo {
                println!("Importing tasks from {user}/{repo}...");

                let tasks = list_github_issues(user.to_string(), repo.to_string());
                let tasks = tokio::runtime::Runtime::new().unwrap().block_on(tasks);

                if tasks.is_empty() {
                    println!("No tasks found");
                } else {
                    tasks.into_iter().for_each(|task| {
                        match gittask::create_task(task) {
                            Ok(id) => println!("Task id {id} imported"),
                            Err(e) => eprintln!("ERROR: {e}"),
                        };
                    });
                }
            }
        },
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

async fn list_github_issues(user: String, repo: String) -> Vec<Task> {
    octocrab::instance().issues(user, repo)
        .list()
        .state(params::State::All)
        .per_page(100)
        .send()
        .await.unwrap()
        .take_items()
        .into_iter()
        .map(|issue| {
            let mut props = HashMap::new();
            props.insert(String::from("name"), issue.title);
            props.insert(String::from("status"), if issue.state == Open { String::from("OPEN") } else { String::from("CLOSED") } );
            props.insert(String::from("description"), issue.body.unwrap_or(String::new()));
            props.insert(String::from("created"), issue.created_at.timestamp().to_string());
            let id = match Regex::new("/issues/(\\d+)").unwrap().captures(issue.url.path()) {
                Some(caps) if caps.len() == 2 => {
                    caps.get(1).unwrap().as_str().to_string()
                },
                _ => String::new()
            };
            Task::from_properties(id, props).unwrap()
        })
        .collect()
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

                let mut task_props = task.get_all_properties().to_owned();
                task_props.insert("id".to_string(), task.get_id().unwrap());
                result.push(task_props);
            }

            let func = if pretty { serde_json::to_string_pretty } else { serde_json::to_string };

            if let Ok(result) = func(&result) {
                println!("{}", result);
            } else {
                eprintln!("ERROR serializing task list");
            }
        },
        Err(e) => {
            eprintln!("ERROR: {e}");
        }
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
        Ok(None) => eprintln!("Task id {id} not found"),
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

    let name_title = DarkGray.paint("Name");
    println!("{}: {}", name_title, task.get_property("name").unwrap());

    let status_title = DarkGray.paint("Status");
    println!("{}: {}", status_title, format_status(task.get_property("status").unwrap()));

    task.get_all_properties().iter().filter(|entry| {
        entry.0 != "name" && entry.0 != "status" && entry.0 != "description" && entry.0 != "created"
    }).for_each(|entry| {
        let title = DarkGray.paint(capitalize(entry.0));
        println!("{}: {}", title, entry.1);
    });

    let description = task.get_property("description").unwrap_or(&empty_string);
    if !description.is_empty() {
        let description_title = DarkGray.paint("Description");
        println!("{}: {}", description_title, description);
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn format_status(status: &str) -> AnsiString {
    match status {
        "OPEN" => Red.paint("OPEN"),
        "IN_PROGRESS" => Yellow.paint("IN_PROGRESS"),
        "CLOSED" => Green.paint("CLOSED"),
        s => s.into()
    }
}

fn task_list(status: Option<String>, keyword: Option<String>, from: Option<String>, until: Option<String>, columns: Option<Vec<String>>) {
    match gittask::list_tasks() {
        Ok(mut tasks) => {
            tasks.sort_by_key(|task| std::cmp::Reverse(task.get_id().unwrap().parse::<u64>().unwrap_or(0)));

            let from = match from {
                Some(from) => {
                    let naive_date = NaiveDate::parse_from_str(&from, "%Y-%m-%d").unwrap();
                    Some(Local.from_local_datetime(&naive_date.and_hms_opt(0, 0, 0).unwrap()))
                }
                None => None
            };

            let until = match until {
                Some(until) => {
                    let naive_date = NaiveDate::parse_from_str(&until, "%Y-%m-%d").unwrap();
                    Some(Local.from_local_datetime(&naive_date.and_hms_opt(0, 0, 0).unwrap()))
                }
                None => None
            };

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
        "created" => print!("{} ", format_datetime(value.parse().unwrap_or(0))),
        "status" => print!("{} ", format_status(value)),
        _ => print!("{} ", value),
    }
}

fn format_datetime(seconds: u64) -> String {
    if seconds == 0 {
        return String::new();
    }

    let seconds = UNIX_EPOCH + Duration::from_secs(seconds);
    let datetime = DateTime::<Local>::from(seconds);
    datetime.format("%Y-%m-%d %H:%M").to_string()
}