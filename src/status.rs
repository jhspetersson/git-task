use nu_ansi_term::AnsiString;
use nu_ansi_term::Color::{Black, Blue, Cyan, DarkGray, Default, Green, LightBlue, LightCyan, LightGray, LightGreen, LightMagenta, LightPurple, LightRed, LightYellow, Magenta, Purple, Red, White, Yellow};

pub struct Status {
    name: String,
    shortcut: String,
    color: String,
}

impl Status {
    pub(crate) fn get_name(&self) -> &str {
        &self.name
    }
}

pub struct StatusManager {
    statuses: Vec<Status>,
}

impl StatusManager {
    pub fn new() -> StatusManager {
        StatusManager {
            statuses: vec![
                Status {
                    name: String::from("OPEN"),
                    shortcut: String::from("o"),
                    color: String::from("Red"),
                },
                Status {
                    name: String::from("IN_PROGRESS"),
                    shortcut: String::from("i"),
                    color: String::from("Yellow"),
                },
                Status {
                    name: String::from("CLOSED"),
                    shortcut: String::from("c"),
                    color: String::from("Green"),
                }
            ]
        }
    }

    pub fn get_statuses(&self) -> &Vec<Status> {
        &self.statuses
    }

    pub fn format_status<'a>(&self, status: &'a str, no_color: bool) -> AnsiString<'a> {
        match no_color {
            false => {
                let status_color = self.statuses.iter().find_map(|saved_status| {
                    if status == saved_status.name { Some(saved_status.color.clone()) } else { None }
                }).or_else(|| Some("Default".to_string())).unwrap();

                let status_color = match status_color.as_str() {
                    "Black" => Black,
                    "DarkGray" => DarkGray,
                    "Red" => Red,
                    "LightRed" => LightRed,
                    "Green" => Green,
                    "LightGreen" => LightGreen,
                    "Yellow" => Yellow,
                    "LightYellow" => LightYellow,
                    "Blue" => Blue,
                    "LightBlue" => LightBlue,
                    "Purple" => Purple,
                    "LightPurple" => LightPurple,
                    "Magenta" => Magenta,
                    "LightMagenta" => LightMagenta,
                    "Cyan" => Cyan,
                    "LightCyan" => LightCyan,
                    "White" => White,
                    "LightGray" => LightGray,
                    _ => Default
                };

                status_color.paint(&*status)
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
        self.statuses.first().unwrap().name.clone()
    }
}