extern crate gittask;

use std::collections::HashMap;
use clap::{Parser, Subcommand};
use nu_ansi_term::AnsiString;
use nu_ansi_term::Color::{DarkGray, Green, LightBlue, LightGray, Yellow};
use octocrab::models::IssueState::Open;
use octocrab::params;
use regex::Regex;
use gittask::{create_task, list_remotes, Task};

#[derive(Parser)]
#[command(arg_required_else_help(true))]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// List all tasks
    List,
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
        /// status
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
        Some(Command::List) => task_list(),
        Some(Command::Show { id }) => task_show(id),
        Some(Command::Create { name }) => task_create(name),
        Some(Command::Status { id, status }) => task_status(id, status),
        Some(Command::Get { id, prop_name }) => task_get(id, prop_name),
        Some(Command::Set { id, prop_name, value }) => task_set(id, prop_name, value),
        Some(Command::Import { source }) => task_import(source),
        Some(Command::Delete { ids }) => task_delete(ids),
        Some(Command::Clear) => task_clear(),
        None => { }
    }
}

fn task_create(name: String) {
    let task = Task::new(name, String::from(""), "CREATED".to_owned());

    match gittask::create_task(task.unwrap()) {
        Ok(id) => println!("Task id {id} created"),
        Err(e) => eprintln!("ERROR: {e}"),
    };
}

fn task_status(id: String, status: String) {
    task_set(id, "status".to_string(), status);
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

fn task_import(_source: Option<String>) {
    match list_remotes() {
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
                        match create_task(task) {
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
            props.insert(String::from("status"), if issue.state == Open { String::from("CREATED") } else { String::from("CLOSED") } );
            props.insert(String::from("description"), issue.body.unwrap_or(String::new()));
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

    let name_title = DarkGray.paint("Name");
    println!("{}: {}", name_title, task.get_property("name").unwrap());

    let status_title = DarkGray.paint("Status");
    println!("{}: {}", status_title, format_status(task.get_property("status").unwrap()));

    task.get_all_properties().iter().filter(|entry| {
        entry.0 != "name" && entry.0 != "status" && entry.0 != "description"
    }).for_each(|entry| {
        let title = DarkGray.paint(capitalize(entry.0));
        println!("{}: {}", title, entry.1);
    });

    let empty_string = String::new();
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
        "CREATED" => LightBlue.paint("CREATED"),
        "IN_PROGRESS" => Yellow.paint("IN_PROGRESS"),
        "DONE" => Green.paint("DONE"),
        "CLOSED" => LightGray.paint("CLOSED"),
        s => s.into()
    }
}

fn task_list() {
    match gittask::list_tasks() {
        Ok(mut tasks) => {
            tasks.sort_by_key(|task| task.get_id().unwrap().parse::<i64>().unwrap_or(0));
            for task in tasks {
                print_task_line(task);
            }
        },
        Err(e) => {
            eprintln!("ERROR: {e}");
        }
    }
}

fn print_task_line(task: Task) {
    println!("{} {} {}",
             task.get_id().unwrap_or(DarkGray.paint("---").to_string()),
             format_status(task.get_property("status").unwrap()),
             task.get_property("name").unwrap()
    );
}