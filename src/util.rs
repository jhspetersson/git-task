use std::env::VarError;
use std::fs::File;
use std::io::{IsTerminal, Read, Write};
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

use chrono::{DateTime, Local, MappedLocalTime, NaiveDate, TimeZone};
use nu_ansi_term::Color;

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn colorize_string(s: &str, color: Color, no_color: bool) -> String {
    if no_color { s.to_string() } else { color.paint(s).to_string() }
}

pub fn format_datetime(seconds: u64) -> String {
    if seconds == 0 {
        return String::new();
    }

    let seconds = UNIX_EPOCH + Duration::from_secs(seconds);
    let datetime = DateTime::<Local>::from(seconds);
    datetime.format("%Y-%m-%d %H:%M").to_string()
}

pub fn parse_date(date: Option<String>) -> Option<MappedLocalTime<DateTime<Local>>> {
    match date {
        Some(date) => {
            let naive_date = NaiveDate::parse_from_str(&date, "%Y-%m-%d").unwrap();
            Some(Local.from_local_datetime(&naive_date.and_hms_opt(0, 0, 0).unwrap()))
        }
        None => None
    }
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

pub fn get_text_from_editor(text: Option<&String>) -> Option<String> {
    let tmp_file = tempfile::Builder::new().prefix("git-task").suffix(".txt").keep(true).tempfile().ok()?;
    let mut file = File::create(tmp_file.path()).unwrap();

    if let Some(text) = text {
        write!(file, "{}", text).ok()?;
    }

    let editor = std::env::var("GIT_EDITOR")
        .or_else(|_| gittask::get_config_value("core.editor"))
        .or_else(|_| std::env::var("VISUAL"))
        .or_else(|_| std::env::var("EDITOR"))
        .or_else(|_| Ok::<String, VarError>("vi".to_string()))
        .unwrap();

    let mut status = Command::new(editor)
        .arg(tmp_file.path().to_str()?)
        .status();

    if status.is_err() {
        status = Command::new("notepad")
            .arg(tmp_file.path().to_str()?)
            .status();
    }

    if !status.unwrap().success() {
        let _ = tmp_file.close();
        eprintln!("Editor exited with a non-zero status. Changes might not be saved.");
        return None;
    }

    let mut file = File::open(tmp_file.path()).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).ok()?;

    let _ = tmp_file.close();

    Some(contents)
}