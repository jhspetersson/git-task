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

    let task = gittask::Task::new(name, String::from(""), "CREATED".to_owned());

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
                Ok(Some(task)) => {
                    println!("{} {} {}",
                             task.get_id().unwrap_or("---".to_owned()),
                             task.get_property(&"status".to_owned()).unwrap(),
                             task.get_property(&"name".to_owned()).unwrap()
                    )
                },
                Ok(None) => eprintln!("Task id {id} not found"),
                Err(e) => eprintln!("ERROR: {e}"),
            }
        },
        _ => eprintln!("Task id is required!"),
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
            eprintln!("ERROR: {e}");
        }
    }
}