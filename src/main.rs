// File Tracker — scan a folder and fill a tracking spreadsheet, or copy the
// list straight into any spreadsheet. Native GUI for macOS and Windows.
#![cfg_attr(windows, windows_subsystem = "windows")] // no console window on Windows

mod core;

use crate::core::*;
use eframe::egui::{self, Align, Color32, Layout, RichText};
use egui_extras::{Column, TableBuilder};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};

const APP_NAME: &str = "File Tracker";
const HINT: Color32 = Color32::from_rgb(128, 128, 128);

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([920.0, 900.0])
            .with_min_inner_size([760.0, 600.0]),
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
    Preview(Preview),
    Error(String),
}

struct ResultsView {
    data: Preview,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    include_headers: bool,
    msg: String,
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
    Run,
    ShowList,
}

struct App {
    s: Settings,
    rows: Vec<MapRow>,
    sheets: Vec<String>,
    status: String,
    busy: bool,
    rx: Option<mpsc::Receiver<Msg>>,
    results: Option<ResultsView>,
}

fn alert(text: &str) {
    MessageDialog::new().set_title(APP_NAME).set_level(MessageLevel::Warning).set_description(text).show();
}

fn error(text: &str) {
    MessageDialog::new().set_title(APP_NAME).set_level(MessageLevel::Error).set_description(text).show();
}

fn ask(text: &str) -> bool {
    matches!(
        MessageDialog::new()
            .set_title(APP_NAME)
            .set_description(text)
            .set_buttons(MessageButtons::YesNo)
            .show(),
        MessageDialogResult::Yes
    )
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
            results: None,
        };
        app.apply_sheet_mode(true);
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
                self.status = format!("Found {n} column{} on row {} of “{}”.", if n == 1 { "" } else { "s" }, p.header_row, p.sheet);
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

    // ------------------------------------------------------------ actions

    fn do_action(&mut self, a: Action, ctx: &egui::Context) {
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
                if let Some(f) = FileDialog::new().set_title("Load a saved setup").add_filter("File Tracker setup", &["json"]).pick_file() {
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
            Action::Run => self.run(ctx, false),
            Action::ShowList => self.run(ctx, true),
        }
    }

    fn run(&mut self, ctx: &egui::Context, preview: bool) {
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
        let sheet_path: Option<PathBuf> = if create_new {
            if preview {
                None
            } else {
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
                        Some(p)
                    }
                    None => return,
                }
            }
        } else {
            let p = PathBuf::from(self.s.sheet_path.trim());
            if !p.is_file() {
                alert(if preview {
                    "Choose your spreadsheet in section 2, or switch to “Create a new one” and type the columns you want."
                } else {
                    "Choose your tracking spreadsheet (section 2)."
                });
                return;
            }
            if !preview && self.s.run_mode == "fresh" {
                let msg = format!(
                    "Start fresh?\n\nThis clears every row under the headers — including anything typed by hand — and rebuilds the list from the folder.\n\n{}\n\nContinue?",
                    if self.s.backup { "A backup copy is made first." } else { "Backups are switched OFF." }
                );
                if !ask(&msg) {
                    return;
                }
            }
            Some(p)
        };

        self.remember_mapping();
        save_settings(&self.s, &settings_file());
        let job = Job {
            folder,
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
        };

        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.busy = true;
        self.status = "Working…".into();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let report = |m: String| {
                let _ = tx.send(Msg::Progress(m));
                ctx.request_repaint();
            };
            let msg = if preview {
                match preview_job(&job, &report) {
                    Ok(p) => Msg::Preview(p),
                    Err(e) => Msg::Error(e.0),
                }
            } else {
                match run_job(&job, &report) {
                    Ok(r) => Msg::Done(r, job.create_new),
                    Err(e) => Msg::Error(e.0),
                }
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn poll(&mut self) {
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
            Msg::Preview(p) => {
                let n = p.rows.len();
                self.status = format!("Listed {n} of {} file(s) found — nothing was saved.", p.files);
                if n == 0 {
                    alert(if p.skipped > 0 {
                        "No new files to list — everything in the folder is already in the spreadsheet."
                    } else {
                        "No files were found (check the folder and filters in section 1)."
                    });
                } else {
                    self.results = Some(ResultsView {
                        include_headers: !p.from_sheet,
                        data: p,
                        selected: BTreeSet::new(),
                        anchor: None,
                        msg: "Tip: click rows to select them (Ctrl/⌘ or Shift for several).".into(),
                    });
                }
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
                let name = r.path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let mut lines = vec![format!("{} files scanned.", r.files), format!("• {} new file(s) added", r.added)];
                if r.matched > 0 {
                    lines.push(format!("• {} already listed{}", r.matched, if r.refreshed { " (details refreshed)" } else { "" }));
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

    // ------------------------------------------------------------ UI

    fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
        egui::Frame::group(ui.style()).inner_margin(10.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().size(15.0));
            ui.add_space(4.0);
            add(ui);
        });
        ui.add_space(8.0);
    }

    fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(egui::vec2(118.0, 22.0), Layout::left_to_right(Align::Center), |ui| {
                ui.set_min_width(118.0);
                ui.label(label);
            });
            add(ui);
        });
    }

    fn main_ui(&mut self, ui: &mut egui::Ui) -> Action {
        let mut action = Action::None;

        Self::section(ui, "1   Folder to scan", |ui| {
            Self::row(ui, "Folder:", |ui| {
                let w = ui.available_width() - 90.0;
                ui.add(egui::TextEdit::singleline(&mut self.s.folder).desired_width(w));
                if ui.button("Choose…").clicked() {
                    action = Action::BrowseFolder;
                }
            });
            Self::row(ui, "", |ui| {
                ui.checkbox(&mut self.s.recursive, "Include subfolders");
                ui.add_space(12.0);
                ui.checkbox(&mut self.s.skip_hidden, "Skip hidden & system files");
            });
            Self::row(ui, "Only these types:", |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.s.extensions).desired_width(320.0));
                ui.colored_label(HINT, "e.g. pdf, docx (blank = all)");
            });
            Self::row(ui, "Skip names like:", |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.s.exclude).desired_width(320.0));
                ui.colored_label(HINT, "e.g. *.tmp, Archive, Old*");
            });
        });

        Self::section(ui, "2   Tracking spreadsheet", |ui| {
            ui.horizontal(|ui| {
                if ui.radio_value(&mut self.s.sheet_mode, "existing".into(), "Use my existing spreadsheet (.xlsx, .xlsm, .csv)").changed() {
                    action = Action::ApplySheetMode;
                }
                ui.add_space(12.0);
                if ui.radio_value(&mut self.s.sheet_mode, "new".into(), "Create a new one").changed() {
                    action = Action::ApplySheetMode;
                }
            });
            let existing = self.s.sheet_mode == "existing";
            Self::row(ui, "File:", |ui| {
                let w = ui.available_width() - 90.0;
                ui.add_enabled(existing, egui::TextEdit::singleline(&mut self.s.sheet_path).desired_width(w));
                if ui.add_enabled(existing, egui::Button::new("Choose…")).clicked() {
                    action = Action::BrowseSheet;
                }
            });
            Self::row(ui, "Sheet:", |ui| {
                ui.add_enabled_ui(existing, |ui| {
                    let before = self.s.sheet_name.clone();
                    egui::ComboBox::from_id_salt("sheet").width(200.0).selected_text(self.s.sheet_name.as_str()).show_ui(ui, |ui| {
                        for n in &self.sheets {
                            ui.selectable_value(&mut self.s.sheet_name, n.clone(), n);
                        }
                    });
                    if self.s.sheet_name != before {
                        action = Action::ReloadColumns(true);
                    }
                    ui.label("   Headers are on row:");
                    ui.add(egui::DragValue::new(&mut self.s.header_row).range(1..=500));
                    if ui.button("Reload columns").clicked() {
                        action = Action::ReloadColumns(false);
                    }
                });
            });
            Self::row(ui, "Columns:", |ui| {
                let w = ui.available_width() - 150.0;
                ui.add_enabled(!existing, egui::TextEdit::singleline(&mut self.s.new_columns).desired_width(w));
                if ui.add_enabled(!existing, egui::Button::new("Use these columns")).clicked() {
                    action = Action::ApplyNewColumns;
                }
            });
        });

        Self::section(ui, "3   What goes in each column", |ui| {
            ui.colored_label(
                HINT,
                "Columns set to “— leave as is —” are never changed, so your own status, notes and review columns stay safe.",
            );
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt("map").max_height(250.0).auto_shrink([false, true]).show(ui, |ui| {
                if self.rows.is_empty() {
                    ui.colored_label(HINT, "No columns yet — choose a spreadsheet above (or enter columns for a new one).");
                    return;
                }
                let mut changed = false;
                egui::Grid::new("map_grid").num_columns(4).striped(true).spacing([10.0, 4.0]).show(ui, |ui| {
                    ui.strong("Column");
                    ui.strong("Fill with");
                    ui.strong("Setting");
                    ui.label("");
                    ui.end_row();
                    for (i, r) in self.rows.iter_mut().enumerate() {
                        let mut head = r.header.clone();
                        if head.chars().count() > 26 {
                            head = head.chars().take(25).collect::<String>() + "…";
                        }
                        ui.label(format!("{}   {}", col_letter(r.col), head)).on_hover_text(&r.header);
                        let before = r.field.clone();
                        egui::ComboBox::from_id_salt(("field", i)).width(270.0).selected_text(field(&r.field).label).show_ui(
                            ui,
                            |ui| {
                                for f in FIELDS {
                                    ui.selectable_value(&mut r.field, f.key.to_string(), f.label);
                                }
                            },
                        );
                        if r.field != before {
                            r.param = field(&r.field).default.unwrap_or("").to_string();
                            changed = true;
                        }
                        let def = field(&r.field);
                        if def.default.is_some() {
                            ui.add(egui::TextEdit::singleline(&mut r.param).desired_width(110.0));
                        } else {
                            ui.add_enabled(false, egui::TextEdit::singleline(&mut String::new()).desired_width(110.0));
                        }
                        ui.colored_label(HINT, def.hint);
                        ui.end_row();
                    }
                });
                if changed {
                    self.remember_mapping();
                    self.refresh_key();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Match files to existing rows using column:");
                let choices = self.key_choices();
                egui::ComboBox::from_id_salt("key").width(200.0).selected_text(self.s.key_header.as_str()).show_ui(ui, |ui| {
                    for c in choices {
                        ui.selectable_value(&mut self.s.key_header, c.clone(), c);
                    }
                });
                ui.add_space(16.0);
                ui.label("File-name separator:");
                ui.add(egui::TextEdit::singleline(&mut self.s.delimiter).desired_width(28.0));
                ui.colored_label(HINT, "(for “Part of the file name”)");
            });
        });

        Self::section(ui, "4   Options", |ui| {
            ui.radio_value(&mut self.s.run_mode, "update".into(), "Add new files and keep existing rows (recommended)");
            ui.radio_value(&mut self.s.run_mode, "fresh".into(), "Start fresh — clear every row under the headers and rebuild the list");
            ui.add_space(2.0);
            ui.checkbox(&mut self.s.update_existing, "Refresh details (size, dates, paths…) of files already listed");
            ui.checkbox(&mut self.s.backup, format!("Back up the spreadsheet before saving (into a “{BACKUP_DIR}” folder)"));
            ui.checkbox(&mut self.s.only_new, "In the copy list, show only files that aren't in the spreadsheet yet");
        });
        action
    }
}

fn results_ui(ui: &mut egui::Ui, rv: &mut ResultsView, close: &mut bool) {
    let ctx = ui.ctx().clone();
    let n = rv.data.rows.len();
    let paste = if cfg!(target_os = "macos") { "⌘V" } else { "Ctrl+V" };

    let copy = |rv: &mut ResultsView, only_selected: bool| {
        let idx: Vec<usize> = if only_selected { rv.selected.iter().copied().collect() } else { (0..n).collect() };
        if idx.is_empty() {
            rv.msg = "Select one or more rows first (or use Copy all).".into();
            return;
        }
        let mut lines = Vec::new();
        if rv.include_headers {
            lines.push(rv.data.columns.join("\t"));
        }
        for i in &idx {
            lines.push(rv.data.rows[*i].iter().map(|(_, c)| c.as_str()).collect::<Vec<_>>().join("\t"));
        }
        ctx.copy_text(lines.join("\n"));
        rv.msg = format!("Copied {} row{}. Now paste into your spreadsheet.", idx.len(), if idx.len() == 1 { "" } else { "s" });
    };

    // Keyboard: Ctrl/⌘+C copies (selected rows if any), Ctrl/⌘+A selects all
    let (copy_key, select_all, mods) = ui.input(|i| {
        (
            i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
            i.modifiers.command && i.key_pressed(egui::Key::A),
            // modifier keys held during the click itself (most reliable across platforms)
            i.events
                .iter()
                .rev()
                .find_map(|e| match e {
                    egui::Event::PointerButton { modifiers, .. } => Some(*modifiers | i.modifiers),
                    _ => None,
                })
                .unwrap_or(i.modifiers),
        )
    });
    if select_all {
        rv.selected = (0..n).collect();
    }
    if copy_key {
        let sel = !rv.selected.is_empty();
        copy(rv, sel);
    }

    egui::Panel::top("results_top").show(ui, |ui| {
        ui.add_space(6.0);
        let mut text = if rv.data.from_sheet {
            format!(
                "In your spreadsheet, click the first empty cell under “{}” (column {}) and paste with {paste}.",
                rv.data.first_header, rv.data.first_col
            )
        } else {
            format!("Click a cell in any spreadsheet and paste with {paste}.")
        };
        if rv.data.skipped > 0 {
            text += &format!("  ({} file(s) already in the spreadsheet were left out.)", rv.data.skipped);
        }
        ui.label(text);
        ui.add_space(4.0);
    });

    egui::Panel::bottom("results_bottom").show(ui, |ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut rv.include_headers, "Include the header row");
            ui.colored_label(HINT, rv.msg.as_str());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    *close = true;
                }
                if ui.button("Copy selected rows").clicked() {
                    copy(rv, true);
                }
                if ui.button(RichText::new("Copy all").strong()).clicked() {
                    copy(rv, false);
                }
            });
        });
        ui.add_space(6.0);
    });

    egui::CentralPanel::default().show(ui, |ui| {
        let ncols = rv.data.columns.len();
        let mut clicked: Option<usize> = None;
        egui::ScrollArea::horizontal()
            .id_salt("results_h")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
            let mut tb = TableBuilder::new(ui)
                .id_salt("results_table")
                .striped(true)
                .resizable(true)
                .sense(egui::Sense::click())
                .cell_layout(Layout::left_to_right(Align::Center));
            for c in 0..ncols {
                let longest = rv.data.rows.iter().take(300).map(|r| r[c].0.chars().count()).max().unwrap_or(0);
                let chars = longest.max(rv.data.columns[c].chars().count());
                tb = tb.column(Column::initial((chars as f32 * 7.5 + 18.0).clamp(60.0, 420.0)).at_least(40.0).clip(true));
            }
            tb.header(24.0, |mut h| {
                for name in &rv.data.columns {
                    h.col(|ui| {
                        ui.strong(name);
                    });
                }
            })
            .body(|body| {
                body.rows(20.0, n, |mut row| {
                    let i = row.index();
                    row.set_selected(rv.selected.contains(&i));
                    for (shown, _) in &rv.data.rows[i] {
                        row.col(|ui| {
                            ui.add(egui::Label::new(shown.as_str()).selectable(false).truncate());
                        });
                    }
                    if row.response().clicked() {
                        clicked = Some(i);
                    }
                });
            });
        });
        if let Some(i) = clicked {
            if mods.shift {
                let a = rv.anchor.unwrap_or(i);
                rv.selected = (a.min(i)..=a.max(i)).collect();
            } else if mods.command {
                if !rv.selected.remove(&i) {
                    rv.selected.insert(i);
                }
                rv.anchor = Some(i);
            } else {
                rv.selected = BTreeSet::from([i]);
                rv.anchor = Some(i);
            }
        }
    });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll();
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if ui.input(|i| i.viewport().close_requested()) {
            self.remember_mapping();
            save_settings(&self.s, &settings_file());
        }

        let mut action = Action::None;
        egui::Panel::bottom("actions").show(ui, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save setup…").clicked() {
                    action = Action::SaveSetup;
                }
                if ui.button("Load setup…").clicked() {
                    action = Action::LoadSetup;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_enabled_ui(!self.busy, |ui| {
                        if ui.button(RichText::new("Update spreadsheet  ▶").strong()).clicked() {
                            action = Action::Run;
                        }
                        if ui.button(RichText::new("Show list to copy").strong()).clicked() {
                            action = Action::ShowList;
                        }
                    });
                    if ui.button("Open spreadsheet").clicked() {
                        action = Action::OpenSheet;
                    }
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if self.busy {
                    ui.spinner();
                }
                ui.label(self.status.as_str());
            });
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("main").show(ui, |ui| {
                let a = self.main_ui(ui);
                if a != Action::None {
                    action = a;
                }
            });
        });
        if action != Action::None {
            self.do_action(action, &ctx);
        }

        if let Some(rv) = &mut self.results {
            let mut close = false;
            let n = rv.data.rows.len();
            let title = format!("{APP_NAME} — {n} file{} ready to copy", if n == 1 { "" } else { "s" });
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("results"),
                egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_inner_size([1050.0, 600.0])
                    .with_min_inner_size([600.0, 320.0]),
                |ui, _class| {
                    results_ui(ui, rv, &mut close);
                    if ui.input(|i| i.viewport().close_requested()) {
                        close = true;
                    }
                },
            );
            if close {
                self.results = None;
            }
        }
    }
}
