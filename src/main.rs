extern crate gittask;

use clap::{Parser, Subcommand};
use nu_ansi_term::AnsiString;
use nu_ansi_term::Color::{DarkGray, Green, LightBlue, LightGray, Yellow};
use gittask::Task;

#[derive(Parser)]
#[command(arg_required_else_help(true))]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    List,
    Show {
        id: String,
    },
    Create {
        name: String,
    },
    Status {
        id: String,
        status: String,
    },
    Get {
        id: String,
        prop_name: String,
    },
    Set {
        id: String,
        prop_name: String,
        value: String,
    },
    Delete {
        id: String,
    }
}

fn main() {
    let args = Args::parse();
    match args.command {
        Some(Command::List) => list_tasks(),
        Some(Command::Show { id }) => show_task(id),
        Some(Command::Create { name }) => new_task(name),
        Some(Command::Status { id, status }) => update_status(id, status),
        Some(Command::Get { id, prop_name }) => get_prop(id, prop_name),
        Some(Command::Set { id, prop_name, value }) => set_prop(id, prop_name, value),
        Some(Command::Delete { id }) => delete_task(id),
        None => { }
    }
}

fn new_task(name: String) {
    let task = Task::new(name, String::from(""), "CREATED".to_owned());

    match gittask::create_task(task.unwrap()) {
        Ok(id) => println!("Task id {id} created"),
        Err(e) => eprintln!("ERROR: {e}"),
    };
}

fn update_status(id: String, status: String) {
    let task = gittask::find_task(&id);

    if task.is_err() {
        eprintln!("ERROR: {}", task.err().unwrap());
        return;
    }

    let task = task.unwrap();
    if task.is_none() {
        eprintln!("Task id {id} not found");
        return;
    }

    let mut task = task.unwrap();
    task.set_property("status".to_owned(), status);

    match gittask::update_task(task) {
        Ok(_) => println!("Task id {id} updated"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

fn get_prop(id: String, prop_name: String) {
    let task = gittask::find_task(&id);

    if task.is_err() {
        eprintln!("ERROR: {}", task.err().unwrap());
        return;
    }

    let task = task.unwrap();
    if task.is_none() {
        eprintln!("Task id {id} not found");
        return;
    }

    let task = task.unwrap();

    match task.get_property(&prop_name) {
        Some(value) => println!("{value}"),
        None => eprintln!("Task property {prop_name} not found")
    }
}

fn set_prop(id: String, prop_name: String, value: String) {
    let task = gittask::find_task(&id);

    if task.is_err() {
        eprintln!("ERROR: {}", task.err().unwrap());
        return;
    }

    let task = task.unwrap();
    if task.is_none() {
        eprintln!("Task id {id} not found");
        return;
    }

    let mut task = task.unwrap();
    task.set_property(prop_name, value);

    match gittask::update_task(task) {
        Ok(_) => println!("Task id {id} updated"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

fn delete_task(id: String) {
    match gittask::delete_task(&id) {
        Ok(_) => println!("Task id {id} deleted"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

fn show_task(id: String) {
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

fn list_tasks() {
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