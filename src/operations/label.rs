use crate::operations::get_user_repo;
use crate::util::{error_message};

pub(crate) fn task_label_add(task_id: String, name: String, color: Option<String>, description: Option<String>, push: bool, remote: &Option<String>) -> bool {
    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            let label = task.add_label(name.clone(), description.clone(), color.clone());
            match gittask::update_task(task) {
                Ok(_) => {
                    println!("Task ID {task_id} updated");
                    let mut success = false;
                    if push {
                        match get_user_repo(remote) {
                            Ok((connector, user, repo)) => {
                                match connector.create_remote_label(&user, &repo, &task_id, &label) {
                                    Ok(_) => {
                                        println!("Added REMOTE label {}", label.get_name());
                                        success = true;
                                    },
                                    Err(e) => eprintln!("ERROR adding REMOTE label: {e}")
                                }
                            },
                            Err(e) => eprintln!("ERROR: {e}"),
                        }
                    }
                    success
                },
                Err(e) => error_message(format!("ERROR: {e}")),
            }
        },
        Ok(None) => error_message(format!("Task ID {task_id} not found")),
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}

pub(crate) fn task_label_edit(task_id: String, name: String, color: Option<String>, description: Option<String>, push: bool, remote: &Option<String>) -> bool {
    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            let mut labels = task.get_labels().clone();

            if labels.is_none() || labels.as_ref().unwrap().is_empty() {
                return error_message("Task has no labels".to_string());
            }

            let label = labels.as_mut().unwrap().iter_mut().find(|label| label.get_name() == name);

            if label.is_none() {
                return error_message("Label not found".to_string());
            }

            let label = label.unwrap();
            let label_sync = label.clone();

            if let Some(color) = color {
                label.set_color(color);
            }
            if let Some(description) = description {
                label.set_description(description);
            }

            task.set_labels(labels.unwrap());

            match gittask::update_task(task) {
                Ok(_) => {
                    println!("Task ID {task_id} updated");
                    let mut success = false;
                    if push {
                        match get_user_repo(remote) {
                            Ok((connector, user, repo)) => {
                                match connector.update_remote_label(&user, &repo, &task_id, &label_sync) {
                                    Ok(_) => {
                                        println!("Sync: REMOTE label '{name}' has been updated");
                                        success = true;
                                    },
                                    Err(e) => eprintln!("ERROR: {e}")
                                }
                            },
                            Err(e) => eprintln!("ERROR: {e}"),
                        }
                    }
                    success
                },
                Err(e) => error_message(format!("ERROR: {e}")),
            }
        },
        Ok(None) => error_message(format!("Task ID {task_id} not found")),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_label_delete(task_id: String, name: String, push: bool, remote: &Option<String>) -> bool {
    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            match task.delete_label(&name) {
                Ok(_) => {
                    match gittask::update_task(task) {
                        Ok(_) => {
                            println!("Task ID {task_id} updated");
                            let mut success = false;
                            if push {
                                match get_user_repo(remote) {
                                    Ok((connector, user, repo)) => {
                                        match connector.delete_remote_label(&user, &repo, &task_id, &name) {
                                            Ok(_) => {
                                                println!("Sync: REMOTE label '{name}' has been deleted");
                                                success = true;
                                            },
                                            Err(e) => eprintln!("ERROR: {e}")
                                        }
                                    },
                                    Err(e) => eprintln!("ERROR: {e}"),
                                }
                            }
                            success
                        },
                        Err(e) => error_message(format!("ERROR: {e}")),
                    }
                },
                Err(e) => error_message(format!("ERROR: {e}")),
            }
        },
        Ok(None) => error_message(format!("Task ID {task_id} not found")),
        Err(e) => error_message(format!("ERROR: {e}")),
    }
}