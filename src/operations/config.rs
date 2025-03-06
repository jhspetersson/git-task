use crate::connectors::get_config_options_from_connectors;
use crate::util::{error_message, success_message};

pub(crate) mod status;
pub(crate) mod properties;

pub(crate) fn task_config_get(param: String) -> bool {
    match param.as_str() {
        "task.list.columns" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("id, created, status, name")))),
        "task.list.sort" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("id desc")))),
        "task.status.open" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("OPEN")))),
        "task.status.in_progress" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("IN_PROGRESS")))),
        "task.status.closed" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("CLOSED")))),
        "task.ref" => success_message(format!("{}", gittask::get_ref_path())),
        _ => {
            if get_config_options_from_connectors().contains(&param) {
                match gittask::get_config_value(&param) {
                    Ok(value) => success_message(format!("{}", value)),
                    Err(e) => error_message(format!("ERROR: {e}"))
                }
            } else {
                error_message(format!("Unknown parameter: {param}"))
            }
        }
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
        "task.status.in_progress" => {
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
        _ => {
            if get_config_options_from_connectors().contains(&param) {
                match gittask::set_config_value(&param, &value) {
                    Ok(_) => success_message(format!("{param} has been updated")),
                    Err(e) => error_message(format!("ERROR: {e}"))
                }
            } else {
                error_message(format!("Unknown parameter: {param}"))
            }
        }
    }
}

pub(crate) fn task_config_list() -> bool {
    let from_connectors = get_config_options_from_connectors().join("\n");
    success_message("task.list.columns\ntask.list.sort\ntask.status.open\ntask.status.closed\ntask.ref\n".to_string() + &from_connectors)
}