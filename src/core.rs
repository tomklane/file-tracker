//! Everything that isn't GUI: scanning folders, working out cell values,
//! reading/writing spreadsheets (.xlsx/.xlsm/.csv) and building the copy list.

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use umya_spreadsheet as umya;

pub const BACKUP_DIR: &str = "_tracker_backups";
pub const MAX_BACKUPS: usize = 20;
pub const DEFAULT_NEW_COLUMNS: &str =
    "ID, File Name, Type, Folder, Full Path, Size, Date Modified, Date Added, On Disk, Status, Notes";

/// key, label shown in the app, default setting (None = no setting box), hint for the setting
pub struct FieldDef {
    pub key: &'static str,
    pub label: &'static str,
    pub default: Option<&'static str>,
    pub hint: &'static str,
}

macro_rules! f {
    ($k:expr, $l:expr) => {
        FieldDef { key: $k, label: $l, default: None, hint: "" }
    };
    ($k:expr, $l:expr, $d:expr, $h:expr) => {
        FieldDef { key: $k, label: $l, default: Some($d), hint: $h }
    };
}

pub const FIELDS: &[FieldDef] = &[
    f!("", "— leave as is —"),
    f!("name", "File name (or folder name)"),
    f!("stem", "File name without extension"),
    f!("part", "Part of the file name", "1", "1 = first part, -1 = last"),
    f!("ext", "Extension (pdf)"),
    f!("ext_upper", "Type (PDF)"),
    f!("parent", "Folder it's in"),
    f!("level", "Folder at level…", "1", "1 = top subfolder, -1 = its own folder"),
    f!("rel_folder", "Folder path inside scanned folder"),
    f!("rel_path", "File path inside scanned folder"),
    f!("full_path", "Full file path"),
    f!("link", "Clickable link to the file"),
    f!("size_h", "Size (e.g. 2.4 MB)"),
    f!("size_kb", "Size in KB"),
    f!("size_mb", "Size in MB"),
    f!("size_gb", "Size in GB"),
    f!("size_tb", "Size in TB"),
    f!("size_b", "Size in bytes"),
    f!("count", "Number of files (in a folder)"),
    f!("modified_d", "Date modified"),
    f!("modified", "Date & time modified"),
    f!("created_d", "Date created"),
    f!("created", "Date & time created"),
    f!("scanned", "Date added to sheet (new rows only)"),
    f!("index", "Running number / ID (new rows only)", "#", "e.g.  #  or  DOC-###"),
    f!("status", "Present / Missing", "Present|Missing", "found text | missing text"),
    f!("text", "Fixed text", "", "text to write"),
    f!("extract", "Custom: pick part of a folder or file name…", "", "Click Edit to choose"),
];

pub fn field(key: &str) -> &'static FieldDef {
    FIELDS.iter().find(|f| f.key == key).unwrap_or(&FIELDS[0])
}

const NEW_ONLY: &[&str] = &["scanned", "index"];
pub const BAD_KEYS: &[&str] = &["", "scanned", "index", "status", "text"];
pub const KEY_PRIORITY: &[&str] = &["full_path", "rel_path", "link", "name", "stem"];

/// A plain-words error for the user.
#[derive(Debug)]
pub struct UserError(pub String);
impl<E: std::fmt::Display> From<E> for UserError {
    fn from(e: E) -> Self {
        UserError(e.to_string())
    }
}
pub type Res<T> = Result<T, UserError>;
macro_rules! bail {
    ($($t:tt)*) => { return Err(UserError(format!($($t)*))) };
}

// ------------------------------------------------------------------ settings

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub folder: String,
    pub recursive: bool,
    pub skip_hidden: bool,
    pub extensions: String,
    pub exclude: String,
    pub sheet_mode: String, // "existing" | "new"
    pub sheet_path: String,
    pub sheet_name: String,
    pub header_row: u32,
    pub new_columns: String,
    pub delimiter: String,
    pub key_header: String,
    pub run_mode: String, // "update" | "fresh"
    pub update_existing: bool,
    pub backup: bool,
    pub only_new: bool,
    pub auto_refresh: bool,
    pub copy_headers: bool,
    /// grid sort: column heading ("" = not sorted) and direction
    pub sort_col: String,
    pub sort_desc: bool,
    pub check_updates: bool,
    /// a release the user chose to skip
    pub skip_version: String,
    /// what each row is: "files", "level" (folders at folder_level) or "withfiles"
    pub row_mode: String,
    pub folder_level: u32,
    /// for row_mode "pattern": which folder names make a row
    pub folder_pattern: String,
    pub show_sidebar: bool,
    /// 1000 (like Finder) or 1024 (like Windows)
    pub size_base: u32,
    /// header text -> [field key, setting]
    pub mapping: HashMap<String, (String, String)>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            folder: String::new(),
            recursive: true,
            skip_hidden: true,
            extensions: String::new(),
            exclude: String::new(),
            sheet_mode: "new".into(),
            sheet_path: String::new(),
            sheet_name: String::new(),
            header_row: 1,
            new_columns: DEFAULT_NEW_COLUMNS.into(),
            delimiter: "_".into(),
            key_header: String::new(),
            run_mode: "update".into(),
            update_existing: true,
            backup: true,
            only_new: true,
            auto_refresh: true,
            copy_headers: false,
            sort_col: String::new(),
            sort_desc: false,
            check_updates: true,
            skip_version: String::new(),
            row_mode: "files".into(),
            folder_level: 1,
            folder_pattern: DATE_FOLDER_PATTERN.into(),
            show_sidebar: true,
            size_base: default_size_base(),
            mapping: HashMap::new(),
        }
    }
}

pub fn settings_file() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".file_tracker_settings.json")
}

pub fn load_settings(path: &Path) -> Option<Settings> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// Read a saved setup, whatever its file is called (downloads sometimes end
/// up as .json.txt or with no extension). Only accepts real File Tracker setups.
pub fn read_setup(path: &Path) -> Result<Settings, String> {
    let raw = fs::read(path).map_err(|e| format!("Couldn't open that file ({e})."))?;
    let text = String::from_utf8_lossy(&raw);
    let text = text.trim_start_matches('\u{feff}');
    let v: serde_json::Value = serde_json::from_str(text).map_err(|_| "That file isn't a File Tracker setup.".to_string())?;
    let known = ["mapping", "row_mode", "folder", "sheet_mode", "new_columns", "key_header"];
    if !v.as_object().map(|o| known.iter().any(|k| o.contains_key(*k))).unwrap_or(false) {
        return Err("That file isn't a File Tracker setup.".into());
    }
    serde_json::from_value(v).map_err(|e| format!("That setup couldn't be read ({e})."))
}

/// Does this look like a setup file (for files dropped onto the window)?
pub fn is_setup_file(path: &Path) -> bool {
    let small = fs::metadata(path).map(|m| m.len() < 2_000_000).unwrap_or(false);
    small && read_setup(path).is_ok()
}

pub fn save_settings(s: &Settings, path: &Path) {
    if let Ok(txt) = serde_json::to_string_pretty(s) {
        let _ = fs::write(path, txt);
    }
}

// ------------------------------------------------------------------ small helpers

pub fn clean(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn split_list(s: &str) -> Vec<String> {
    s.split([',', ';']).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

pub fn col_letter(mut n: u32) -> String {
    let mut s = Vec::new();
    while n > 0 {
        let r = ((n - 1) % 26) as u8;
        s.push(b'A' + r);
        n = (n - 1) / 26;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

fn letters_to_col(s: &str) -> u32 {
    s.bytes().fold(0, |acc, b| acc * 26 + (b.to_ascii_uppercase() - b'A' + 1) as u32)
}

/// "B3:F20" -> ((2,3),(6,20)) as (col,row)
fn parse_range(r: &str) -> Option<((u32, u32), (u32, u32))> {
    let r = r.replace('$', "");
    let mut it = r.split(':');
    let a = it.next()?;
    let b = it.next().unwrap_or(a);
    let one = |c: &str| -> Option<(u32, u32)> {
        let split = c.find(|ch: char| ch.is_ascii_digit())?;
        let (l, n) = c.split_at(split);
        Some((letters_to_col(l), n.parse().ok()?))
    };
    Some((one(a)?, one(b)?))
}

pub fn dedupe_headers(pairs: Vec<(u32, String)>) -> Vec<(u32, String)> {
    let mut seen: HashMap<String, u32> = HashMap::new();
    let mut out = Vec::new();
    for (c, t) in pairs {
        if t.is_empty() {
            continue;
        }
        let n = seen.entry(t.to_lowercase()).or_insert(0);
        *n += 1;
        out.push((c, if *n == 1 { t } else { format!("{t} ({n})") }));
    }
    out
}

fn is_text(v: &str) -> bool {
    !v.trim().is_empty() && v.trim().parse::<f64>().is_err()
}

pub fn detect_header_row(rows: &[Vec<String>]) -> u32 {
    let (mut best, mut best_n) = (1, 0);
    for (i, row) in rows.iter().take(30).enumerate() {
        let n = row.iter().filter(|v| is_text(v)).count();
        if n > best_n {
            best = i as u32 + 1;
            best_n = n;
        }
    }
    best
}

fn to_int(s: &str, d: i64) -> i64 {
    s.trim().parse().unwrap_or(d)
}

/// Sizes the way Finder (1 KB = 1,000 bytes) or Windows (1 KB = 1,024 bytes) shows them.
pub fn human_size(n: u64, base: f64) -> String {
    let mut v = n as f64;
    for unit in ["bytes", "KB", "MB", "GB", "TB", "PB"] {
        if v < base || unit == "PB" {
            return if unit == "bytes" {
                format!("{v:.0} {unit}")
            } else if v < 10.0 {
                format!("{v:.2} {unit}")
            } else {
                format!("{v:.1} {unit}")
            };
        }
        v /= base;
    }
    unreachable!()
}

/// The size unit people expect on this computer: Finder uses 1,000, Windows 1,024.
pub fn default_size_base() -> u32 {
    if cfg!(target_os = "windows") {
        1024
    } else {
        1000
    }
}

pub fn status_texts(param: &str) -> (String, String) {
    let p = if param.trim().is_empty() { "Present|Missing" } else { param };
    let mut it = p.split('|');
    let a = it.next().map(str::trim).filter(|x| !x.is_empty()).unwrap_or("Present");
    let b = it.next().map(str::trim).filter(|x| !x.is_empty()).unwrap_or("Missing");
    (a.to_string(), b.to_string())
}

pub fn norm_key(v: &str) -> String {
    let mut t = v.trim().to_string();
    if let Ok(f) = t.parse::<f64>() {
        if f.fract() == 0.0 && f.abs() < 1e15 {
            t = format!("{}", f as i64);
        }
    }
    t.replace('\\', "/").to_lowercase()
}

fn wildcard(pat: &str, name: &str) -> bool {
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn matches_any(name: &str, pats: &[String]) -> bool {
    pats.iter().any(|p| wildcard(p, name))
}

/// Guess a field from a column header, when each row is a folder.
pub fn auto_field_for(header: &str, folder_rows: bool) -> &'static str {
    let f = auto_field(header);
    if folder_rows && f == "parent" {
        "name" // a "Folder" column means the row's own folder
    } else {
        f
    }
}

/// Guess a field from a column header. "" means leave it alone.
pub fn auto_field(header: &str) -> &'static str {
    let low = header.to_lowercase();
    let t: String = low
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '#' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let words: HashSet<&str> = t.split(' ').collect();
    let has = |ps: &[&str]| ps.iter().any(|p| t.contains(p));
    let w = |ps: &[&str]| ps.iter().any(|p| words.contains(p));
    if w(&["by", "owner", "author", "reviewer", "comment", "comments", "notes", "remarks", "status",
        "description", "approved", "approver"]) {
        return "";
    }
    if ["#", "no", "nr", "item", "row", "s no", "sl no", "sr no", "index", "id"].contains(&t.as_str()) {
        return "index";
    }
    if has(&["hyperlink", "link"]) {
        return "link";
    }
    if has(&["on disk", "exists", "file found", "present"]) {
        return "status";
    }
    if has(&["relative path", "path in", "subpath"]) {
        return "rel_path";
    }
    if has(&["folder path", "directory path"]) {
        return "rel_folder";
    }
    if has(&["full path", "file path", "filepath", "location"]) || t == "path" {
        return "full_path";
    }
    if w(&["size"]) || has(&["file size", "filesize"]) {
        return if w(&["tb"]) {
            "size_tb"
        } else if w(&["gb"]) {
            "size_gb"
        } else if w(&["mb"]) {
            "size_mb"
        } else if w(&["kb"]) {
            "size_kb"
        } else if w(&["bytes", "byte", "b"]) {
            "size_b"
        } else {
            "size_h"
        };
    }
    if has(&["file count", "number of files", "no of files", "num files", "files count", "clip count", "number of clips", "no of clips", "clips count"])
        || ["count", "# files", "# clips", "clips"].contains(&t.as_str())
    {
        return "count";
    }
    if has(&["folder", "directory"]) {
        return "parent";
    }
    if has(&["extension"]) || w(&["ext"]) {
        return "ext";
    }
    if has(&["file type", "filetype", "format"]) || t == "type" {
        return "ext_upper";
    }
    if has(&["created", "creation"]) {
        return "created_d";
    }
    if has(&["modified", "last updated", "updated", "last changed", "last saved"]) {
        return "modified_d";
    }
    if has(&["date added", "added", "logged", "date entered", "first seen"]) {
        return "scanned";
    }
    if has(&["file name", "filename"]) || t == "name" || t == "file" {
        return "name";
    }
    if has(&["title", "document name", "doc name"]) {
        return "stem";
    }
    ""
}

// ------------------------------------------------------------------ scanning

#[derive(Clone)]
pub struct FileInfo {
    pub path: PathBuf,
    pub name: String,
    pub stem: String,
    pub ext: String,
    pub rel: String,
    pub rel_folder: String,
    pub folders: Vec<String>,
    /// bytes; for a folder (or a Mac package) the total of everything inside
    pub size: u64,
    /// newest modification inside, for folders
    pub mtime: NaiveDateTime,
    pub ctime: Option<NaiveDateTime>,
    /// this row is a folder (folder rows mode)
    pub is_folder: bool,
    /// number of files: 1 for a file, everything inside for a folder or package
    pub count: u64,
}

const JUNK: &[&str] = &[".ds_store", "thumbs.db", "desktop.ini", "icon\r", ".localized"];

fn local(t: std::time::SystemTime) -> NaiveDateTime {
    let dt: DateTime<Local> = t.into();
    dt.naive_local().with_nanosecond_zero()
}

trait NoNanos {
    fn with_nanosecond_zero(self) -> Self;
}
impl NoNanos for NaiveDateTime {
    fn with_nanosecond_zero(self) -> Self {
        use chrono::Timelike;
        self.with_nanosecond(0).unwrap_or(self)
    }
}

#[allow(unused_variables)]
fn is_hidden(path: &Path, md: &fs::Metadata) -> bool {
    if path.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false) {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return md.file_attributes() & (0x2 | 0x4) != 0; // hidden | system
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        return md.st_flags() & 0x8000 != 0; // UF_HIDDEN
    }
    #[allow(unreachable_code)]
    false
}

/// What each grid row stands for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RowMode {
    Files,
    /// folders at this depth (1 = folders directly inside the scanned folder),
    /// each with the total size of everything inside
    FolderLevel(u32),
    /// every folder that holds files, with the size of its own files
    FoldersWithFiles,
    /// folders whose names match this pattern (e.g. card folders that start
    /// with a date), at any depth, each with the total of everything inside
    FolderPattern(String),
}

/// The usual card-folder pattern: names that start with a date like 20261025_
pub const DATE_FOLDER_PATTERN: &str = r"^\d{8}_";

impl RowMode {
    pub fn from_settings(mode: &str, level: u32, pattern: &str) -> Self {
        match mode {
            "level" => RowMode::FolderLevel(level.max(1)),
            "withfiles" => RowMode::FoldersWithFiles,
            "pattern" => RowMode::FolderPattern(if pattern.trim().is_empty() { DATE_FOLDER_PATTERN.into() } else { pattern.trim().into() }),
            _ => RowMode::Files,
        }
    }
}

pub struct ScanOptions {
    pub recursive: bool,
    pub skip_hidden: bool,
    pub extensions: Vec<String>,
    pub exclude: Vec<String>,
}

/// Folders that Finder shows as single files (apps, Final Cut / Photos /
/// Logic libraries…). They're measured as one item instead of opened up.
const PACKAGES: &[&str] = &[
    "app", "bundle", "framework", "plugin", "kext", "fcpbundle", "fcpxmld", "photoslibrary", "photolibrary", "aplibrary",
    "imovielibrary", "theater", "tvlibrary", "musiclibrary", "logicx", "band", "rtfd", "pages", "numbers", "key",
    "sparsebundle", "xcodeproj", "xcworkspace", "playground", "lrdata", "lrlibrary", "dvdmedia", "prproj-bundle",
    "motn", "moef", "moti", "motr", "mpkg", "workflow", "scptd", "nib", "storyboardc", "xcarchive", "scnassets",
];

pub fn is_package(name: &str) -> bool {
    Path::new(name)
        .extension()
        .map(|e| PACKAGES.contains(&e.to_string_lossy().to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Total size, file count and newest change of everything inside a folder.
fn measure_dir(dir: &Path) -> (u64, u64, Option<NaiveDateTime>) {
    let (mut size, mut count, mut newest) = (0u64, 0u64, None::<NaiveDateTime>);
    for e in walkdir::WalkDir::new(dir).follow_links(false).into_iter().flatten() {
        if e.file_type().is_dir() {
            continue;
        }
        if let Ok(md) = e.metadata() {
            size += md.len();
            count += 1;
            if let Ok(m) = md.modified() {
                let m = local(m);
                newest = Some(newest.map_or(m, |n| n.max(m)));
            }
        }
    }
    (size, count, newest)
}

pub fn scan_folder(
    root: &Path,
    o: &ScanOptions,
    skip: &[PathBuf],
    progress: &dyn Fn(String),
) -> Vec<FileInfo> {
    let exts: HashSet<String> = o
        .extensions
        .iter()
        .map(|e| e.trim().trim_start_matches('*').trim_start_matches('.').to_lowercase())
        .collect();
    let mut exclude = o.exclude.clone();
    exclude.push(BACKUP_DIR.to_string());
    let skip: HashSet<PathBuf> = skip.iter().filter_map(|p| p.canonicalize().ok()).collect();
    let root_name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();

    let mut it = walkdir::WalkDir::new(root)
        .follow_links(false)
        .max_depth(if o.recursive { usize::MAX } else { 1 })
        .sort_by(|a, b| {
            (a.file_type().is_dir(), a.file_name().to_string_lossy().to_lowercase())
                .cmp(&(b.file_type().is_dir(), b.file_name().to_string_lossy().to_lowercase()))
        })
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            let n = e.file_name().to_string_lossy();
            if matches_any(&n, &exclude) {
                return false;
            }
            if o.skip_hidden {
                if let Ok(md) = e.metadata() {
                    if is_hidden(e.path(), &md) {
                        return false;
                    }
                }
            }
            true
        });

    let mut out = Vec::new();
    loop {
        let e = match it.next() {
            None => break,
            Some(Ok(e)) => e,
            Some(Err(_)) => continue, // a folder we aren't allowed to open
        };
        let name = e.file_name().to_string_lossy().to_string();
        let ft = e.file_type();
        let package = ft.is_dir() && e.depth() > 0 && is_package(&name);
        if ft.is_dir() && !package {
            continue;
        }
        if package {
            it.skip_current_dir(); // measured as a whole below
        }
        let low = name.to_lowercase();
        if JUNK.contains(&low.as_str()) || name.starts_with("~$") || name.starts_with("._") {
            continue;
        }
        if matches_any(&name, &exclude) {
            continue;
        }
        let p = e.path();
        let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !exts.is_empty() && !exts.contains(&ext) {
            continue;
        }
        // shortcuts (symlinks) to files are measured by what they point to
        let md = match fs::metadata(p).or_else(|_| fs::symlink_metadata(p)) {
            Ok(md) => md,
            Err(_) => continue,
        };
        if md.is_dir() && !package {
            continue; // a shortcut to a folder
        }
        if o.skip_hidden && is_hidden(p, &md) {
            continue;
        }
        let full = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        if skip.contains(&full) {
            continue;
        }
        let (size, count, newest) = if package { measure_dir(p) } else { (md.len(), 1, None) };
        let rel = p.strip_prefix(root).unwrap_or(p);
        let parent = rel.parent().unwrap_or(Path::new(""));
        let mut folders = vec![root_name.clone()];
        folders.extend(parent.components().map(|c| c.as_os_str().to_string_lossy().to_string()));
        let mtime = md.modified().map(local).unwrap_or_default();
        out.push(FileInfo {
            stem: p.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default(),
            name,
            ext,
            rel: rel.display().to_string(),
            rel_folder: parent.display().to_string(),
            folders,
            size,
            mtime: newest.map_or(mtime, |n| n.max(mtime)),
            ctime: md.created().ok().map(local),
            path: strip_unc(full),
            is_folder: false,
            count,
        });
        if out.len() % 500 == 0 {
            progress(format!("Scanning… {} files found", out.len()));
        }
    }
    out
}

/// Turn a list of files into one row per folder, adding up sizes.
pub fn folder_rows(root: &Path, files: &[FileInfo], mode: &RowMode) -> Vec<FileInfo> {
    use std::collections::BTreeMap;
    let pattern = match mode {
        RowMode::FolderPattern(p) => regex::RegexBuilder::new(p).case_insensitive(true).build().ok(),
        _ => None,
    };
    let root_name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    // key: folder path inside the scanned folder (lower case, for ordering)
    let mut groups: BTreeMap<Vec<String>, (Vec<String>, u64, u64, NaiveDateTime)> = BTreeMap::new();
    for f in files {
        let inner = &f.folders[1..];
        let key: Vec<String> = match mode {
            RowMode::FolderLevel(n) => {
                let n = *n as usize;
                if inner.len() < n {
                    continue; // files that sit above that level
                }
                inner[..n].to_vec()
            }
            RowMode::FoldersWithFiles => inner.to_vec(),
            RowMode::FolderPattern(_) => {
                // the outermost folder (below the scanned one) whose name matches
                let Some(rx) = &pattern else { return vec![] };
                match inner.iter().position(|name| rx.is_match(name)) {
                    Some(k) => inner[..=k].to_vec(),
                    None => continue,
                }
            }
            RowMode::Files => return files.to_vec(),
        };
        let lower: Vec<String> = key.iter().map(|k| k.to_lowercase()).collect();
        let g = groups.entry(lower).or_insert_with(|| (key, 0, 0, f.mtime));
        g.1 += f.size;
        g.2 += f.count;
        g.3 = g.3.max(f.mtime);
    }
    let sep = std::path::MAIN_SEPARATOR_STR;
    groups
        .into_values()
        .map(|(key, size, count, newest)| {
            let path = key.iter().fold(root.to_path_buf(), |p, k| p.join(k));
            let created = fs::metadata(&path).ok().and_then(|m| m.created().ok()).map(local);
            let name = key.last().cloned().unwrap_or_else(|| root_name.clone());
            let mut folders = vec![root_name.clone()];
            if !key.is_empty() {
                folders.extend(key[..key.len() - 1].iter().cloned());
            }
            FileInfo {
                path: strip_unc(path.canonicalize().unwrap_or(path)),
                stem: name.clone(),
                name,
                ext: String::new(),
                rel: key.join(sep),
                rel_folder: if key.is_empty() { String::new() } else { key[..key.len() - 1].join(sep) },
                folders,
                size,
                mtime: newest,
                ctime: created,
                is_folder: true,
                count,
            }
        })
        .collect()
}

/// Windows canonicalize() returns \\?\C:\... — turn it back into a normal path.
fn strip_unc(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    p
}

// ------------------------------------------------------------------ values

#[derive(Clone, Debug)]
pub enum Val {
    Text(String),
    Num(f64),
    Date(NaiveDate),
    DateTime(NaiveDateTime),
    Link { name: String, uri: String, path: String },
}

pub struct Ctx {
    pub delimiter: String,
    pub today: NaiveDate,
    /// 1000 (like Finder) or 1024 (like Windows)
    pub base: f64,
    /// tables for custom rules that look values up in another sheet
    pub lookups: Lookups,
}

/// Lookup tables read from the spreadsheet: key → (lower-case value → result).
pub type Lookups = HashMap<String, HashMap<String, String>>;

fn round_to(v: f64, places: i32) -> f64 {
    let m = 10f64.powi(places);
    (v * m).round() / m
}

pub fn field_value(f: &str, param: &str, info: &FileInfo, ctx: &Ctx) -> Val {
    let t = |s: &str| Val::Text(s.to_string());
    match f {
        "name" => t(&info.name),
        "stem" => t(&info.stem),
        "part" => {
            let n = to_int(param, 1);
            let d = if ctx.delimiter.is_empty() { "_" } else { ctx.delimiter.as_str() };
            let bits: Vec<&str> = info.stem.split(d).collect();
            let idx = if n > 0 { n - 1 } else if n < 0 { bits.len() as i64 + n } else { 0 };
            t(bits.get(idx.max(-1) as usize).filter(|_| idx >= 0).map(|s| s.trim()).unwrap_or(""))
        }
        "ext" => t(&info.ext),
        "ext_upper" => t(&info.ext.to_uppercase()),
        "parent" => t(info.folders.last().map(String::as_str).unwrap_or("")),
        "level" => {
            let n = to_int(param, 1);
            let idx = if n >= 0 { n } else { info.folders.len() as i64 + n };
            t(if idx >= 0 { info.folders.get(idx as usize).map(String::as_str).unwrap_or("") } else { "" })
        }
        "rel_folder" => t(&info.rel_folder),
        "rel_path" => t(&info.rel),
        "full_path" => t(&info.path.display().to_string()),
        "link" => Val::Link {
            name: info.name.clone(),
            uri: url::Url::from_file_path(&info.path)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| info.path.display().to_string()),
            path: info.path.display().to_string(),
        },
        "size_h" => t(&human_size(info.size, ctx.base)),
        "size_kb" => Val::Num(round_to(info.size as f64 / ctx.base, 1)),
        "size_mb" => Val::Num(round_to(info.size as f64 / ctx.base.powi(2), 2)),
        "size_gb" => Val::Num(round_to(info.size as f64 / ctx.base.powi(3), 3)),
        "size_tb" => Val::Num(round_to(info.size as f64 / ctx.base.powi(4), 4)),
        "size_b" => Val::Num(info.size as f64),
        "count" => Val::Num(info.count as f64),
        "modified" => Val::DateTime(info.mtime),
        "modified_d" => Val::Date(info.mtime.date()),
        "created" => info.ctime.map(Val::DateTime).unwrap_or(Val::Text(String::new())),
        "created_d" => info.ctime.map(|c| Val::Date(c.date())).unwrap_or(Val::Text(String::new())),
        "scanned" => Val::Date(ctx.today),
        "status" => Val::Text(status_texts(param).0),
        "text" => t(param),
        "extract" => {
            let r = crate::extract::parse(param);
            let table = r.lookup_key().and_then(|k| ctx.lookups.get(&k));
            let e = crate::extract::apply_with(&r, info, table);
            match e.date {
                Some(d) => Val::Date(d),
                None => Val::Text(e.text),
            }
        }
        _ => t(""),
    }
}

/// The text used to recognise a file in the key column.
fn key_of(v: &Val, is_csv: bool) -> String {
    match v {
        Val::Link { name, path, .. } => norm_key(if is_csv { path } else { name }),
        other => norm_key(&cell_text(other).1),
    }
}

/// (what the copy window shows, what gets copied/pasted)
pub fn cell_text(v: &Val) -> (String, String) {
    match v {
        Val::Text(s) => {
            let s = clean(s);
            (s.clone(), s)
        }
        Val::Num(n) => {
            let s = if n.fract() == 0.0 { format!("{}", *n as i64) } else { format!("{n}") };
            (s.clone(), s)
        }
        Val::Date(d) => {
            let s = d.format("%Y-%m-%d").to_string();
            (s.clone(), s)
        }
        Val::DateTime(d) => {
            let s = d.format("%Y-%m-%d %H:%M").to_string();
            (s.clone(), s)
        }
        Val::Link { name, path, .. } => {
            let q = |x: &str| x.replace('"', "\"\"");
            (name.clone(), format!("=HYPERLINK(\"{}\",\"{}\")", q(path), q(name)))
        }
    }
}

fn excel_serial(d: NaiveDateTime) -> f64 {
    let base = NaiveDate::from_ymd_opt(1899, 12, 30).unwrap().and_hms_opt(0, 0, 0).unwrap();
    let secs = (d - base).num_seconds() as f64;
    (secs / 86400.0 * 1e6).round() / 1e6
}

// ------------------------------------------------------------------ spreadsheet files

pub fn is_csv_path(p: &Path) -> bool {
    matches!(p.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref(), Some("csv" | "tsv" | "txt"))
}

fn is_xlsx_path(p: &Path) -> bool {
    matches!(p.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref(), Some("xlsx" | "xlsm"))
}

fn unsupported(p: &Path) -> UserError {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    UserError(match ext.as_str() {
        "xls" => "Old .xls files aren't supported.\nOpen it in Excel, choose File › Save As › Excel Workbook (.xlsx), then pick the new file.".into(),
        "numbers" => "Numbers files aren't supported.\nIn Numbers choose File › Export To › Excel, then pick the exported file.".into(),
        _ => format!("This app works with .xlsx, .xlsm and .csv files (not .{ext})."),
    })
}

fn locked_msg(p: &Path) -> UserError {
    UserError(format!(
        "Couldn't open {}. If it's open in Excel, close it and try again.",
        p.file_name().unwrap_or_default().to_string_lossy()
    ))
}

pub fn read_csv(path: &Path) -> Res<(Vec<Vec<String>>, u8)> {
    let raw = fs::read(path).map_err(|_| locked_msg(path))?;
    let text = match std::str::from_utf8(&raw) {
        Ok(s) => s.trim_start_matches('\u{feff}').to_string(),
        Err(_) => encoding_rs::WINDOWS_1252.decode(&raw).0.into_owned(),
    };
    let first = text.lines().next().unwrap_or("");
    let mut delim = if path.extension().map(|e| e == "tsv").unwrap_or(false) { b'\t' } else { b',' };
    let mut best = first.matches(delim as char).count();
    for d in *b";\t|" {
        let n = first.matches(d as char).count();
        if n > best {
            best = n;
            delim = d;
        }
    }
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delim)
        .from_reader(text.as_bytes());
    let rows = rdr.records().filter_map(|r| r.ok()).map(|r| r.iter().map(String::from).collect()).collect();
    Ok((rows, delim))
}

pub struct Book {
    pub book: umya::Spreadsheet,
    pub sheet: String,
    pub csv_delim: Option<u8>,
}

impl Book {
    pub fn ws(&self) -> &umya::Worksheet {
        self.book.get_sheet_by_name(&self.sheet).expect("sheet")
    }
    pub fn ws_mut(&mut self) -> &mut umya::Worksheet {
        let name = self.sheet.clone();
        self.book.get_sheet_by_name_mut(&name).expect("sheet")
    }
}

/// Open a spreadsheet; `sheet` None picks the active sheet.
pub fn load_book(path: &Path, sheet: Option<&str>) -> Res<Book> {
    if is_xlsx_path(path) {
        fs::File::open(path).map_err(|_| locked_msg(path))?;
        let book = umya::reader::xlsx::read(path)
            .map_err(|e| UserError(format!("Couldn't read that spreadsheet: {e}")))?;
        let names: Vec<String> = book.get_sheet_collection().iter().map(|s| s.get_name().to_string()).collect();
        let name = match sheet {
            Some(s) if names.iter().any(|n| n == s) => s.to_string(),
            Some(s) if !s.is_empty() => bail!("Sheet \"{s}\" wasn't found. Click \"Reload columns\"."),
            _ => book.get_active_sheet().get_name().to_string(),
        };
        return Ok(Book { book, sheet: name, csv_delim: None });
    }
    if is_csv_path(path) {
        let (rows, delim) = read_csv(path)?;
        let mut book = umya::new_file();
        let ws = book.get_sheet_mut(&0).unwrap();
        ws.set_name("CSV");
        for (r, row) in rows.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                if !v.is_empty() {
                    ws.get_cell_mut((c as u32 + 1, r as u32 + 1)).set_value_string(v.as_str());
                }
            }
        }
        return Ok(Book { book, sheet: "CSV".into(), csv_delim: Some(delim) });
    }
    Err(unsupported(path))
}

pub fn sheet_names(b: &Book) -> Vec<String> {
    if b.csv_delim.is_some() {
        return vec!["(CSV file)".into()];
    }
    b.book.get_sheet_collection().iter().map(|s| s.get_name().to_string()).collect()
}

fn row_values(ws: &umya::Worksheet, r: u32, max_c: u32) -> Vec<String> {
    (1..=max_c).map(|c| ws.get_value((c, r))).collect()
}

pub fn read_headers(ws: &umya::Worksheet, hr: u32) -> Vec<(u32, String)> {
    let max_c = ws.get_highest_column();
    dedupe_headers(row_values(ws, hr, max_c).into_iter().enumerate().map(|(i, v)| (i as u32 + 1, clean(&v))).collect())
}

pub struct Peek {
    pub sheets: Vec<String>,
    pub sheet: String,
    pub header_row: u32,
    pub headers: Vec<(u32, String)>,
}

pub fn peek(path: &Path, sheet: Option<&str>, header_row: Option<u32>) -> Res<Peek> {
    let sheet = sheet.filter(|s| !s.is_empty() && *s != "(CSV file)");
    let b = match load_book(path, sheet) {
        Err(_) if sheet.is_some() => load_book(path, None)?,
        other => other?,
    };
    let ws = b.ws();
    let max_c = ws.get_highest_column();
    let limit = ws.get_highest_row().min(40.max(header_row.unwrap_or(0)));
    let rows: Vec<Vec<String>> = (1..=limit).map(|r| row_values(ws, r, max_c)).collect();
    let hr = header_row.unwrap_or_else(|| detect_header_row(&rows)).max(1);
    Ok(Peek {
        sheets: sheet_names(&b),
        sheet: if b.csv_delim.is_some() { "(CSV file)".into() } else { b.sheet.clone() },
        header_row: hr,
        headers: read_headers(ws, hr),
    })
}

/// The columns that decide where the data ends: the ones the app fills, plus
/// the match column. Columns it leaves alone (checkboxes, notes, statuses
/// that are pre-filled down the sheet) don't count.
fn data_cols(p: &Prepared) -> Vec<u32> {
    let mut cols: Vec<u32> = p.plan.iter().map(|(c, _, _)| *c).collect();
    cols.push(p.key.0);
    cols.sort();
    cols.dedup();
    cols
}

fn last_data_row(ws: &umya::Worksheet, hr: u32, cols: &[u32]) -> u32 {
    let mut r = ws.get_highest_row();
    while r > hr {
        if cols.iter().any(|c| !ws.get_value((*c, r)).trim().is_empty()) {
            return r;
        }
        r -= 1;
    }
    hr
}

fn new_book(columns: &[String]) -> Book {
    let mut book = umya::new_file();
    let ws = book.get_sheet_mut(&0).unwrap();
    ws.set_name("File Tracker");
    for (i, name) in columns.iter().enumerate() {
        let c = i as u32 + 1;
        ws.get_cell_mut((c, 1)).set_value_string(name.as_str());
        let st = ws.get_style_mut((c, 1));
        st.get_font_mut().set_bold(true);
        st.get_font_mut().get_color_mut().set_argb("FFFFFFFF");
        st.set_background_color("FF2F5496");
    }
    Book { book, sheet: "File Tracker".into(), csv_delim: None }
}

fn merged_cells(ws: &umya::Worksheet) -> HashSet<(u32, u32)> {
    let mut set = HashSet::new();
    for r in ws.get_merge_cells() {
        if let Some(((c1, r1), (c2, r2))) = parse_range(&r.get_range()) {
            for c in c1..=c2 {
                for rr in r1..=r2 {
                    if (c, rr) != (c1, r1) {
                        set.insert((c, rr));
                    }
                }
            }
        }
    }
    set
}

fn put(ws: &mut umya::Worksheet, c: u32, r: u32, v: &Val, is_csv: bool) {
    let general = |ws: &umya::Worksheet| {
        ws.get_style((c, r))
            .get_number_format()
            .map(|n| n.get_format_code() == "General" || n.get_format_code().is_empty())
            .unwrap_or(true)
    };
    match v {
        Val::Text(s) => {
            ws.get_cell_mut((c, r)).set_value_string(s.as_str());
        }
        Val::Num(n) => {
            ws.get_cell_mut((c, r)).set_value_number(*n);
        }
        Val::Date(_) | Val::DateTime(_) if is_csv => {
            ws.get_cell_mut((c, r)).set_value_string(cell_text(v).1);
        }
        Val::Date(d) => {
            let g = general(ws);
            ws.get_cell_mut((c, r)).set_value_number(excel_serial(d.and_hms_opt(0, 0, 0).unwrap()));
            if g {
                ws.get_style_mut((c, r)).get_number_format_mut().set_format_code("yyyy-mm-dd");
            }
        }
        Val::DateTime(d) => {
            let g = general(ws);
            ws.get_cell_mut((c, r)).set_value_number(excel_serial(*d));
            if g {
                ws.get_style_mut((c, r)).get_number_format_mut().set_format_code("yyyy-mm-dd hh:mm");
            }
        }
        Val::Link { name, uri, path } => {
            if is_csv {
                ws.get_cell_mut((c, r)).set_value_string(path.as_str());
            } else {
                let cell = ws.get_cell_mut((c, r));
                cell.set_value_string(name.as_str());
                cell.get_hyperlink_mut().set_url(uri.as_str());
                let font = ws.get_style_mut((c, r)).get_font_mut();
                font.get_color_mut().set_argb("FF0563C1");
                font.set_underline("single");
            }
        }
    }
}

fn save_book(b: &Book, path: &Path) -> Res<()> {
    let fail = || {
        UserError(format!(
            "Couldn't save {} — it's probably open in Excel.\nClose it there and try again.",
            path.file_name().unwrap_or_default().to_string_lossy()
        ))
    };
    if is_csv_path(path) {
        let ws = b.ws();
        let (max_r, max_c) = (ws.get_highest_row(), ws.get_highest_column());
        let mut rows: Vec<Vec<String>> = (1..=max_r).map(|r| row_values(ws, r, max_c)).collect();
        while rows.last().map(|r| r.iter().all(|v| v.is_empty())).unwrap_or(false) {
            rows.pop();
        }
        let mut out = b"\xEF\xBB\xBF".to_vec(); // UTF-8 BOM so Excel reads accents correctly
        {
            let mut w = csv::WriterBuilder::new().delimiter(b.csv_delim.unwrap_or(b',')).from_writer(&mut out);
            for row in rows {
                w.write_record(row)?;
            }
            w.flush()?;
        }
        fs::write(path, out).map_err(|_| fail())
    } else {
        umya::writer::xlsx::write(&b.book, path).map_err(|_| fail())
    }
}

fn make_backup(path: &Path) -> Res<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new(".")).join(BACKUP_DIR);
    fs::create_dir_all(&dir)?;
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let dest = dir.join(format!("{stem} {}{ext}", Local::now().format("%Y-%m-%d %H%M%S")));
    fs::copy(path, &dest)?;
    let prefix = format!("{stem} ");
    let mut olds: Vec<(std::time::SystemTime, PathBuf)> = fs::read_dir(&dir)?
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with(&prefix) && n.ends_with(&ext)
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    olds.sort();
    if olds.len() > MAX_BACKUPS {
        for (_, p) in &olds[..olds.len() - MAX_BACKUPS] {
            let _ = fs::remove_file(p);
        }
    }
    Ok(dest)
}

// ------------------------------------------------------------------ jobs

#[derive(Clone, Debug)]
pub struct MapEntry {
    pub col: u32,
    pub header: String,
    pub field: String,
    pub param: String,
}

#[derive(Clone)]
pub struct Job {
    pub folder: PathBuf,
    pub scan: std::sync::Arc<ScanOptions>,
    pub sheet_path: Option<PathBuf>,
    pub create_new: bool,
    pub new_columns: Vec<String>,
    pub sheet_name: String,
    pub header_row: u32,
    pub mapping: Vec<MapEntry>,
    pub key_header: String,
    pub delimiter: String,
    pub fresh: bool,
    pub update_existing: bool,
    pub backup: bool,
    pub only_new: bool,
    pub row_mode: RowMode,
    pub size_base: u32,
}

struct Prepared {
    book: Book,
    hr: u32,
    headers: Vec<(u32, String)>,
    plan: Vec<(u32, String, String)>,
    key: (u32, String, String),
    is_csv: bool,
    lookups: Lookups,
}

fn prepare(j: &Job, progress: &dyn Fn(String)) -> Res<Prepared> {
    if !j.folder.is_dir() {
        bail!("Choose a folder to scan (section 1).");
    }
    let is_csv = j.sheet_path.as_deref().map(is_csv_path).unwrap_or(false);
    let (book, hr) = if j.create_new {
        (new_book(&j.new_columns), 1)
    } else {
        let Some(p) = j.sheet_path.as_deref().filter(|p| p.is_file()) else {
            bail!("Choose your tracking spreadsheet (section 2).");
        };
        progress("Opening spreadsheet…".into());
        (load_book(p, Some(&j.sheet_name).filter(|s| *s != "(CSV file)").map(|s| s.as_str()))?, j.header_row.max(1))
    };
    let headers = read_headers(book.ws(), hr);
    if headers.is_empty() {
        bail!("No column headers were found on row {hr}.");
    }
    let by_text = |h: &str| headers.iter().find(|(_, t)| t.to_lowercase() == h.to_lowercase()).map(|(c, _)| *c);
    let mut plan = Vec::new();
    for m in j.mapping.iter().filter(|m| !m.field.is_empty()) {
        let same = headers.iter().any(|(c, t)| *c == m.col && t.to_lowercase() == m.header.to_lowercase());
        let Some(col) = (if same { Some(m.col) } else { by_text(&m.header) }) else {
            bail!("Column \"{}\" isn't on row {hr} any more. Click \"Reload columns\" and try again.", m.header);
        };
        plan.push((col, m.field.clone(), m.param.clone()));
    }
    if plan.is_empty() {
        bail!("Choose what goes into at least one column (section 3).");
    }
    let key_col = by_text(&j.key_header);
    let key = plan
        .iter()
        .find(|(c, f, _)| Some(*c) == key_col && !BAD_KEYS.contains(&f.as_str()))
        .or_else(|| KEY_PRIORITY.iter().find_map(|k| plan.iter().find(|(_, f, _)| f == k)))
        .or_else(|| plan.iter().find(|(_, f, _)| !BAD_KEYS.contains(&f.as_str())))
        .unwrap_or(&plan[0])
        .clone();
    let lookups = load_lookups(&book, &plan);
    Ok(Prepared { book, hr, headers, plan, key, is_csv, lookups })
}

/// Read the tables that custom rules look values up in (e.g. initials → name
/// from a "Shooter List" sheet). Missing sheets just give empty tables.
fn load_lookups(book: &Book, plan: &[(u32, String, String)]) -> Lookups {
    let mut out = Lookups::new();
    if book.csv_delim.is_some() {
        return out;
    }
    for (_, f, prm) in plan {
        if f != "extract" {
            continue;
        }
        let r = crate::extract::parse(prm);
        let Some(key) = r.lookup_key() else { continue };
        if out.contains_key(&key) {
            continue;
        }
        let mut table = HashMap::new();
        if let Some(ws) = book.book.get_sheet_by_name(&r.lookup_sheet) {
            let (fc, rc) = (letters_to_col(r.lookup_find.trim()), letters_to_col(r.lookup_return.trim()));
            if fc > 0 && rc > 0 {
                for row in 1..=ws.get_highest_row() {
                    let k = clean(&ws.get_value((fc, row)));
                    let v = clean(&ws.get_value((rc, row)));
                    if !k.is_empty() && !v.is_empty() {
                        table.entry(k.to_lowercase()).or_insert(v);
                    }
                }
            }
        }
        out.insert(key, table);
    }
    out
}

fn scan_for(j: &Job, progress: &dyn Fn(String)) -> Vec<FileInfo> {
    progress("Scanning folder…".into());
    let skip: Vec<PathBuf> = j.sheet_path.iter().cloned().collect();
    let files = scan_folder(&j.folder, &j.scan, &skip, progress);
    match &j.row_mode {
        RowMode::Files => files,
        mode => folder_rows(&j.folder, &files, mode),
    }
}

fn existing_rows(ws: &umya::Worksheet, hr: u32, last: u32, key_col: u32) -> HashMap<String, Vec<u32>> {
    let mut m: HashMap<String, Vec<u32>> = HashMap::new();
    for r in hr + 1..=last {
        let k = norm_key(&ws.get_value((key_col, r)));
        if !k.is_empty() {
            m.entry(k).or_default().push(r);
        }
    }
    m
}

/// How a running-number column is written, so the grid can renumber rows
/// after sorting them.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexSpec {
    pub pre: String,
    pub width: usize,
    pub suf: String,
    /// the number the first new row gets
    pub first: i64,
    pub plain: bool,
}

impl IndexSpec {
    pub fn text(&self, n: i64) -> String {
        if self.plain {
            n.to_string()
        } else {
            format!("{}{:0w$}{}", self.pre, n, self.suf, w = self.width)
        }
    }
}

/// Running-number columns carry on from the highest number already in the sheet.
struct Indexer {
    state: HashMap<u32, (String, usize, String, i64, bool)>,
}

impl Indexer {
    fn new(ws: &umya::Worksheet, hr: u32, last: u32, plan: &[(u32, String, String)]) -> Self {
        let mut state = HashMap::new();
        for (c, f, prm) in plan {
            if f != "index" {
                continue;
            }
            let mut pat = if prm.trim().is_empty() { "#".to_string() } else { prm.trim().to_string() };
            if !pat.contains('#') {
                pat.push('#');
            }
            let start = pat.find('#').unwrap();
            let width = pat[start..].chars().take_while(|ch| *ch == '#').count();
            let (pre, suf) = (pat[..start].to_string(), pat[start + width..].to_string());
            let rx = regex::RegexBuilder::new(&format!("^{}(\\d+){}$", regex::escape(&pre), regex::escape(&suf)))
                .case_insensitive(true)
                .build()
                .unwrap();
            let mut top = 0i64;
            for r in hr + 1..=last {
                let v = ws.get_value((*c, r));
                let v = v.trim();
                if let Some(m) = rx.captures(v) {
                    top = top.max(m[1].parse().unwrap_or(0));
                } else if pre.is_empty() && suf.is_empty() {
                    if let Ok(n) = v.parse::<f64>() {
                        top = top.max(n as i64);
                    }
                }
            }
            state.insert(*c, (pre, width, suf, top, pat == "#"));
        }
        Indexer { state }
    }
    fn spec(&self, c: u32) -> Option<IndexSpec> {
        self.state.get(&c).map(|s| IndexSpec { pre: s.0.clone(), width: s.1, suf: s.2.clone(), first: s.3 + 1, plain: s.4 })
    }

    fn next(&mut self, c: u32) -> Val {
        let s = self.state.get_mut(&c).unwrap();
        s.3 += 1;
        if s.4 {
            Val::Num(s.3 as f64)
        } else {
            Val::Text(format!("{}{:0w$}{}", s.0, s.3, s.2, w = s.1))
        }
    }
}

pub struct RunResult {
    pub path: PathBuf,
    pub files: usize,
    pub added: usize,
    pub matched: usize,
    pub missing: usize,
    pub backup: Option<PathBuf>,
    pub status_col: bool,
    pub refreshed: bool,
}

/// Scan the folder and update (or create) the spreadsheet.
pub fn run_job(j: &Job, progress: &dyn Fn(String)) -> Res<RunResult> {
    let Some(sheet_path) = j.sheet_path.clone() else { bail!("Choose where to save the spreadsheet.") };
    let mut p = prepare(j, progress)?;
    let files = scan_for(j, progress);
    let hr = p.hr;
    let is_csv = p.is_csv;
    let all_cols: Vec<u32> = p.headers.iter().map(|(c, _)| *c).collect();
    let fill_cols = data_cols(&p);
    let ws = p.book.ws_mut();

    let mut last = last_data_row(ws, hr, &fill_cols);
    // new rows copy the look of the last row with data (not the header)
    let template: Vec<(u32, umya::Style)> = if last > hr {
        all_cols.iter().map(|c| (*c, ws.get_style((*c, last)).clone())).collect()
    } else if hr < ws.get_highest_row() {
        all_cols.iter().map(|c| (*c, ws.get_style((*c, hr + 1)).clone())).collect()
    } else {
        vec![]
    };
    if files.is_empty() && last > hr && !j.fresh {
        bail!("No files were found in that folder (check the folder and the filters in section 1). Nothing was changed.");
    }
    let merged = merged_cells(ws);
    if j.fresh && last > hr {
        for r in hr + 1..=last {
            for c in &all_cols {
                if !merged.contains(&(*c, r)) {
                    ws.get_cell_mut((*c, r)).set_value_string("");
                }
            }
        }
        last = hr;
    }
    let mut existing = existing_rows(ws, hr, last, p.key.0);
    let mut indexer = Indexer::new(ws, hr, last, &p.plan);
    let ctx = Ctx { delimiter: j.delimiter.clone(), today: Local::now().date_naive(), base: if j.size_base == 1024 { 1024.0 } else { 1000.0 }, lookups: p.lookups.clone() };
    let (mut added, mut matched) = (0, 0);
    let (_kc, kf, kp) = p.key.clone();

    for (i, info) in files.iter().enumerate() {
        let k = key_of(&field_value(&kf, &kp, info, &ctx), is_csv);
        let hit = existing.get_mut(&k).and_then(|v| if v.is_empty() { None } else { Some(v.remove(0)) });
        let (r, is_new) = match hit {
            Some(r) => {
                matched += 1;
                (r, false)
            }
            None => {
                last += 1;
                added += 1;
                // keep rows that are already formatted (e.g. a pre-made template area)
                for (c, st) in &template {
                    if ws.get_style((*c, last)) == &umya::Style::default() {
                        ws.set_style((*c, last), st.clone());
                    }
                }
                (last, true)
            }
        };
        for (c, f, prm) in &p.plan {
            if !is_new {
                if NEW_ONLY.contains(&f.as_str()) {
                    continue;
                }
                if f != "status" && !j.update_existing {
                    continue;
                }
            }
            if merged.contains(&(*c, r)) {
                continue;
            }
            let v = if f == "index" { indexer.next(*c) } else { field_value(f, prm, info, &ctx) };
            // nothing found: leave the cell alone. A refresh never replaces
            // something with nothing (e.g. an environment typed by hand)
            if matches!(&v, Val::Text(t) if t.trim().is_empty()) {
                continue;
            }
            put(ws, *c, r, &v, is_csv);
        }
        if (i + 1) % 500 == 0 {
            progress(format!("Updating rows… {} of {}", i + 1, files.len()));
        }
    }

    let status_cols: Vec<(u32, String)> =
        p.plan.iter().filter(|(_, f, _)| f == "status").map(|(c, _, prm)| (*c, status_texts(prm).1)).collect();
    let mut missing = 0;
    for rows in existing.values() {
        for r in rows {
            missing += 1;
            for (c, txt) in &status_cols {
                put(ws, *c, *r, &Val::Text(txt.clone()), is_csv);
            }
        }
    }

    if !is_csv {
        for t in ws.get_tables_mut().iter_mut() {
            let (s, e) = t.get_area().clone();
            let (sc, sr, ec, er) = (*s.get_col_num(), *s.get_row_num(), *e.get_col_num(), *e.get_row_num());
            if sr == hr && last > er {
                t.set_area(((sc, sr), (ec, last)));
            }
        }
        if let Some(af) = ws.get_auto_filter_mut() {
            if let Some(((c1, r1), (c2, r2))) = parse_range(&af.get_range().get_range()) {
                if r1 == hr && last > r2 {
                    af.get_range_mut().set_range(format!("{}{}:{}{}", col_letter(c1), r1, col_letter(c2), last));
                }
            }
        }
        if j.create_new {
            let maxc = *all_cols.iter().max().unwrap_or(&1);
            ws.set_auto_filter(format!("A1:{}{}", col_letter(maxc), last.max(1)));
            for c in &all_cols {
                let w = (1..=last.min(500)).map(|r| ws.get_value((*c, r)).chars().count()).max().unwrap_or(10);
                ws.get_column_dimension_mut(&col_letter(*c)).set_width(((w + 2) as f64).clamp(10.0, 60.0));
            }
        }
    }

    progress("Saving…".into());
    let backup = if !j.create_new && j.backup { Some(make_backup(&sheet_path)?) } else { None };
    save_book(&p.book, &sheet_path)?;
    Ok(RunResult {
        path: sheet_path,
        files: files.len(),
        added,
        matched,
        missing,
        backup,
        status_col: !status_cols.is_empty(),
        refreshed: j.update_existing,
    })
}

pub struct Preview {
    pub columns: Vec<String>,
    /// each cell: (shown, copied)
    pub rows: Vec<Vec<(String, String)>>,
    pub files: usize,
    pub skipped: usize,
    pub from_sheet: bool,
    /// column letter for each grid column (A, B, …)
    pub letters: Vec<String>,
    /// whether each grid column is filled by the app (false = "leave as is")
    pub filled: Vec<bool>,
    /// the spreadsheet row the first grid row lands on when pasted
    pub start_row: u32,
    /// a spread of scanned files, used as examples in the column builder
    pub samples: Vec<FileInfo>,
    /// for running-number columns: how to renumber rows after sorting
    pub index_specs: Vec<Option<IndexSpec>>,
    /// the file or folder behind each row (for Show in Finder / Open)
    pub paths: Vec<PathBuf>,
    /// lookup tables, so the column window can show looked-up results
    pub lookups: Lookups,
}

/// Scan and lay the files out in the spreadsheet's column order. Nothing is saved.
pub fn preview_job(j: &Job, progress: &dyn Fn(String)) -> Res<Preview> {
    let p = prepare(j, progress)?;
    let files = scan_for(j, progress);
    let ws = p.book.ws();
    let use_sheet = !j.create_new && j.only_new;
    let all_cols: Vec<u32> = p.headers.iter().map(|(c, _)| *c).collect();
    let sheet_last = if j.create_new { p.hr } else { last_data_row(ws, p.hr, &data_cols(&p)) };
    let last = if use_sheet { sheet_last } else { p.hr };
    let existing = if use_sheet { existing_rows(ws, p.hr, last, p.key.0) } else { HashMap::new() };
    let mut indexer = Indexer::new(ws, p.hr, last, &p.plan);

    let (lo, hi) = (*all_cols.iter().min().unwrap(), *all_cols.iter().max().unwrap());
    let index_specs: Vec<Option<IndexSpec>> = (lo..=hi).map(|c| indexer.spec(c)).collect();
    let header_of: HashMap<u32, &String> = p.headers.iter().map(|(c, h)| (*c, h)).collect();
    let columns: Vec<String> = (lo..=hi)
        .map(|c| header_of.get(&c).map(|h| h.to_string()).unwrap_or_else(|| format!("(column {})", col_letter(c))))
        .collect();
    let fields: HashMap<u32, (&String, &String)> = p.plan.iter().map(|(c, f, prm)| (*c, (f, prm))).collect();
    let ctx = Ctx { delimiter: j.delimiter.clone(), today: Local::now().date_naive(), base: if j.size_base == 1024 { 1024.0 } else { 1000.0 }, lookups: p.lookups.clone() };
    let (_, kf, kp) = &p.key;
    let (mut rows, mut skipped, mut paths) = (Vec::new(), 0, Vec::new());
    for info in &files {
        if !existing.is_empty() && existing.contains_key(&key_of(&field_value(kf, kp, info, &ctx), false)) {
            skipped += 1;
            continue;
        }
        let row = (lo..=hi)
            .map(|c| match fields.get(&c) {
                None => (String::new(), String::new()),
                Some((f, prm)) => {
                    let v = if f.as_str() == "index" { indexer.next(c) } else { field_value(f, prm, info, &ctx) };
                    cell_text(&v)
                }
            })
            .collect();
        rows.push(row);
        paths.push(info.path.clone());
    }
    Ok(Preview {
        letters: (lo..=hi).map(col_letter).collect(),
        filled: (lo..=hi).map(|c| fields.contains_key(&c)).collect(),
        // rows go after the sheet's last row with data
        start_row: sheet_last + 1,
        index_specs,
        paths,
        lookups: p.lookups.clone(),
        samples: {
            let step = (files.len() / 60).max(1);
            files.iter().step_by(step).take(60).cloned().collect()
        },
        columns,
        rows,
        files: files.len(),
        skipped,
        from_sheet: !j.create_new,
    })
}

pub fn open_path(p: &Path) {
    let _ = open::that_detached(p);
}

/// What the computer's file browser is called.
pub fn file_manager_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Finder"
    } else if cfg!(target_os = "windows") {
        "File Explorer"
    } else {
        "file manager"
    }
}

/// The command that shows (selects) a file in Finder / File Explorer.
pub fn reveal_command(p: &Path) -> (String, Vec<std::ffi::OsString>) {
    if cfg!(target_os = "macos") {
        ("open".into(), vec!["-R".into(), p.as_os_str().to_owned()])
    } else if cfg!(target_os = "windows") {
        // explorer wants "/select," and the path as separate words
        ("explorer".into(), vec!["/select,".into(), p.as_os_str().to_owned()])
    } else {
        let dir = p.parent().unwrap_or(p);
        ("xdg-open".into(), vec![dir.as_os_str().to_owned()])
    }
}

/// Show these files in Finder / File Explorer.
pub fn reveal(paths: &[PathBuf]) {
    for p in paths.iter().take(10) {
        let (cmd, args) = reveal_command(p);
        let mut c = std::process::Command::new(cmd);
        c.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x08000000); // no console window
        }
        let _ = c.spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(dir: &Path, sheet: &str, mapping: Vec<MapEntry>) -> Job {
        Job {
            folder: dir.join("Project Alpha"),
            scan: std::sync::Arc::new(ScanOptions {
                recursive: true,
                skip_hidden: true,
                extensions: vec![],
                exclude: vec!["Archive".into()],
            }),
            sheet_path: Some(dir.join(sheet)),
            create_new: false,
            new_columns: vec![],
            sheet_name: "Register".into(),
            header_row: 3,
            mapping,
            key_header: "Link".into(),
            delimiter: "_".into(),
            fresh: false,
            update_existing: true,
            backup: true,
            only_new: true,
            row_mode: RowMode::Files,
            size_base: 1024,
        }
    }

    fn mapping_for(path: &Path, sheet: Option<&str>, hr: Option<u32>, over: &[(&str, &str, &str)]) -> Vec<MapEntry> {
        let p = peek(path, sheet, hr).unwrap();
        p.headers
            .iter()
            .map(|(c, h)| {
                let f = auto_field(h);
                let mut m = MapEntry { col: *c, header: h.clone(), field: f.into(), param: field(f).default.unwrap_or("").into() };
                if let Some((_, f, prm)) = over.iter().find(|(hh, _, _)| hh == h) {
                    m.field = f.to_string();
                    m.param = prm.to_string();
                }
                m
            })
            .collect()
    }

    #[test]
    fn end_to_end() {
        let Ok(dir) = std::env::var("FT_SANDBOX").map(PathBuf::from) else {
            println!("skipped: set FT_SANDBOX to a folder made by tests/make_fixture.py");
            return;
        };
        let quiet = |_m: String| {};
        let xlsx = dir.join("Tracker.xlsx");
        let p = peek(&xlsx, None, None).unwrap();
        println!("peek: sheet={} row={} headers={:?}", p.sheet, p.header_row, p.headers);
        assert_eq!(p.header_row, 3);
        let map = mapping_for(&xlsx, Some("Register"), Some(3), &[
            ("No.", "index", "#"),
            ("Document No", "part", "1"),
            ("Title", "part", "2"),
            ("Rev", "part", "-1"),
            ("Discipline", "level", "2"),
        ]);
        let j = job(&dir, "Tracker.xlsx", map.clone());

        let pv = preview_job(&j, &quiet).unwrap();
        println!("preview cols {:?}", pv.columns);
        for r in &pv.rows {
            println!("  {:?}", r.iter().map(|c| c.1.as_str()).collect::<Vec<_>>());
        }
        assert_eq!(pv.rows.len(), 6);

        let r1 = run_job(&j, &quiet).unwrap();
        println!("run1 added={} matched={} missing={}", r1.added, r1.matched, r1.missing);
        assert_eq!(r1.added, 6);
        let r2 = run_job(&j, &quiet).unwrap();
        assert_eq!((r2.added, r2.matched), (0, 6), "re-run must not duplicate");
        let pv2 = preview_job(&j, &quiet).unwrap();
        assert_eq!((pv2.rows.len(), pv2.skipped), (0, 6));

        fs::remove_file(dir.join("Project Alpha/readme.txt")).unwrap();
        fs::write(dir.join("Project Alpha/Specs/PRJ001_New-Item_RevA.xlsx"), vec![b'y'; 5000]).unwrap();
        let r3 = run_job(&j, &quiet).unwrap();
        println!("run3 added={} matched={} missing={}", r3.added, r3.matched, r3.missing);
        assert_eq!((r3.added, r3.missing), (1, 1));

        // CSV with ; delimiter, DOC-### ids and Present/Missing
        let csvp = dir.join("list.csv");
        let mut m2 = mapping_for(&csvp, None, None, &[("Ref", "index", "DOC-###")]);
        m2.retain(|m| m.header != "Comments");
        let mut j2 = job(&dir, "list.csv", m2);
        j2.sheet_name = String::new();
        j2.header_row = 1;
        j2.key_header = "File Name".into();
        j2.scan = std::sync::Arc::new(ScanOptions { recursive: true, skip_hidden: true, extensions: vec!["pdf".into()], exclude: vec!["Archive".into()] });
        let r = run_job(&j2, &quiet).unwrap();
        println!("csv added={} missing={}", r.added, r.missing);
        println!("{}", fs::read_to_string(&csvp).unwrap());

        // brand-new workbook from the default columns
        let cols = split_list(DEFAULT_NEW_COLUMNS);
        let m3: Vec<MapEntry> = cols
            .iter()
            .enumerate()
            .map(|(i, h)| MapEntry { col: i as u32 + 1, header: h.clone(), field: auto_field(h).into(), param: field(auto_field(h)).default.unwrap_or("").into() })
            .collect();
        let mut j3 = job(&dir, "New Tracker.xlsx", m3);
        j3.create_new = true;
        j3.new_columns = cols;
        j3.header_row = 1;
        j3.key_header = String::new();
        let r = run_job(&j3, &quiet).unwrap();
        println!("new added={}", r.added);
        // preview with no spreadsheet at all
        j3.sheet_path = None;
        let pv = preview_job(&j3, &quiet).unwrap();
        assert_eq!(pv.rows.len(), 6);
        assert!(!pv.from_sheet);
    }

    #[test]
    fn folder_sizes_packages_and_shortcuts() {
        let base = std::env::temp_dir().join(format!("ft-sizes-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("Media");
        let mk = |rel: &str, bytes: usize| {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, vec![b'x'; bytes]).unwrap();
        };
        mk("Shot_010/plates/f_0001.exr", 1000);
        mk("Shot_010/plates/f_0002.exr", 1000);
        mk("Shot_010/notes.txt", 50);
        mk("Shot_020/plates/f_0001.exr", 4000);
        mk("Edit.fcpbundle/Media/clip.mov", 7000); // a Mac package
        mk("Edit.fcpbundle/CurrentVersion.fcpevent", 300);
        mk("readme.txt", 10);
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("Shot_020/plates/f_0001.exr"), root.join("link.exr")).unwrap();

        let o = ScanOptions { recursive: true, skip_hidden: true, extensions: vec![], exclude: vec![] };
        let files = scan_folder(&root, &o, &[], &|_| {});
        let by = |n: &str| files.iter().find(|f| f.name == n).map(|f| (f.size, f.count));
        // the package is one item with the total of everything inside, not opened up
        assert_eq!(by("Edit.fcpbundle"), Some((7300, 2)));
        assert!(by("clip.mov").is_none());
        #[cfg(unix)]
        assert_eq!(by("link.exr"), Some((4000, 1)), "shortcuts are measured by what they point to");

        let lvl = folder_rows(&root, &files, &RowMode::FolderLevel(1));
        let row = |rows: &[FileInfo], n: &str| rows.iter().find(|f| f.name == n).map(|f| (f.size, f.count, f.is_folder));
        assert_eq!(row(&lvl, "Shot_010"), Some((2050, 3, true)));
        assert_eq!(row(&lvl, "Shot_020"), Some((4000, 1, true)));
        assert_eq!(lvl.len(), 2, "files above level 1 (and packages) aren't folder rows: {:?}", lvl.iter().map(|f| &f.name).collect::<Vec<_>>());

        let with = folder_rows(&root, &files, &RowMode::FoldersWithFiles);
        let names: Vec<&str> = with.iter().map(|f| f.rel.as_str()).collect();
        assert!(names.contains(&"Shot_010") && names.contains(&"") , "{names:?}");
        let plates = with.iter().find(|f| f.rel == format!("Shot_010{}plates", std::path::MAIN_SEPARATOR)).unwrap();
        assert_eq!((plates.size, plates.count, plates.name.as_str()), (2000, 2, "plates"));
        assert_eq!(plates.folders, vec!["Media".to_string(), "Shot_010".to_string()]);

        // GB / TB columns
        let ctx = Ctx { delimiter: "_".into(), today: Local::now().date_naive(), base: 1000.0, lookups: Default::default() };
        let mut big = lvl[0].clone();
        big.size = 2_345_000_000_000;
        assert!(matches!(field_value("size_tb", "", &big, &ctx), Val::Num(v) if v == 2.345));
        assert!(matches!(field_value("size_gb", "", &big, &ctx), Val::Num(v) if v == 2345.0));
        assert!(matches!(field_value("size_h", "", &big, &ctx), Val::Text(t) if t == "2.35 TB"));
        assert!(matches!(field_value("count", "", &lvl[0], &ctx), Val::Num(v) if v == 3.0));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn setups_load_whatever_their_name() {
        let dir = std::env::temp_dir().join(format!("ft-setup-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let good = r#"{"folder":"","row_mode":"pattern","mapping":{"CAM OP":["name",""]}}"#;
        for name in ["a.json", "a.json.txt", "a"] {
            let p = dir.join(name);
            fs::write(&p, format!("\u{feff}{good}")).unwrap();
            let s = read_setup(&p).unwrap();
            assert_eq!(s.row_mode, "pattern");
            assert!(is_setup_file(&p));
        }
        fs::write(dir.join("other.json"), r#"{"name":"some other app"}"#).unwrap();
        assert!(read_setup(&dir.join("other.json")).is_err());
        fs::write(dir.join("text.json"), "hello").unwrap();
        assert!(!is_setup_file(&dir.join("text.json")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn helpers() {
        assert!(wildcard("old*", "Old drawings"));
        assert!(wildcard("*.tmp", "a.TMP"));
        assert!(!wildcard("*.tmp", "a.tmpx"));
        assert_eq!(col_letter(28), "AB");
        assert_eq!(parse_range("A3:J11"), Some(((1, 3), (10, 11))));
        assert_eq!(auto_field("Date Modified"), "modified_d");
        assert_eq!(auto_field("Reviewer Notes"), "");
        assert_eq!(human_size(2_500_000, 1024.0), "2.38 MB");
        assert_eq!(human_size(2_500_000, 1000.0), "2.50 MB");
        assert_eq!(human_size(3_200_000_000_000, 1000.0), "3.20 TB");
        assert_eq!(human_size(512, 1000.0), "512 bytes");
        assert_eq!(auto_field("Folder Size"), "size_h");
        assert_eq!(auto_field("Size (GB)"), "size_gb");
        assert_eq!(auto_field("Size TB"), "size_tb");
        assert_eq!(auto_field("File count"), "count");
        assert_eq!(auto_field("Clip Count"), "count");
        assert_eq!(auto_field("Card Size (GB)"), "size_gb");
        assert_eq!(auto_field("Folder"), "parent");
        assert_eq!(auto_field_for("Folder", true), "name");
        assert_eq!(auto_field_for("Folder Size", true), "size_h");
    }
}

#[cfg(test)]
mod reveal_tests {
    use super::*;
    #[test]
    fn reveal_command_for_this_computer() {
        let p = PathBuf::from("/a b/c.pdf");
        let (cmd, args) = reveal_command(&p);
        if cfg!(target_os = "macos") {
            assert_eq!((cmd.as_str(), args.len()), ("open", 2));
        } else if cfg!(target_os = "windows") {
            assert_eq!((cmd.as_str(), args[0].to_str()), ("explorer", Some("/select,")));
        } else {
            assert_eq!((cmd.as_str(), args[0].to_str()), ("xdg-open", Some("/a b")));
        }
    }
}

#[cfg(test)]
mod setup_harness {
    use super::*;

    /// Load a saved setup the way the app does and build a job from it.
    fn job_from_setup(s: &Settings, folder: &Path, sheet: &Path) -> Job {
        let p = peek(sheet, Some(&s.sheet_name), Some(s.header_row)).expect("peek");
        let mapping = p
            .headers
            .iter()
            .map(|(c, h)| {
                let (f, prm) = s.mapping.get(h).cloned().unwrap_or_else(|| {
                    let f = auto_field_for(h, s.row_mode != "files");
                    (f.to_string(), field(f).default.unwrap_or("").to_string())
                });
                MapEntry { col: *c, header: h.clone(), field: f, param: prm }
            })
            .collect();
        Job {
            folder: folder.to_path_buf(),
            scan: std::sync::Arc::new(ScanOptions {
                recursive: s.recursive,
                skip_hidden: s.skip_hidden,
                extensions: split_list(&s.extensions),
                exclude: split_list(&s.exclude),
            }),
            sheet_path: Some(sheet.to_path_buf()),
            create_new: false,
            new_columns: vec![],
            sheet_name: p.sheet,
            header_row: s.header_row,
            mapping,
            key_header: s.key_header.clone(),
            delimiter: s.delimiter.clone(),
            fresh: false,
            update_existing: s.update_existing,
            backup: false,
            only_new: s.only_new,
            row_mode: RowMode::from_settings(&s.row_mode, s.folder_level, &s.folder_pattern),
            size_base: s.size_base,
        }
    }

    /// cargo test setup_harness -- --ignored --nocapture
    /// with FT_SETUP, FT_FOLDER and FT_SHEET (a copy: it gets updated)
    #[test]
    #[ignore]
    fn run_setup() {
        let env = |k: &str| PathBuf::from(std::env::var(k).unwrap_or_else(|_| panic!("set {k}")));
        let s = load_settings(&env("FT_SETUP")).expect("setup");
        let (folder, sheet) = (env("FT_FOLDER"), env("FT_SHEET"));
        let quiet = |_m: String| {};
        let mut j = job_from_setup(&s, &folder, &sheet);
        for (label, only_new) in [("NEW CARDS ONLY (what the grid shows)", true), ("ALL CARDS", false)] {
            j.only_new = only_new;
            let p = preview_job(&j, &quiet).expect("preview");
            println!("\n=== {label}: {} rows, {} already in sheet, paste at row {}", p.rows.len(), p.skipped, p.start_row);
            println!("{}", p.columns.iter().take(15).cloned().collect::<Vec<_>>().join(" | "));
            for r in &p.rows {
                println!("{}", r.iter().take(15).map(|c| c.1.as_str()).collect::<Vec<_>>().join(" | "));
            }
            println!("lookup tables: {:?}", p.lookups.values().map(|t| t.len()).collect::<Vec<_>>());
        }
        j.only_new = true;
        let r = run_job(&j, &quiet).expect("run");
        println!("\nUPDATE: added {}, matched {}, missing {}", r.added, r.matched, r.missing);
    }
}
