use std::env::VarError;
use std::fs::File;
use std::io::{IsTerminal, Read, Write};
use std::iter::Iterator;
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

use chrono::{DateTime, Local, MappedLocalTime, NaiveDate, TimeZone, Utc};
use nu_ansi_term::{Color, Style};
use nu_ansi_term::Color::{Black, Blue, Cyan, DarkGray, Default, Fixed, Green, LightBlue, LightCyan, LightGray, LightGreen, LightMagenta, LightPurple, LightRed, LightYellow, Magenta, Purple, Red, White, Yellow};

pub trait ExpandRange {
    fn expand_range(self) -> impl Iterator<Item = String>;
}

impl<I> ExpandRange for I
where
    I: Iterator<Item = String>
{
    fn expand_range(self) -> impl Iterator<Item = String> {
        self.flat_map(|s| {
            if let Some((start, end)) = s.split_once("..") {
                let start_num = start.parse::<u64>().unwrap();
                let end_num = end.parse::<u64>().unwrap();
                (start_num..=end_num).map(|n| n.to_string()).collect::<Vec<_>>()
            } else {
                vec![s]
            }
        })
    }
}

pub fn parse_ids(ids: String) -> Vec<String> {
    ids.split(",").map(|s| s.to_string()).expand_range().collect::<Vec<_>>()
}

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn str_to_color(color: &str, style: &Option<String>) -> Style {
    let color = match color {
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
        s => match s.parse::<u8>() {
            Ok(n) => Fixed(n),
            _ => {
                match str_to_rgb(s) {
                    Some(rgb) => rgb,
                    _ => Default
                }
            }
        }
    };

    match style {
        Some(s) => {
            let mut color = color.normal();
            let values = s.split(",").collect::<Vec<&str>>();
            for value in values {
                match value {
                    "bold" => color = color.bold(),
                    "dimmed" => color = color.dimmed(),
                    "italic" => color = color.italic(),
                    "strikethrough" => color = color.strikethrough(),
                    "underline" => color = color.underline(),
                    _ => {}
                }
            }
            color
        },
        None => color.normal()
    }
}

fn str_to_rgb(color: &str) -> Option<Color> {
    let color = color.trim_start_matches('#');

    if color.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&color[0..2], 16).ok()?;
    let g = u8::from_str_radix(&color[2..4], 16).ok()?;
    let b = u8::from_str_radix(&color[4..6], 16).ok()?;

    Some(Color::Rgb(r, g, b))
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
    date.map(|date| {
        let naive_date = NaiveDate::parse_from_str(&date, "%Y-%m-%d").unwrap();
        Local.from_local_datetime(&naive_date.and_hms_opt(0, 0, 0).unwrap())
    })
}

pub fn parse_datetime_to_seconds(datetime: String) -> String {
    DateTime::parse_from_rfc3339(&datetime).unwrap().with_timezone(&Utc).timestamp().to_string()
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

pub fn success_message(message: String) -> bool {
    println!("{message}");
    true
}

pub fn error_message(message: String) -> bool {
    eprintln!("{message}");
    false
}