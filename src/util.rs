use std::io::{IsTerminal, Read};
use std::time::{Duration, UNIX_EPOCH};
use chrono::{DateTime, Local};

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn format_datetime(seconds: u64) -> String {
    if seconds == 0 {
        return String::new();
    }

    let seconds = UNIX_EPOCH + Duration::from_secs(seconds);
    let datetime = DateTime::<Local>::from(seconds);
    datetime.format("%Y-%m-%d %H:%M").to_string()
}

pub fn read_from_pipe() -> Option<String> {
    let mut buf = String::new();
    match std::io::stdin().is_terminal() {
        false => {
            std::io::stdin().read_to_string(&mut buf).ok()?;
            Some(buf)
        },
        true => None
    }
}