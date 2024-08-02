extern crate gittask;

use std::env;

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
        "show" => show_task(args),
        "list" => list_tasks(),
        _ => unknown_command(),
    }
}

fn show_help() {
    println!("Available commands:\n\nnew\nshow\nlist\nhelp");
}

fn unknown_command() {
    println!("Unknown command!\n");
    show_help();
}

fn new_task(args: Vec<String>) {
    let name: String = args.into_iter().skip(1).collect::<Vec<String>>().join(" ");
    if name.is_empty() {
        println!("Task name is required!");
        return;
    }

    let task = gittask::Task::new(name, String::from(""), "CREATED".to_owned());

    match gittask::create_task(task.unwrap()) {
        Ok(id) => println!("Task id {id} created"),
        Err(e) => println!("ERROR: {e}"),
    };
}

fn show_task(args: Vec<String>) {
    let id = args.get(1);
    match id {
        Some(id) => {
            match gittask::find_task(id) {
                Ok(Some(task)) => {
                    println!("{} {} {}",
                             task.get_id().unwrap_or("---".to_owned()),
                             task.get_property(&"status".to_owned()).unwrap(),
                             task.get_property(&"name".to_owned()).unwrap()
                    )
                },
                Ok(None) => println!("Task id {id} not found"),
                Err(e) => println!("ERROR: {e}"),
            }
        },
        _ => println!("Task id is required!"),
    }
}

fn list_tasks() {
    match gittask::list_tasks() {
        Ok(tasks) => {
            for task in tasks {
                println!("{} {} {}",
                         task.get_id().unwrap_or("---".to_owned()),
                         task.get_property(&"status".to_owned()).unwrap(),
                         task.get_property(&"name".to_owned()).unwrap()
                );
            }
        },
        Err(e) => {
            println!("ERROR: {e}");
        }
    }
}