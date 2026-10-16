mod github;
mod gitlab;
pub(crate) mod jira;
mod redmine;

use gittask::{Comment, Label, Subtask, Task};
use crate::connectors::github::GithubRemoteConnector;
use crate::connectors::gitlab::GitlabRemoteConnector;
use crate::connectors::jira::JiraRemoteConnector;
use crate::connectors::redmine::RedmineRemoteConnector;

#[derive(Debug, Clone, PartialEq)]
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
    /// The default remote target (user/repo, domain/project...) for a git remote URL.
    fn supports_remote(&self, url: &str) -> Option<(String, String)>;
    /// All remote targets for a git remote URL. Connectors that manage several projects
    /// at once (e.g. Jira) override this; the default target comes first.
    fn supports_remotes(&self, url: &str) -> Vec<(String, String)> {
        self.supports_remote(url).into_iter().collect()
    }
    /// Whether the connector can serve several projects from a single repository.
    fn supports_multiple_projects(&self) -> bool {
        false
    }
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

    fn supports_subtasks(&self) -> bool {
        false
    }

    #[allow(unused)]
    fn list_remote_subtasks(&self, user: &String, repo: &String, task_id: &String, task_statuses: &Vec<String>) -> Result<Vec<Subtask>, String> {
        Err(format!("Subtasks are not supported for {}", self.type_name()))
    }

    #[allow(unused)]
    fn create_remote_subtask(&self, user: &String, repo: &String, task_id: &String, subtask: &Subtask, is_done: bool) -> Result<String, String> {
        Err(format!("Subtasks are not supported for {}", self.type_name()))
    }

    #[allow(unused)]
    fn update_remote_subtask(&self, user: &String, repo: &String, task_id: &String, subtask: &Subtask, is_done: bool) -> Result<(), String> {
        Err(format!("Subtasks are not supported for {}", self.type_name()))
    }

    #[allow(unused)]
    fn delete_remote_subtask(&self, user: &String, repo: &String, task_id: &String, subtask_id: &String) -> Result<(), String> {
        Err(format!("Subtasks are not supported for {}", self.type_name()))
    }
}

const CONNECTORS: [&dyn RemoteConnector; 4] = [
    &GithubRemoteConnector,
    &GitlabRemoteConnector,
    &JiraRemoteConnector,
    &RedmineRemoteConnector,
];

pub type RemoteTarget = (Box<&'static dyn RemoteConnector>, String, String);

/// Find connectors that can serve the given git remotes.
///
/// When `project` is set, every target of every connector is considered and only those
/// whose project (the second element, e.g. repo name or Jira project key) matches are returned.
/// When `all_projects` is set, every target is returned; otherwise only the default target
/// of each connector is returned, which is the historic single-project behaviour.
pub fn get_matching_remote_connectors(remotes: Vec<String>,
                                      connector_type: &Option<String>,
                                      project: &Option<String>,
                                      all_projects: bool,
) -> Vec<RemoteTarget> {
    let mut result: Vec<RemoteTarget> = vec![];

    for remote in remotes {
        for connector in CONNECTORS {
            if let Some(connector_type) = connector_type {
                if connector_type != connector.type_name() {
                    continue;
                }
            }

            let targets = if project.is_some() || all_projects {
                connector.supports_remotes(&remote)
            } else {
                connector.supports_remote(&remote).into_iter().collect()
            };

            for (user, repo) in targets {
                if let Some(project) = project {
                    if !repo.eq_ignore_ascii_case(project) {
                        continue;
                    }
                }

                let duplicate = result.iter().any(|(c, u, r)| {
                    c.type_name() == connector.type_name() && *u == user && *r == repo
                });
                if !duplicate {
                    result.push((Box::new(connector), user, repo));
                }
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
