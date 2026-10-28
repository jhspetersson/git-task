//! Conversion between the Atlassian Document Format (ADF) used by Jira Cloud
//! and a markdown subset that can be stored as plain task text.
//!
//! The markdown dialect covers paragraphs with hard line breaks, headings,
//! bullet and ordered lists (nested), task lists, fenced code blocks, block
//! quotes, horizontal rules, pipe tables and the inline marks bold, italic,
//! strikethrough, code and links. Anything else is flattened to its text.

use serde_json::{json, Value};

/// Renders an ADF document as markdown.
pub fn adf_to_markdown(doc: &Value) -> String {
    match doc.get("content").and_then(Value::as_array) {
        Some(blocks) => render_blocks(blocks),
        None => String::new(),
    }
}

/// Parses markdown into an ADF document.
pub fn markdown_to_adf(text: &str) -> Value {
    let lines: Vec<&str> = text.lines().collect();
    let mut content = parse_blocks(&lines);
    if content.is_empty() {
        content.push(json!({"type": "paragraph", "content": []}));
    }
    json!({"type": "doc", "version": 1, "content": content})
}

// ---------------------------------------------------------------------------
// ADF -> markdown
// ---------------------------------------------------------------------------

fn render_blocks(blocks: &[Value]) -> String {
    blocks.iter()
        .map(render_block)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn node_type(node: &Value) -> &str {
    node.get("type").and_then(Value::as_str).unwrap_or("")
}

fn children(node: &Value) -> &[Value] {
    node.get("content").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn attr_str<'a>(node: &'a Value, name: &str) -> Option<&'a str> {
    node.get("attrs").and_then(|a| a.get(name)).and_then(Value::as_str)
}

fn attr_u64(node: &Value, name: &str) -> Option<u64> {
    node.get("attrs").and_then(|a| a.get(name)).and_then(Value::as_u64)
}

fn render_block(node: &Value) -> String {
    match node_type(node) {
        "paragraph" => render_inline(children(node)),
        "heading" => {
            let level = attr_u64(node, "level").unwrap_or(1).clamp(1, 6) as usize;
            format!("{} {}", "#".repeat(level), render_inline(children(node)).replace('\n', "<br>"))
        }
        "bulletList" => render_list(children(node), None),
        "orderedList" => render_list(children(node), Some(attr_u64(node, "order").unwrap_or(1))),
        "taskList" => children(node).iter().map(|item| {
            let done = attr_str(item, "state") == Some("DONE");
            let marker = if done { "- [x] " } else { "- [ ] " };
            indent_continuation(&render_inline(children(item)), marker)
        }).collect::<Vec<_>>().join("\n"),
        "codeBlock" => {
            let language = attr_str(node, "language").unwrap_or("");
            let code = plain_text(children(node));
            // The fence must be longer than any backtick run opening a line of the code,
            // otherwise that line would close the block early.
            let longest_run = code.lines().map(|l| leading_backticks(l.trim_start())).max().unwrap_or(0);
            let fence = "`".repeat(longest_run.max(2) + 1);
            format!("{fence}{language}\n{code}\n{fence}")
        }
        "blockquote" | "panel" => render_blocks(children(node)).lines()
            .map(|l| if l.is_empty() { ">".to_string() } else { format!("> {l}") })
            .collect::<Vec<_>>().join("\n"),
        "rule" => "---".to_string(),
        "table" => render_table(children(node)),
        "expand" | "nestedExpand" => {
            let body = render_blocks(children(node));
            match attr_str(node, "title").filter(|t| !t.is_empty()) {
                Some(title) => format!("**{}**\n\n{body}", escape_text(title)),
                None => body,
            }
        }
        "mediaSingle" | "mediaGroup" | "media" | "mediaInline" => String::new(),
        _ => {
            let kids = children(node);
            if kids.iter().any(is_inline) {
                render_inline(kids)
            } else {
                render_blocks(kids)
            }
        }
    }
}

fn is_inline(node: &Value) -> bool {
    matches!(node_type(node), "text" | "hardBreak" | "mention" | "emoji" | "inlineCard" | "date" | "status" | "placeholder")
}

fn render_list(items: &[Value], ordered_from: Option<u64>) -> String {
    items.iter().enumerate().map(|(i, item)| {
        let marker = match ordered_from {
            Some(start) => format!("{}. ", start + i as u64),
            None => "- ".to_string(),
        };
        indent_continuation(&render_item_blocks(children(item)), &marker)
    }).collect::<Vec<_>>().join("\n")
}

/// Like `render_blocks`, but a list nested directly under a paragraph follows
/// it on the next line, as markdown lists are usually written.
fn render_item_blocks(blocks: &[Value]) -> String {
    let mut out = String::new();
    let mut previous: Option<&str> = None;
    for block in blocks {
        let rendered = render_block(block);
        if rendered.is_empty() {
            continue;
        }
        let kind = node_type(block);
        if let Some(prev) = previous {
            let tight = prev == "paragraph" && matches!(kind, "bulletList" | "orderedList" | "taskList");
            out.push_str(if tight { "\n" } else { "\n\n" });
        }
        out.push_str(&rendered);
        previous = Some(kind);
    }
    out
}

/// Puts `marker` before the first line and indents the following lines by its width.
fn indent_continuation(body: &str, marker: &str) -> String {
    let pad = " ".repeat(marker.chars().count());
    let mut out = String::new();
    for (i, line) in body.lines().enumerate() {
        if i == 0 {
            out.push_str(marker);
        } else {
            out.push('\n');
            if !line.is_empty() {
                out.push_str(&pad);
            }
        }
        out.push_str(line);
    }
    if body.is_empty() {
        out.push_str(marker.trim_end());
    }
    out
}

fn render_table(rows: &[Value]) -> String {
    let rendered: Vec<Vec<String>> = rows.iter().map(|row| {
        children(row).iter().map(|cell| {
            encode_cell(&render_blocks(children(cell))).replace('|', "\\|")
        }).collect()
    }).collect();
    if rendered.is_empty() {
        return String::new();
    }
    let width = rendered.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let line = |cells: &Vec<String>| {
        let mut cells = cells.clone();
        cells.resize(width, String::new());
        format!("| {} |", cells.join(" | "))
    };
    let mut out = vec![line(&rendered[0]), format!("|{}", " --- |".repeat(width))];
    out.extend(rendered[1..].iter().map(line));
    out.join("\n")
}

fn plain_text(nodes: &[Value]) -> String {
    nodes.iter().map(|n| match node_type(n) {
        "text" => n.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
        "hardBreak" => "\n".to_string(),
        _ => plain_text(children(n)),
    }).collect()
}

#[derive(Clone, PartialEq, Debug)]
struct Marks {
    code: bool,
    strong: bool,
    em: bool,
    strike: bool,
    link: Option<String>,
}

fn marks_of(node: &Value) -> Marks {
    let mut marks = Marks { code: false, strong: false, em: false, strike: false, link: None };
    if let Some(list) = node.get("marks").and_then(Value::as_array) {
        for mark in list {
            match node_type(mark) {
                "code" => marks.code = true,
                "strong" => marks.strong = true,
                "em" => marks.em = true,
                "strike" => marks.strike = true,
                "link" => marks.link = attr_str(mark, "href").map(str::to_string),
                _ => {}
            }
        }
    }
    marks
}

fn render_inline(nodes: &[Value]) -> String {
    // Collect (text, marks) runs, merging neighbours with identical marks so
    // that "**a****b**" never appears.
    let mut runs: Vec<(String, Marks)> = vec![];
    let plain = Marks { code: false, strong: false, em: false, strike: false, link: None };
    let mut push = |text: String, marks: Marks| {
        if text.is_empty() {
            return;
        }
        match runs.last_mut() {
            Some((last, last_marks)) if *last_marks == marks => last.push_str(&text),
            _ => runs.push((text, marks)),
        }
    };
    for node in nodes {
        match node_type(node) {
            "text" => push(node.get("text").and_then(Value::as_str).unwrap_or("").to_string(), marks_of(node)),
            "hardBreak" => push("\n".to_string(), plain.clone()),
            "mention" | "status" | "placeholder" => push(attr_str(node, "text").unwrap_or("").to_string(), plain.clone()),
            "emoji" => push(attr_str(node, "shortName").or(attr_str(node, "text")).unwrap_or("").to_string(), plain.clone()),
            "inlineCard" => push(attr_str(node, "url").unwrap_or("").to_string(), plain.clone()),
            "date" => push(attr_str(node, "timestamp").unwrap_or("").to_string(), plain.clone()),
            _ => push(plain_text(children(node)), plain.clone()),
        }
    }

    let mut candidates = vec![];
    for smart in [false, true] {
        candidates.push(render_runs(&runs, None, smart, None, None));
    }
    for order in MARK_ORDERS {
        for smart in [false, true] {
            candidates.push(render_runs(&runs, Some(&order), smart, None, None));
        }
    }
    for smart in [false, true] {
        candidates.push(render_flat(&runs, smart));
    }
    for candidate in candidates {
        let md = escape_line_starts(&candidate);
        if inline_round_trips(&md, &runs) {
            return md;
        }
    }
    let unmarked: Vec<(String, Marks)> = runs.iter()
        .map(|(text, marks)| (text.clone(), Marks { strong: false, em: false, strike: false, ..marks.clone() }))
        .collect();
    escape_line_starts(&render_runs(&unmarked, None, false, None, None))
}

const MARK_ORDERS: [[MarkKind; 4]; 6] = [
    [MarkKind::Link, MarkKind::Strong, MarkKind::Em, MarkKind::Strike],
    [MarkKind::Link, MarkKind::Strong, MarkKind::Strike, MarkKind::Em],
    [MarkKind::Link, MarkKind::Em, MarkKind::Strong, MarkKind::Strike],
    [MarkKind::Link, MarkKind::Em, MarkKind::Strike, MarkKind::Strong],
    [MarkKind::Link, MarkKind::Strike, MarkKind::Strong, MarkKind::Em],
    [MarkKind::Link, MarkKind::Strike, MarkKind::Em, MarkKind::Strong],
];

fn render_flat(runs: &[(String, Marks)], smart: bool) -> String {
    let runs = merge_runs(runs);
    let mut out = String::new();
    for (k, run) in runs.iter().enumerate() {
        let prev = out.chars().last();
        let next = runs.get(k + 1).and_then(|(text, _)| text.chars().next());
        out.push_str(&render_runs(std::slice::from_ref(run), None, smart, prev, next));
    }
    out
}

fn inline_round_trips(md: &str, runs: &[(String, Marks)]) -> bool {
    let plain = Marks { code: false, strong: false, em: false, strike: false, link: None };
    let parsed: Vec<(String, Marks)> = parse_inline(md).iter().map(|node| match node_type(node) {
        "hardBreak" => ("\n".to_string(), plain.clone()),
        _ => (node.get("text").and_then(Value::as_str).unwrap_or("").to_string(), marks_of(node)),
    }).collect();
    comparable_runs(&parsed) == comparable_runs(runs)
}

fn comparable_runs(runs: &[(String, Marks)]) -> Vec<(String, Marks)> {
    let plain = Marks { code: false, strong: false, em: false, strike: false, link: None };
    let mut split: Vec<(String, Marks)> = vec![];
    for (text, marks) in runs {
        if marks.code {
            split.push((text.clone(), marks.clone()));
            continue;
        }
        let core = text.trim();
        if core.is_empty() {
            split.push((text.clone(), plain.clone()));
            continue;
        }
        let lead = &text[..text.len() - text.trim_start().len()];
        let trail = &text[text.trim_end().len()..];
        split.push((lead.to_string(), plain.clone()));
        split.push((core.to_string(), marks.clone()));
        split.push((trail.to_string(), plain.clone()));
    }
    merge_runs(&split)
}

fn merge_runs(runs: &[(String, Marks)]) -> Vec<(String, Marks)> {
    let mut out: Vec<(String, Marks)> = vec![];
    for (text, marks) in runs {
        if text.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some((t, m)) if m == marks => t.push_str(text),
            _ => out.push((text.clone(), marks.clone())),
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum MarkKind { Link, Strong, Em, Strike }

fn has_mark(kind: MarkKind, marks: &Marks) -> bool {
    match kind {
        MarkKind::Link => marks.link.is_some(),
        MarkKind::Strong => marks.strong,
        MarkKind::Em => marks.em,
        MarkKind::Strike => marks.strike,
    }
}

const MARK_KINDS: [MarkKind; 4] = [MarkKind::Link, MarkKind::Strong, MarkKind::Em, MarkKind::Strike];

fn same_group(kind: MarkKind, a: &Marks, b: &Marks) -> bool {
    has_mark(kind, a) == has_mark(kind, b) && (kind != MarkKind::Link || a.link == b.link)
}

fn cut_cost(kind: MarkKind, runs: &[(String, Marks)]) -> usize {
    runs.windows(2)
        .filter(|pair| !same_group(kind, &pair[0].1, &pair[1].1))
        .map(|pair| MARK_KINDS.iter()
            .filter(|other| **other != kind && has_mark(**other, &pair[0].1) && same_group(**other, &pair[0].1, &pair[1].1))
            .count())
        .sum()
}

fn without_mark(kind: MarkKind, marks: &Marks) -> Marks {
    let mut m = marks.clone();
    match kind {
        MarkKind::Link => m.link = None,
        MarkKind::Strong => m.strong = false,
        MarkKind::Em => m.em = false,
        MarkKind::Strike => m.strike = false,
    }
    m
}

/// Renders marked text runs, nesting shared marks so "**a *b* c**" comes out
/// as one bold span rather than three.
type Group = (bool, Option<String>, Vec<(String, Marks)>);

fn render_runs(runs: &[(String, Marks)], order: Option<&[MarkKind]>, smart: bool, before: Option<char>, after: Option<char>) -> String {
    let kinds: Vec<MarkKind> = match order {
        Some(order) => order.to_vec(),
        None => {
            let mut kinds = MARK_KINDS.to_vec();
            kinds.sort_by_key(|kind| cut_cost(*kind, runs));
            kinds
        }
    };
    for kind in kinds {
        if !runs.iter().any(|(_, m)| has_mark(kind, m)) {
            continue;
        }
        let mut groups: Vec<Group> = vec![];
        let mut i = 0;
        while i < runs.len() {
            let flag = has_mark(kind, &runs[i].1);
            let href = runs[i].1.link.clone();
            let mut j = i;
            while j < runs.len() && has_mark(kind, &runs[j].1) == flag && (kind != MarkKind::Link || runs[j].1.link == href) {
                j += 1;
            }
            groups.push((flag, href, runs[i..j].iter().map(|(t, m)| (t.clone(), without_mark(kind, m))).collect()));
            i = j;
        }
        let firsts: Vec<Option<char>> = groups.iter()
            .map(|(flag, _, group)| if *flag {
                Some(match kind { MarkKind::Link => '[', MarkKind::Strike => '~', _ => '*' })
            } else {
                render_runs(group, order, false, None, None).chars().next()
            })
            .collect();
        let mut out = String::new();
        for (k, (flag, href, group)) in groups.iter().enumerate() {
            let prev = out.chars().last().or(before);
            let next = firsts.get(k + 1).copied().unwrap_or(after);
            if *flag {
                let inner = render_runs(group, order, smart, Some('*'), Some('*'));
                let underscores = smart && underscores_for(kind, &inner, prev, next);
                out.push_str(&wrap_mark(kind, &inner, href.as_deref(), underscores));
            } else {
                out.push_str(&render_runs(group, order, smart, prev, next));
            }
        }
        return out;
    }
    runs.iter().map(|(text, marks)| {
        if marks.code {
            let runs: Vec<usize> = text.split(|c| c != '`').map(str::len).filter(|n| *n > 0).collect();
            let fence = "`".repeat((1..).find(|n| !runs.contains(n)).unwrap());
            let all_spaces = text.chars().all(|c| c == ' ');
            let pad = !all_spaces && (text.starts_with('`') || text.ends_with('`') || (text.starts_with(' ') && text.ends_with(' ')));
            let pad = if pad { " " } else { "" };
            format!("{fence}{pad}{text}{pad}{fence}")
        } else {
            escape_text(text)
        }
    }).collect()
}

fn underscores_for(kind: MarkKind, inner: &str, before: Option<char>, after: Option<char>) -> bool {
    if !matches!(kind, MarkKind::Em | MarkKind::Strong) {
        return false;
    }
    let trimmed = inner.trim();
    let abuts = |d: char| before == Some(d) || after == Some(d) || trimmed.starts_with(d) || trimmed.ends_with(d);
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    abuts('*') && !abuts('_') && !word(before) && !word(after)
}

/// Wraps rendered text in a mark, keeping surrounding whitespace outside the
/// delimiters so the result parses back.
fn wrap_mark(kind: MarkKind, text: &str, href: Option<&str>, underscores: bool) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return text.to_string();
    }
    let lead = &text[..text.len() - text.trim_start().len()];
    let trail = &text[text.trim_end().len()..];
    let body = match kind {
        MarkKind::Link => format!("[{trimmed}]({})", escape_href(href.unwrap_or(""))),
        MarkKind::Strong if underscores => format!("__{trimmed}__"),
        MarkKind::Strong => format!("**{trimmed}**"),
        MarkKind::Em if underscores => format!("_{trimmed}_"),
        MarkKind::Em => format!("*{trimmed}*"),
        MarkKind::Strike => format!("~~{trimmed}~~"),
    };
    format!("{lead}{body}{trail}")
}

/// Escapes the characters that would otherwise be read as inline markdown.
fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        match c {
            '\\' | '*' | '`' | '[' | ']' => { out.push('\\'); out.push(*c); }
            '<' if chars.get(i + 1).is_some_and(|b| b.eq_ignore_ascii_case(&'b')) && chars.get(i + 2).is_some_and(|r| r.eq_ignore_ascii_case(&'r')) => { out.push('\\'); out.push('<'); }
            '~' if chars.get(i + 1) == Some(&'~') || (i > 0 && chars[i - 1] == '~') => { out.push('\\'); out.push('~'); }
            '_' => {
                let word_before = i > 0 && chars[i - 1].is_alphanumeric();
                let word_after = chars.get(i + 1).is_some_and(|n| n.is_alphanumeric());
                if word_before && word_after { out.push('_'); } else { out.push('\\'); out.push('_'); }
            }
            _ => out.push(*c),
        }
    }
    out
}

/// Escapes a line that would otherwise start a block (heading, list, quote, rule, table, fence).
fn escape_line_starts(text: &str) -> String {
    text.split('\n').map(|line| {
        if !starts_block(line) {
            return line.to_string();
        }
        // The backslash goes before the first punctuation character of the marker
        // ("1\. item"), since a backslash before a digit is not an escape.
        match line.char_indices().find(|(_, c)| c.is_ascii_punctuation()) {
            Some((at, _)) => format!("{}\\{}", &line[..at], &line[at..]),
            None => line.to_string(),
        }
    }).collect::<Vec<_>>().join("\n")
}

fn starts_block(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('#') || t.starts_with('>') || t.starts_with('|') || opening_fence(t).is_some()
        || list_marker(line).is_some() || is_rule(line)
}

// ---------------------------------------------------------------------------
// markdown -> ADF
// ---------------------------------------------------------------------------

fn parse_blocks(lines: &[&str]) -> Vec<Value> {
    let mut blocks = vec![];
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        if trimmed.is_empty() {
            i += 1;
        } else if let Some(fence) = opening_fence(trimmed) {
            let language = trimmed[fence..].trim().to_string();
            let mut code = vec![];
            i += 1;
            while i < lines.len() && !closes_fence(lines[i], fence) {
                code.push(lines[i]);
                i += 1;
            }
            i += 1; // closing fence (or end of input)
            let mut node = json!({"type": "codeBlock", "content": text_nodes(&code.join("\n"))});
            if !language.is_empty() {
                node["attrs"] = json!({"language": language});
            }
            blocks.push(node);
        } else if let Some((level, text)) = heading(trimmed) {
            blocks.push(json!({"type": "heading", "attrs": {"level": level}, "content": parse_inline(text)}));
            i += 1;
        } else if is_rule(line) {
            blocks.push(json!({"type": "rule"}));
            i += 1;
        } else if trimmed.starts_with('>') {
            let mut inner = vec![];
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let l = lines[i].trim_start()[1..].strip_prefix(' ').unwrap_or(&lines[i].trim_start()[1..]);
                inner.push(l);
                i += 1;
            }
            blocks.push(json!({"type": "blockquote", "content": parse_blocks(&inner)}));
        } else if trimmed.starts_with('|') && i + 1 < lines.len() && is_table_separator(lines[i + 1]) {
            let mut rows = vec![];
            rows.push(table_cells(lines[i]));
            i += 2;
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                rows.push(table_cells(lines[i]));
                i += 1;
            }
            blocks.push(table_node(rows));
        } else if list_marker(line).is_some() {
            let (node, consumed) = parse_list(&lines[i..]);
            blocks.push(node);
            i += consumed;
        } else {
            let mut para = vec![];
            while i < lines.len() && !lines[i].trim().is_empty() && !interrupts_paragraph(lines[i]) {
                para.push(lines[i].trim_end());
                i += 1;
            }
            if para.is_empty() {
                // A block start we could not parse; treat the line as text and move on.
                para.push(lines[i].trim_end());
                i += 1;
            }
            blocks.push(json!({"type": "paragraph", "content": parse_inline(&para.join("\n"))}));
        }
    }
    blocks
}

fn leading_backticks(line: &str) -> usize {
    line.chars().take_while(|c| *c == '`').count()
}

/// Length of the fence if the (left-trimmed) line opens a fenced code block.
fn opening_fence(trimmed: &str) -> Option<usize> {
    let fence = leading_backticks(trimmed);
    // As in CommonMark, the info string of a backtick fence cannot contain backticks,
    // so a line such as "```a ` b```" holds an inline code span, not a block.
    (fence >= 3 && !trimmed[fence..].contains('`')).then_some(fence)
}

/// A closing fence has at least as many backticks as the opening one and nothing else.
fn closes_fence(line: &str, fence: usize) -> bool {
    let t = line.trim();
    leading_backticks(t) >= fence && t.chars().all(|c| c == '`')
}

fn interrupts_paragraph(line: &str) -> bool {
    let t = line.trim_start();
    opening_fence(t).is_some() || heading(t).is_some() || is_rule(line) || t.starts_with('>') || list_marker(line).is_some()
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    if rest.is_empty() {
        Some((hashes, ""))
    } else {
        rest.strip_prefix(' ').map(|t| (hashes, t.trim()))
    }
}

fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*') || t.chars().all(|c| c == '_'))
}

fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.len() > 1 && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) && t.contains('-')
}

fn cell_blocks(cell: &str) -> Vec<Value> {
    let text = decode_cell(cell);
    let lines: Vec<&str> = text.split('\n').collect();
    let mut blocks = parse_blocks(&lines);
    if blocks.is_empty() {
        blocks.push(json!({"type": "paragraph", "content": []}));
    }
    blocks
}

fn encode_cell(text: &str) -> String {
    let mut lines = vec![];
    let mut fence: Option<usize> = None;
    for line in text.split('\n') {
        match fence {
            Some(len) => {
                lines.push(escape_br(line));
                if closes_fence(line, len) {
                    fence = None;
                }
            }
            None => match opening_fence(line.trim_start()) {
                Some(len) => {
                    fence = Some(len);
                    lines.push(escape_br(line));
                }
                None => lines.push(escape_br_outside_code(line)),
            },
        }
    }
    lines.join("<br>")
}

fn escape_br(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("<br") {
        out.push_str(&rest[..pos]);
        out.push_str("<br");
        rest = &rest[pos + 3..];
        let slashes = rest.chars().take_while(|c| *c == '/').count();
        if rest[slashes..].starts_with('>') {
            out.push_str(&"/".repeat(slashes + 1));
            out.push('>');
            rest = &rest[slashes + 1..];
        }
    }
    out.push_str(rest);
    out
}

fn escape_br_outside_code(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut segment = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            let run = chars[i..].iter().take_while(|c| **c == '`').count();
            if let Some(end) = find_backtick_run(&chars, i + run, run) {
                out.push_str(&escape_br(&segment));
                segment.clear();
                out.extend(&chars[i..end + run]);
                i = end + run;
                continue;
            }
        }
        segment.push(chars[i]);
        i += 1;
    }
    out.push_str(&escape_br(&segment));
    out
}

fn unescape_br(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("<br") {
        out.push_str(&rest[..pos]);
        rest = &rest[pos + 3..];
        let slashes = rest.chars().take_while(|c| *c == '/').count();
        if slashes > 0 && rest[slashes..].starts_with('>') {
            out.push_str("<br");
            out.push_str(&"/".repeat(slashes - 1));
            out.push('>');
            rest = &rest[slashes + 1..];
        } else {
            out.push_str("<br");
        }
    }
    out.push_str(rest);
    out
}

fn decode_cell(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let br = ['<', 'b', 'r', '>'];
    let mut out = String::with_capacity(text.len());
    let mut fence: Option<usize> = None;
    let mut line_start = true;
    let mut i = 0;
    while i < chars.len() {
        if line_start {
            line_start = false;
            let line_end = (i..chars.len()).find(|&j| chars[j..].starts_with(&br)).unwrap_or(chars.len());
            let line: String = chars[i..line_end].iter().collect();
            match fence {
                Some(len) if closes_fence(&line, len) => fence = None,
                None => fence = opening_fence(line.trim_start()),
                _ => {}
            }
            if fence.is_some() {
                out.push_str(&unescape_br(&line));
                i = line_end;
                continue;
            }
        }
        if chars[i] == '`' {
            let run = chars[i..].iter().take_while(|c| **c == '`').count();
            let end = find_backtick_run(&chars, i + run, run).map(|end| end + run).unwrap_or(i + run);
            out.extend(&chars[i..end]);
            i = end;
            continue;
        }
        if chars[i..].starts_with(&['<', 'b', 'r']) {
            let slashes = chars[i + 3..].iter().take_while(|c| **c == '/').count();
            if chars.get(i + 3 + slashes) == Some(&'>') {
                if slashes == 0 {
                    out.push('\n');
                    line_start = true;
                } else {
                    out.push_str("<br");
                    out.push_str(&"/".repeat(slashes - 1));
                    out.push('>');
                }
                i += 4 + slashes;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn table_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let mut cells = vec![];
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => { cur.push('|'); chars.next(); }
            '|' => { cells.push(cur.trim().to_string()); cur = String::new(); }
            _ => cur.push(c),
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

fn table_node(rows: Vec<Vec<String>>) -> Value {
    let row = |cells: &Vec<String>, kind: &str| json!({
        "type": "tableRow",
        "content": cells.iter().map(|c| json!({
            "type": kind,
            "attrs": {},
            "content": cell_blocks(c)
        })).collect::<Vec<_>>()
    });
    let mut content = vec![row(&rows[0], "tableHeader")];
    content.extend(rows[1..].iter().map(|r| row(r, "tableCell")));
    json!({"type": "table", "attrs": {"isNumberColumnEnabled": false, "layout": "default"}, "content": content})
}

#[derive(Clone, Copy, PartialEq)]
enum ListKind {
    Bullet,
    Ordered(u64),
    Task,
}

/// Returns (indent, kind, content offset, task done) when the line starts a list item.
fn list_marker(line: &str) -> Option<(usize, ListKind, usize, bool)> {
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];
    let (kind, marker_len) = if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ") {
        (ListKind::Bullet, 2)
    } else if matches!(rest, "-" | "*" | "+") {
        // An empty item: the marker alone, as the renderer writes it.
        (ListKind::Bullet, 1)
    } else {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let after = &rest[digits..];
        let marker_len = if after.starts_with(". ") || after.starts_with(") ") {
            digits + 2
        } else if after == "." || after == ")" {
            digits + 1
        } else {
            return None;
        };
        (ListKind::Ordered(rest[..digits].parse().unwrap_or(1)), marker_len)
    };
    let after = &rest[marker_len..];
    if kind == ListKind::Bullet {
        if let Some(t) = after.strip_prefix("[ ] ").or_else(|| after.strip_prefix("[ ]").filter(|t| t.is_empty())) {
            return Some((indent, ListKind::Task, rest.len() - t.len(), false));
        }
        let done = after.strip_prefix("[x] ").or_else(|| after.strip_prefix("[X] "))
            .or_else(|| after.strip_prefix("[x]").or_else(|| after.strip_prefix("[X]")).filter(|t| t.is_empty()));
        if let Some(t) = done {
            return Some((indent, ListKind::Task, rest.len() - t.len(), true));
        }
    }
    Some((indent, kind, marker_len, false))
}

fn parse_list(lines: &[&str]) -> (Value, usize) {
    let (base_indent, kind, _, _) = list_marker(lines[0]).unwrap();
    let mut items: Vec<(Vec<String>, bool)> = vec![];
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        match list_marker(line) {
            Some((indent, item_kind, offset, done)) if indent <= base_indent && same_kind(item_kind, kind) => {
                if indent < base_indent {
                    break;
                }
                items.push((vec![line[indent + offset..].to_string()], done));
                i += 1;
                // Continuation lines: anything indented past the marker, or blank lines followed by such.
                while i < lines.len() {
                    let l = lines[i];
                    let l_indent = l.len() - l.trim_start().len();
                    if l.trim().is_empty() {
                        // Blank lines, however many, belong to the item when the text
                        // after them is still indented (a code block may hold several).
                        let mut after = i;
                        while after < lines.len() && lines[after].trim().is_empty() {
                            after += 1;
                        }
                        let continues = lines.get(after).is_some_and(|n| n.len() - n.trim_start().len() > base_indent);
                        if continues {
                            for blank in &lines[i..after] {
                                let strip = indent_boundary(blank, indent + offset);
                                items.last_mut().unwrap().0.push(blank[strip..].to_string());
                            }
                            i = after;
                            continue;
                        }
                        break;
                    }
                    if l_indent > base_indent {
                        let strip = indent_boundary(l, indent + offset);
                        items.last_mut().unwrap().0.push(l[strip..].to_string());
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            _ => break,
        }
    }

    let item_nodes: Vec<Value> = items.iter().map(|(body, done)| {
        let body_refs: Vec<&str> = body.iter().map(String::as_str).collect();
        if kind == ListKind::Task {
            json!({
                "type": "taskItem",
                "attrs": {"localId": next_local_id(), "state": if *done { "DONE" } else { "TODO" }},
                "content": parse_inline(body_refs.join("\n").trim())
            })
        } else {
            let mut content = parse_blocks(&body_refs);
            if content.is_empty() {
                content.push(json!({"type": "paragraph", "content": []}));
            }
            json!({"type": "listItem", "content": content})
        }
    }).collect();

    let node = match kind {
        ListKind::Bullet => json!({"type": "bulletList", "content": item_nodes}),
        ListKind::Ordered(start) => {
            let mut n = json!({"type": "orderedList", "content": item_nodes});
            if start != 1 {
                n["attrs"] = json!({"order": start});
            }
            n
        }
        ListKind::Task => json!({"type": "taskList", "attrs": {"localId": next_local_id()}, "content": item_nodes}),
    };
    (node, i)
}

/// Byte index just past the leading whitespace of `line`, stopping once `max_bytes` is
/// reached. Whitespace characters are consumed whole, so the result is always a char
/// boundary even for multi-byte whitespace such as U+2003 (em space).
fn indent_boundary(line: &str, max_bytes: usize) -> usize {
    let mut end = 0;
    for (idx, ch) in line.char_indices() {
        if !ch.is_whitespace() || idx >= max_bytes {
            break;
        }
        end = idx + ch.len_utf8();
    }
    end
}

fn same_kind(a: ListKind, b: ListKind) -> bool {
    matches!((a, b), (ListKind::Bullet, ListKind::Bullet) | (ListKind::Ordered(_), ListKind::Ordered(_)) | (ListKind::Task, ListKind::Task))
}

fn next_local_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    format!("git-task-{}", COUNTER.fetch_add(1, Ordering::SeqCst))
}

fn text_nodes(text: &str) -> Vec<Value> {
    if text.is_empty() { vec![] } else { vec![json!({"type": "text", "text": text})] }
}

/// Parses inline markdown into ADF text and hardBreak nodes.
fn parse_inline(text: &str) -> Vec<Value> {
    let chars: Vec<char> = text.chars().collect();
    let mut nodes = vec![];
    parse_inline_into(&chars, &Marks { code: false, strong: false, em: false, strike: false, link: None }, &mut nodes);
    nodes
}

fn parse_inline_into(chars: &[char], marks: &Marks, nodes: &mut Vec<Value>) {
    let mut buf: Option<(String, Marks)> = None;
    for piece in resolve_inline(chars, marks) {
        let (text, marks) = match piece {
            Piece::Break => {
                flush_run(&mut buf, nodes);
                nodes.push(json!({"type": "hardBreak"}));
                continue;
            }
            Piece::Run(text, marks) => (text, marks),
            Piece::Delim { ch, count, marks, .. } => (ch.to_string().repeat(count), marks),
        };
        if text.is_empty() {
            continue;
        }
        match &mut buf {
            Some((t, m)) if *m == marks => t.push_str(&text),
            _ => {
                flush_run(&mut buf, nodes);
                buf = Some((text, marks));
            }
        }
    }
    flush_run(&mut buf, nodes);
}

fn flush_run(buf: &mut Option<(String, Marks)>, nodes: &mut Vec<Value>) {
    if let Some((text, marks)) = buf.take() {
        push_text(nodes, text, &marks);
    }
}

#[derive(Debug)]
enum Piece {
    Run(String, Marks),
    Delim { ch: char, count: usize, orig: usize, can_open: bool, can_close: bool, marks: Marks },
    Break,
}

fn resolve_inline(chars: &[char], marks: &Marks) -> Vec<Piece> {
    let mut pieces = tokenize(chars, marks);
    process_emphasis(&mut pieces);
    pieces
}

fn tokenize(chars: &[char], marks: &Marks) -> Vec<Piece> {
    let mut pieces = vec![];
    let mut buf = String::new();
    let flush = |buf: &mut String, pieces: &mut Vec<Piece>| {
        if !buf.is_empty() {
            pieces.push(Piece::Run(std::mem::take(buf), marks.clone()));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            // As in CommonMark, a backslash escapes only ASCII punctuation;
            // elsewhere (paths, regexes) it is an ordinary character.
            '\\' if i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() => {
                buf.push(chars[i + 1]);
                i += 2;
            }
            '\n' => {
                flush(&mut buf, &mut pieces);
                pieces.push(Piece::Break);
                i += 1;
            }
            '`' => {
                let fence = chars[i..].iter().take_while(|c| **c == '`').count();
                if let Some(end) = find_backtick_run(chars, i + fence, fence) {
                    flush(&mut buf, &mut pieces);
                    let mut code: String = chars[i + fence..end].iter().collect();
                    if code.len() >= 2 && code.starts_with(' ') && code.ends_with(' ') && !code.chars().all(|c| c == ' ') {
                        code = code[1..code.len() - 1].to_string();
                    }
                    pieces.push(Piece::Run(code, Marks { code: true, ..marks.clone() }));
                    i = end + fence;
                } else {
                    buf.extend(std::iter::repeat_n('`', fence));
                    i += fence;
                }
            }
            '[' => {
                if let Some((label_end, href_end)) = find_link(chars, i) {
                    flush(&mut buf, &mut pieces);
                    let href = unescape_href(&chars[label_end + 2..href_end]);
                    let inner = Marks { link: Some(href), ..marks.clone() };
                    pieces.extend(resolve_inline(&chars[i + 1..label_end], &inner).into_iter().map(|piece| match piece {
                        Piece::Delim { ch, count, marks, .. } => Piece::Run(ch.to_string().repeat(count), marks),
                        other => other,
                    }));
                    i = href_end + 1;
                } else {
                    buf.push(c);
                    i += 1;
                }
            }
            '<' if br_tag_len(&chars[i..]).is_some() => {
                flush(&mut buf, &mut pieces);
                pieces.push(Piece::Break);
                i += br_tag_len(&chars[i..]).unwrap_or(1);
            }
            '*' | '_' | '~' => {
                flush(&mut buf, &mut pieces);
                let count = chars[i..].iter().take_while(|x| **x == c).count();
                let before = if i == 0 { None } else { Some(chars[i - 1]) };
                let after = chars.get(i + count).copied();
                let (can_open, can_close) = flanking(c, count, before, after);
                pieces.push(Piece::Delim { ch: c, count, orig: count, can_open, can_close, marks: marks.clone() });
                i += count;
            }
            _ => {
                buf.push(c);
                i += 1;
            }
        }
    }
    flush(&mut buf, &mut pieces);
    pieces
}

fn br_tag_len(chars: &[char]) -> Option<usize> {
    ["<br>", "<br/>", "<br />"].into_iter()
        .find(|tag| chars.len() >= tag.len() && chars.iter().zip(tag.chars()).all(|(a, b)| a.eq_ignore_ascii_case(&b)))
        .map(str::len)
}

fn flanking(ch: char, count: usize, before: Option<char>, after: Option<char>) -> (bool, bool) {
    let space = |c: Option<char>| c.is_none_or(char::is_whitespace);
    let punct = |c: Option<char>| c.is_some_and(|c| !c.is_alphanumeric() && !c.is_whitespace());
    let left = !space(after);
    let right = !space(before);
    match ch {
        '_' => (left && (!right || punct(before)), right && (!left || punct(after))),
        '~' => (left && count == 2, right && count == 2),
        _ => (left, right),
    }
}

fn process_emphasis(pieces: &mut [Piece]) {
    let mut closer = 0;
    while closer < pieces.len() {
        let Piece::Delim { ch, count: closer_count, orig: closer_orig, can_open: closer_can_open, can_close: true, .. } = pieces[closer] else {
            closer += 1;
            continue;
        };
        if closer_count == 0 {
            closer += 1;
            continue;
        }
        let opener = (0..closer).rev().find(|&k| match pieces[k] {
            Piece::Delim { ch: och, count, orig, can_open: true, can_close: ocan_close, .. } if och == ch && count > 0 => {
                if ch == '~' {
                    count == 2 && closer_count == 2
                } else {
                    !((ocan_close || closer_can_open) && (orig + closer_orig).is_multiple_of(3) && !(orig.is_multiple_of(3) && closer_orig.is_multiple_of(3)))
                }
            }
            _ => false,
        });
        let Some(opener) = opener else {
            closer += 1;
            continue;
        };
        let opener_count = match pieces[opener] { Piece::Delim { count, .. } => count, _ => 0 };
        let used = if ch == '~' || (opener_count >= 2 && closer_count >= 2) { 2 } else { 1 };
        for piece in &mut pieces[opener + 1..closer] {
            match piece {
                Piece::Run(_, m) => apply_emphasis(m, ch, used),
                Piece::Delim { marks, can_open, can_close, .. } => {
                    apply_emphasis(marks, ch, used);
                    *can_open = false;
                    *can_close = false;
                }
                Piece::Break => {}
            }
        }
        if let Piece::Delim { count, can_open, .. } = &mut pieces[opener] {
            *count -= used;
            if *count == 0 {
                *can_open = false;
            }
        }
        if let Piece::Delim { count, can_close, .. } = &mut pieces[closer] {
            *count -= used;
            if *count == 0 {
                *can_close = false;
                closer += 1;
            }
        }
    }
}

fn apply_emphasis(marks: &mut Marks, ch: char, used: usize) {
    match (ch, used) {
        ('~', _) => marks.strike = true,
        (_, 2) => marks.strong = true,
        _ => marks.em = true,
    }
}

fn find_backtick_run(chars: &[char], from: usize, length: usize) -> Option<usize> {
    let mut j = from;
    while j < chars.len() {
        if chars[j] == '`' {
            let run = chars[j..].iter().take_while(|c| **c == '`').count();
            if run == length {
                return Some(j);
            }
            j += run;
        } else {
            j += 1;
        }
    }
    None
}

/// For a '[' at `start`, returns (index of ']', index of ')') of a `[label](href)` link.
fn find_link(chars: &[char], start: usize) -> Option<(usize, usize)> {
    let mut depth = 0;
    let mut j = start;
    while j < chars.len() {
        match chars[j] {
            '\\' => j += 1,
            '`' => {
                let fence = chars[j..].iter().take_while(|c| **c == '`').count();
                if let Some(end) = find_backtick_run(chars, j + fence, fence) {
                    j = end + fence;
                    continue;
                }
            }
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            '\n' => return None,
            _ => {}
        }
        j += 1;
    }
    if j >= chars.len() || chars.get(j + 1) != Some(&'(') {
        return None;
    }
    let mut depth = 0;
    let mut k = j + 2;
    while k < chars.len() {
        match chars[k] {
            '\\' if k + 1 < chars.len() && chars[k + 1].is_ascii_punctuation() => k += 1,
            '(' => depth += 1,
            ')' if depth == 0 => return Some((j, k)),
            ')' => depth -= 1,
            c if c.is_whitespace() => return None,
            _ => {}
        }
        k += 1;
    }
    None
}

fn unescape_href(chars: &[char]) -> String {
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() {
            i += 1;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn escape_href(href: &str) -> String {
    let mut depth: i64 = 0;
    let balanced = href.chars().all(|c| {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        depth >= 0
    }) && depth == 0;
    let mut out = String::with_capacity(href.len());
    for c in href.chars() {
        if c == '\\' || (!balanced && matches!(c, '(' | ')')) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn push_text(nodes: &mut Vec<Value>, text: String, marks: &Marks) {
    let mut list = vec![];
    if marks.code { list.push(json!({"type": "code"})); }
    if marks.strong { list.push(json!({"type": "strong"})); }
    if marks.em { list.push(json!({"type": "em"})); }
    if marks.strike { list.push(json!({"type": "strike"})); }
    if let Some(href) = &marks.link { list.push(json!({"type": "link", "attrs": {"href": href}})); }
    let mut node = json!({"type": "text", "text": text});
    if !list.is_empty() {
        node["marks"] = Value::Array(list);
    }
    nodes.push(node);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Value { json!({"type": "text", "text": t}) }
    fn para(content: Vec<Value>) -> Value { json!({"type": "paragraph", "content": content}) }
    fn doc(content: Vec<Value>) -> Value { json!({"type": "doc", "version": 1, "content": content}) }

    fn round_trip(markdown: &str) {
        let adf = markdown_to_adf(markdown);
        let back = adf_to_markdown(&adf);
        assert_eq!(back, markdown, "markdown must survive a round trip through ADF\nADF: {adf}");
    }

    #[test]
    fn test_plain_paragraphs() {
        let adf = doc(vec![para(vec![text("First")]), para(vec![text("Second")])]);
        assert_eq!(adf_to_markdown(&adf), "First\n\nSecond");
        assert_eq!(markdown_to_adf("First\n\nSecond"), adf);
    }

    #[test]
    fn test_hard_breaks_inside_a_paragraph() {
        let adf = doc(vec![para(vec![text("line one"), json!({"type": "hardBreak"}), text("line two")])]);
        assert_eq!(adf_to_markdown(&adf), "line one\nline two");
        assert_eq!(markdown_to_adf("line one\nline two"), adf);
    }

    #[test]
    fn test_empty_description() {
        assert_eq!(adf_to_markdown(&markdown_to_adf("")), "");
        assert_eq!(markdown_to_adf(""), doc(vec![json!({"type": "paragraph", "content": []})]));
        assert_eq!(adf_to_markdown(&json!(null)), "");
    }

    #[test]
    fn test_headings_lists_code_quotes_and_rules_survive() {
        round_trip("# Title\n\nIntro paragraph.\n\n## Steps\n\n1. First\n2. Second\n\n- bullet\n- another\n\n```rust\nfn main() {}\n```\n\n> quoted\n> text\n\n---\n\nThe end.");
    }

    #[test]
    fn test_nested_lists_and_multi_paragraph_items() {
        let md = "- parent\n  - child\n  - child two\n- second parent\n\n  continuation paragraph\n\n1. one\n   1. nested one\n2. two";
        round_trip(md);
        let adf = markdown_to_adf(md);
        let first_item = &adf["content"][0]["content"][0];
        assert_eq!(first_item["type"], "listItem");
        assert_eq!(first_item["content"][0], para(vec![text("parent")]));
        assert_eq!(first_item["content"][1]["type"], "bulletList");
        assert_eq!(adf["content"][0]["content"][1]["content"].as_array().unwrap().len(), 2, "the second item keeps its extra paragraph");
    }

    #[test]
    fn test_task_lists() {
        let md = "- [ ] todo\n- [x] done";
        round_trip(md);
        let adf = markdown_to_adf(md);
        assert_eq!(adf["content"][0]["type"], "taskList");
        assert_eq!(adf["content"][0]["content"][0]["attrs"]["state"], "TODO");
        assert_eq!(adf["content"][0]["content"][1]["attrs"]["state"], "DONE");
    }

    #[test]
    fn test_inline_marks_and_links() {
        let md = "Some **bold**, *italic*, ~~gone~~, `code` and a [link](https://example.com/a?b=c).";
        round_trip(md);
        let adf = markdown_to_adf(md);
        let content = adf["content"][0]["content"].as_array().unwrap();
        assert_eq!(content[1], json!({"type": "text", "text": "bold", "marks": [{"type": "strong"}]}));
        assert_eq!(content[3], json!({"type": "text", "text": "italic", "marks": [{"type": "em"}]}));
        assert_eq!(content[5], json!({"type": "text", "text": "gone", "marks": [{"type": "strike"}]}));
        assert_eq!(content[7], json!({"type": "text", "text": "code", "marks": [{"type": "code"}]}));
        assert_eq!(content[9], json!({"type": "text", "text": "link", "marks": [{"type": "link", "attrs": {"href": "https://example.com/a?b=c"}}]}));
    }

    #[test]
    fn test_combined_marks_nest() {
        round_trip("**bold *and italic* inside** and [**bold link**](https://x.y)");
        let adf = markdown_to_adf("***both***");
        let node = &adf["content"][0]["content"][0];
        assert_eq!(node["text"], "both");
        let marks = node["marks"].as_array().unwrap();
        assert!(marks.iter().any(|m| m["type"] == "strong") && marks.iter().any(|m| m["type"] == "em"));
    }

    #[test]
    fn test_underscores_inside_words_are_literal() {
        let adf = markdown_to_adf("snake_case_name and _emphasis_");
        let content = adf["content"][0]["content"].as_array().unwrap();
        assert_eq!(content[0], text("snake_case_name and "));
        assert_eq!(content[1], json!({"type": "text", "text": "emphasis", "marks": [{"type": "em"}]}));
        assert_eq!(adf_to_markdown(&adf), "snake_case_name and *emphasis*");
    }

    #[test]
    fn test_unbalanced_markers_stay_literal() {
        let adf = markdown_to_adf("2 * 3 * 4 and a lone ` tick and [not a link]");
        assert_eq!(adf["content"][0]["content"][0], text("2 * 3 * 4 and a lone ` tick and [not a link]"));
    }

    #[test]
    fn test_special_characters_in_remote_text_are_escaped_and_restored() {
        // Plain Jira text that merely looks like markdown must come back unchanged.
        let original = "price * qty, 2 ** 8, use `ls`, [tag] and a\\backslash";
        let adf = doc(vec![para(vec![text(original)])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(markdown_to_adf(&md), adf, "escaping must make the text parse back to the same ADF");
    }

    #[test]
    fn test_lines_that_look_like_blocks_are_escaped() {
        for original in ["# not a heading", "- not a list", "> not a quote", "1. not ordered", "---", "| not | table |"] {
            let adf = doc(vec![para(vec![text(original)])]);
            let md = adf_to_markdown(&adf);
            assert_eq!(markdown_to_adf(&md), adf, "{original:?} must survive the round trip");
        }
    }

    #[test]
    fn test_tables() {
        let md = "| Name | Value |\n| --- | --- |\n| a | 1 |\n| b | 2 |";
        round_trip(md);
        let adf = markdown_to_adf(md);
        assert_eq!(adf["content"][0]["type"], "table");
        assert_eq!(adf["content"][0]["content"][0]["content"][0]["type"], "tableHeader");
        assert_eq!(adf["content"][0]["content"][1]["content"][1]["type"], "tableCell");
        assert_eq!(adf["content"][0]["content"][1]["content"][1]["content"][0], para(vec![text("1")]));
    }

    #[test]
    fn test_jira_specific_nodes_degrade_to_text() {
        let adf = doc(vec![
            para(vec![
                json!({"type": "mention", "attrs": {"id": "1", "text": "@Jane"}}),
                text(" said "),
                json!({"type": "emoji", "attrs": {"shortName": ":tada:"}}),
                text(" see "),
                json!({"type": "inlineCard", "attrs": {"url": "https://example.com"}}),
            ]),
            json!({"type": "panel", "attrs": {"panelType": "info"}, "content": [para(vec![text("inside panel")])]}),
            json!({"type": "mediaSingle", "content": [json!({"type": "media", "attrs": {"id": "x"}})]}),
            json!({"type": "expand", "attrs": {"title": "More"}, "content": [para(vec![text("hidden")])]}),
        ]);
        assert_eq!(adf_to_markdown(&adf), "@Jane said :tada: see https://example.com\n\n> inside panel\n\n**More**\n\nhidden");
    }

    #[test]
    fn test_legacy_flat_output_is_still_readable() {
        // Descriptions pulled by older versions stored paragraphs separated by one newline.
        let adf = markdown_to_adf("one\ntwo");
        assert_eq!(adf["content"].as_array().unwrap().len(), 1);
        assert_eq!(adf_to_markdown(&adf), "one\ntwo");
    }

    #[test]
    fn test_code_block_keeps_markdown_looking_content_verbatim() {
        round_trip("```\n# not a heading\n- not a list\n**not bold**\n```");
    }

    #[test]
    fn test_backslashes_before_ordinary_characters_are_literal() {
        // Paths and regexes typed by a user are not markdown escapes.
        for original in [r"C:\Users\alice\logs", r"\d+\s+", r"a\ b", r"trailing\"] {
            let adf = markdown_to_adf(original);
            assert_eq!(adf["content"][0]["content"][0], text(original), "{original:?} must keep its backslashes");
        }
    }

    #[test]
    fn test_escaped_punctuation_is_still_unescaped() {
        let adf = markdown_to_adf(r"\*not bold\* and \\ one backslash");
        assert_eq!(adf["content"][0]["content"][0], text(r"*not bold* and \ one backslash"));
    }

    #[test]
    fn test_remote_backslashes_survive_the_round_trip() {
        for original in [r"C:\Users\alice", r"\d+\s+", r"a\*b", r"\\server\share"] {
            let adf = doc(vec![para(vec![text(original)])]);
            assert_eq!(markdown_to_adf(&adf_to_markdown(&adf)), adf, "{original:?} must survive the round trip");
        }
    }

    #[test]
    fn test_code_block_containing_fences_survives_the_round_trip() {
        let code = "Use a fence:\n```sh\nls\n```\nand a longer one:\n````\nx\n````";
        let adf = doc(vec![json!({"type": "codeBlock", "attrs": {"language": "markdown"}, "content": vec![text(code)]})]);

        let md = adf_to_markdown(&adf);
        let back = markdown_to_adf(&md);

        assert_eq!(back, adf, "the embedded fences must stay inside the code block\nmarkdown: {md}");
    }

    #[test]
    fn test_longer_fences_close_only_on_a_matching_fence() {
        let adf = markdown_to_adf("`````\n```\ncode\n```\n`````\n\nafter");
        let blocks = adf["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2, "{adf}");
        assert_eq!(blocks[0]["content"][0], text("```\ncode\n```"));
        assert_eq!(blocks[1], para(vec![text("after")]));
    }

    #[test]
    fn test_links_with_balanced_parentheses_in_the_href() {
        let md = "See [Function](https://en.wikipedia.org/wiki/Function_(math)) (really).";
        round_trip(md);
        let adf = markdown_to_adf(md);
        let content = adf["content"][0]["content"].as_array().unwrap();
        assert_eq!(content[1], json!({"type": "text", "text": "Function", "marks": [{"type": "link", "attrs": {"href": "https://en.wikipedia.org/wiki/Function_(math)"}}]}));
        assert_eq!(content[2], text(" (really)."));
    }

    #[test]
    fn test_remote_links_with_unbalanced_parentheses_survive_the_round_trip() {
        for href in ["https://example.com/a)b", "https://example.com/a(b", "https://example.com/x_(y))", "https://example.com/back\\slash"] {
            let link = json!({"type": "text", "text": "link", "marks": [{"type": "link", "attrs": {"href": href}}]});
            let adf = doc(vec![para(vec![text("see "), link, text(" now")])]);
            assert_eq!(markdown_to_adf(&adf_to_markdown(&adf)), adf, "{href:?} must survive the round trip");
        }
    }

    #[test]
    fn test_remote_inline_code_with_backticks_survives_the_round_trip() {
        for code in ["a``b", "a`b", "```", "`start", "end`", "`", " padded ", "a `` b ``` c"] {
            let span = json!({"type": "text", "text": code, "marks": [{"type": "code"}]});
            let adf = doc(vec![para(vec![text("use "), span, text(" here")])]);
            let md = adf_to_markdown(&adf);
            assert_eq!(markdown_to_adf(&md), adf, "{code:?} must survive the round trip\nmarkdown: {md}");
        }
    }

    #[test]
    fn test_code_span_closes_only_on_a_run_of_the_same_length() {
        let adf = markdown_to_adf("`a``b` and `` `x` `` done");
        let content = adf["content"][0]["content"].as_array().unwrap();
        assert_eq!(content[0], json!({"type": "text", "text": "a``b", "marks": [{"type": "code"}]}));
        assert_eq!(content[1], text(" and "));
        assert_eq!(content[2], json!({"type": "text", "text": "`x`", "marks": [{"type": "code"}]}));
        assert_eq!(content[3], text(" done"));
    }

    #[test]
    fn test_unicode_indented_continuation_line_does_not_panic() {
        // An em space (U+2003) is three bytes; stripping must never cut inside it.
        let md = "- item\n\u{2003}continued";
        let adf = markdown_to_adf(md);
        let item = &adf["content"][0]["content"][0];
        assert_eq!(item["type"], "listItem");
        assert_eq!(adf_to_markdown(&adf), "- item\n  continued");
        // Other multi-byte whitespace as well, mixed with ASCII.
        markdown_to_adf("1. one\n\u{3000}\u{3000}two\n\n\u{a0} three");
        markdown_to_adf("- a\n  - b\n\u{2003}\u{2003}\u{2003}c");
    }

    #[test]
    fn test_escaped_delimiters_inside_emphasis_are_literal() {
        let adf = doc(vec![para(vec![json!({"type": "text", "text": "a*b", "marks": [{"type": "em"}]})])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(md, r"*a\*b*");
        assert_eq!(markdown_to_adf(&md), adf, "an escaped asterisk must not close the emphasis");
        round_trip(&md);
        // Adding a paragraph (as a user editing the pulled description would) keeps the original intact.
        let edited = format!("{md}\n\nmore");
        assert_eq!(adf_to_markdown(&markdown_to_adf(&edited)), edited);
        // Same for strong, strike and underscores, and an escaped backslash right before the closer.
        for original in [r"**a\*\*b**", r"~~a\~\~b~~", r"*a\\*"] {
            round_trip(original);
        }
        assert_eq!(
            markdown_to_adf(r"_a\_b_")["content"][0]["content"][0],
            json!({"type": "text", "text": "a_b", "marks": [{"type": "em"}]})
        );
        assert_eq!(
            markdown_to_adf(r"*a\\*")["content"][0]["content"][0],
            json!({"type": "text", "text": r"a\", "marks": [{"type": "em"}]})
        );
    }

    fn marked(t: &str, marks: &[&str]) -> Value {
        json!({"type": "text", "text": t, "marks": marks.iter().map(|m| json!({"type": m})).collect::<Vec<_>>()})
    }

    fn mark_names(node: &Value) -> Vec<String> {
        let mut names: Vec<String> = node["marks"].as_array().map(|marks| marks.iter().filter_map(|m| m["type"].as_str().map(str::to_string)).collect()).unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn test_nested_strong_and_em_survive_the_round_trip() {
        let adf = doc(vec![para(vec![marked("one", &["strong"]), marked("two", &["strong", "em"])])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(md, "**one*two***");
        let content = &markdown_to_adf(&md)["content"][0]["content"];
        assert_eq!(content.as_array().unwrap().len(), 2, "{content}");
        assert_eq!((content[0]["text"].as_str(), mark_names(&content[0])), (Some("one"), vec!["strong".to_string()]));
        assert_eq!((content[1]["text"].as_str(), mark_names(&content[1])), (Some("two"), vec!["em".to_string(), "strong".to_string()]));

        let adf = doc(vec![para(vec![marked("one", &["em"]), marked("two", &["em", "strong"])])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(md, "*one**two***");
        let content = &markdown_to_adf(&md)["content"][0]["content"];
        assert_eq!((content[0]["text"].as_str(), mark_names(&content[0])), (Some("one"), vec!["em".to_string()]));
        assert_eq!((content[1]["text"].as_str(), mark_names(&content[1])), (Some("two"), vec!["em".to_string(), "strong".to_string()]));

        // Bold-italic at the start of an emphasized or bold span.
        let adf = doc(vec![para(vec![marked("a", &["em", "strong"]), marked(" b", &["em"])])]);
        assert_eq!(adf_to_markdown(&adf), "***a** b*");
        let content = &markdown_to_adf("***a** b*")["content"][0]["content"];
        assert_eq!((content[0]["text"].as_str(), mark_names(&content[0])), (Some("a"), vec!["em".to_string(), "strong".to_string()]));
        assert_eq!((content[1]["text"].as_str(), mark_names(&content[1])), (Some(" b"), vec!["em".to_string()]));
        let content = &markdown_to_adf("***a* b**")["content"][0]["content"];
        assert_eq!((content[0]["text"].as_str(), mark_names(&content[0])), (Some("a"), vec!["em".to_string(), "strong".to_string()]));
        assert_eq!((content[1]["text"].as_str(), mark_names(&content[1])), (Some(" b"), vec!["strong".to_string()]));

        for md in ["**one*two***", "*one**two***", "***a** b*", "***a* b**", "**a *b***", "***both***"] {
            round_trip(md);
        }
    }

    #[test]
    fn test_link_labels_with_brackets_survive_the_round_trip() {
        let adf = doc(vec![para(vec![json!({"type": "text", "text": "[docs]", "marks": [{"type": "link", "attrs": {"href": "https://example.com"}}]})])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(md, r"[\[docs\]](https://example.com)");
        let node = &markdown_to_adf(&md)["content"][0]["content"][0];
        assert_eq!(node["text"], "[docs]");
        assert_eq!(mark_names(node), vec!["link".to_string()]);
        round_trip(&md);
        round_trip(r"see \[not a link\] here");
    }

    #[test]
    fn test_inline_code_with_mixed_backticks_at_the_start_of_a_paragraph() {
        let adf = doc(vec![para(vec![marked("a ` b `` c", &["code"]), text(" tail")])]);
        let md = adf_to_markdown(&adf);
        assert!(md.starts_with("```"), "{md}");
        assert_eq!(markdown_to_adf(&md), adf, "a code span is not a fence");
        round_trip(&md);
        // A one-line span that looks like a fence is inline code, as in CommonMark.
        assert_eq!(markdown_to_adf("```foo```"), doc(vec![para(vec![marked("foo", &["code"])])]));
        // A real fence still opens a block.
        assert_eq!(markdown_to_adf("```rust\nfn x() {}\n```")["content"][0]["type"], "codeBlock");
    }

    #[test]
    fn test_empty_list_items_survive_the_round_trip() {
        let item = |content: Vec<Value>| json!({"type": "listItem", "content": content});
        let adf = doc(vec![json!({"type": "bulletList", "content": [item(vec![para(vec![text("a")])]), item(vec![para(vec![])])]})]);
        let md = adf_to_markdown(&adf);
        assert_eq!(md, "- a\n-");
        assert_eq!(markdown_to_adf(&md), adf);
        round_trip("1. a\n2.\n3. c");
        round_trip("-");
        // A paragraph consisting of a dash alone is escaped so it does not become a list.
        round_trip(r"\-");
        assert_eq!(adf_to_markdown(&doc(vec![para(vec![text("-")])])), r"\-");
    }

    /// The inline spans of the first paragraph: text and sorted mark names.
    fn spans(md: &str) -> Vec<(String, Vec<String>)> {
        markdown_to_adf(md)["content"][0]["content"].as_array().unwrap().iter()
            .map(|node| (node["text"].as_str().unwrap_or("").to_string(), mark_names(node)))
            .collect()
    }

    fn span(text: &str, marks: &[&str]) -> (String, Vec<String>) {
        let mut marks: Vec<String> = marks.iter().map(|m| m.to_string()).collect();
        marks.sort();
        (text.to_string(), marks)
    }

    #[test]
    fn test_three_adjacent_spans_with_mixed_marks_survive_the_round_trip() {
        let adf = doc(vec![para(vec![marked("a", &["strong"]), marked("b", &["strong", "em"]), marked("c", &["em"])])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(spans(&md), vec![span("a", &["strong"]), span("b", &["em", "strong"]), span("c", &["em"])], "{md}");

        let adf = doc(vec![para(vec![marked("a", &["em"]), marked("b", &["em", "strong"]), marked("c", &["strong"])])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(spans(&md), vec![span("a", &["em"]), span("b", &["em", "strong"]), span("c", &["strong"])], "{md}");

        for md in ["*one**two***", "**one*two***", "***a** b*", "***a* b**"] {
            round_trip(md);
        }
        assert_eq!(spans("*foo**"), vec![span("foo", &["em"]), span("*", &[])], "a surplus closing character stays literal");
    }

    #[test]
    fn test_formatting_characters_inside_urls_and_code_do_not_close_emphasis() {
        let linked = |t: &str, href: &str, mark: &str| json!({"type": "text", "text": t, "marks": [{"type": "link", "attrs": {"href": href}}, {"type": mark}]});
        for (mark, href) in [("em", "https://x.test/a*b"), ("strong", "https://x.test/a**b"), ("strike", "https://x.test/a~~b"), ("em", "https://x.test/_a_b_")] {
            let adf = doc(vec![para(vec![marked("see ", &[mark]), linked("here", href, mark)])]);
            let md = adf_to_markdown(&adf);
            let parsed = markdown_to_adf(&md);
            assert_eq!(adf_to_markdown(&parsed), md, "{md}");
            let content = &parsed["content"][0]["content"];
            assert_eq!(content.as_array().unwrap().len(), 2, "{md} => {content}");
            assert_eq!(mark_names(&content[0]), vec![mark.to_string()], "{md}");
            assert_eq!(mark_names(&content[1]), span("", &["link", mark]).1, "{md}");
            let link_href = content[1]["marks"].as_array().unwrap().iter().find_map(|m| m["attrs"]["href"].as_str());
            assert_eq!(link_href, Some(href), "{md}");
        }
        assert_eq!(spans("*a `*` b*"), vec![span("a ", &["em"]), span("*", &["code", "em"]), span(" b", &["em"])]);
        round_trip("*a `*` b*");
    }

    #[test]
    fn test_blank_lines_inside_a_nested_code_block_keep_the_list_together() {
        let md = "- item\n\n  ```\n  a\n\n\n  b\n  ```\n- next\n\nafter";
        let adf = markdown_to_adf(md);
        assert_eq!(adf["content"].as_array().unwrap().len(), 2, "a list and a paragraph: {adf}");
        let list = &adf["content"][0];
        assert_eq!(list["type"], "bulletList");
        assert_eq!(list["content"].as_array().unwrap().len(), 2, "both items stay in the list");
        let code = &list["content"][0]["content"][1];
        assert_eq!(code["type"], "codeBlock");
        assert_eq!(code["content"][0]["text"], "a\n\n\nb");
        assert_eq!(adf["content"][1], para(vec![text("after")]));
        round_trip(md);
    }

    #[test]
    fn test_empty_task_items_keep_their_state() {
        let md = "- [x]\n- [ ]\n- [x] done";
        let adf = markdown_to_adf(md);
        assert_eq!(adf["content"][0]["type"], "taskList");
        let states: Vec<&str> = adf["content"][0]["content"].as_array().unwrap().iter().map(|i| i["attrs"]["state"].as_str().unwrap()).collect();
        assert_eq!(states, ["DONE", "TODO", "DONE"]);
        round_trip(md);
        let done_empty = json!({"type": "taskList", "attrs": {"localId": "l"}, "content": [{"type": "taskItem", "attrs": {"localId": "i", "state": "DONE"}, "content": []}]});
        assert_eq!(adf_to_markdown(&doc(vec![done_empty])), "- [x]");
    }

    #[test]
    fn test_bold_inside_italic_survives_the_round_trip() {
        let adf = doc(vec![para(vec![marked("one", &["em"]), marked("two", &["em", "strong"]), marked("three", &["em"])])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(md, "*one**two**three*");
        assert_eq!(spans(&md), vec![span("one", &["em"]), span("two", &["em", "strong"]), span("three", &["em"])]);

        let adf = doc(vec![para(vec![marked("one", &["strong"]), marked("two", &["em", "strong"]), marked("three", &["strong"])])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(md, "**one*two*three**");
        assert_eq!(spans(&md), vec![span("one", &["strong"]), span("two", &["em", "strong"]), span("three", &["strong"])]);

        for md in ["*one**two**three*", "**one*two*three**", "*a**b** c*", "*a**b**c* and *d*"] {
            round_trip(md);
        }
        assert_eq!(spans("*a* and *b*"), vec![span("a", &["em"]), span(" and ", &[]), span("b", &["em"])]);
    }

    #[test]
    fn test_brackets_inside_linked_code_keep_the_link() {
        let adf = doc(vec![para(vec![json!({"type": "text", "text": "a]b", "marks": [{"type": "code"}, {"type": "link", "attrs": {"href": "https://x.test/"}}]})])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(md, "[`a]b`](https://x.test/)");
        let node = &markdown_to_adf(&md)["content"][0]["content"][0];
        assert_eq!(node["text"], "a]b");
        assert_eq!(mark_names(node), span("", &["code", "link"]).1);
        round_trip(&md);
        round_trip("[`x[0]`](https://x.test/) and [`]`](https://y.test/)");
    }

    #[test]
    fn test_whitespace_only_lines_inside_a_nested_code_block_are_kept() {
        let code = json!({"type": "codeBlock", "attrs": {"language": "python"}, "content": [text("s = \"\"\"\n    \nx\"\"\"")]});
        let adf = doc(vec![json!({"type": "bulletList", "content": [{"type": "listItem", "content": [para(vec![text("item")]), code]}]})]);
        let md = adf_to_markdown(&adf);
        assert_eq!(md, "- item\n\n  ```python\n  s = \"\"\"\n      \n  x\"\"\"\n  ```");
        assert_eq!(markdown_to_adf(&md), adf, "the four-space line must come back as four spaces");
        round_trip(md.as_str());
        assert_eq!(markdown_to_adf("- a\n \n  b")["content"][0]["content"][0]["content"].as_array().unwrap().len(), 2);
    }

    fn merged_spans(nodes: &[Value]) -> Vec<(String, Vec<String>)> {
        let mut out: Vec<(String, Vec<String>)> = vec![];
        for node in nodes {
            let (t, m) = (node["text"].as_str().unwrap_or("").to_string(), mark_names(node));
            match out.last_mut() {
                Some((text, marks)) if *marks == m => text.push_str(&t),
                _ => out.push((t, m)),
            }
        }
        out
    }

    #[test]
    fn test_bold_italic_bold_sequence_survives_the_round_trip() {
        let adf = doc(vec![para(vec![marked("one", &["strong"]), marked("two", &["em"]), marked("three", &["strong"])])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(spans(&md), vec![span("one", &["strong"]), span("two", &["em"]), span("three", &["strong"])], "{md}");
        round_trip(&md);
    }

    #[test]
    fn test_every_combination_of_marks_over_four_adjacent_spans_survives() {
        let options: [&[&str]; 8] = [&[], &["strong"], &["em"], &["strike"], &["strong", "em"], &["strong", "strike"], &["em", "strike"], &["strong", "em", "strike"]];
        let mut failures = vec![];
        for a in options {
            for b in options {
                for c in options {
                    for d in options {
                        for (texts, tail) in [(["a", "b", "c", "d"], None), (["one", "two", "three", "four"], Some(" and more"))] {
                            let mut nodes = vec![marked(texts[0], a), marked(texts[1], b), marked(texts[2], c), marked(texts[3], d)];
                            nodes.extend(tail.map(|t| text(t)));
                            let md = adf_to_markdown(&doc(vec![para(nodes.clone())]));
                            let back = markdown_to_adf(&md);
                            let got = merged_spans(back["content"][0]["content"].as_array().unwrap());
                            if got != merged_spans(&nodes) {
                                failures.push(format!("{a:?} {b:?} {c:?} {d:?} {texts:?} {tail:?} => {md:?} => {got:?}"));
                            }
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{} combinations do not survive:\n{}", failures.len(), failures.join("\n"));
    }

    #[test]
    fn test_plain_text_before_bold_italic_spans_survives() {
        let adf = doc(vec![para(vec![text("a"), marked("b", &["strong", "em"]), marked("c", &["strong"]), marked("d", &["strong", "em"])])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(spans(&md), vec![span("a", &[]), span("b", &["em", "strong"]), span("c", &["strong"]), span("d", &["em", "strong"])], "{md}");
    }

    #[test]
    fn test_line_break_encoding_is_reversible() {
        let table = |rows: Vec<Value>| json!({"type": "table", "attrs": {"isNumberColumnEnabled": false, "layout": "default"}, "content": rows});
        let row = |cells: Vec<Value>| json!({"type": "tableRow", "content": cells});
        let header = |t: &str| json!({"type": "tableHeader", "attrs": {}, "content": [para(vec![text(t)])]});
        let cell = |content: Vec<Value>| json!({"type": "tableCell", "attrs": {}, "content": content});

        let code = json!({"type": "codeBlock", "content": [text("a<br>b\nc<br/>d\\\ne <BR> f")]});
        let adf = doc(vec![table(vec![row(vec![header("H")]), row(vec![cell(vec![code])])])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(markdown_to_adf(&md), adf, "code in a cell with literal <br> tags and a trailing backslash: {md}");

        let heading = json!({"type": "heading", "attrs": {"level": 3}, "content": [text("a"), {"type": "hardBreak"}, text("b")]});
        let adf = doc(vec![table(vec![row(vec![header("H")]), row(vec![cell(vec![heading, para(vec![text("p")])])])])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(markdown_to_adf(&md), adf, "a heading with a hard break inside a cell: {md}");

        let adf = doc(vec![json!({"type": "heading", "attrs": {"level": 1}, "content": [marked("x<br>y", &["code"]), text(" <br> <BR/> z")]})]);
        let md = adf_to_markdown(&adf);
        assert_eq!(md, "# `x<br>y` \\<br> \\<BR/> z");
        assert_eq!(markdown_to_adf(&md), adf);

        assert_eq!(markdown_to_adf("a<br>b"), doc(vec![para(vec![text("a"), json!({"type": "hardBreak"}), text("b")])]));
        assert_eq!(markdown_to_adf("a<br />b")["content"][0]["content"][1]["type"], "hardBreak");
        round_trip(r"a \<br> b");
    }

    #[test]
    fn test_code_blocks_inside_table_cells() {
        let table = |rows: Vec<Value>| json!({"type": "table", "attrs": {"isNumberColumnEnabled": false, "layout": "default"}, "content": rows});
        let row = |cells: Vec<Value>| json!({"type": "tableRow", "content": cells});
        let header = |t: &str| json!({"type": "tableHeader", "attrs": {}, "content": [para(vec![text(t)])]});
        let cell = |content: Vec<Value>| json!({"type": "tableCell", "attrs": {}, "content": content});
        let code = json!({"type": "codeBlock", "attrs": {"language": "python"}, "content": [text("first\nsecond")]});

        let adf = doc(vec![table(vec![row(vec![header("Name"), header("Code")]), row(vec![cell(vec![para(vec![text("a")])]), cell(vec![code])])])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(md, "| Name | Code |\n| --- | --- |\n| a | ```python<br>first<br>second<br>``` |");
        assert_eq!(markdown_to_adf(&md), adf);

        let adf = doc(vec![table(vec![
            row(vec![header("H")]),
            row(vec![cell(vec![para(vec![text("x"), json!({"type": "hardBreak"}), text("y <br> z")]), para(vec![text("second paragraph")])])]),
        ])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(markdown_to_adf(&md), adf, "hard breaks, literal <br> and several blocks in a cell: {md}");
        round_trip(&md);
    }

    #[test]
    fn test_multiline_headings_keep_their_hard_breaks() {
        let adf = doc(vec![json!({"type": "heading", "attrs": {"level": 2}, "content": [text("a"), {"type": "hardBreak"}, text("b")]})]);
        let md = adf_to_markdown(&adf);
        assert_eq!(md, "## a<br>b");
        assert_eq!(markdown_to_adf(&md), adf);
        let literal = doc(vec![json!({"type": "heading", "attrs": {"level": 1}, "content": [text("title <br> literal")]})]);
        let md = adf_to_markdown(&literal);
        assert_eq!(md, r"# title \<br> literal");
        assert_eq!(markdown_to_adf(&md), literal);
        round_trip("# plain heading");
    }

    fn trimmed_spans(nodes: &[Value]) -> Vec<(String, Vec<String>)> {
        merged_spans(nodes).into_iter()
            .filter(|(t, _)| !t.trim().is_empty())
            .map(|(t, m)| (t.trim().to_string(), m))
            .collect()
    }

    #[test]
    fn test_marked_whitespace_keeps_the_formatting_of_the_paragraph() {
        let adf = doc(vec![para(vec![marked("one ", &["strong"]), marked("two", &["em"]), text(" "), marked("three", &["strike"])])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(md, "**one** *two* ~~three~~");
        assert_eq!(spans(&md), vec![span("one", &["strong"]), span(" ", &[]), span("two", &["em"]), span(" ", &[]), span("three", &["strike"])]);
        assert_eq!(adf_to_markdown(&doc(vec![para(vec![marked("a", &["strong"]), marked(" b", &["strong", "em"])])])), "**a *b***");

        let options: [&[&str]; 8] = [&[], &["strong"], &["em"], &["strike"], &["strong", "em"], &["strong", "strike"], &["em", "strike"], &["strong", "em", "strike"]];
        for a in options {
            for b in options {
                for c in options {
                    let nodes = vec![marked("one ", a), marked(" two ", b), marked("three", c)];
                    let md = adf_to_markdown(&doc(vec![para(nodes.clone())]));
                    let back = markdown_to_adf(&md);
                    assert_eq!(trimmed_spans(back["content"][0]["content"].as_array().unwrap()), trimmed_spans(&nodes), "{a:?} {b:?} {c:?} => {md}");
                }
            }
        }
    }

    #[test]
    fn test_br_typed_inside_cell_code_stays_verbatim() {
        let md = "| H |\n| --- |\n| `a<br>b` and x<br>y and `c<br/>d` |";
        let adf = markdown_to_adf(md);
        let content = &adf["content"][0]["content"][1]["content"][0]["content"][0]["content"];
        assert_eq!(content[0], marked("a<br>b", &["code"]));
        assert_eq!(content[1], text(" and x"));
        assert_eq!(content[2], json!({"type": "hardBreak"}));
        assert_eq!(content[3], text("y and "));
        assert_eq!(content[4], marked("c<br/>d", &["code"]));
        round_trip(md);

        let table = |rows: Vec<Value>| json!({"type": "table", "attrs": {"isNumberColumnEnabled": false, "layout": "default"}, "content": rows});
        let row = |cells: Vec<Value>| json!({"type": "tableRow", "content": cells});
        let header = |t: &str| json!({"type": "tableHeader", "attrs": {}, "content": [para(vec![text(t)])]});
        let cell = |content: Vec<Value>| json!({"type": "tableCell", "attrs": {}, "content": content});
        let code = json!({"type": "codeBlock", "content": [text("first<br>x\nsecond `<br>`\n```")]});
        let adf = doc(vec![table(vec![row(vec![header("H")]), row(vec![cell(vec![para(vec![marked("k<br>v", &["code"])]), code])])])]);
        let md = adf_to_markdown(&adf);
        assert_eq!(markdown_to_adf(&md), adf, "{md}");
    }
}
