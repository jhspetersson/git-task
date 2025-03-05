use std::collections::HashMap;
use crate::operations::get_user_repo;
use crate::util::{error_message, get_text_from_editor};

pub(crate) fn task_comment_add(
    task_id: String,
    text: Option<String>,
    push: bool,
    remote: &Option<String>,
    connector_type: &Option<String>,
) -> bool {
    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            let text = text.or_else(|| get_text_from_editor(None));
            if text.is_none() {
                return error_message("No text specified".to_string());
            }
            let text = text.unwrap();

            let comment = task.add_comment(None, HashMap::new(), text);
            match gittask::update_task(task) {
                Ok(_) => {
                    println!("Task ID {task_id} updated");
                    let mut success = false;
                    if push {
                        match get_user_repo(remote, connector_type) {
                            Ok((connector, user, repo)) => {
                                match connector.create_remote_comment(&user, &repo, &task_id, &comment) {
                                    Ok(remote_comment_id) => {
                                        println!("Created REMOTE comment ID {}", remote_comment_id);
                                        match gittask::update_comment_id(&task_id, &comment.get_id().unwrap(), &remote_comment_id) {
                                            Ok(_) => {
                                                println!("Comment ID {} -> {} updated", &comment.get_id().unwrap(), remote_comment_id);
                                                success = true;
                                            },
                                            Err(e) => eprintln!("ERROR: {e}"),
                                        }
                                    },
                                    Err(e) => eprintln!("ERROR creating REMOTE comment: {e}")
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

pub(crate) fn task_comment_edit(
    task_id: String,
    comment_id: String,
    push: bool,
    remote: &Option<String>,
    connector_type: &Option<String>,
) -> bool {
    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            let mut comments = task.get_comments().clone();
            if comments.is_none() || comments.as_ref().unwrap().is_empty() {
                return error_message("Task has no comments".to_string());
            }
            let comment = comments.as_mut().unwrap().iter_mut().find(|comment| comment.get_id().unwrap() == comment_id);
            if comment.is_none() {
                return error_message("Comment not found".to_string());
            }
            let comment = comment.unwrap();
            match get_text_from_editor(Some(&comment.get_text())) {
                Some(text) => {
                    comment.set_text(text.clone());
                    task.set_comments(comments.unwrap());

                    match gittask::update_task(task) {
                        Ok(_) => {
                            println!("Task ID {task_id} updated");
                            let mut success = false;
                            if push {
                                match get_user_repo(remote, connector_type) {
                                    Ok((connector, user, repo)) => {
                                        match connector.update_remote_comment(&user, &repo, &task_id, &comment_id, &text) {
                                            Ok(_) => {
                                                println!("Sync: REMOTE comment ID {comment_id} has been updated");
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
                None => error_message("No text specified".to_string())
            }
        },
        Ok(None) => error_message(format!("Task ID {task_id} not found")),
        Err(e) => error_message(format!("ERROR: {e}"))
    }
}

pub(crate) fn task_comment_delete(
    task_id: String,
    comment_id: String,
    push: bool,
    remote: &Option<String>,
    connector_type: &Option<String>,
) -> bool {
    match gittask::find_task(&task_id) {
        Ok(Some(mut task)) => {
            match task.delete_comment(&comment_id) {
                Ok(_) => {
                    match gittask::update_task(task) {
                        Ok(_) => {
                            println!("Task ID {task_id} updated");
                            let mut success = false;
                            if push {
                                match get_user_repo(remote, connector_type) {
                                    Ok((connector, user, repo)) => {
                                        match connector.delete_remote_comment(&user, &repo, &task_id, &comment_id) {
                                            Ok(_) => {
                                                println!("Sync: REMOTE comment ID {comment_id} has been deleted");
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