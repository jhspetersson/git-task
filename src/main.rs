extern crate gittask;

use std::env;
use nu_ansi_term::AnsiString;
use nu_ansi_term::Color::{DarkGray, Green, LightBlue, LightGray, Yellow};
use gittask::Task;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        show_help();
        return;
    }

    let command = args.first().unwrap().to_lowercase();
    let command = command.as_str();

    match command {
        "help" => show_help(),
        "new" => new_task(args),
        "status" => update_status(args),
        "get" => get_prop(args),
        "set" => set_prop(args),
        "del" => delete_task(args),
        "show" => show_task(args),
        "list" => list_tasks(),
        _ => unknown_command(),
    }
}

fn show_help() {
    println!("Available commands:\n\nnew\nstatus\nget\nset\ndel\nshow\nlist\nhelp");
}

fn unknown_command() {
    eprintln!("Unknown command!\n");
    show_help();
}

fn new_task(args: Vec<String>) {
    let name: String = args.into_iter().skip(1).collect::<Vec<String>>().join(" ");
    if name.is_empty() {
        eprintln!("Task name is required!");
        return;
    }

    let task = Task::new(name, String::from(""), "CREATED".to_owned());

    match gittask::create_task(task.unwrap()) {
        Ok(id) => println!("Task id {id} created"),
        Err(e) => eprintln!("ERROR: {e}"),
    };
}

fn update_status(args: Vec<String>) {
    let id = args.get(1);

    if id.is_none() {
        eprintln!("Task id is required!");
        return;
    }
    let id = id.unwrap().to_string();

    let status = args.get(2);

    if status.is_none() {
        eprintln!("Task status is required!");
        return;
    }

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
    task.set_property("status".to_owned(), status.unwrap().to_owned());

    match gittask::update_task(task) {
        Ok(_) => println!("Task id {id} updated"),
        Err(e) => eprintln!("ERROR: {e}"),
    }
}

fn get_prop(args: Vec<String>) {
    let id = args.get(1);

    if id.is_none() {
        eprintln!("Task id is required!");
        return;
    }
    let id = id.unwrap().to_string();

    let prop_name = args.get(2);

    if prop_name.is_none() {
        eprintln!("Task property name is required!");
        return;
    }
    let prop_name = prop_name.unwrap();

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

    match task.get_property(prop_name) {
        Some(value) => println!("{value}"),
        None => eprintln!("Task property {prop_name} not found")
    }
}

fn set_prop(args: Vec<String>) {
    let id = args.get(1);

    if id.is_none() {
        eprintln!("Task id is required!");
        return;
    }
    let id = id.unwrap().to_string();

    let prop_name = args.get(2);

    if prop_name.is_none() {
        eprintln!("Task property name is required!");
        return;
    }
    let prop_name = prop_name.unwrap().to_string();

    let value: String = args.into_iter().skip(3).collect::<Vec<_>>().join(" ");

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

fn delete_task(args: Vec<String>) {
    let id = args.get(1);
    match id {
        Some(id) => {
            match gittask::delete_task(id) {
                Ok(_) => println!("Task id {id} deleted"),
                Err(e) => eprintln!("ERROR: {e}"),
            }
        },
        _ => eprintln!("Task id is required!"),
    }
}

fn show_task(args: Vec<String>) {
    let id = args.get(1);
    match id {
        Some(id) => {
            match gittask::find_task(id) {
                Ok(Some(task)) => print_task(task),
                Ok(None) => eprintln!("Task id {id} not found"),
                Err(e) => eprintln!("ERROR: {e}"),
            }
        },
        _ => eprintln!("Task id is required!"),
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

pub fn capitalize(s: &str) -> String {
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