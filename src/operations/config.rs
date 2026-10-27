use std::collections::HashSet;

use crate::connectors::{get_config_options_from_connectors, get_matching_remote_connectors};
use crate::operations::DEFAULT_LIST_COLUMNS;
use crate::status::StatusManager;
use crate::util::{error_message, success_message};

pub(crate) mod status;
pub(crate) mod properties;

pub(crate) fn task_config_get(param: String) -> bool {
    match param.as_str() {
        "task.list.columns" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from(DEFAULT_LIST_COLUMNS)))),
        "task.list.show.headers" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("false")))),
        "task.list.sort" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("id desc")))),
        "task.status.open" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("OPEN")))),
        "task.status.in_progress" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("IN_PROGRESS")))),
        "task.status.closed" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::from("CLOSED")))),
        "task.default.connector" => success_message(format!("{}", gittask::get_config_value(&param).unwrap_or_else(|_| String::new()))),
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

pub(crate) fn task_config_set(param: String, value: String, move_ref: bool, force: bool, rename_tasks: bool) -> bool {
    if rename_tasks && param != "task.jira.projects" {
        return error_message("--rename-tasks only applies to task.jira.projects".to_string());
    }

    if (move_ref || force) && param != "task.ref" {
        return error_message("--move and --force only apply to task.ref".to_string());
    }

    match param.as_str() {
        "task.jira.projects" => {
            match gittask::set_config_value(&param, &value) {
                Ok(_) => {
                    println!("{param} has been updated");
                    if rename_tasks {
                        rename_numeric_tasks_with_jira_project()
                    } else {
                        println!("NOTE: Jira tasks are now identified by their full issue key (e.g. ABC-42). Run with --rename-tasks to prefix existing numeric task and subtask IDs with the default project.");
                        true
                    }
                },
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.jira.fields" => {
            if let Err(e) = crate::connectors::jira::parse_field_mappings(&value) {
                return error_message(format!("ERROR: {e}"));
            }
            match gittask::set_config_value(&param, &value) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.redmine.fields" => {
            if let Err(e) = crate::connectors::redmine::parse_field_mappings(&value) {
                return error_message(format!("ERROR: {e}"));
            }
            match gittask::set_config_value(&param, &value) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.list.columns" => {
            match gittask::set_config_value(&param, &value) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.list.show.headers" => {
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
        "task.status.open" | "task.status.in_progress" | "task.status.closed" => {
            let status = match StatusManager::new().resolve_status(value.trim()) {
                Ok(status) => status,
                Err(e) => return error_message(format!("ERROR: {e}")),
            };
            match gittask::set_config_value(&param, &status) {
                Ok(_) => success_message(format!("{param} has been updated")),
                Err(e) => error_message(format!("ERROR: {e}"))
            }
        },
        "task.default.connector" => {
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

            match gittask::set_ref_path(&value, move_ref, force) {
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

/// Migration to multi-project Jira mode: numeric task IDs belonged to the single
/// (default) project, so they become `PROJECT-<id>` to match the IDs pulled from now on.
fn rename_numeric_tasks_with_jira_project() -> bool {
    let project = match crate::connectors::jira::get_default_project() {
        Some(project) => project,
        None => return error_message("No default Jira project found: set task.jira.project, a project URL in task.jira.url or task.jira.projects".to_string()),
    };

    let tasks = match gittask::list_tasks() {
        Ok(tasks) => tasks,
        Err(e) => return error_message(format!("ERROR: {e}")),
    };

    let existing: HashSet<String> = tasks.iter().filter_map(|task| task.get_id()).collect();
    let mut to_write = vec![];
    let mut old_ids = vec![];
    let mut renamed_subtasks = 0;
    for mut task in tasks {
        let id = task.get_id().unwrap_or_default();
        let mut changed = false;
        if let Some(new_id) = prefixed_jira_id(&project, &id) {
            if existing.contains(&new_id) {
                return error_message(format!("ERROR: Task ID {new_id} already exists, task ID {id} can't be renamed"));
            }
            println!("Task ID {id} -> {new_id} updated");
            old_ids.push(id.clone());
            task.set_id(new_id);
            changed = true;
        }
        if let Some(subtasks) = task.get_subtasks().clone() {
            let renamed = subtasks.into_iter().map(|mut subtask| {
                if let Some(new_id) = subtask.get_id().and_then(|subtask_id| prefixed_jira_id(&project, &subtask_id)) {
                    subtask.set_id(new_id);
                    renamed_subtasks += 1;
                    changed = true;
                }
                subtask
            }).collect::<Vec<_>>();
            let mut seen = HashSet::new();
            if let Some(duplicate) = renamed.iter().filter_map(|subtask| subtask.get_id()).find(|subtask_id| !seen.insert(subtask_id.clone())) {
                return error_message(format!("ERROR: Subtask ID {duplicate} already exists in task ID {id}, its subtasks can't be renamed"));
            }
            task.set_subtasks(renamed);
        }
        if changed {
            to_write.push(task);
        }
    }

    let renamed = old_ids.len();
    if let Err(e) = gittask::update_tasks(to_write) {
        return error_message(format!("ERROR: {e}"));
    }
    if !old_ids.is_empty() {
        if let Err(e) = gittask::delete_tasks(&old_ids.iter().map(String::as_str).collect::<Vec<_>>()) {
            return error_message(format!("ERROR: {e}"));
        }
    }
    println!("{renamed} task(s) and {renamed_subtasks} subtask(s) renamed");
    true
}

fn prefixed_jira_id(project: &str, id: &str) -> Option<String> {
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_digit())).then(|| format!("{project}-{id}"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn subtask_ids(task: &gittask::Task) -> Vec<String> {
        task.get_subtasks().as_ref().unwrap().iter().filter_map(|s| s.get_id()).collect()
    }

    #[test]
    fn test_rename_tasks_prefixes_subtask_ids_along_with_parent_ids() {
        let _repo = gittask::testing::TestRepo::new();
        let mut parent = gittask::Task::new("Parent".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        parent.add_subtask(Some("2".to_string()), "Child".to_string(), "OPEN".to_string(), HashMap::from([("priority".to_string(), "high".to_string())])).unwrap();
        parent.add_subtask(Some("OTHER-9".to_string()), "Foreign child".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        let parent_id = gittask::create_task(parent).unwrap().get_id().unwrap();
        let mut prefixed = gittask::Task::new("Already prefixed".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        prefixed.set_id("TEST-50".to_string());
        prefixed.add_subtask(Some("3".to_string()), "Child".to_string(), "OPEN".to_string(), HashMap::new()).unwrap();
        gittask::create_task(prefixed).unwrap();

        assert!(task_config_set("task.jira.projects".to_string(), "TEST,OTHER".to_string(), false, false, true));

        assert!(gittask::find_task(&parent_id).unwrap().is_none(), "the numeric task must be gone");
        let parent = gittask::find_task(&format!("TEST-{parent_id}")).unwrap().unwrap();
        assert_eq!(subtask_ids(&parent), vec!["TEST-2".to_string(), "OTHER-9".to_string()], "numeric subtask IDs get the prefix, issue keys are kept");
        assert_eq!(parent.get_subtasks().as_ref().unwrap()[0].get_property("priority"), Some(&"high".to_string()), "subtask properties must survive");

        let prefixed = gittask::find_task(&"TEST-50".to_string()).unwrap().unwrap();
        assert_eq!(subtask_ids(&prefixed), vec!["TEST-3".to_string()], "subtasks of an already prefixed task are migrated too");
    }

    #[test]
    fn test_rename_tasks_is_a_single_batch_that_refuses_collisions() {
        let _repo = gittask::testing::TestRepo::new();
        let numeric = gittask::Task::new("Numeric".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        let numeric_id = gittask::create_task(numeric).unwrap().get_id().unwrap();
        let mut colliding = gittask::Task::new("Colliding".to_string(), "".to_string(), "OPEN".to_string()).unwrap();
        colliding.set_id(format!("TEST-{numeric_id}"));
        gittask::create_task(colliding).unwrap();

        assert!(!task_config_set("task.jira.projects".to_string(), "TEST".to_string(), false, false, true));

        assert_eq!(gittask::find_task(&numeric_id).unwrap().unwrap().get_property("name").unwrap(), "Numeric", "nothing is renamed when any ID collides");
        assert_eq!(gittask::find_task(&format!("TEST-{numeric_id}")).unwrap().unwrap().get_property("name").unwrap(), "Colliding");
    }
}

pub(crate) fn task_config_list() -> bool {
    let from_connectors = get_config_options_from_connectors().join("\n");
    success_message("task.list.columns\ntask.list.show.headers\ntask.list.sort\ntask.status.open\ntask.status.in_progress\ntask.status.closed\ntask.default.connector\ntask.ref\n".to_string() + &from_connectors)
}

/// Shows the custom fields in use on the remote project and how they map to task properties.
pub(crate) fn task_config_fields_list(remote: &Option<String>, connector_type: &Option<String>, project: &Option<String>) -> bool {
    let (connector, user, repo) = match crate::operations::get_user_repo(remote, connector_type, project) {
        Ok(target) => target,
        Err(e) => return error_message(format!("ERROR: {e}")),
    };
    match connector.discover_remote_fields(&user, &repo) {
        Ok(fields) if fields.is_empty() => success_message(format!("No custom fields with values found in {repo}")),
        Ok(fields) => {
            let width = fields.iter().map(|f| f.property.len()).max().unwrap_or(0);
            let lines: Vec<String> = fields.iter().map(|f| format!("{:<width$}  {}:{}  ({})", f.property, f.id, f.kind, f.name)).collect();
            success_message(lines.join("\n"))
        },
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

/// Saves the discovered custom field mapping to the connector's config option, so that
/// it no longer depends on the remote field catalog and can be edited by hand.
pub(crate) fn task_config_fields_save(remote: &Option<String>, connector_type: &Option<String>, project: &Option<String>) -> bool {
    let (connector, user, repo) = match crate::operations::get_user_repo(remote, connector_type, project) {
        Ok(target) => target,
        Err(e) => return error_message(format!("ERROR: {e}")),
    };
    let Some(option) = connector.fields_config_option() else {
        return error_message(format!("ERROR: Custom fields are not supported for {}", connector.type_name()));
    };
    match connector.discover_remote_fields(&user, &repo) {
        Ok(fields) if fields.is_empty() => error_message(format!("No custom fields with values found in {repo}, nothing to save")),
        Ok(fields) => {
            let value = fields.iter().map(|f| format!("{}={}:{}", f.property, f.id, f.kind)).collect::<Vec<_>>().join(",");
            match gittask::set_config_value(&option, &value) {
                Ok(_) => success_message(format!("{option} has been set to:\n{value}")),
                Err(e) => error_message(format!("ERROR: {e}")),
            }
        },
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

pub(crate) fn task_config_connectors_list(name: Option<String>) -> bool {
    match gittask::list_remotes(&name) {
        Ok(remotes) => {
            let connectors = get_matching_remote_connectors(remotes, &None, &None, true);
            let mut result = vec![];
            for (connector, user, repo) in connectors {
                result.push(format!("{}: {}/{}", connector.type_name(), user, repo));
            }
            success_message(result.join("\n"))
        },
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}