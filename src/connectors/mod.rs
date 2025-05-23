mod github;
mod gitlab;
mod jira;
mod redmine;

use gittask::{Comment, Label, Task};
use crate::connectors::github::GithubRemoteConnector;
use crate::connectors::gitlab::GitlabRemoteConnector;
use crate::connectors::jira::JiraRemoteConnector;
use crate::connectors::redmine::RedmineRemoteConnector;

#[derive(Debug, PartialEq)]
pub enum RemoteTaskState {
    All,
    Open(String, String),
    Closed(String, String),
}

pub trait RemoteConnector {
    fn type_name(&self) -> &str;
    fn get_config_options(&self) -> Option<Vec<String>> {
        None
    }
    fn supports_remote(&self, url: &str) -> Option<(String, String)>;
    fn list_remote_tasks(&self, user: &String, repo: &String, with_comments: bool, with_labels: bool, limit: Option<usize>, state: RemoteTaskState, task_statuses: &Vec<String>) -> Result<Vec<Task>, String>;
    fn get_remote_task(&self, user: &String, repo: &String, task_id: &String, with_comments: bool, with_labels: bool, task_statuses: &Vec<String>) -> Result<Task, String>;
    fn create_remote_task(&self, user: &String, repo: &String, task: &Task) -> Result<String, String>;
    fn create_remote_comment(&self, user: &String, repo: &String, task_id: &String, comment: &Comment) -> Result<String, String>;
    fn create_remote_label(&self, user: &String, repo: &String, task_id: &String, label: &Label) -> Result<(), String>;
    fn update_remote_task(&self, user: &String, repo: &String, task: &Task, labels: Option<&Vec<Label>>, state: RemoteTaskState) -> Result<(), String>;
    fn update_remote_comment(&self, user: &String, repo: &String, task_id: &String, comment_id: &String, text: &String) -> Result<(), String>;
    fn delete_remote_task(&self, user: &String, repo: &String, task_id: &String) -> Result<(), String>;
    fn delete_remote_comment(&self, user: &String, repo: &String, task_id: &String, comment_id: &String) -> Result<(), String>;
    fn delete_remote_label(&self, user: &String, repo: &String, task_id: &String, name: &String) -> Result<(), String>;
}

const CONNECTORS: [&dyn RemoteConnector; 4] = [
    &GithubRemoteConnector,
    &GitlabRemoteConnector,
    &JiraRemoteConnector,
    &RedmineRemoteConnector,
];

pub fn get_matching_remote_connectors(remotes: Vec<String>,
                                      connector_type: &Option<String>
) -> Vec<(Box<&'static dyn RemoteConnector>, String, String)> {
    let mut result = vec![];

    for remote in remotes {
        for connector in CONNECTORS {
            if let Some(connector_type) = connector_type {
                if connector_type != connector.type_name() {
                    continue;
                }
            }

            if let Some((user, repo)) = connector.supports_remote(&remote) {
                result.push((Box::new(connector), user, repo));
            }
        }
    }

    result
}

pub(crate) fn get_config_options_from_connectors() -> Vec<String> {
    CONNECTORS
        .iter()
        .filter_map(|c| c.get_config_options())
        .flatten()
        .collect()
}
