mod github;
mod gitlab;

use gittask::{Comment, Label, Task};
use crate::connectors::github::GithubRemoteConnector;
use crate::connectors::gitlab::GitlabRemoteConnector;

#[derive(PartialEq)]
pub enum RemoteTaskState {
    All,
    Open,
    Closed,
}

pub trait RemoteConnector {
    fn supports_remote(&self, url: &str) -> Option<(String, String)>;
    fn list_remote_tasks(&self, user: &String, repo: &String, with_comments: bool, with_labels: bool, limit: Option<usize>, state: RemoteTaskState, task_statuses: &Vec<String>) -> Vec<Task>;
    fn get_remote_task(&self, user: &String, repo: &String, task_id: &String, with_comments: bool, with_labels: bool, task_statuses: &Vec<String>) -> Option<Task>;
    fn create_remote_task(&self, user: &String, repo: &String, task: &Task) -> Result<String, String>;
    fn create_remote_comment(&self, user: &String, repo: &String, task_id: &String, comment: &Comment) -> Result<String, String>;
    fn create_remote_label(&self, user: &String, repo: &String, task_id: &String, label: &Label) -> Result<(), String>;
    fn update_remote_task(&self, user: &String, repo: &String, task: &Task, state: RemoteTaskState) -> Result<(), String>;
    fn update_remote_comment(&self, user: &String, repo: &String, task_id: &String, comment_id: &String, text: &String) -> Result<(), String>;
    fn delete_remote_task(&self, user: &String, repo: &String, task_id: &String) -> Result<(), String>;
    fn delete_remote_comment(&self, user: &String, repo: &String, task_id: &String, comment_id: &String) -> Result<(), String>;
    fn delete_remote_label(&self, user: &String, repo: &String, task_id: &String, name: &String) -> Result<(), String>;
}

const CONNECTORS: [&dyn RemoteConnector; 2] = [
    &GithubRemoteConnector,
    &GitlabRemoteConnector,
];

pub fn get_matching_remote_connectors(remotes: Vec<String>) -> Vec<(Box<&'static dyn RemoteConnector>, String, String)> {
    let mut result = vec![];

    for remote in remotes {
        for connector in CONNECTORS {
            if let Some((user, repo)) = connector.supports_remote(&remote) {
                result.push((Box::new(connector), user, repo));
            }
        }
    }

    result
}