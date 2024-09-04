use nu_ansi_term::AnsiString;
use nu_ansi_term::Color::{Default, Green, Red, Yellow};

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

pub(crate) fn get_statuses() -> Vec<Status> {
    vec![
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

pub(crate) fn format_status(status: &str, no_color: bool) -> AnsiString {
    match no_color {
        false => {
            let status_color = get_statuses().iter().find_map(|saved_status| {
                if status == saved_status.name { Some(saved_status.color.clone()) } else { None }
            }).or(Some("Default".to_string())).unwrap();

            let status_color = match status_color.as_str() {
                "Red" => Red,
                "Yellow" => Yellow,
                "Green" => Green,
                _ => Default
            };

            status_color.paint(&*status)
        },
        true => status.into()
    }
}

pub(crate) fn get_full_status_name(status: &String) -> String {
    get_statuses().iter().find_map(|saved_status| {
        if status == saved_status.shortcut.as_str() { Some(saved_status.name.clone()) } else { None }
    }).unwrap_or(status.clone())
}