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
    ids
        .split(",")
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .expand_range()
        .collect::<Vec<_>>()
}

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn str_to_color(color: &str, style: &Option<String>) -> Style {
    let color = match color.to_lowercase().as_str() {
        "black" => Black,
        "darkgray" | "darkgrey" => DarkGray,
        "red" => Red,
        "lightred" => LightRed,
        "green" => Green,
        "lightgreen" => LightGreen,
        "yellow" => Yellow,
        "lightyellow" => LightYellow,
        "blue" => Blue,
        "lightblue" => LightBlue,
        "purple" => Purple,
        "lightpurple" => LightPurple,
        "magenta" => Magenta,
        "lightmagenta" => LightMagenta,
        "cyan" => Cyan,
        "lightcyan" => LightCyan,
        "white" => White,
        "lightgray" | "lightgrey" => LightGray,
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

pub fn color_str_to_rgb_str(color: &str) -> String {
    let color = color.trim_start_matches('#').to_lowercase();
    let color = match color.as_str() {
        "black" => "000000",
        "darkgray" | "darkgrey" => "808080",
        "red" => "800000",
        "lightred" => "ff0000",
        "green" => "008000",
        "lightgreen" => "00ff00",
        "yellow" => "808000",
        "lightyellow" => "ffff00",
        "blue" => "000080",
        "lightblue" => "0000ff",
        "purple" => "800080",
        "lightpurple" => "ff00ff",
        "magenta" => "800080",
        "lightmagenta" => "ff00ff",
        "cyan" => "008080",
        "lightcyan" => "00ffff",
        "white" | "default" => "c0c0c0",
        "lightgray" | "lightgrey" => "ffffff",
        s => match s.parse::<u8>() {
            Ok(n) => fixed_to_rgb_str(n),
            _ => {
                match str_to_rgb(s) {
                    Some(_) => s,
                    _ => "000000"
                }
            }
        }
    };

    color.to_string()
}

fn fixed_to_rgb_str(color: u8) -> &'static str {
    match color {
        0 => "000000",
        1 => "800000",
        2 => "008000",
        3 => "808000",
        4 => "000080",
        5 => "800080",
        6 => "008080",
        7 => "c0c0c0",
        8 => "808080",
        9 => "ff0000",
        10 => "ffff00",
        11 => "00ff00",
        12 => "0000ff",
        13 => "ff00ff",
        14 => "00ffff",
        15 => "ffffff",
        16 => "000000",
        17 => "00005f",
        18 => "000087",
        19 => "0000af",
        20 => "0000d7",
        21 => "0000ff",
        22 => "005f00",
        23 => "005f5f",
        24 => "005f87",
        25 => "005faf",
        26 => "005fd7",
        27 => "005fff",
        28 => "008700",
        29 => "00875f",
        30 => "008787",
        31 => "0087af",
        32 => "0087d7",
        33 => "0087ff",
        34 => "00af00",
        35 => "00af5f",
        36 => "00af87",
        37 => "00afaf",
        38 => "00afd7",
        39 => "00afff",
        40 => "00d700",
        41 => "00d75f",
        42 => "00d787",
        43 => "00d7af",
        44 => "00d7d7",
        45 => "00d7ff",
        46 => "00ff00",
        47 => "00ff5f",
        48 => "00ff87",
        49 => "00ffaf",
        50 => "00ffd7",
        51 => "00ffff",
        52 => "5f0000",
        53 => "5f005f",
        54 => "5f0087",
        55 => "5f00af",
        56 => "5f00d7",
        57 => "5f00ff",
        58 => "5f5f00",
        59 => "5f5f5f",
        60 => "5f5f87",
        61 => "5f5faf",
        62 => "5f5fd7",
        63 => "5f5fff",
        64 => "5f8700",
        65 => "5f875f",
        66 => "5f8787",
        67 => "5f87af",
        68 => "5f87d7",
        69 => "5f87ff",
        70 => "5faf00",
        71 => "5faf5f",
        72 => "5faf87",
        73 => "5fafaf",
        74 => "5fafd7",
        75 => "5fafff",
        76 => "5fd700",
        77 => "5fd75f",
        78 => "5fd787",
        79 => "5fd7af",
        80 => "5fd7d7",
        81 => "5fd7ff",
        82 => "5fff00",
        83 => "5fff5f",
        84 => "5fff87",
        85 => "5fffaf",
        86 => "5fffd7",
        87 => "5fffff",
        88 => "870000",
        89 => "87005f",
        90 => "870087",
        91 => "8700af",
        92 => "8700d7",
        93 => "8700ff",
        94 => "875f00",
        95 => "875f5f",
        96 => "875f87",
        97 => "875faf",
        98 => "875fd7",
        99 => "875fff",
        100 => "878700",
        101 => "87875f",
        102 => "878787",
        103 => "8787af",
        104 => "8787d7",
        105 => "8787ff",
        106 => "87af00",
        107 => "87af5f",
        108 => "87af87",
        109 => "87afaf",
        110 => "87afd7",
        111 => "87afff",
        112 => "87d700",
        113 => "87d75f",
        114 => "87d787",
        115 => "87d7af",
        116 => "87d7d7",
        117 => "87d7ff",
        118 => "87ff00",
        119 => "87ff5f",
        120 => "87ff87",
        121 => "87ffaf",
        122 => "87ffd7",
        123 => "87ffff",
        124 => "af0000",
        125 => "af005f",
        126 => "af0087",
        127 => "af00af",
        128 => "af00d7",
        129 => "af00ff",
        130 => "af5f00",
        131 => "af5f5f",
        132 => "af5f87",
        133 => "af5faf",
        134 => "af5fd7",
        135 => "af5fff",
        136 => "af8700",
        137 => "af875f",
        138 => "af8787",
        139 => "af87af",
        140 => "af87d7",
        141 => "af87ff",
        142 => "afaf00",
        143 => "afaf5f",
        144 => "afaf87",
        145 => "afafaf",
        146 => "afafd7",
        147 => "afafff",
        148 => "afd700",
        149 => "afd75f",
        150 => "afd787",
        151 => "afd7af",
        152 => "afd7d7",
        153 => "afd7ff",
        154 => "afff00",
        155 => "afff5f",
        156 => "afff87",
        157 => "afffaf",
        158 => "afffd7",
        159 => "afffff",
        160 => "d70000",
        161 => "d7005f",
        162 => "d70087",
        163 => "d700af",
        164 => "d700d7",
        165 => "d700ff",
        166 => "d75f00",
        167 => "d75f5f",
        168 => "d75f87",
        169 => "d75faf",
        170 => "d75fd7",
        171 => "d75fff",
        172 => "d78700",
        173 => "d7875f",
        174 => "d78787",
        175 => "d787af",
        176 => "d787d7",
        177 => "d787ff",
        178 => "d7af00",
        179 => "d7af5f",
        180 => "d7af87",
        181 => "d7afaf",
        182 => "d7afd7",
        183 => "d7afff",
        184 => "d7d700",
        185 => "d7d75f",
        186 => "d7d787",
        187 => "d7d7af",
        188 => "d7d7d7",
        189 => "d7d7ff",
        190 => "d7ff00",
        191 => "d7ff5f",
        192 => "d7ff87",
        193 => "d7ffaf",
        194 => "d7ffd7",
        195 => "d7ffff",
        196 => "ff0000",
        197 => "ff005f",
        198 => "ff0087",
        199 => "ff00af",
        200 => "ff00d7",
        201 => "ff00ff",
        202 => "ff5f00",
        203 => "ff5f5f",
        204 => "ff5f87",
        205 => "ff5faf",
        206 => "ff5fd7",
        207 => "ff5fff",
        208 => "ff8700",
        209 => "ff875f",
        210 => "ff8787",
        211 => "ff87af",
        212 => "ff87d7",
        213 => "ff87ff",
        214 => "ffaf00",
        215 => "ffaf5f",
        216 => "ffaf87",
        217 => "ffafaf",
        218 => "ffafd7",
        219 => "ffafff",
        220 => "ffd700",
        221 => "ffd75f",
        222 => "ffd787",
        223 => "ffd7af",
        224 => "ffd7d7",
        225 => "ffd7ff",
        226 => "ffff00",
        227 => "ffff5f",
        228 => "ffff87",
        229 => "ffffaf",
        230 => "ffffd7",
        231 => "ffffff",
        232 => "080808",
        233 => "121212",
        234 => "1c1c1c",
        235 => "262626",
        236 => "303030",
        237 => "3a3a3a",
        238 => "444444",
        239 => "4e4e4e",
        240 => "585858",
        241 => "626262",
        242 => "6c6c6c",
        243 => "767676",
        244 => "808080",
        245 => "8a8a8a",
        246 => "949494",
        247 => "9e9e9e",
        248 => "a8a8a8",
        249 => "b2b2b2",
        250 => "bcbcbc",
        251 => "c6c6c6",
        252 => "d0d0d0",
        253 => "dadada",
        254 => "e4e4e4",
        255 => "eeeeee",
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
    let tmp_file = tempfile::Builder::new().prefix("git-task").suffix(".txt").disable_cleanup(true).tempfile().ok()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_range_single() {
        let input = vec!["1".to_string()];
        let expected: Vec<String> = vec!["1".to_string()];
        let result: Vec<String> = input.into_iter().expand_range().collect();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_expand_range_range() {
        let input = vec!["1..3".to_string()];
        let expected: Vec<String> = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        let result: Vec<String> = input.into_iter().expand_range().collect();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_expand_range_mixed() {
        let input = vec!["1".to_string(), "3..5".to_string(), "7".to_string()];
        let expected: Vec<String> = vec!["1".to_string(), "3".to_string(), "4".to_string(), "5".to_string(), "7".to_string()];
        let result: Vec<String> = input.into_iter().expand_range().collect();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_expand_range_invalid_range() {
        let input = vec!["1..x".to_string()];
        let result: Result<Vec<String>, _> = std::panic::catch_unwind(|| {
            input.into_iter().expand_range().collect::<Vec<String>>()
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_ids_single() {
        let input = "1".to_string();
        let expected = vec!["1".to_string()];
        let result = parse_ids(input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_parse_ids_multiple() {
        let input = "1,2,3".to_string();
        let expected = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        let result = parse_ids(input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_parse_ids_range() {
        let input = "1..3".to_string();
        let expected = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        let result = parse_ids(input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_parse_ids_mixed() {
        let input = "1,3..5,7".to_string();
        let expected = vec!["1".to_string(), "3".to_string(), "4".to_string(), "5".to_string(), "7".to_string()];
        let result = parse_ids(input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_parse_ids_empty() {
        let input = "".to_string();
        let expected: Vec<String> = vec![];
        let result = parse_ids(input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_parse_ids_invalid_range() {
        let input = "1..x".to_string();
        let result: Result<Vec<String>, _> = std::panic::catch_unwind(|| {
            parse_ids(input)
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_capitalize_lowercase() {
        let input = "hello";
        let expected = "Hello".to_string();
        let result = capitalize(input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_capitalize_unicode() {
        let input = "превед";
        let expected = "Превед".to_string();
        let result = capitalize(input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_str_to_color_basic() {
        let color_str = "red";
        let expected = Style::new().fg(Red);
        let result = str_to_color(color_str, &None);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_str_to_color_with_style() {
        let color_str = "green";
        let style = Some("bold,underline".to_string());
        let expected = Style::new().fg(Green).bold().underline();
        let result = str_to_color(color_str, &style);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_str_to_color_hex() {
        let color_str = "#00ff00";
        let expected = Style::new().fg(Color::Rgb(0, 255, 0));
        let result = str_to_color(color_str, &None);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_str_to_color_invalid_hex() {
        let color_str = "#zzzzzz";
        let expected = Style::new().fg(Default);
        let result = str_to_color(color_str, &None);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_str_to_color_fixed() {
        let color_str = "123";
        let expected = Style::new().fg(Fixed(123));
        let result = str_to_color(color_str, &None);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_str_to_color_default() {
        let color_str = "unknowncolor";
        let expected = Style::new().fg(Default);
        let result = str_to_color(color_str, &None);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_color_str_to_rgb_str_named_colors() {
        let input = "red";
        let expected = "800000".to_string();
        let result = color_str_to_rgb_str(input);
        assert_eq!(result, expected);

        let input = "lightred";
        let expected = "ff0000".to_string();
        let result = color_str_to_rgb_str(input);
        assert_eq!(result, expected);

        let input = "green";
        let expected = "008000".to_string();
        let result = color_str_to_rgb_str(input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_color_str_to_rgb_str_hex() {
        let input = "#00ff00";
        let expected = "00ff00".to_string();
        let result = color_str_to_rgb_str(input);
        assert_eq!(result, expected);

        let input = "#0000ff";
        let expected = "0000ff".to_string();
        let result = color_str_to_rgb_str(input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_color_str_to_rgb_str_invalid_hex() {
        let input = "#zzzzzz";
        let expected = "000000".to_string();
        let result = color_str_to_rgb_str(input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_color_str_to_rgb_str_fixed_color() {
        let input = "123";
        let expected = "87ffff".to_string();
        let result = color_str_to_rgb_str(input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_color_str_to_rgb_str_default() {
        let input = "unknowncolor";
        let expected = "000000".to_string();
        let result = color_str_to_rgb_str(input);
        assert_eq!(result, expected);
    }
}