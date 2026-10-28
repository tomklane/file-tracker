//! Custom column rules: take a piece of a folder or file name.
//!
//! A rule has three steps:
//! 1. **source**: which text to start from (file name, a folder level, the path…)
//! 2. **method**: which part of it to keep (split at separators, characters,
//!    text between two markers, or a pattern)
//! 3. **tidy**: trim, change case, or fall back to some text when empty
//!
//! Rules are stored as JSON in the column's setting so saved setups keep working.

use crate::core::FileInfo;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Source {
    FileName,
    #[default]
    Stem,
    Extension,
    /// a folder, picked by `level`
    Folder,
    /// path of the folder inside the scanned folder
    RelFolder,
    /// path of the file inside the scanned folder
    RelPath,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Method {
    Whole,
    #[default]
    Split,
    Chars,
    Between,
    Pattern,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Case {
    #[default]
    AsIs,
    Upper,
    Lower,
    Title,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Rule {
    pub source: Source,
    /// Folder level. Counted from the top: 0 = the scanned folder, 1 = the first
    /// subfolder… Negative counts up from the file: -1 = the folder it's in.
    pub level: i64,
    pub method: Method,
    /// every character in here splits the text
    pub seps: String,
    /// 1 = first part, -1 = last part
    pub part_from: i64,
    /// 0 = just `part_from`; otherwise the last part to include
    pub part_to: i64,
    pub to_end: bool,
    /// 1 = first character, -1 = last character
    pub char_from: i64,
    /// 0 = all the rest
    pub char_len: i64,
    pub after: String,
    pub before: String,
    pub ignore_case: bool,
    pub pattern: String,
    pub trim: bool,
    pub case: Case,
    pub if_empty: String,
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            source: Source::Stem,
            level: 1,
            method: Method::Split,
            seps: "_".into(),
            part_from: 1,
            part_to: 0,
            to_end: false,
            char_from: 1,
            char_len: 0,
            after: String::new(),
            before: String::new(),
            ignore_case: true,
            pattern: String::new(),
            trim: true,
            case: Case::AsIs,
            if_empty: String::new(),
        }
    }
}

pub fn parse(param: &str) -> Rule {
    serde_json::from_str(param).unwrap_or_default()
}

pub fn to_param(r: &Rule) -> String {
    serde_json::to_string(r).unwrap_or_default()
}

/// The result of applying a rule, with the span (in characters) of the source
/// text that was kept, so the app can highlight it.
pub struct Extracted {
    pub source: String,
    pub span: Option<(usize, usize)>,
    pub text: String,
}

/// Folder by level: 0 = scanned folder, 1 = first subfolder, -1 = the file's own folder.
pub fn folder_at(info: &FileInfo, level: i64) -> Option<&str> {
    let n = info.folders.len() as i64;
    let idx = if level >= 0 { level } else { n + level };
    if (0..n).contains(&idx) {
        Some(info.folders[idx as usize].as_str())
    } else {
        None
    }
}

pub fn source_text(r: &Rule, info: &FileInfo) -> String {
    match r.source {
        Source::FileName => info.name.clone(),
        Source::Stem => info.stem.clone(),
        Source::Extension => info.ext.clone(),
        Source::Folder => folder_at(info, r.level).unwrap_or("").to_string(),
        Source::RelFolder => info.rel_folder.replace('\\', "/"),
        Source::RelPath => info.rel.replace('\\', "/"),
    }
}

/// Split into parts at any of `seps`, skipping empty parts. Spans are in characters.
pub fn split_spans(text: &str, seps: &str) -> Vec<(usize, usize)> {
    let seps: Vec<char> = if seps.is_empty() { vec!['_'] } else { seps.chars().collect() };
    let mut out = Vec::new();
    let mut start = None;
    let mut n = 0;
    for (i, ch) in text.chars().enumerate() {
        n = i + 1;
        if seps.contains(&ch) {
            if let Some(s) = start.take() {
                out.push((s, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push((s, n));
    }
    out
}

/// 1-based from the start, negative from the end.
fn pick(k: i64, len: usize) -> Option<usize> {
    let idx = if k > 0 {
        k - 1
    } else if k < 0 {
        len as i64 + k
    } else {
        0
    };
    if (0..len as i64).contains(&idx) {
        Some(idx as usize)
    } else {
        None
    }
}

fn find_chars(hay: &[char], needle: &[char], from: usize, ignore_case: bool) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    let low = |c: char| if ignore_case { c.to_lowercase().next().unwrap_or(c) } else { c };
    (from..=hay.len() - needle.len()).find(|&i| needle.iter().enumerate().all(|(j, n)| low(hay[i + j]) == low(*n)))
}

thread_local! {
    static REGEX_CACHE: RefCell<HashMap<String, Option<regex::Regex>>> = RefCell::new(HashMap::new());
}

pub fn regex_error(pattern: &str) -> Option<String> {
    regex::Regex::new(pattern).err().map(|e| e.to_string().lines().last().unwrap_or("invalid pattern").trim().to_string())
}

fn regex_span(pattern: &str, text: &str, ignore_case: bool) -> Option<(usize, usize)> {
    if pattern.is_empty() {
        return None;
    }
    let key = format!("{}{pattern}", if ignore_case { "(?i)" } else { "" });
    let rx = REGEX_CACHE.with(|c| c.borrow_mut().entry(key.clone()).or_insert_with(|| regex::Regex::new(&key).ok()).clone())?;
    let caps = rx.captures(text)?;
    let m = caps.get(1).or_else(|| caps.get(0))?;
    // byte offsets -> character offsets
    let to_char = |b: usize| text[..b].chars().count();
    Some((to_char(m.start()), to_char(m.end())))
}

fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut new_word = true;
    for ch in s.chars() {
        if ch.is_alphanumeric() {
            if new_word {
                out.extend(ch.to_uppercase());
            } else {
                out.extend(ch.to_lowercase());
            }
            new_word = false;
        } else {
            out.push(ch);
            new_word = true;
        }
    }
    out
}

pub fn apply(r: &Rule, info: &FileInfo) -> Extracted {
    let source = source_text(r, info);
    let chars: Vec<char> = source.chars().collect();
    let n = chars.len();
    let span = match r.method {
        Method::Whole => Some((0, n)),
        Method::Split => {
            let parts = split_spans(&source, &r.seps);
            pick(r.part_from, parts.len()).and_then(|a| {
                let b = if r.to_end {
                    Some(parts.len() - 1)
                } else if r.part_to == 0 {
                    Some(a)
                } else {
                    pick(r.part_to, parts.len())
                }?;
                let (a, b) = (a.min(b), a.max(b));
                Some((parts[a].0, parts[b].1))
            })
        }
        Method::Chars => {
            let start = if r.char_from > 0 {
                r.char_from - 1
            } else if r.char_from < 0 {
                (n as i64 + r.char_from).max(0)
            } else {
                0
            } as usize;
            if start >= n {
                None
            } else {
                let end = if r.char_len <= 0 { n } else { (start + r.char_len as usize).min(n) };
                Some((start, end))
            }
        }
        Method::Between => {
            let after: Vec<char> = r.after.chars().collect();
            let before: Vec<char> = r.before.chars().collect();
            let start = if after.is_empty() {
                Some(0)
            } else {
                find_chars(&chars, &after, 0, r.ignore_case).map(|i| i + after.len())
            };
            start.map(|s| {
                let e = if before.is_empty() { n } else { find_chars(&chars, &before, s, r.ignore_case).unwrap_or(n) };
                (s, e.max(s))
            })
        }
        Method::Pattern => regex_span(&r.pattern, &source, r.ignore_case),
    };
    let mut text: String = match span {
        Some((a, b)) => chars[a..b].iter().collect(),
        None => String::new(),
    };
    if r.trim {
        text = text.trim().to_string();
    }
    text = match r.case {
        Case::AsIs => text,
        Case::Upper => text.to_uppercase(),
        Case::Lower => text.to_lowercase(),
        Case::Title => title_case(&text),
    };
    if text.is_empty() && !r.if_empty.is_empty() {
        text = r.if_empty.clone();
    }
    Extracted { source, span, text }
}

fn ordinal(k: i64) -> String {
    match k {
        -1 => "last".into(),
        -2 => "2nd-last".into(),
        -3 => "3rd-last".into(),
        k if k < 0 => format!("{}th-last", -k),
        1 => "1st".into(),
        2 => "2nd".into(),
        3 => "3rd".into(),
        k => format!("{k}th"),
    }
}

pub fn source_label(r: &Rule) -> String {
    match r.source {
        Source::FileName => "file name".into(),
        Source::Stem => "name".into(),
        Source::Extension => "extension".into(),
        Source::RelFolder => "folder path".into(),
        Source::RelPath => "file path".into(),
        Source::Folder => match r.level {
            0 => "top folder".into(),
            -1 => "folder it's in".into(),
            l if l < 0 => format!("folder {} up", -l),
            l => format!("folder level {l}"),
        },
    }
}

/// A short description, e.g. "part 2 of name" or "chars 1–6 of folder level 2".
pub fn describe(r: &Rule) -> String {
    let src = source_label(r);
    match r.method {
        Method::Whole => format!("whole {src}"),
        Method::Split => {
            if r.to_end {
                format!("{} part to end of {src}", ordinal(r.part_from))
            } else if r.part_to != 0 && r.part_to != r.part_from {
                format!("parts {}–{} of {src}", r.part_from, r.part_to)
            } else {
                format!("{} part of {src}", ordinal(r.part_from))
            }
        }
        Method::Chars => {
            if r.char_from < 0 && r.char_len <= 0 {
                format!("last {} chars of {src}", -r.char_from)
            } else if r.char_len <= 0 {
                format!("from char {} of {src}", r.char_from)
            } else {
                format!("{} chars from {} of {src}", r.char_len, r.char_from)
            }
        }
        Method::Between => format!("text after “{}” of {src}", r.after),
        Method::Pattern => format!("pattern in {src}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn info(rel: &str) -> FileInfo {
        let p = PathBuf::from(rel);
        let parent = p.parent().unwrap();
        let mut folders = vec!["Projects".to_string()];
        folders.extend(parent.components().map(|c| c.as_os_str().to_string_lossy().to_string()));
        FileInfo {
            path: PathBuf::from("/x").join(&p),
            name: p.file_name().unwrap().to_string_lossy().into(),
            stem: p.file_stem().unwrap().to_string_lossy().into(),
            ext: p.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default(),
            rel: rel.into(),
            rel_folder: parent.display().to_string(),
            folders,
            size: 1,
            mtime: Default::default(),
            ctime: None,
        }
    }

    fn get(r: &Rule, rel: &str) -> String {
        apply(r, &info(rel)).text
    }

    #[test]
    fn split_variable_lengths() {
        let mut r = Rule { seps: "_".into(), ..Default::default() };
        // sections of different lengths still split cleanly
        assert_eq!(get(&r, "A/PRJ001_Beam-Layout_RevA.pdf"), "PRJ001");
        assert_eq!(get(&r, "A/P7_Ground floor plan_RevC2.pdf"), "P7");
        r.part_from = -1;
        assert_eq!(get(&r, "A/PRJ001_Beam-Layout_RevA.pdf"), "RevA");
        assert_eq!(get(&r, "A/P7_Ground floor plan_extra_RevC2.pdf"), "RevC2");
        r.part_from = 2;
        r.to_end = true;
        assert_eq!(get(&r, "A/P7_Ground floor plan_extra_RevC2.pdf"), "Ground floor plan_extra_RevC2");
        // several separators, doubled separators ignored
        let r = Rule { seps: "_- ".into(), part_from: 3, ..Default::default() };
        assert_eq!(get(&r, "A/PRJ001__Beam-Layout RevA.pdf"), "Layout");
        let r = Rule { seps: "_".into(), part_from: 2, part_to: 3, ..Default::default() };
        assert_eq!(get(&r, "A/a_bb_ccc_d.pdf"), "bb_ccc");
        // missing part -> empty, or fallback text
        let r = Rule { part_from: 5, if_empty: "—".into(), ..Default::default() };
        assert_eq!(get(&r, "A/a_b.pdf"), "—");
    }

    #[test]
    fn folders_and_sources() {
        let r = Rule { source: Source::Folder, level: 1, method: Method::Whole, ..Default::default() };
        assert_eq!(get(&r, "2026-03 Client Smith/Drawings/x.pdf"), "2026-03 Client Smith");
        let r = Rule { source: Source::Folder, level: -1, method: Method::Whole, ..Default::default() };
        assert_eq!(get(&r, "Job/Deep/Deeper/x.pdf"), "Deeper");
        // a section of a folder name, at different lengths
        let r = Rule { source: Source::Folder, level: 1, method: Method::Split, seps: " ".into(), part_from: 2, to_end: true, ..Default::default() };
        assert_eq!(get(&r, "2026-03 Client Smith/x.pdf"), "Client Smith");
        assert_eq!(get(&r, "2026-11 Big Client Holdings Ltd/x.pdf"), "Big Client Holdings Ltd");
        let r = Rule { source: Source::Extension, method: Method::Whole, case: Case::Upper, ..Default::default() };
        assert_eq!(get(&r, "a/b.pdf"), "PDF");
        let r = Rule { source: Source::Folder, level: 9, method: Method::Whole, ..Default::default() };
        assert_eq!(get(&r, "a/b.pdf"), "");
    }

    #[test]
    fn chars_between_pattern() {
        let r = Rule { method: Method::Chars, char_from: 1, char_len: 4, ..Default::default() };
        assert_eq!(get(&r, "a/2026-03 Plan.pdf"), "2026");
        let r = Rule { method: Method::Chars, char_from: -3, char_len: 0, ..Default::default() };
        assert_eq!(get(&r, "a/Plan_R12.pdf"), "R12");
        let r = Rule { method: Method::Between, after: "rev".into(), before: "_".into(), ..Default::default() };
        assert_eq!(get(&r, "a/Plan_REV12_final.pdf"), "12");
        assert_eq!(get(&r, "a/Plan_rev3.pdf"), "3");
        assert_eq!(get(&r, "a/Plan.pdf"), "");
        let r = Rule { method: Method::Pattern, pattern: r"(\d{4}-\d{2})".into(), ..Default::default() };
        assert_eq!(get(&r, "a/Report 2026-03 final.pdf"), "2026-03");
        let e = apply(&r, &info("a/Report 2026-03 final.pdf"));
        assert_eq!(e.span, Some((7, 14)));
        let r = Rule { method: Method::Whole, case: Case::Title, ..Default::default() };
        assert_eq!(get(&r, "a/beam LAYOUT plan.pdf"), "Beam Layout Plan");
        // unicode is counted in characters
        let r = Rule { method: Method::Chars, char_from: 1, char_len: 3, ..Default::default() };
        assert_eq!(get(&r, "a/Café_x.pdf"), "Caf");
    }

    #[test]
    fn round_trip_and_describe() {
        let r = Rule { part_from: -1, ..Default::default() };
        assert_eq!(parse(&to_param(&r)), r);
        assert_eq!(parse("not json"), Rule::default());
        assert_eq!(describe(&r), "last part of name");
    }
}
