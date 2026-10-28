// File Tracker — scan a folder and lay the files out in your spreadsheet's
// column layout in a live grid you can copy from, or update the sheet directly.
// Native GUI for macOS and Windows.
#![cfg_attr(windows, windows_subsystem = "windows")] // no console window on Windows

mod builder;
mod core;
mod extract;
mod gridview;
mod panels;
mod sorting;
mod update;

use crate::core::*;
use eframe::egui::{self, Color32, Pos2, Rect, RichText};
use notify::Watcher;
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const APP_NAME: &str = "File Tracker";
const HINT: Color32 = Color32::from_rgb(128, 128, 128);
/// wait this long after the last change before refreshing the grid
const SETTINGS_DEBOUNCE: Duration = Duration::from_millis(450);
const FILES_DEBOUNCE: Duration = Duration::from_millis(900);

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([1420.0, 900.0])
            .with_min_inner_size([1000.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(APP_NAME, options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

struct MapRow {
    col: u32,
    header: String,
    field: String,
    param: String,
}

enum Msg {
    Progress(String),
    Done(RunResult, bool),
    Error(String),
}

/// Where an in-app update is up to.
enum Upd {
    Idle,
    Checking { manual: bool },
    UpToDate,
    Available(update::Release),
    Downloading { rel: update::Release, got: u64, total: u64 },
    Installing,
    Failed { msg: String, rel: Option<update::Release> },
}

enum UpdMsg {
    Checked(Result<update::Release, String>, bool),
    Progress(u64, u64),
    Installed(Result<PathBuf, String>),
}

/// What the mouse is over in the grid.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    /// the sort arrow in a column heading
    Sort(usize),
    Cell(usize, usize),
    Row(usize),
    Col(usize),
    Corner,
}

#[derive(Clone, Copy, PartialEq)]
enum DragKind {
    Cell,
    Row,
    Col,
}

/// A rectangular selection in grid coordinates: (row, column), inclusive.
#[derive(Clone, Copy, Debug)]
struct Sel {
    a: (usize, usize),
    b: (usize, usize),
}

impl Sel {
    fn bounds(&self) -> (usize, usize, usize, usize) {
        (self.a.0.min(self.b.0), self.a.0.max(self.b.0), self.a.1.min(self.b.1), self.a.1.max(self.b.1))
    }
    fn contains(&self, r: usize, c: usize) -> bool {
        let (r0, r1, c0, c1) = self.bounds();
        (r0..=r1).contains(&r) && (c0..=c1).contains(&c)
    }
    fn has_col(&self, c: usize) -> bool {
        let (_, _, c0, c1) = self.bounds();
        (c0..=c1).contains(&c)
    }
    fn has_row(&self, r: usize) -> bool {
        let (r0, r1, _, _) = self.bounds();
        (r0..=r1).contains(&r)
    }
}

#[derive(PartialEq)]
enum Action {
    BrowseFolder,
    BrowseSheet,
    ReloadColumns(bool),
    ApplySheetMode,
    UseNewColumns,
    SaveSetup,
    LoadSetup,
    OpenSheet,
    Update,
    Refresh,
    Copy,
}

/// The right-click menu on the grid.
struct GridMenu {
    pos: Pos2,
    target: Hit,
    /// true on the frame it opened (so that click doesn't close it again)
    fresh: bool,
}

struct Grid {
    data: Option<Preview>,
    /// shown instead of the grid (e.g. "choose a folder")
    message: Option<String>,
    busy: bool,
    rx: Option<mpsc::Receiver<Result<Preview, String>>>,
    signature: String,
    refresh_at: Option<Instant>,
    updated_at: String,
    sel: Option<Sel>,
    drag: Option<DragKind>,
    copy_msg: String,
    /// visible on-screen areas of grid cells this frame, for mouse hit-testing
    hits: Vec<(Rect, Hit)>,
    /// display order of the rows (indexes into data.rows), after sorting
    order: Vec<usize>,
}

struct App {
    s: Settings,
    rows: Vec<MapRow>,
    sheets: Vec<String>,
    status: String,
    busy: bool,
    rx: Option<mpsc::Receiver<Msg>>,
    grid: Grid,
    watchers: Vec<notify::RecommendedWatcher>,
    watch_key: String,
    files_changed: Arc<AtomicBool>,
    ctx: egui::Context,
    /// the column open in the column builder window (index into `rows`)
    editing: Option<usize>,
    edit_sample: usize,
    upd: Upd,
    upd_rx: Option<mpsc::Receiver<UpdMsg>>,
    /// row mode seen last frame, to re-guess untouched columns when it changes
    last_row_mode: String,
    /// text in the filter box: only rows containing it are shown
    filter: String,
    filter_applied: String,
    menu: Option<GridMenu>,
    /// scroll the grid to show this row next frame
    scroll_to: Option<usize>,
    /// a button press to act on after drawing this frame
    pending: Option<Action>,
    /// the column name being typed in the column window: (column, text)
    rename_buf: Option<(usize, String)>,
}

fn alert(text: &str) {
    MessageDialog::new().set_title(APP_NAME).set_level(MessageLevel::Warning).set_description(text).show();
}

fn error(text: &str) {
    MessageDialog::new().set_title(APP_NAME).set_level(MessageLevel::Error).set_description(text).show();
}

fn ask(text: &str) -> bool {
    matches!(
        MessageDialog::new().set_title(APP_NAME).set_description(text).set_buttons(MessageButtons::YesNo).show(),
        MessageDialogResult::Yes
    )
}

/// Keys pressed this frame, each with the modifier keys held *at that moment*.
/// (A quick key press can start and end within one frame, so the frame's
/// overall modifier state may already show the keys released.)
fn pressed_keys(ctx: &egui::Context) -> Vec<(egui::Key, egui::Modifiers)> {
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
            .collect()
    })
}

fn open_url(url: &str) {
    let _ = open::that_detached(url);
}

fn paste_keys() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘V"
    } else {
        "Ctrl+V"
    }
}

/// Does a file-system event mean the grid could be out of date?
fn relevant_event(ev: &notify::Event, only: Option<&std::ffi::OsStr>) -> bool {
    use notify::event::{EventKind, MetadataKind, ModifyKind};
    match ev.kind {
        EventKind::Access(_) => return false,
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)) => return false,
        _ => {}
    }
    ev.paths.iter().any(|p| {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if let Some(only) = only {
            return p.file_name() == Some(only);
        }
        !p.components().any(|c| c.as_os_str() == BACKUP_DIR)
            && !name.starts_with("~$")
            && !name.starts_with(".~lock")
            && !name.eq_ignore_ascii_case(".ds_store")
    })
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.global_style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        cc.egui_ctx.set_global_style(style);

        let mut app = App {
            s: load_settings(&settings_file()).unwrap_or_default(),
            rows: vec![],
            sheets: vec![],
            status: "Ready.".into(),
            busy: false,
            rx: None,
            grid: Grid {
                data: None,
                message: None,
                busy: false,
                rx: None,
                signature: String::new(),
                refresh_at: None,
                updated_at: String::new(),
                sel: None,
                drag: None,
                copy_msg: String::new(),
                hits: vec![],
                order: vec![],
            },
            watchers: vec![],
            watch_key: String::new(),
            files_changed: Arc::new(AtomicBool::new(false)),
            ctx: cc.egui_ctx.clone(),
            editing: None,
            edit_sample: 0,
            upd: Upd::Idle,
            upd_rx: None,
            last_row_mode: String::new(),
            filter: String::new(),
            filter_applied: String::new(),
            menu: None,
            scroll_to: None,
            pending: None,
            rename_buf: None,
        };
        update::cleanup_after_update();
        app.apply_sheet_mode(true);
        app.last_row_mode = app.s.row_mode.clone();
        if app.s.check_updates {
            app.check_for_updates(false);
        }
        app
    }

    // ------------------------------------------------------------ mapping

    fn remember_mapping(&mut self) {
        for r in &self.rows {
            self.s.mapping.insert(r.header.clone(), (r.field.clone(), r.param.clone()));
        }
    }

    fn set_columns(&mut self, headers: Vec<(u32, String)>) {
        self.rows = headers
            .into_iter()
            .map(|(col, header)| {
                let (field, param) = match self.s.mapping.get(&header) {
                    Some((f, p)) if FIELDS.iter().any(|d| d.key == f) => (f.clone(), p.clone()),
                    _ => {
                        let f = auto_field_for(&header, self.s.row_mode != "files");
                        (f.to_string(), field(f).default.unwrap_or("").to_string())
                    }
                };
                MapRow { col, header, field, param }
            })
            .collect();
        self.refresh_key();
    }

    fn key_choices(&self) -> Vec<String> {
        self.rows.iter().filter(|r| !BAD_KEYS.contains(&r.field.as_str())).map(|r| r.header.clone()).collect()
    }

    fn refresh_key(&mut self) {
        let choices = self.key_choices();
        if choices.contains(&self.s.key_header) {
            return;
        }
        for k in KEY_PRIORITY {
            if let Some(r) = self.rows.iter().find(|r| r.field == *k) {
                self.s.key_header = r.header.clone();
                return;
            }
        }
        self.s.key_header = choices.first().cloned().unwrap_or_default();
    }

    fn mapping(&self) -> Vec<MapEntry> {
        self.rows
            .iter()
            .map(|r| MapEntry { col: r.col, header: r.header.clone(), field: r.field.clone(), param: r.param.clone() })
            .collect()
    }

    // ------------------------------------------------------------ spreadsheet section

    fn apply_sheet_mode(&mut self, quiet: bool) {
        if self.s.sheet_mode == "existing" {
            if Path::new(&self.s.sheet_path).is_file() {
                self.load_columns(false, quiet);
            } else {
                self.set_columns(vec![]);
            }
        } else {
            self.apply_new_columns();
        }
    }

    fn apply_new_columns(&mut self) {
        self.remember_mapping();
        let cols = split_list(&self.s.new_columns);
        self.set_columns(dedupe_headers(cols.into_iter().enumerate().map(|(i, c)| (i as u32 + 1, c)).collect()));
    }

    fn load_columns(&mut self, detect: bool, quiet: bool) {
        let path = PathBuf::from(self.s.sheet_path.trim());
        if !path.is_file() {
            if !quiet {
                alert("Choose your tracking spreadsheet first.");
            }
            self.set_columns(vec![]);
            return;
        }
        self.remember_mapping();
        let hr = if detect { None } else { Some(self.s.header_row.max(1)) };
        match peek(&path, Some(&self.s.sheet_name), hr) {
            Ok(p) => {
                let n = p.headers.len();
                self.status =
                    format!("Found {n} column{} on row {} of “{}”.", if n == 1 { "" } else { "s" }, p.header_row, p.sheet);
                self.sheets = p.sheets;
                self.s.sheet_name = p.sheet;
                self.s.header_row = p.header_row;
                self.set_columns(p.headers);
            }
            Err(e) => {
                if !quiet {
                    error(&e.0);
                }
                self.status = e.0;
            }
        }
    }

    fn use_spreadsheet(&mut self, f: &Path) {
        self.s.sheet_mode = "existing".into();
        self.s.sheet_path = f.display().to_string();
        self.s.sheet_name.clear();
        self.load_columns(true, false);
    }

    /// Load a saved setup. A setup without a folder or spreadsheet keeps the
    /// ones already chosen, so a shared setup can be dropped onto any project.
    fn load_setup(&mut self, f: &Path) {
        match read_setup(f) {
            Ok(mut s) => {
                if s.folder.trim().is_empty() {
                    s.folder = self.s.folder.clone();
                }
                if s.sheet_path.trim().is_empty() && s.sheet_mode == "existing" {
                    s.sheet_path = self.s.sheet_path.clone();
                }
                s.show_sidebar = true;
                self.s = s;
                self.last_row_mode = self.s.row_mode.clone();
                self.apply_sheet_mode(true);
                let name = f.file_name().unwrap_or_default().to_string_lossy().to_string();
                self.status = format!("Loaded setup “{name}”.");
                self.grid.copy_msg = if self.s.folder.trim().is_empty() || (self.s.sheet_mode == "existing" && self.s.sheet_path.trim().is_empty()) {
                    format!("Loaded “{name}”. Now choose the folder (📁) and spreadsheet (📊) in the top bar.")
                } else {
                    format!("Loaded “{name}”.")
                };
            }
            Err(e) => error(&e),
        }
    }

    /// A folder, spreadsheet or saved setup dropped onto the window.
    fn dropped(&mut self, paths: Vec<PathBuf>) {
        for p in paths {
            if p.is_file() && is_setup_file(&p) {
                self.load_setup(&p);
            } else if p.is_dir() {
                self.s.folder = p.display().to_string();
            } else if matches!(
                p.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref(),
                Some("xlsx" | "xlsm" | "csv" | "tsv")
            ) {
                self.use_spreadsheet(&p);
            } else if let Some(parent) = p.parent().filter(|d| d.is_dir()) {
                self.s.folder = parent.display().to_string(); // a file: scan the folder it's in
            }
        }
    }

    // ------------------------------------------------------------ jobs

    fn make_job(&self, sheet_path: Option<PathBuf>, create_new: bool) -> Job {
        Job {
            folder: PathBuf::from(self.s.folder.trim()),
            scan: Arc::new(ScanOptions {
                recursive: self.s.recursive,
                skip_hidden: self.s.skip_hidden,
                extensions: split_list(&self.s.extensions),
                exclude: split_list(&self.s.exclude),
            }),
            sheet_path,
            create_new,
            new_columns: dedupe_headers(
                split_list(&self.s.new_columns).into_iter().enumerate().map(|(i, c)| (i as u32 + 1, c)).collect(),
            )
            .into_iter()
            .map(|(_, h)| h)
            .collect(),
            sheet_name: self.s.sheet_name.clone(),
            header_row: self.s.header_row,
            mapping: self.mapping(),
            key_header: self.s.key_header.clone(),
            delimiter: self.s.delimiter.clone(),
            fresh: !create_new && self.s.run_mode == "fresh",
            update_existing: self.s.update_existing,
            backup: self.s.backup,
            only_new: self.s.only_new,
            row_mode: RowMode::from_settings(&self.s.row_mode, self.s.folder_level, &self.s.folder_pattern),
            size_base: self.s.size_base,
        }
    }

    /// Everything the grid depends on. When this changes, the grid refreshes.
    fn signature(&self) -> String {
        let s = &self.s;
        format!(
            "{:?}",
            (
                (s.folder.trim(), s.recursive, s.skip_hidden, &s.extensions, &s.exclude),
                (&s.row_mode, s.folder_level, &s.folder_pattern, s.size_base),
                (&s.sheet_mode, s.sheet_path.trim(), &s.sheet_name, s.header_row),
                (&s.key_header, &s.delimiter, s.only_new),
                self.rows.iter().map(|r| (r.col, &r.header, &r.field, &r.param)).collect::<Vec<_>>(),
            )
        )
    }

    fn start_grid_refresh(&mut self) {
        let g = &mut self.grid;
        g.refresh_at = None;
        let folder = PathBuf::from(self.s.folder.trim());
        let create_new = self.s.sheet_mode == "new";
        let sheet = PathBuf::from(self.s.sheet_path.trim());
        let problem = if self.s.folder.trim().is_empty() {
            Some("Choose a folder to scan.")
        } else if !folder.is_dir() {
            Some("That folder can't be found. Choose it again.")
        } else if !create_new && !sheet.is_file() {
            Some("Choose your tracking spreadsheet, or start from new columns instead.")
        } else if self.rows.is_empty() {
            Some("No columns found. Check the header row in Settings › Spreadsheet.")
        } else if !self.rows.iter().any(|r| !r.field.is_empty()) {
            Some("Choose what goes into at least one column (Settings › Columns, or double-click a heading).")
        } else {
            None
        };
        if let Some(p) = problem {
            g.data = None;
            g.message = Some(p.to_string());
            g.sel = None;
            return;
        }
        let job = self.make_job(if create_new { None } else { Some(sheet) }, create_new);
        let (tx, rx) = mpsc::channel();
        let g = &mut self.grid;
        g.rx = Some(rx);
        g.busy = true;
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let res = preview_job(&job, &|_m| {}).map_err(|e| e.0);
            let _ = tx.send(res);
            ctx.request_repaint();
        });
    }

    fn poll_grid(&mut self) {
        let Some(rx) = &self.grid.rx else { return };
        let Ok(res) = rx.try_recv() else { return };
        let g = &mut self.grid;
        g.rx = None;
        g.busy = false;
        g.updated_at = chrono::Local::now().format("%H:%M:%S").to_string();
        match res {
            Ok(p) => {
                // the list changed: the old selection would point at different files
                let same = g.data.as_ref().map(|d| d.rows == p.rows && d.columns == p.columns).unwrap_or(false);
                if !same {
                    g.sel = None;
                    g.copy_msg.clear();
                }
                g.message = None;
                g.data = Some(p);
                self.rebuild_order();
            }
            Err(e) => {
                g.data = None;
                g.sel = None;
                g.message = Some(e);
            }
        }
    }

    /// Watch the scanned folder (and the spreadsheet) so the grid follows changes.
    fn update_watchers(&mut self) {
        let key = format!(
            "{}|{}|{}|{}|{}",
            self.s.auto_refresh,
            self.s.folder.trim(),
            self.s.recursive,
            self.s.sheet_mode,
            self.s.sheet_path.trim()
        );
        if key == self.watch_key {
            return;
        }
        self.watch_key = key;
        self.watchers.clear();
        if !self.s.auto_refresh {
            return;
        }
        let make = |only: Option<std::ffi::OsString>, flag: Arc<AtomicBool>, ctx: egui::Context| {
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(ev) = res {
                    if relevant_event(&ev, only.as_deref()) {
                        flag.store(true, Ordering::SeqCst);
                        ctx.request_repaint();
                    }
                }
            })
        };
        let folder = PathBuf::from(self.s.folder.trim());
        if folder.is_dir() {
            if let Ok(mut w) = make(None, self.files_changed.clone(), self.ctx.clone()) {
                let mode = if self.s.recursive { notify::RecursiveMode::Recursive } else { notify::RecursiveMode::NonRecursive };
                if w.watch(&folder, mode).is_ok() {
                    self.watchers.push(w);
                }
            }
        }
        let sheet = PathBuf::from(self.s.sheet_path.trim());
        if self.s.sheet_mode == "existing" && sheet.is_file() {
            if let (Some(parent), Some(name)) = (sheet.parent(), sheet.file_name()) {
                if let Ok(mut w) = make(Some(name.to_os_string()), self.files_changed.clone(), self.ctx.clone()) {
                    if w.watch(parent, notify::RecursiveMode::NonRecursive).is_ok() {
                        self.watchers.push(w);
                    }
                }
            }
        }
    }

    /// Switching between file rows and folder rows: columns the user hasn't
    /// changed follow the new mode (e.g. "Folder" = the row's own folder).
    fn follow_row_mode(&mut self) {
        if self.s.row_mode == self.last_row_mode {
            return;
        }
        let (was, now) = (self.last_row_mode != "files", self.s.row_mode != "files");
        self.last_row_mode = self.s.row_mode.clone();
        if was == now {
            return;
        }
        for r in &mut self.rows {
            if r.field == auto_field_for(&r.header, was) {
                let f = auto_field_for(&r.header, now);
                r.field = f.to_string();
                r.param = field(f).default.unwrap_or("").to_string();
            }
        }
        self.remember_mapping();
        self.refresh_key();
    }

    /// Decide whether the grid needs refreshing this frame.
    fn schedule_grid(&mut self) {
        let now = Instant::now();
        let sig = self.signature();
        if sig != self.grid.signature {
            let first = self.grid.signature.is_empty();
            self.grid.signature = sig;
            self.grid.refresh_at = Some(if first { now } else { now + SETTINGS_DEBOUNCE });
        }
        if self.files_changed.swap(false, Ordering::SeqCst) && self.s.auto_refresh {
            self.grid.refresh_at = Some(now + FILES_DEBOUNCE);
        }
        if let Some(t) = self.grid.refresh_at {
            if now >= t && !self.grid.busy && !self.busy {
                self.start_grid_refresh();
            } else {
                self.ctx.request_repaint_after(t.saturating_duration_since(now).max(Duration::from_millis(50)));
            }
        }
        if self.grid.busy {
            self.ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    // ------------------------------------------------------------ sorting

    fn sort_index(&self) -> Option<usize> {
        let d = self.grid.data.as_ref()?;
        d.columns.iter().position(|h| !self.s.sort_col.is_empty() && *h == self.s.sort_col)
    }

    /// Renumber running-number columns when the rows shown aren't in folder
    /// order (sorted by something else, or filtered), so pastes stay in sequence.
    fn renumber(&self) -> bool {
        if !self.filter_applied.trim().is_empty() {
            return true;
        }
        match (self.sort_index(), &self.grid.data) {
            (Some(c), Some(d)) => d.index_specs.get(c).map(|s| s.is_none()).unwrap_or(true),
            _ => false,
        }
    }

    fn rebuild_order(&mut self) {
        let sc = self.sort_index();
        self.filter_applied = self.filter.clone();
        self.grid.order = match &self.grid.data {
            Some(d) => sorting::view_order(d, sc, self.s.sort_desc, &self.filter),
            None => vec![],
        };
        if let Some(sel) = self.grid.sel {
            let (_, r1, _, _) = sel.bounds();
            if r1 >= self.grid.order.len() {
                self.grid.sel = None;
            }
        }
    }

    /// Number of rows shown (after filtering).
    fn shown_rows(&self) -> usize {
        self.grid.order.len()
    }

    fn set_sort(&mut self, c: usize, descending: Option<bool>) {
        let Some(name) = self.grid.data.as_ref().and_then(|d| d.columns.get(c).cloned()) else { return };
        match descending {
            Some(desc) => {
                self.s.sort_col = name;
                self.s.sort_desc = desc;
            }
            None => {
                self.s.sort_col.clear();
                self.s.sort_desc = false;
            }
        }
        self.grid.sel = None;
        self.grid.copy_msg.clear();
        self.rebuild_order();
    }

    /// Clicking a column's sort arrow: A→Z, then Z→A, then back to folder order.
    fn toggle_sort(&mut self, c: usize) {
        let Some(name) = self.grid.data.as_ref().and_then(|d| d.columns.get(c).cloned()) else { return };
        if self.s.sort_col != name {
            self.s.sort_col = name;
            self.s.sort_desc = false;
        } else if !self.s.sort_desc {
            self.s.sort_desc = true;
        } else {
            self.s.sort_col.clear();
            self.s.sort_desc = false;
        }
        self.grid.sel = None;
        self.grid.copy_msg.clear();
        self.rebuild_order();
    }

    // ------------------------------------------------------------ updates

    fn check_for_updates(&mut self, manual: bool) {
        if matches!(self.upd, Upd::Checking { .. } | Upd::Downloading { .. } | Upd::Installing) {
            return;
        }
        self.upd = Upd::Checking { manual };
        let (tx, rx) = mpsc::channel();
        self.upd_rx = Some(rx);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(UpdMsg::Checked(update::check_latest(), manual));
            ctx.request_repaint();
        });
    }

    fn start_update(&mut self, rel: update::Release) {
        self.remember_mapping();
        save_settings(&self.s, &settings_file());
        self.upd = Upd::Downloading { rel: rel.clone(), got: 0, total: rel.asset_size };
        let (tx, rx) = mpsc::channel();
        self.upd_rx = Some(rx);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let report = |got: u64, total: u64| {
                let _ = tx.send(UpdMsg::Progress(got, total));
                ctx.request_repaint();
            };
            let res = update::download(&rel, &report).and_then(|file| update::install(&file));
            let _ = tx.send(UpdMsg::Installed(res));
            ctx.request_repaint();
        });
    }

    fn poll_updates(&mut self) {
        let Some(rx) = &self.upd_rx else { return };
        let mut done = false;
        while let Ok(m) = rx.try_recv() {
            match m {
                UpdMsg::Checked(res, manual) => {
                    done = true;
                    self.upd = match res {
                        Ok(rel) if update::is_newer(&rel.version, &update::current_version()) && (manual || rel.version != self.s.skip_version) => {
                            Upd::Available(rel)
                        }
                        Ok(_) if manual => Upd::UpToDate,
                        Err(e) if manual => Upd::Failed { msg: e, rel: None },
                        _ => Upd::Idle,
                    };
                }
                UpdMsg::Progress(got, total) => {
                    if let Upd::Downloading { got: g, total: t, .. } = &mut self.upd {
                        *g = got;
                        *t = total;
                        if got >= total && total > 0 {
                            self.upd = Upd::Installing;
                        }
                    }
                }
                UpdMsg::Installed(res) => {
                    done = true;
                    match res {
                        Ok(target) => {
                            self.remember_mapping();
                            save_settings(&self.s, &settings_file());
                            update::relaunch(&target);
                        }
                        Err(msg) => {
                            let rel = match std::mem::replace(&mut self.upd, Upd::Idle) {
                                Upd::Downloading { rel, .. } | Upd::Available(rel) => Some(rel),
                                _ => None,
                            };
                            self.upd = Upd::Failed { msg, rel };
                        }
                    }
                }
            }
        }
        if done {
            self.upd_rx = None;
        }
    }

    /// The bar across the top that offers and shows an update.
    fn update_bar(&mut self, ui: &mut egui::Ui) {
        let releases_page = format!("https://github.com/{}/releases/latest", update::REPO);
        let mut next: Option<Upd> = None;
        let mut start: Option<update::Release> = None;
        let mut check = false;
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| match &self.upd {
            Upd::Idle => {}
            Upd::Checking { .. } => {
                ui.spinner();
                ui.label("Checking for updates…");
            }
            Upd::UpToDate => {
                ui.label(format!("✔ You have the latest version ({}).", update::current_version()));
                if ui.button("OK").clicked() {
                    next = Some(Upd::Idle);
                }
            }
            Upd::Available(rel) => {
                ui.label(RichText::new(format!("File Tracker {} is available", rel.version)).strong());
                ui.colored_label(HINT, format!("(you have {})", update::current_version()));
                if update::can_self_update() && rel.asset_url.is_some() {
                    if ui.add_enabled(!self.busy, egui::Button::new(RichText::new("Update now").strong())).clicked() {
                        start = Some(rel.clone());
                    }
                } else if ui.button("Open download page").clicked() {
                    open_url(&rel.page);
                }
                let notes: String = rel.notes.chars().take(700).collect();
                let whats_new = ui.button("What's new");
                let whats_new = if notes.trim().is_empty() { whats_new } else { whats_new.on_hover_text(notes) };
                if whats_new.clicked() {
                    open_url(&rel.page);
                }
                if ui.button("Skip this version").clicked() {
                    self.s.skip_version = rel.version.clone();
                    next = Some(Upd::Idle);
                }
                if ui.button("Later").clicked() {
                    next = Some(Upd::Idle);
                }
            }
            Upd::Downloading { rel, got, total } => {
                ui.label(format!("Downloading File Tracker {}…", rel.version));
                let frac = if *total > 0 { *got as f32 / *total as f32 } else { 0.0 };
                ui.add(
                    egui::ProgressBar::new(frac)
                        .desired_width(260.0)
                        .text(format!("{:.1} of {:.1} MB", *got as f64 / 1e6, *total as f64 / 1e6)),
                );
            }
            Upd::Installing => {
                ui.spinner();
                ui.label("Installing — File Tracker will restart in a moment…");
            }
            Upd::Failed { msg, rel } => {
                ui.colored_label(Color32::from_rgb(220, 120, 80), format!("The update didn't finish: {msg}"));
                if rel.is_some() && ui.button("Try again").clicked() {
                    start = rel.clone();
                }
                if rel.is_none() && ui.button("Try again").clicked() {
                    check = true;
                }
                if ui.button("Open download page").clicked() {
                    open_url(&rel.as_ref().map(|r| r.page.clone()).unwrap_or_else(|| releases_page.clone()));
                }
                if ui.button("Close").clicked() {
                    next = Some(Upd::Idle);
                }
            }
        });
        ui.add_space(4.0);
        if let Some(n) = next {
            self.upd = n;
        }
        if let Some(rel) = start {
            self.start_update(rel);
        }
        if check {
            self.upd = Upd::Idle;
            self.check_for_updates(true);
        }
    }

    fn show_update_bar(&self) -> bool {
        match &self.upd {
            Upd::Idle => false,
            Upd::Checking { manual } => *manual,
            _ => true,
        }
    }

    // ------------------------------------------------------------ copy

    fn copy_range(&mut self, r0: usize, r1: usize, c0: usize, c1: usize) {
        let Some(d) = &self.grid.data else { return };
        let mut lines = Vec::new();
        if self.s.copy_headers {
            lines.push(d.columns[c0..=c1].join("\t"));
        }
        let renumber = self.renumber();
        for r in r0..=r1 {
            let cells: Vec<String> = (c0..=c1).map(|c| sorting::cell(d, &self.grid.order, renumber, r, c).1).collect();
            lines.push(cells.join("\t"));
        }
        self.ctx.copy_text(lines.join("\n"));
        let (nr, nc) = (r1 - r0 + 1, c1 - c0 + 1);
        let target = if self.s.copy_headers {
            "Paste into the top-left cell of an empty sheet".to_string()
        } else {
            format!("Click cell {}{} in your sheet and paste with {}", d.letters[c0], d.start_row as usize + r0, paste_keys())
        };
        self.grid.copy_msg = format!(
            "Copied {nr} row{} × {nc} column{}. {target}.",
            if nr == 1 { "" } else { "s" },
            if nc == 1 { "" } else { "s" }
        );
    }

    fn copy_all(&mut self) {
        let Some(d) = &self.grid.data else { return };
        let nr = self.shown_rows();
        if nr == 0 {
            self.grid.copy_msg = "There are no rows to copy.".into();
            return;
        }
        let nc = d.columns.len();
        self.grid.sel = Some(Sel { a: (0, 0), b: (nr - 1, nc - 1) });
        self.copy_range(0, nr - 1, 0, nc - 1);
    }

    fn copy_selection(&mut self) {
        match self.grid.sel {
            Some(sel) if self.grid.data.is_some() => {
                let (r0, r1, c0, c1) = sel.bounds();
                self.copy_range(r0, r1, c0, c1);
            }
            _ => self.copy_all(),
        }
    }

    /// Copy with the header row this once, whatever the checkbox says.
    fn copy_with_headers(&mut self) {
        let keep = self.s.copy_headers;
        self.s.copy_headers = true;
        self.copy_selection();
        self.s.copy_headers = keep;
    }

    /// Paths of the files/folders in the selected rows (in shown order).
    fn selected_paths(&self) -> Vec<PathBuf> {
        let (Some(d), Some(sel)) = (&self.grid.data, self.grid.sel) else { return vec![] };
        let (r0, r1, _, _) = sel.bounds();
        (r0..=r1.min(self.grid.order.len().saturating_sub(1)))
            .filter_map(|r| self.grid.order.get(r).and_then(|&i| d.paths.get(i)).cloned())
            .collect()
    }

    fn copy_paths(&mut self, names_only: bool) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let text: Vec<String> = paths
            .iter()
            .map(|p| {
                if names_only {
                    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
                } else {
                    p.display().to_string()
                }
            })
            .collect();
        self.ctx.copy_text(text.join("\n"));
        let what = if names_only { "name" } else { "path" };
        self.grid.copy_msg = format!("Copied {} {what}{}.", text.len(), if text.len() == 1 { "" } else { "s" });
    }

    fn reveal_selected(&mut self) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        reveal(&paths);
        self.grid.copy_msg = if paths.len() > 10 {
            format!("Showing the first 10 of {} items in {}.", paths.len(), file_manager_name())
        } else {
            format!("Showing in {}.", file_manager_name())
        };
    }

    fn open_selected(&mut self) {
        let paths = self.selected_paths();
        if paths.len() > 10 && !ask(&format!("Open {} items? Each opens in its own app window.", paths.len())) {
            return;
        }
        for p in &paths {
            open_path(p);
        }
    }

    /// Add a name to "Skip names like" (section 1), so matching items drop out.
    fn skip_name(&mut self, name: &str) {
        let mut list = split_list(&self.s.exclude);
        if !list.iter().any(|x| x.eq_ignore_ascii_case(name)) {
            list.push(name.to_string());
        }
        self.s.exclude = list.join(", ");
        self.grid.sel = None;
        self.grid.copy_msg = format!("Skipping anything named “{name}”. Remove it under “Skip names like” to bring it back.");
    }

    // ------------------------------------------------------------ editing columns

    /// Columns can be added, renamed, moved and removed when they're the app's
    /// own (no spreadsheet). With a spreadsheet, they come from its header row.
    fn columns_editable(&self) -> bool {
        self.s.sheet_mode == "new"
    }

    /// The column-list row (index into `rows`) shown as grid column `c`.
    fn row_for_grid_col(&self, c: usize) -> Option<usize> {
        let letter = self.grid.data.as_ref()?.letters.get(c)?.clone();
        self.rows.iter().position(|r| col_letter(r.col) == letter)
    }

    fn new_cols(&self) -> Vec<String> {
        split_list(&self.s.new_columns)
    }

    fn set_new_cols(&mut self, cols: Vec<String>) {
        self.remember_mapping();
        self.s.new_columns = cols.join(", ");
        self.apply_new_columns();
        self.grid.sel = None;
    }

    fn unique_col_name(&self, base: &str) -> String {
        let cols = self.new_cols();
        let taken = |n: &str| cols.iter().any(|c| c.eq_ignore_ascii_case(n));
        if !taken(base) {
            return base.to_string();
        }
        (2..).map(|k| format!("{base} {k}")).find(|n| !taken(n)).unwrap()
    }

    /// Add a column at position `at` and open it in the column window.
    fn insert_column(&mut self, at: usize) {
        let mut cols = self.new_cols();
        let name = self.unique_col_name("New column");
        let at = at.min(cols.len());
        cols.insert(at, name);
        self.set_new_cols(cols);
        self.editing = Some(at);
    }

    fn delete_column(&mut self, i: usize) {
        let mut cols = self.new_cols();
        if i < cols.len() && cols.len() > 1 {
            let gone = cols.remove(i);
            if self.s.sort_col == gone {
                self.s.sort_col.clear();
            }
            self.set_new_cols(cols);
            self.editing = None;
        }
    }

    fn move_column(&mut self, i: usize, delta: i64) {
        let mut cols = self.new_cols();
        let j = i as i64 + delta;
        if i < cols.len() && (0..cols.len() as i64).contains(&j) {
            cols.swap(i, j as usize);
            self.set_new_cols(cols);
            if self.editing == Some(i) {
                self.editing = Some(j as usize);
            }
        }
    }

    /// Rename a column, keeping what it's filled with. Returns false if the
    /// name is empty, has a comma, or is already used.
    fn rename_column(&mut self, i: usize, name: &str) -> bool {
        let name = name.trim();
        let mut cols = self.new_cols();
        if name.is_empty() || name.contains([',', ';']) || i >= cols.len() {
            return false;
        }
        if cols.iter().enumerate().any(|(k, c)| k != i && c.eq_ignore_ascii_case(name)) {
            return false;
        }
        let old = std::mem::replace(&mut cols[i], name.to_string());
        if old == name {
            return true;
        }
        self.remember_mapping();
        if let Some(m) = self.s.mapping.remove(&old) {
            self.s.mapping.insert(name.to_string(), m);
        }
        if self.s.sort_col == old {
            self.s.sort_col = name.to_string();
        }
        if self.s.key_header == old {
            self.s.key_header = name.to_string();
        }
        self.s.new_columns = cols.join(", ");
        self.apply_new_columns();
        true
    }

    // ------------------------------------------------------------ actions

    fn do_action(&mut self, a: Action) {
        match a {
            Action::BrowseFolder => {
                let mut d = FileDialog::new().set_title("Choose the folder to scan");
                if Path::new(&self.s.folder).is_dir() {
                    d = d.set_directory(&self.s.folder);
                }
                if let Some(f) = d.pick_folder() {
                    self.s.folder = f.display().to_string();
                }
            }
            Action::BrowseSheet => {
                let mut d = FileDialog::new()
                    .set_title("Choose your tracking spreadsheet")
                    .add_filter("Spreadsheets", &["xlsx", "xlsm", "csv", "tsv"]);
                if let Some(parent) = Path::new(&self.s.sheet_path).parent().filter(|p| p.is_dir()) {
                    d = d.set_directory(parent);
                } else if Path::new(&self.s.folder).is_dir() {
                    d = d.set_directory(&self.s.folder);
                }
                if let Some(f) = d.pick_file() {
                    self.use_spreadsheet(&f);
                }
            }
            Action::ReloadColumns(detect) => self.load_columns(detect, false),
            Action::UseNewColumns => {
                self.s.sheet_mode = "new".into();
                self.apply_sheet_mode(false);
            }
            Action::ApplySheetMode => self.apply_sheet_mode(false),
            Action::SaveSetup => {
                self.remember_mapping();
                if let Some(f) = FileDialog::new()
                    .set_title("Save this setup")
                    .set_file_name("My file tracker setup.json")
                    .add_filter("File Tracker setup", &["json"])
                    .save_file()
                {
                    save_settings(&self.s, &f);
                    self.status = format!("Setup saved to {}.", f.file_name().unwrap_or_default().to_string_lossy());
                }
            }
            Action::LoadSetup => {
                // no file-type filter: downloaded setups sometimes get a different ending
                if let Some(f) = FileDialog::new().set_title("Load a saved setup").pick_file() {
                    self.load_setup(&f);
                }
            }
            Action::OpenSheet => {
                let p = Path::new(&self.s.sheet_path);
                if self.s.sheet_mode == "existing" && p.is_file() {
                    open_path(p);
                } else {
                    alert("There's no spreadsheet to open yet.");
                }
            }
            Action::Update => self.run_update(),
            Action::Refresh => self.grid.refresh_at = Some(Instant::now()),
            Action::Copy => self.copy_selection(),
        }
    }

    fn run_update(&mut self) {
        if self.busy {
            return;
        }
        let folder = PathBuf::from(self.s.folder.trim());
        if !folder.is_dir() {
            alert("Choose a folder to scan first.");
            return;
        }
        if !self.rows.iter().any(|r| !r.field.is_empty()) {
            alert("Choose what goes into at least one column first (Settings › Columns).");
            return;
        }
        let create_new = self.s.sheet_mode == "new";
        let sheet_path = if create_new {
            let start = folder.parent().unwrap_or(&folder).to_path_buf();
            match FileDialog::new()
                .set_title("Save the new tracking spreadsheet as")
                .set_directory(start)
                .set_file_name("File Tracker.xlsx")
                .add_filter("Excel workbook", &["xlsx"])
                .add_filter("CSV file", &["csv"])
                .save_file()
            {
                Some(mut p) => {
                    if p.extension().is_none() {
                        p.set_extension("xlsx");
                    }
                    p
                }
                None => return,
            }
        } else {
            let p = PathBuf::from(self.s.sheet_path.trim());
            if !p.is_file() {
                alert("Choose your tracking spreadsheet first.");
                return;
            }
            if self.s.run_mode == "fresh" {
                let msg = format!(
                    "Start fresh?\n\nThis clears every row under the headers — including anything typed by hand — and rebuilds the list from the folder.\n\n{}\n\nContinue?",
                    if self.s.backup { "A backup copy is made first." } else { "Backups are switched OFF." }
                );
                if !ask(&msg) {
                    return;
                }
            }
            p
        };

        self.remember_mapping();
        save_settings(&self.s, &settings_file());
        let job = self.make_job(Some(sheet_path), create_new);
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.busy = true;
        self.status = "Updating spreadsheet…".into();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let report = |m: String| {
                let _ = tx.send(Msg::Progress(m));
                ctx.request_repaint();
            };
            let msg = match run_job(&job, &report) {
                Ok(r) => Msg::Done(r, job.create_new),
                Err(e) => Msg::Error(e.0),
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn poll_update(&mut self) {
        let Some(rx) = &self.rx else { return };
        let mut finished = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Progress(t) => self.status = t,
                other => finished = Some(other),
            }
        }
        let Some(m) = finished else { return };
        self.rx = None;
        self.busy = false;
        match m {
            Msg::Error(e) => {
                self.status = "Stopped — nothing was saved.".into();
                error(&e);
            }
            Msg::Done(r, created) => {
                if created {
                    self.s.sheet_mode = "existing".into();
                    self.s.sheet_path = r.path.display().to_string();
                    self.s.sheet_name.clear();
                    self.remember_mapping();
                    self.apply_sheet_mode(true);
                }
                save_settings(&self.s, &settings_file());
                self.grid.refresh_at = Some(Instant::now());
                let name = r.path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let mut lines = vec![format!("{} files scanned.", r.files), format!("• {} new file(s) added", r.added)];
                if r.matched > 0 {
                    lines.push(format!(
                        "• {} already listed{}",
                        r.matched,
                        if r.refreshed { " (details refreshed)" } else { "" }
                    ));
                }
                if r.missing > 0 {
                    lines.push(format!(
                        "• {} listed file(s) weren't found in the folder{}",
                        r.missing,
                        if r.status_col { " (marked in the sheet)" } else { "" }
                    ));
                }
                if r.backup.is_some() {
                    lines.push(format!("\nBackup saved in “{BACKUP_DIR}”."));
                }
                self.status = format!("Done — {} added, {} missing. Saved {name}.", r.added, r.missing);
                if ask(&format!("{name} is updated.\n\n{}\n\nOpen it now?", lines.join("\n"))) {
                    open_path(&r.path);
                }
            }
            Msg::Progress(_) => {}
        }
    }
}
