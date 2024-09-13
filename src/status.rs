use nu_ansi_term::AnsiString;
use serde::{Deserialize, Serialize};

use crate::util::str_to_color;

#[derive(Clone, Serialize, Deserialize)]
pub struct Status {
    name: String,
    shortcut: String,
    color: String,
    is_done: bool,
}

impl Status {
    pub(crate) fn get_name(&self) -> &str {
        &self.name
    }

    pub(crate) fn get_shortcut(&self) -> &str {
        &self.shortcut
    }

    pub(crate) fn get_color(&self) -> &str {
        &self.color
    }

    pub(crate) fn is_done(&self) -> &bool {
        &self.is_done
    }
}

pub struct StatusManager {
    statuses: Vec<Status>,
}

impl StatusManager {
    pub fn new() -> StatusManager {
        let statuses = read_config().unwrap_or_else(|_| Self::get_defaults());

        StatusManager {
            statuses
        }
    }

    fn get_defaults() -> Vec<Status> {
        vec![
            Status {
                name: String::from("OPEN"),
                shortcut: String::from("o"),
                color: String::from("Red"),
                is_done: false,
            },
            Status {
                name: String::from("IN_PROGRESS"),
                shortcut: String::from("i"),
                color: String::from("Yellow"),
                is_done: false,
            },
            Status {
                name: String::from("CLOSED"),
                shortcut: String::from("c"),
                color: String::from("Green"),
                is_done: true,
            }
        ]
    }

    pub fn get_statuses(&self) -> &Vec<Status> {
        &self.statuses
    }

    pub fn set_statuses(&mut self, statuses: Vec<Status>) -> Result<(), String> {
        let name_contains_comma = statuses.iter().find(|s| s.name.contains(",") || s.shortcut.contains(",")).is_some();
        match name_contains_comma {
            true => Err("Status name and shortcut can't contain comma".to_string()),
            false => {
                self.statuses = statuses;
                save_config(&self.statuses)
            }
        }
    }

    pub fn set_defaults(&mut self) -> Result<(), String> {
        self.set_statuses(Self::get_defaults())
    }

    pub fn add_status(&mut self, name: String, shortcut: String, color: String, is_done: bool) -> Result<(), String> {
        if name.contains(",") || shortcut.contains(",") {
            return Err("Status name and shortcut can't contain comma".to_string());
        }

        let status = Status {
            name,
            shortcut,
            color,
            is_done,
        };
        self.statuses.push(status);
        save_config(&self.statuses)
    }

    pub fn delete_status(&mut self, name: String) -> Result<(), String> {
        let prev_status_count = self.statuses.len();
        self.statuses.retain(|s| s.name != name);
        match prev_status_count == self.statuses.len() {
            true => Err("Status not found".to_string()),
            false => save_config(&self.statuses),
        }
    }

    pub fn format_status<'a>(&self, status: &'a str, no_color: bool) -> AnsiString<'a> {
        match no_color {
            false => {
                let status_color = self.statuses.iter().find_map(|saved_status| {
                    if status == saved_status.name { Some(saved_status.color.clone()) } else { None }
                }).or_else(|| Some("Default".to_string())).unwrap();
                let status_color = str_to_color(&status_color);
                status_color.paint(status)
            },
            true => status.into()
        }
    }

    pub fn get_full_status_name(&self, status: &String) -> String {
        self.statuses.iter().find_map(|saved_status| {
            if status == saved_status.shortcut.as_str() { Some(saved_status.name.clone()) } else { None }
        }).unwrap_or(status.clone())
    }

    pub fn get_starting_status(&self) -> String {
        match gittask::get_config_value("task.status.open") {
            Ok(s) => s,
            _ => self.statuses.first().unwrap().name.clone()
        }
    }

    pub fn get_final_status(&self) -> String {
        match gittask::get_config_value("task.status.closed") {
            Ok(s) => s,
            _ => {
                self.statuses.iter().find_map(|saved_status| {
                    if saved_status.is_done { Some(saved_status.name.clone()) } else { None }
                }).unwrap()
            }
        }
    }

    pub fn is_done(&self, status: &str) -> bool {
        self.statuses.iter().find_map(|saved_status| {
            if saved_status.name == status { Some(saved_status.is_done) } else { None }
        }).unwrap_or(false)
    }

    pub fn get_property(&self, status: &str, property: &str) -> Option<String> {
        self.statuses.iter().find_map(|saved_status| {
            if status == saved_status.name.as_str() {
                match property {
                    "name" => return Some(saved_status.name.clone()),
                    "shortcut" => return Some(saved_status.shortcut.clone()),
                    "color" => return Some(saved_status.color.clone()),
                    "is_done" => return Some(saved_status.is_done.to_string()),
                    _ => None
                }
            } else { None }
        })
    }

    pub fn set_property(&mut self, status: &String, property: &String, value: &String) -> Result<Option<String>, String> {
        let statuses = self.statuses.clone();
        let status = self.statuses.iter_mut().find(|saved_status| {
            status == saved_status.name.as_str()
        });
        match status {
            Some(saved_status) => {
                let set_result = match property.as_str() {
                    "name" => {
                        if value.contains(",") {
                            return Err("Status name can't contain comma".to_string());
                        }

                        let prev_value = saved_status.name.clone();
                        if statuses.iter().find(|status| status.name == value.to_string()).is_some() {
                            Err("Name already exists for another status".to_string())
                        } else {
                            saved_status.name = value.clone();
                            Ok(Some(prev_value))
                        }
                    },
                    "shortcut" => {
                        if value.contains(",") {
                            return Err("Status shortcut can't contain comma".to_string());
                        }

                        if statuses.iter().find(|status| status.shortcut == value.to_string()).is_some() {
                            Err("Shortcut already exists for another status".to_string())
                        } else {
                            saved_status.shortcut = value.clone(); Ok(None)
                        }
                    },
                    "color" => {
                        saved_status.color = value.clone(); Ok(None)
                    },
                    "is_done" => {
                        saved_status.is_done = value.parse::<bool>().unwrap(); Ok(None)
                    },
                    _ => Err("Unknown property".to_string())
                };
                match set_result {
                    Ok(prev_value) => {
                        match save_config(&self.statuses) {
                            Ok(_) => Ok(prev_value),
                            Err(e) => Err(e)
                        }
                    },
                    Err(e) => Err(e)
                }
            },
            None => Err("No such status".into())
        }
    }
}

fn read_config() -> Result<Vec<Status>, String> {
    match gittask::get_config_value("task.statuses") {
        Ok(s) => parse_statuses(s),
        Err(e) => Err(e)
    }
}

fn save_config(statuses: &Vec<Status>) -> Result<(), String> {
    let statuses = serde_json::to_string(&statuses).map_err(|_| "Could not serialize statuses".to_string())?;
    match gittask::set_config_value("task.statuses", &statuses) {
        Ok(_) => Ok(()),
        Err(e) => Err(e)
    }
}

pub fn parse_statuses(input: String) -> Result<Vec<Status>, String> {
    let result: Vec<Status> = serde_json::from_str(&input).map_err(|e| e.to_string())?;
    Ok(result)
}