use crate::property::PropertyManager;
use crate::status;
use crate::status::StatusManager;
use crate::util::{error_message, read_from_pipe, success_message};

pub(crate) fn task_config_get(param: String) -> bool {
    match param.as_str() {
        "task.list.columns" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("id, created, status, name")))),
        "task.list.sort" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("id desc")))),
        "task.ref" => success_message(format!("{}", gittask::get_ref_path())),
        _ => error_message(format!("Unknown parameter: {param}"))
    }
}

pub(crate) fn task_config_set(param: String, value: String, move_ref: bool) -> bool {
    match param.as_str() {
        "task.list.columns" => {
            match gittask::set_config_value(&param, &value) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.list.sort" => {
            match gittask::set_config_value(&param, &value) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.status.open" => {
            match gittask::set_config_value(&param, &value) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.status.closed" => {
            match gittask::set_config_value(&param, &value) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.ref" => {
            let value = match value {
                value if !value.contains('/') => "refs/heads/".to_string() + value.as_str(),
                value if value.chars().filter(|c| *c == '/').count() == 1 && !value.starts_with('/') && !value.ends_with('/') => "refs/".to_string() + value.as_str(),
                value => value,
            };

            match gittask::set_ref_path(&value, move_ref) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        _ => error_message(format!("Unknown parameter: {param}"))
    }
}

pub(crate) fn task_config_list() -> bool {
    success_message("task.list.columns\ntask.list.sort\ntask.status.open\ntask.status.closed\ntask.ref".to_string())
}

pub(crate) fn task_config_status_add(name: String, shortcut: String, color: String, is_done: Option<bool>) -> bool {
    let mut status_manager = StatusManager::new();
    match status_manager.add_status(name, shortcut, color, is_done.unwrap_or(false)) {
        Ok(_) => success_message("Status has been added".to_string()),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_config_status_delete(name: String, force: bool) -> bool {
    let mut status_manager = StatusManager::new();
    let name = status_manager.get_full_status_name(&name);

    if !force {
        if let Ok(tasks) = gittask::list_tasks() {
            let task_exists = tasks.iter().any(|task| task.get_property("status").unwrap() == name.as_str());
            if task_exists {
                return error_message("Can't delete a status, some tasks still have it. Use --force option to override.".to_string());
            }
        }
    }

    match status_manager.delete_status(name) {
        Ok(_) => success_message("Status has been deleted".to_string()),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_config_status_get(name: String, param: String) -> bool {
    let status_manager = StatusManager::new();
    match status_manager.get_property(&name, &param) {
        Some(value) => success_message(value),
        None => error_message(format!("Unknown status {name} or property: {param}"))
    }
}

pub(crate) fn task_config_status_set(name: String, param: String, value: String) -> bool {
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

            true
        },
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_config_status_list() -> bool {
    let status_manager = StatusManager::new();
    println!("Name\tShortcut\tColor\tStyle\tIs DONE");
    status_manager.get_statuses().iter().for_each(|status| {
        println!("{}\t{}\t{}\t{}\t{}", status.get_name(), status.get_shortcut(), status.get_color(), status.get_style().unwrap_or_else(|| ""), status.is_done());
    });
    true
}

pub(crate) fn task_config_status_import() -> bool {
    if let Some(input) = read_from_pipe() {
        match status::parse_statuses(input) {
            Ok(statuses) => {
                let mut status_manager = StatusManager::new();
                match status_manager.set_statuses(statuses) {
                    Ok(_) => success_message("Import successful".to_string()),
                    Err(e) => error_message(format!("ERROR: {e}"))
                }
            },
            Err(e) => error_message(e)
        }
    } else {
        error_message("Can't read from pipe".to_string())
    }
}

pub(crate) fn task_config_status_export(pretty: bool) -> bool {
    let status_manager = StatusManager::new();
    let func = if pretty { serde_json::to_string_pretty } else { serde_json::to_string };

    if let Ok(result) = func(&status_manager.get_statuses()) {
        success_message(result)
    } else {
        error_message("ERROR serializing status list".to_string())
    }
}

pub(crate) fn task_config_status_reset() -> bool {
    let mut status_manager = StatusManager::new();
    match status_manager.set_defaults() {
        Ok(_) => success_message("Statuses have been reset".to_string()),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_config_properties_add(name: String, value_type: String, color: String, enum_values: Option<Vec<String>>) -> bool {
    let mut prop_manager = PropertyManager::new();
    match prop_manager.add_property(name.clone(), value_type, color, None, enum_values) {
        Ok(_) => success_message(format!("Property {name} has been added")),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_config_properties_delete(name: String, force: bool) -> bool {
    let mut prop_manager = PropertyManager::new();

    if !force {
        if let Ok(tasks) = gittask::list_tasks() {
            let task_exists = tasks.iter().any(|task| task.has_property(&name));
            if task_exists {
                return error_message("Can't delete a property, some tasks still have it. Use --force option to override.".to_string());
            }
        }
    }

    match prop_manager.delete_property(&name) {
        Ok(_) => success_message(format!("Property {name} has been deleted")),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_config_properties_get(name: String, param: String) -> bool {
    let prop_manager = PropertyManager::new();
    match prop_manager.get_parameter(&name, &param) {
        Some(value) => success_message(value),
        None => error_message(format!("Unknown property {name} or parameter: {param}"))
    }
}

pub(crate) fn task_config_properties_set(name: String, param: String, value: String) -> bool {
    let mut prop_manager = PropertyManager::new();
    match prop_manager.set_parameter(&name, &param, &value) {
        Ok(_) => {
            println!("{name} {param} has been updated");

            if param.as_str() == "name" {
                match gittask::list_tasks() {
                    Ok(tasks) => {
                        for mut task in tasks {
                            if task.has_property(&name) {
                                let task_prop_value = task.get_property(&name).unwrap();
                                task.set_property(value.clone(), task_prop_value.to_string());
                                task.delete_property(&name);
                                if let Err(e) = gittask::update_task(task) {
                                    eprintln!("ERROR: {e}");
                                }
                            }
                        }
                    },
                    Err(e) => eprintln!("ERROR: {e}")
                }
            }

            true
        },
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_config_properties_list() -> bool {
    let prop_manager = PropertyManager::new();
    println!("Name\tValue type\tColor\tStyle\tEnum values");
    prop_manager.get_properties().iter().for_each(|property| {
        let enums = match property.get_enum_values() {
            Some(enum_values) => {
                enum_values.iter()
                    .map(|saved_enum| saved_enum.get_name().to_string() + "," + saved_enum.get_color() + (if saved_enum.get_style().is_some() { "," } else { "" }) + saved_enum.get_style().unwrap_or_else(|| ""))
                    .collect::<Vec<_>>()
                    .join(";")
            },
            None => String::new()
        };
        println!("{}\t{}\t{}\t{}\t{}", property.get_name(), property.get_value_type(), property.get_color(), property.get_style().unwrap_or_else(|| ""), enums);
    });
    true
}

pub(crate) fn task_config_properties_import() -> bool {
    if let Some(input) = read_from_pipe() {
        match PropertyManager::parse_properties(input) {
            Ok(statuses) => {
                let mut prop_manager = PropertyManager::new();
                match prop_manager.set_properties(statuses) {
                    Ok(_) => success_message("Import successful".to_string()),
                    Err(e) => error_message(format!("ERROR: {e}"))
                }
            },
            Err(e) => error_message(e)
        }
    } else {
        error_message("Can't read from pipe".to_string())
    }
}

pub(crate) fn task_config_properties_export(pretty: bool) -> bool {
    let prop_manager = PropertyManager::new();
    let func = if pretty { serde_json::to_string_pretty } else { serde_json::to_string };

    if let Ok(result) = func(&prop_manager.get_properties()) {
        success_message(result)
    } else {
        error_message("ERROR serializing property list".to_string())
    }
}

pub(crate) fn task_config_properties_reset() -> bool {
    let mut prop_manager = PropertyManager::new();
    match prop_manager.set_defaults() {
        Ok(_) => success_message("Properties have been reset".to_string()),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_config_properties_enum_list(name: String) -> bool {
    let prop_manager = PropertyManager::new();
    let property = prop_manager.get_properties().iter().find(|saved_prop| saved_prop.get_name() == name);
    match property {
        Some(property) => {
            match property.get_enum_values() {
                Some(enum_values) => {
                    for enum_value in enum_values {
                        println!("{} {} {}", enum_value.get_name(), enum_value.get_color(), enum_value.get_style().unwrap_or_else(|| ""));
                    }
                    true
                },
                None => error_message("Property has no enum values".to_string())
            }
        },
        None => error_message("Property not found".to_string())
    }
}

pub(crate) fn task_config_properties_enum_add(name: String, enum_value_name: String, enum_value_color: String, enum_value_style: Option<String>) -> bool {
    let mut prop_manager = PropertyManager::new();
    match prop_manager.add_enum_property(name, enum_value_name, enum_value_color, enum_value_style) {
        Ok(_) => success_message("Property enum has been added".to_string()),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_config_properties_enum_get(property: String, enum_value_name: String, parameter: String) -> bool {
    let prop_manager = PropertyManager::new();
    match prop_manager.get_enum_parameter(property, enum_value_name, parameter) {
        Ok(s) => success_message(s),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_config_properties_enum_set(name: String, enum_value_name: String, enum_value_color: String, enum_value_style: Option<String>) -> bool {
    let mut prop_manager = PropertyManager::new();
    match prop_manager.set_enum_property(name, enum_value_name, enum_value_color, enum_value_style) {
        Ok(_) => success_message("Property enum has been updated".to_string()),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_config_properties_enum_delete(name: String, enum_value_name: String) -> bool {
    let mut prop_manager = PropertyManager::new();
    match prop_manager.delete_enum_property(name, enum_value_name) {
        Ok(_) => success_message("Property enum has been deleted".to_string()),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}