// File Tracker — scan a folder and lay the files out in your spreadsheet's
// column layout in a live grid you can copy from, or update the sheet directly.
// Native GUI for macOS and Windows.
#![cfg_attr(windows, windows_subsystem = "windows")] // no console window on Windows

mod builder;
mod core;
mod extract;
mod sorting;
mod update;

use crate::core::*;
use eframe::egui::{self, Align, Color32, Layout, Pos2, Rect, RichText, Stroke, StrokeKind};
use egui_extras::{Column, TableBuilder};
use notify::Watcher;
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const APP_NAME: &str = "File Tracker";
const HINT: Color32 = Color32::from_rgb(128, 128, 128);
const ROW_H: f32 = 22.0;
const GUTTER_W: f32 = 54.0;
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
    None,
    BrowseFolder,
    BrowseSheet,
    ReloadColumns(bool),
    ApplyNewColumns,
    ApplySheetMode,
    SaveSetup,
    LoadSetup,
    OpenSheet,
    Update,
    Refresh,
    CopyAll,
    CopySelection,
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
        };
        update::cleanup_after_update();
        app.apply_sheet_mode(true);
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
                        let f = auto_field(&header);
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
        }
    }

    /// Everything the grid depends on. When this changes, the grid refreshes.
    fn signature(&self) -> String {
        let s = &self.s;
        format!(
            "{:?}",
            (
                (s.folder.trim(), s.recursive, s.skip_hidden, &s.extensions, &s.exclude),
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
            Some("Choose a folder to scan (section 1 on the left).")
        } else if !folder.is_dir() {
            Some("That folder can't be found. Check the folder in section 1.")
        } else if !create_new && !sheet.is_file() {
            Some("Choose your tracking spreadsheet in section 2,\nor pick “Create a new one” and type the columns you want.")
        } else if self.rows.is_empty() {
            Some("No columns found. Check the header row in section 2.")
        } else if !self.rows.iter().any(|r| !r.field.is_empty()) {
            Some("Choose what goes into at least one column (section 3).")
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

    /// Renumber running-number columns when sorted by something else.
    fn renumber(&self) -> bool {
        match (self.sort_index(), &self.grid.data) {
            (Some(c), Some(d)) => d.index_specs.get(c).map(|s| s.is_none()).unwrap_or(true),
            _ => false,
        }
    }

    fn rebuild_order(&mut self) {
        let sc = self.sort_index();
        self.grid.order = match &self.grid.data {
            Some(d) => sorting::view_order(d, sc, self.s.sort_desc),
            None => vec![],
        };
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
        if d.rows.is_empty() {
            self.grid.copy_msg = "There are no rows to copy.".into();
            return;
        }
        let (nr, nc) = (d.rows.len(), d.columns.len());
        self.grid.sel = Some(Sel { a: (0, 0), b: (nr - 1, nc - 1) });
        self.copy_range(0, nr - 1, 0, nc - 1);
    }

    fn copy_selection(&mut self) {
        match self.grid.sel {
            Some(sel) if self.grid.data.is_some() => {
                let (r0, r1, c0, c1) = sel.bounds();
                self.copy_range(r0, r1, c0, c1);
            }
            _ => self.grid.copy_msg = "Select some cells first (drag across them), or use Copy all rows.".into(),
        }
    }

    // ------------------------------------------------------------ actions

    fn do_action(&mut self, a: Action) {
        match a {
            Action::None => {}
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
                    .add_filter("Spreadsheets", &["xlsx", "xlsm", "csv", "tsv"])
                    .add_filter("All files", &["*"]);
                if let Some(parent) = Path::new(&self.s.sheet_path).parent().filter(|p| p.is_dir()) {
                    d = d.set_directory(parent);
                } else if Path::new(&self.s.folder).is_dir() {
                    d = d.set_directory(&self.s.folder);
                }
                if let Some(f) = d.pick_file() {
                    self.s.sheet_path = f.display().to_string();
                    self.s.sheet_name.clear();
                    self.load_columns(true, false);
                }
            }
            Action::ReloadColumns(detect) => self.load_columns(detect, false),
            Action::ApplyNewColumns => self.apply_new_columns(),
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
                if let Some(f) =
                    FileDialog::new().set_title("Load a saved setup").add_filter("File Tracker setup", &["json"]).pick_file()
                {
                    match load_settings(&f) {
                        Some(s) => {
                            self.s = s;
                            self.apply_sheet_mode(false);
                            self.status = format!("Loaded setup {}.", f.file_name().unwrap_or_default().to_string_lossy());
                        }
                        None => error("That file isn't a File Tracker setup."),
                    }
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
            Action::CopyAll => self.copy_all(),
            Action::CopySelection => self.copy_selection(),
        }
    }

    fn run_update(&mut self) {
        if self.busy {
            return;
        }
        let folder = PathBuf::from(self.s.folder.trim());
        if !folder.is_dir() {
            alert("Choose the folder to scan (section 1).");
            return;
        }
        if !self.rows.iter().any(|r| !r.field.is_empty()) {
            alert("Choose what goes into at least one column (section 3).");
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
                alert("Choose your tracking spreadsheet (section 2).");
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

    // ------------------------------------------------------------ settings sidebar

    fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
        egui::Frame::group(ui.style()).inner_margin(10.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().size(15.0));
            ui.add_space(4.0);
            add(ui);
        });
        ui.add_space(8.0);
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) -> Action {
        let mut action = Action::None;

        Self::section(ui, "1   Folder to scan", |ui| {
            ui.horizontal(|ui| {
                let w = ui.available_width() - 84.0;
                ui.add(egui::TextEdit::singleline(&mut self.s.folder).desired_width(w).hint_text("Choose a folder…"));
                if ui.button("Choose…").clicked() {
                    action = Action::BrowseFolder;
                }
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.s.recursive, "Include subfolders");
                ui.checkbox(&mut self.s.skip_hidden, "Skip hidden & system files");
            });
            egui::Grid::new("s1").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Only these types:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.s.extensions)
                        .desired_width(f32::INFINITY)
                        .hint_text("e.g. pdf, docx (blank = all)"),
                );
                ui.end_row();
                ui.label("Skip names like:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.s.exclude)
                        .desired_width(f32::INFINITY)
                        .hint_text("e.g. *.tmp, Archive, Old*"),
                );
                ui.end_row();
            });
        });

        Self::section(ui, "2   Tracking spreadsheet", |ui| {
            if ui.radio_value(&mut self.s.sheet_mode, "existing".into(), "Use my existing spreadsheet (.xlsx, .xlsm, .csv)").changed() {
                action = Action::ApplySheetMode;
            }
            if ui.radio_value(&mut self.s.sheet_mode, "new".into(), "Start from new columns (no spreadsheet yet)").changed() {
                action = Action::ApplySheetMode;
            }
            ui.add_space(2.0);
            if self.s.sheet_mode == "existing" {
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 84.0;
                    ui.add(egui::TextEdit::singleline(&mut self.s.sheet_path).desired_width(w).hint_text("Choose a spreadsheet…"));
                    if ui.button("Choose…").clicked() {
                        action = Action::BrowseSheet;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Sheet:");
                    let before = self.s.sheet_name.clone();
                    egui::ComboBox::from_id_salt("sheet").width(130.0).selected_text(self.s.sheet_name.as_str()).show_ui(ui, |ui| {
                        for n in &self.sheets {
                            ui.selectable_value(&mut self.s.sheet_name, n.clone(), n);
                        }
                    });
                    if self.s.sheet_name != before {
                        action = Action::ReloadColumns(true);
                    }
                    ui.label("Header row:");
                    ui.add(egui::DragValue::new(&mut self.s.header_row).range(1..=500));
                    if ui.button("Reload").on_hover_text("Read the column headers again").clicked() {
                        action = Action::ReloadColumns(false);
                    }
                });
            } else {
                ui.label("Column names, separated by commas:");
                ui.add(egui::TextEdit::multiline(&mut self.s.new_columns).desired_rows(2).desired_width(f32::INFINITY));
                if ui.button("Use these columns").clicked() {
                    action = Action::ApplyNewColumns;
                }
            }
        });

        Self::section(ui, "3   What goes in each column", |ui| {
            ui.colored_label(
                HINT,
                "Pick “Custom…” to take any piece of a folder or file name. You can also double-click a column heading in the grid. \
                 “— leave as is —” columns stay blank and are never changed in your sheet.",
            );
            ui.add_space(4.0);
            if self.rows.is_empty() {
                ui.colored_label(HINT, "No columns yet — choose a spreadsheet above.");
            } else {
                let mut changed = false;
                let mut open_editor = None;
                egui::Grid::new("map_grid").num_columns(3).striped(true).spacing([8.0, 4.0]).show(ui, |ui| {
                    for (i, r) in self.rows.iter_mut().enumerate() {
                        let mut head = r.header.clone();
                        if head.chars().count() > 18 {
                            head = head.chars().take(17).collect::<String>() + "…";
                        }
                        ui.label(format!("{}  {}", col_letter(r.col), head)).on_hover_text(&r.header);
                        if builder::field_combo(ui, ("field", i), 200.0, &mut r.field, &mut r.param) {
                            changed = true;
                            if r.field == "extract" {
                                open_editor = Some(i);
                            }
                        }
                        let def = field(&r.field);
                        if r.field == "extract" {
                            if ui.button("Edit…").on_hover_text("Choose which piece of the folder or file name goes here").clicked() {
                                open_editor = Some(i);
                            }
                        } else if def.default.is_some() {
                            ui.add(egui::TextEdit::singleline(&mut r.param).desired_width(70.0)).on_hover_text(def.hint);
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                });
                if open_editor.is_some() {
                    self.editing = open_editor;
                }
                if changed {
                    self.remember_mapping();
                    self.refresh_key();
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Match files by:").on_hover_text("The column used to recognise files that are already in your sheet");
                let choices = self.key_choices();
                egui::ComboBox::from_id_salt("key").width(140.0).selected_text(self.s.key_header.as_str()).show_ui(ui, |ui| {
                    for c in choices {
                        ui.selectable_value(&mut self.s.key_header, c.clone(), c);
                    }
                });
                ui.label("Name separator:").on_hover_text("Splits file names for “Part of the file name”");
                ui.add(egui::TextEdit::singleline(&mut self.s.delimiter).desired_width(24.0));
            });
        });

        Self::section(ui, "4   Options", |ui| {
            ui.checkbox(&mut self.s.only_new, "Grid lists only files not yet in the sheet");
            ui.checkbox(&mut self.s.auto_refresh, "Update the grid when files change");
            ui.add_space(4.0);
            ui.label(RichText::new("When using “Update spreadsheet”:").color(HINT));
            ui.radio_value(&mut self.s.run_mode, "update".into(), "Add new files, keep existing rows");
            ui.radio_value(&mut self.s.run_mode, "fresh".into(), "Start fresh (clear rows and rebuild)");
            ui.checkbox(&mut self.s.update_existing, "Refresh details of files already listed");
            ui.checkbox(&mut self.s.backup, "Back up the spreadsheet first");
        });

        ui.horizontal(|ui| {
            if ui.button("Save setup…").clicked() {
                action = Action::SaveSetup;
            }
            if ui.button("Load setup…").clicked() {
                action = Action::LoadSetup;
            }
            if ui.button("Open spreadsheet").clicked() {
                action = Action::OpenSheet;
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if self.busy {
                ui.spinner();
            }
            ui.colored_label(HINT, self.status.as_str());
        });
        ui.add_space(10.0);
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(HINT, format!("File Tracker {}", update::current_version()));
            if ui.small_button("Check for updates").clicked() {
                self.check_for_updates(true);
            }
            ui.checkbox(&mut self.s.check_updates, "Check when the app starts");
        });
        action
    }

    // ------------------------------------------------------------ grid

    fn grid_toolbar(&mut self, ui: &mut egui::Ui) -> Action {
        let mut action = Action::None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Grid").strong().size(17.0));
            ui.add_space(6.0);
            let summary = match &self.grid.data {
                Some(d) => {
                    let n = d.rows.len();
                    let what = if d.from_sheet && self.s.only_new { "new file" } else { "file" };
                    let mut t = format!("{n} {what}{}", if n == 1 { "" } else { "s" });
                    if d.skipped > 0 {
                        t += &format!("  ·  {} already in the sheet", d.skipped);
                    }
                    t
                }
                None => String::new(),
            };
            let found = self.grid.data.as_ref().map(|d| d.files).unwrap_or(0);
            ui.label(summary).on_hover_text(format!("{found} files found in the folder"));
            if let Some(c) = self.sort_index() {
                let name = self.grid.data.as_ref().map(|d| d.columns[c].clone()).unwrap_or_default();
                ui.colored_label(HINT, format!("·  sorted by {name} ({})", if self.s.sort_desc { "Z→A" } else { "A→Z" }));
                if ui.small_button("Unsort").on_hover_text("Back to the order the files were found in").clicked() {
                    self.s.sort_col.clear();
                    self.s.sort_desc = false;
                    self.grid.sel = None;
                    self.rebuild_order();
                }
            }
            if self.grid.busy {
                ui.spinner();
                ui.colored_label(HINT, "updating…");
            } else if !self.grid.updated_at.is_empty() {
                ui.colored_label(HINT, format!("·  updated {}", self.grid.updated_at));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_enabled_ui(!self.busy, |ui| {
                    if ui
                        .button(RichText::new("Update spreadsheet  ▶").strong())
                        .on_hover_text("Write the new files straight into your spreadsheet file")
                        .clicked()
                    {
                        action = Action::Update;
                    }
                });
                if ui.button("Refresh").on_hover_text("Scan the folder again now").clicked() {
                    action = Action::Refresh;
                }
            });
        });
        ui.horizontal(|ui| {
            let has = self.grid.data.as_ref().map(|d| !d.rows.is_empty()).unwrap_or(false);
            ui.add_enabled_ui(has, |ui| {
                if ui.button(RichText::new("Copy selection").strong()).clicked() {
                    action = Action::CopySelection;
                }
                if ui.button("Copy all rows").clicked() {
                    action = Action::CopyAll;
                }
            });
            ui.checkbox(&mut self.s.copy_headers, "Include header row");
            ui.add_space(8.0);
            let hint = if !self.grid.copy_msg.is_empty() {
                self.grid.copy_msg.clone()
            } else if let Some(d) = &self.grid.data {
                let (r, c) = self.grid.sel.map(|s| (s.bounds().0, s.bounds().2)).unwrap_or((0, 0));
                if d.rows.is_empty() {
                    String::new()
                } else {
                    format!(
                        "Drag across cells to select, then copy (⌘C / Ctrl+C). Paste at cell {}{} in your sheet.",
                        d.letters.get(c).cloned().unwrap_or_default(),
                        d.start_row as usize + r
                    )
                }
            } else {
                String::new()
            };
            ui.add(egui::Label::new(RichText::new(hint).color(HINT)).truncate()).on_hover_text(
                "Drag across cells to select them. Shift-click extends the selection. \
                 Click a column letter or a row number to select the whole column or row. \
                 Ctrl/⌘+A selects everything, Esc clears.",
            );
        });
        action
    }

    fn grid_view(&mut self, ui: &mut egui::Ui) {
        let Some(d) = &self.grid.data else {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                if self.grid.busy {
                    ui.spinner();
                    ui.colored_label(HINT, "Scanning…");
                } else {
                    ui.label(
                        RichText::new(self.grid.message.as_deref().unwrap_or("Choose a folder and a spreadsheet on the left."))
                            .size(16.0)
                            .color(HINT),
                    );
                }
            });
            return;
        };
        if d.rows.is_empty() {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                let msg = if d.skipped > 0 {
                    format!(
                        "Everything in the folder is already in your sheet ({} files).\nNew files will appear here automatically.",
                        d.skipped
                    )
                } else {
                    "No files found. Check the folder and the filters in section 1.".to_string()
                };
                ui.label(RichText::new(msg).size(16.0).color(HINT));
            });
            return;
        }

        let nrows = d.rows.len();
        let ncols = d.columns.len();
        let sel = self.grid.sel;
        let sort_c = self.sort_index();
        let sort_desc = self.s.sort_desc;
        let renumber = self.renumber();
        let order = &self.grid.order;
        let visuals = ui.visuals().clone();
        let sel_fill = visuals.selection.bg_fill.gamma_multiply(0.45);
        let head_fill = visuals.faint_bg_color.gamma_multiply(1.6);
        let head_sel_fill = visuals.selection.bg_fill.gamma_multiply(0.7);
        let blank_fill = visuals.extreme_bg_color.gamma_multiply(0.6);
        let line = Stroke::new(0.5, visuals.widgets.noninteractive.bg_stroke.color.gamma_multiply(0.8));
        let mut hits: Vec<(Rect, Hit)> = Vec::new();
        // column widths follow the content; when a column's content changes size
        // noticeably (e.g. it was empty and is now filled) the table re-lays out
        let widths: Vec<f32> = (0..ncols)
            .map(|c| {
                let longest = d.rows.iter().take(300).map(|r| r[c].0.chars().count()).max().unwrap_or(0);
                let content = longest.max(4) as f32 * 7.2 + 20.0;
                let heading = d.columns[c].chars().count() as f32 * 7.8 + 44.0; // room for the sort arrows
                content.max(heading).clamp(70.0, 380.0)
            })
            .collect();
        let buckets: Vec<i32> = widths.iter().map(|w| (w / 40.0).round() as i32).collect();
        let table_key = format!("{}|{:?}", d.columns.join("\u{1f}"), buckets);

        let paint_cell = |ui: &mut egui::Ui, fill: Option<Color32>| {
            let r = ui.max_rect();
            if let Some(f) = fill {
                ui.painter().rect_filled(r, 0.0, f);
            }
            ui.painter().rect_stroke(r, 0.0, line, StrokeKind::Inside);
            r.intersect(ui.clip_rect())
        };

        egui::ScrollArea::horizontal()
            .id_salt("grid_h")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                let mut tb = TableBuilder::new(ui)
                    .id_salt(("grid", &table_key))
                    .striped(false)
                    .resizable(true)
                    .vscroll(true)
                    .auto_shrink([false, false])
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .column(Column::exact(GUTTER_W));
                for w in &widths {
                    tb = tb.column(Column::initial(*w).at_least(36.0).clip(true));
                }
                tb.header(42.0, |mut h| {
                    h.col(|ui| {
                        let r = paint_cell(ui, Some(head_fill));
                        hits.push((r, Hit::Corner));
                    });
                    for c in 0..ncols {
                        h.col(|ui| {
                            let selected = sel.map(|s| s.has_col(c)).unwrap_or(false);
                            let r = paint_cell(ui, Some(if selected { head_sel_fill } else { head_fill }));
                            // sort arrow on the right of the heading
                            let full = ui.max_rect();
                            let arrow = Rect::from_min_size(
                                egui::pos2(full.right() - 22.0, full.center().y - 10.0),
                                egui::vec2(18.0, 20.0),
                            );
                            let state = if sort_c == Some(c) { Some(sort_desc) } else { None };
                            let hovered = ui.rect_contains_pointer(arrow.intersect(ui.clip_rect()));
                            paint_sort_arrow(ui.painter(), arrow, state, hovered, &visuals);
                            hits.push((arrow.intersect(ui.clip_rect()), Hit::Sort(c)));
                            hits.push((r, Hit::Col(c)));
                            ui.add_space(5.0);
                            ui.vertical(|ui| {
                                ui.set_max_width((full.width() - 30.0).max(10.0));
                                ui.add_space(3.0);
                                ui.label(RichText::new(&d.letters[c]).small().color(HINT));
                                let name = RichText::new(&d.columns[c]).strong();
                                let name = if d.filled[c] { name } else { name.italics().color(HINT) };
                                let resp = ui.add(egui::Label::new(name).truncate().selectable(false));
                                resp.on_hover_text(if d.filled[c] {
                                    "Click to select this column. Double-click to choose what goes in it. Click the arrows to sort."
                                } else {
                                    "Left as is — pastes as blank so the other columns line up. Double-click to fill it."
                                });
                            });
                        });
                    }
                })
                .body(|body| {
                    body.rows(ROW_H, nrows, |mut row| {
                        let i = row.index();
                        let row_sel = sel.map(|s| s.has_row(i)).unwrap_or(false);
                        row.col(|ui| {
                            let r = paint_cell(ui, Some(if row_sel { head_sel_fill } else { head_fill }));
                            hits.push((r, Hit::Row(i)));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add_space(6.0);
                                ui.label(RichText::new((d.start_row as usize + i).to_string()).small().color(HINT));
                            });
                        });
                        for c in 0..ncols {
                            let (shown, _) = sorting::cell(d, order, renumber, i, c);
                            row.col(|ui| {
                                let fill = if sel.map(|s| s.contains(i, c)).unwrap_or(false) {
                                    Some(sel_fill)
                                } else if !d.filled[c] {
                                    Some(blank_fill)
                                } else {
                                    None
                                };
                                let r = paint_cell(ui, fill);
                                hits.push((r, Hit::Cell(i, c)));
                                ui.add_space(5.0);
                                ui.add(egui::Label::new(shown.as_str()).selectable(false).truncate());
                            });
                        }
                    });
                });
            });
        self.grid.hits = hits;
        self.grid_mouse(ui.ctx(), nrows, ncols);
    }

    /// Double-clicking a grid heading opens that column in the builder.
    fn open_column_from_grid(&mut self, c: usize) {
        let Some(letter) = self.grid.data.as_ref().and_then(|d| d.letters.get(c).cloned()) else { return };
        if let Some(i) = self.rows.iter().position(|r| col_letter(r.col) == letter) {
            self.editing = Some(i);
            self.grid.drag = None;
        }
    }

    fn column_window(&mut self, ctx: &egui::Context) {
        let Some(i) = self.editing else { return };
        if i >= self.rows.len() {
            self.editing = None;
            return;
        }
        let samples: &[FileInfo] = self.grid.data.as_ref().map(|d| d.samples.as_slice()).unwrap_or(&[]);
        let row = &mut self.rows[i];
        let before = (row.field.clone(), row.param.clone());
        let open = builder::column_window(
            ctx,
            &col_letter(row.col),
            &row.header,
            &mut row.field,
            &mut row.param,
            samples,
            &mut self.edit_sample,
        );
        let changed = (row.field.as_str(), row.param.as_str()) != (before.0.as_str(), before.1.as_str());
        if changed {
            self.remember_mapping();
            self.refresh_key();
            ctx.request_repaint(); // show the new layout straight away
        }
        if !open {
            self.editing = None;
        }
    }

    fn hit_at(&self, pos: Pos2) -> Option<Hit> {
        self.grid.hits.iter().find(|(r, _)| r.contains(pos)).map(|(_, h)| *h)
    }

    /// Excel-style mouse selection: drag across cells, Shift-click to extend,
    /// click a column letter or row number to take the whole column or row.
    fn grid_mouse(&mut self, ctx: &egui::Context, nrows: usize, ncols: usize) {
        let (press_pos, down, pos, shift) = ctx.input(|i| {
            // read the press from the event itself: a quick click or trackpad tap
            // can press and release within one frame
            let press = i.events.iter().find_map(|e| match e {
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers } => {
                    Some((*pos, modifiers.shift))
                }
                _ => None,
            });
            (press, i.pointer.primary_down(), i.pointer.latest_pos(), i.modifiers.shift)
        });
        let pressed = press_pos.is_some();
        let double = ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
        let shift = press_pos.map(|(_, s)| s || shift).unwrap_or(shift);
        let press_pos = press_pos.map(|(p, _)| p);
        let on_grid_layer =
            |p: Pos2| ctx.layer_id_at(p).map(|l| l.order == egui::Order::Background).unwrap_or(false);

        if pressed {
            if let Some(p) = press_pos.filter(|p| on_grid_layer(*p)) {
                if let Some(Hit::Sort(c)) = self.hit_at(p) {
                    self.toggle_sort(c);
                    return;
                }
                if let Some(hit) = self.hit_at(p) {
                    ctx.memory_mut(|m| m.stop_text_input());
                    self.grid.copy_msg.clear();
                    let last_c = ncols - 1;
                    let cur = self.grid.sel;
                    let (sel, drag) = match hit {
                        Hit::Cell(r, c) => match (shift, cur) {
                            (true, Some(s)) => (Sel { a: s.a, b: (r, c) }, DragKind::Cell),
                            _ => (Sel { a: (r, c), b: (r, c) }, DragKind::Cell),
                        },
                        Hit::Row(r) => match (shift, cur) {
                            (true, Some(s)) => (Sel { a: (s.a.0, 0), b: (r, last_c) }, DragKind::Row),
                            _ => (Sel { a: (r, 0), b: (r, last_c) }, DragKind::Row),
                        },
                        Hit::Col(c) => match (shift, cur) {
                            (true, Some(s)) => (Sel { a: (0, s.a.1), b: (nrows - 1, c) }, DragKind::Col),
                            _ => (Sel { a: (0, c), b: (nrows - 1, c) }, DragKind::Col),
                        },
                        Hit::Corner | Hit::Sort(_) => (Sel { a: (0, 0), b: (nrows - 1, last_c) }, DragKind::Cell),
                    };
                    self.grid.sel = Some(sel);
                    self.grid.drag = if hit == Hit::Corner { None } else { Some(drag) };
                    if let (true, Hit::Col(c)) = (double, hit) {
                        self.open_column_from_grid(c);
                    }
                }
            }
        }

        if let (true, Some(kind), Some(p), Some(sel)) = (down, self.grid.drag, pos, self.grid.sel.as_mut()) {
            if let Some(hit) = self.grid.hits.iter().find(|(r, _)| r.contains(p)).map(|(_, h)| *h) {
                let (r, c) = match hit {
                    Hit::Cell(r, c) => (Some(r), Some(c)),
                    Hit::Row(r) => (Some(r), None),
                    Hit::Col(c) => (None, Some(c)),
                    Hit::Corner | Hit::Sort(_) => (None, None),
                };
                match kind {
                    DragKind::Cell => sel.b = (r.unwrap_or(sel.b.0), c.unwrap_or(sel.b.1)),
                    DragKind::Row => {
                        if let Some(r) = r {
                            sel.b = (r, ncols - 1)
                        }
                    }
                    DragKind::Col => {
                        if let Some(c) = c {
                            sel.b = (nrows - 1, c)
                        }
                    }
                }
            }
        }
        if !down {
            self.grid.drag = None;
        }
    }

    /// Keyboard shortcuts for the grid (only when no text box has the cursor).
    fn grid_keys(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused()).is_some() {
            return;
        }
        let (copy, all, esc) = ctx.input(|i| {
            (
                i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
                i.modifiers.command && i.key_pressed(egui::Key::A),
                i.key_pressed(egui::Key::Escape),
            )
        });
        let dims = self.grid.data.as_ref().map(|d| (d.rows.len(), d.columns.len())).filter(|(r, _)| *r > 0);
        if all {
            if let Some((nr, nc)) = dims {
                self.grid.sel = Some(Sel { a: (0, 0), b: (nr - 1, nc - 1) });
            }
        }
        if esc {
            self.grid.sel = None;
            self.grid.copy_msg.clear();
        }
        if copy {
            self.copy_selection();
        }
    }
}

/// Two small triangles; the active direction is drawn bright.
fn paint_sort_arrow(p: &egui::Painter, r: Rect, state: Option<bool>, hovered: bool, v: &egui::Visuals) {
    let dim = if hovered { v.text_color() } else { HINT.gamma_multiply(0.7) };
    let on = v.strong_text_color();
    let c = r.center();
    let (w, h, gap) = (4.5, 4.0, 1.5);
    let up = vec![egui::pos2(c.x - w, c.y - gap), egui::pos2(c.x + w, c.y - gap), egui::pos2(c.x, c.y - gap - h)];
    let down = vec![egui::pos2(c.x - w, c.y + gap), egui::pos2(c.x + w, c.y + gap), egui::pos2(c.x, c.y + gap + h)];
    let (up_col, down_col) = match state {
        Some(false) => (on, Color32::TRANSPARENT),
        Some(true) => (Color32::TRANSPARENT, on),
        None => (dim, dim),
    };
    p.add(egui::Shape::convex_polygon(up, up_col, Stroke::NONE));
    p.add(egui::Shape::convex_polygon(down, down_col, Stroke::NONE));
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_update();
        self.poll_grid();
        self.poll_updates();
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if ui.input(|i| i.viewport().close_requested()) {
            self.remember_mapping();
            save_settings(&self.s, &settings_file());
        }

        if self.show_update_bar() {
            egui::Panel::top("update_bar")
                .frame(egui::Frame::new().fill(ui.visuals().selection.bg_fill.gamma_multiply(0.35)).inner_margin(egui::Margin::symmetric(12, 2)))
                .show(ui, |ui| self.update_bar(ui));
        }
        if matches!(self.upd, Upd::Downloading { .. } | Upd::Installing | Upd::Checking { .. }) {
            ctx.request_repaint_after(Duration::from_millis(150));
        }

        let mut action = Action::None;
        egui::Panel::left("settings")
            .resizable(true)
            .default_size(470.0)
            .size_range(380.0..=700.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("side").show(ui, |ui| {
                    ui.add_space(8.0);
                    let a = self.sidebar(ui);
                    if a != Action::None {
                        action = a;
                    }
                    ui.add_space(8.0);
                });
            });

        egui::Panel::top("grid_toolbar").show(ui, |ui| {
            ui.add_space(6.0);
            let a = self.grid_toolbar(ui);
            if a != Action::None {
                action = a;
            }
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            self.grid_view(ui);
        });

        self.column_window(&ctx);
        self.grid_keys(&ctx);
        if action != Action::None {
            self.do_action(action);
        }
        self.update_watchers();
        self.schedule_grid();
    }
}
