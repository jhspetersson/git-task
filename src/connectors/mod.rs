mod github;

use gittask::{Comment, Task};
use crate::connectors::github::GithubRemoteConnector;

pub enum RemoteTaskState {
    All,
    Open,
    Closed,
}

pub trait RemoteConnector {
    fn supports_remote(&self, url: &str) -> Option<(String, String)>;
    fn list_remote_tasks(&self, user: String, repo: String, with_comments: bool, limit: Option<usize>, state: RemoteTaskState, task_statuses: Vec<String>) -> Vec<Task>;
    fn get_remote_task(&self, user: &String, repo: &String, task_id: &String, with_comments: bool) -> Option<Task>;
    fn create_remote_task(&self, user: &String, repo: &String, task: &Task) -> Result<String, String>;
    fn create_remote_comment(&self, user: &String, repo: &String, task_id: &String, comment: &Comment) -> Result<String, String>;
    fn update_remote_task_status(&self, user: &str, repo: &str, task_id: &String, state: RemoteTaskState) -> Result<(), String>;
    fn update_remote_comment(&self, user: &String, repo: &String, comment_id: &String, text: String) -> Result<(), String>;
    fn delete_remote_task(&self, user: &String, repo: &String, task_id: &String) -> Result<(), String>;
    fn delete_remote_comment(&self, user: &String, repo: &String, comment_id: &String) -> Result<(), String>;
}

const CONNECTORS: [&dyn RemoteConnector; 1] = [
    &GithubRemoteConnector,
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