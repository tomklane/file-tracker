//! The window layout: top bar, settings sidebar, status bar, and the
//! column window.

use crate::core::*;
use crate::{builder, update, Action, App, Upd, HINT};
use eframe::egui::{self, Align, Layout, RichText};
use std::path::Path;
use std::time::Duration;

fn cmd_key(k: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{k}")
    } else {
        format!("Ctrl+{k}")
    }
}

fn file_name_of(p: &str) -> String {
    Path::new(p.trim()).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn filter_id() -> egui::Id {
    egui::Id::unique("filter_box")
}

impl App {
    // ------------------------------------------------------------ top bar

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            // where the files come from
            let folder = file_name_of(&self.s.folder);
            let folder_label = if folder.is_empty() { "Choose a folder…".to_string() } else { folder };
            if ui
                .button(RichText::new(format!("📁  {folder_label}")).strong())
                .on_hover_text(if self.s.folder.trim().is_empty() {
                    "Choose the folder to scan (or drop one onto the window)".to_string()
                } else {
                    format!("{}\nClick to choose another folder", self.s.folder.trim())
                })
                .clicked()
            {
                self.pending = Some(Action::BrowseFolder);
            }
            ui.colored_label(HINT, "→");
            // where the columns come from
            let sheet_label = if self.s.sheet_mode == "new" {
                "New columns (no spreadsheet)".to_string()
            } else {
                let f = file_name_of(&self.s.sheet_path);
                if f.is_empty() {
                    "Choose a spreadsheet…".into()
                } else if self.sheets.len() > 1 {
                    format!("{f} › {}", self.s.sheet_name)
                } else {
                    f
                }
            };
            ui.menu_button(RichText::new(format!("📊  {sheet_label}  ▾")).strong(), |ui| {
                ui.set_min_width(240.0);
                if ui.button("Choose a spreadsheet…").clicked() {
                    self.pending = Some(Action::BrowseSheet);
                    ui.close();
                }
                if self.s.sheet_mode != "new" && ui.button("Start from new columns instead").clicked() {
                    self.pending = Some(Action::UseNewColumns);
                    ui.close();
                }
                if self.s.sheet_mode == "existing" && Path::new(self.s.sheet_path.trim()).is_file() {
                    if self.sheets.len() > 1 {
                        ui.separator();
                        ui.label(RichText::new("Sheet").color(HINT));
                        for n in self.sheets.clone() {
                            if ui.radio(self.s.sheet_name == n, &n).clicked() {
                                self.s.sheet_name = n;
                                self.pending = Some(Action::ReloadColumns(true));
                                ui.close();
                            }
                        }
                    }
                    ui.separator();
                    if ui.button("Open the spreadsheet").clicked() {
                        self.pending = Some(Action::OpenSheet);
                        ui.close();
                    }
                    if ui.button("Read its columns again").clicked() {
                        self.pending = Some(Action::ReloadColumns(false));
                        ui.close();
                    }
                }
            });

            ui.add_space(10.0);
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .id(filter_id())
                    .desired_width(240.0)
                    .hint_text(format!("🔍 Filter rows…  {}", cmd_key("F"))),
            );
            // Esc in the filter box clears it (the box gives up the cursor on Esc by itself)
            if (edit.has_focus() || edit.lost_focus()) && crate::pressed_keys(ui.ctx()).iter().any(|(k, _)| *k == egui::Key::Escape) {
                self.filter.clear();
                edit.surrender_focus();
            }
            if !self.filter.is_empty() && ui.small_button("✕").on_hover_text("Clear the filter").clicked() {
                self.filter.clear();
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let side = if self.s.show_sidebar { "Hide settings" } else { "⚙  Settings" };
                if ui
                    .selectable_label(self.s.show_sidebar, side)
                    .on_hover_text(format!("Show or hide the settings ({})", cmd_key(",")))
                    .clicked()
                {
                    self.s.show_sidebar = !self.s.show_sidebar;
                }
                let save_label = if self.s.sheet_mode == "new" { "Save as spreadsheet…" } else { "Update spreadsheet  ▶" };
                let save_tip = if self.s.sheet_mode == "new" {
                    "Save these rows as a new .xlsx or .csv file"
                } else {
                    "Write the new files straight into your spreadsheet (a backup is made first)"
                };
                if ui.add_enabled(!self.busy, egui::Button::new(save_label)).on_hover_text(save_tip).clicked() {
                    self.pending = Some(Action::Update);
                }
                let has_rows = !self.grid.order.is_empty();
                let copy_label = if self.grid.sel.is_some() { "Copy selection" } else { "Copy all" };
                if ui
                    .add_enabled(has_rows, egui::Button::new(RichText::new(copy_label).strong()))
                    .on_hover_text(format!("Copy to paste into a spreadsheet ({})", cmd_key("C")))
                    .clicked()
                {
                    self.pending = Some(Action::Copy);
                }
            });
        });
        ui.add_space(4.0);
    }

    // ------------------------------------------------------------ status bar

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if let Some(d) = &self.grid.data {
                let noun = if self.s.row_mode == "files" { "file" } else { "folder" };
                let total = d.rows.len();
                let shown = self.grid.order.len();
                let mut t = if d.from_sheet && self.s.only_new {
                    format!("{total} new {noun}{}", if total == 1 { "" } else { "s" })
                } else {
                    format!("{total} {noun}{}", if total == 1 { "" } else { "s" })
                };
                if shown != total {
                    t += &format!("  ·  showing {shown}");
                }
                if d.skipped > 0 {
                    t += &format!("  ·  {} already in the sheet", d.skipped);
                }
                ui.label(t).on_hover_text(format!("{} items found in the folder", d.files));
            }
            if let Some(c) = self.sort_index() {
                let name = self.grid.data.as_ref().map(|d| d.columns[c].clone()).unwrap_or_default();
                ui.colored_label(HINT, format!("·  sorted by {name} ({})", if self.s.sort_desc { "Z→A" } else { "A→Z" }));
                if ui.small_button("Unsort").clicked() {
                    self.set_sort(c, None);
                }
            }
            if self.grid.busy || self.busy {
                ui.spinner();
                ui.colored_label(HINT, if self.busy { self.status.as_str() } else { "updating…" });
            } else if !self.grid.updated_at.is_empty() {
                ui.colored_label(HINT, format!("·  updated {}", self.grid.updated_at));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.checkbox(&mut self.s.copy_headers, "Copy with header row");
                ui.separator();
                let hint = if !self.grid.copy_msg.is_empty() {
                    self.grid.copy_msg.clone()
                } else if let (Some(d), false) = (&self.grid.data, self.grid.order.is_empty()) {
                    let (r, c) = self.grid.sel.map(|s| (s.bounds().0, s.bounds().2)).unwrap_or((0, 0));
                    format!(
                        "Paste at cell {}{} · right-click for more",
                        d.letters.get(c).cloned().unwrap_or_default(),
                        d.start_row as usize + r
                    )
                } else {
                    String::new()
                };
                ui.add(egui::Label::new(RichText::new(hint).color(HINT)).truncate()).on_hover_text(
                    "Drag across cells to select · Shift-click or Shift+arrows extend · click a letter or row number for a whole \
                     column or row · right-click for Show in Finder, Open, copy paths, sort, filter and column options",
                );
            });
        });
        ui.add_space(3.0);
    }

    // ------------------------------------------------------------ settings sidebar

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let header = |t: &str| RichText::new(t).strong().size(14.5);

        egui::CollapsingHeader::new(header("Folder & rows")).default_open(true).show(ui, |ui| {
            ui.horizontal(|ui| {
                let w = ui.available_width() - 84.0;
                ui.add(egui::TextEdit::singleline(&mut self.s.folder).desired_width(w).hint_text("Choose a folder…"));
                if ui.button("Choose…").clicked() {
                    self.pending = Some(Action::BrowseFolder);
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Each row is:");
                ui.radio_value(&mut self.s.row_mode, "files".into(), "a file");
                ui.radio_value(&mut self.s.row_mode, "level".into(), "a folder at level")
                    .on_hover_text("One row per folder at this depth, with the total size of everything inside it");
                ui.add_enabled(self.s.row_mode == "level", egui::DragValue::new(&mut self.s.folder_level).range(1..=20))
                    .on_hover_text("1 = the folders directly inside the folder you scan");
                ui.radio_value(&mut self.s.row_mode, "withfiles".into(), "each folder with files in it")
                    .on_hover_text("One row for every folder that holds files (e.g. image sequences), sized by its own files");
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.s.recursive, "Include subfolders");
                ui.checkbox(&mut self.s.skip_hidden, "Skip hidden & system files");
            });
            egui::Grid::new("s1").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Only these types:");
                ui.add(egui::TextEdit::singleline(&mut self.s.extensions).desired_width(f32::INFINITY).hint_text("e.g. pdf, docx (blank = all)"));
                ui.end_row();
                ui.label("Skip names like:");
                ui.add(egui::TextEdit::singleline(&mut self.s.exclude).desired_width(f32::INFINITY).hint_text("e.g. *.tmp, Archive, Old*"));
                ui.end_row();
            });
        });
        ui.add_space(4.0);

        egui::CollapsingHeader::new(header("Spreadsheet")).default_open(true).show(ui, |ui| {
            if ui.radio_value(&mut self.s.sheet_mode, "existing".into(), "Use my spreadsheet's columns (.xlsx, .xlsm, .csv)").changed() {
                self.pending = Some(Action::ApplySheetMode);
            }
            if ui.radio_value(&mut self.s.sheet_mode, "new".into(), "Use my own columns (no spreadsheet yet)").changed() {
                self.pending = Some(Action::ApplySheetMode);
            }
            if self.s.sheet_mode == "existing" {
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 84.0;
                    ui.add(egui::TextEdit::singleline(&mut self.s.sheet_path).desired_width(w).hint_text("Choose a spreadsheet…"));
                    if ui.button("Choose…").clicked() {
                        self.pending = Some(Action::BrowseSheet);
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
                        self.pending = Some(Action::ReloadColumns(true));
                    }
                    ui.label("Header row:");
                    ui.add(egui::DragValue::new(&mut self.s.header_row).range(1..=500));
                    if ui.button("Reload").on_hover_text("Read the column headers again").clicked() {
                        self.pending = Some(Action::ReloadColumns(false));
                    }
                });
                ui.checkbox(&mut self.s.only_new, "Only list files that aren't in the sheet yet");
            } else {
                ui.colored_label(HINT, "Add, rename and arrange your columns below, or right-click a column heading in the grid.");
            }
        });
        ui.add_space(4.0);

        egui::CollapsingHeader::new(header("Columns")).default_open(true).show(ui, |ui| {
            if self.rows.is_empty() {
                ui.colored_label(HINT, "No columns yet — choose a spreadsheet, or use your own columns.");
            } else {
                let editable = self.columns_editable();
                let n = self.rows.len();
                let mut changed = false;
                let mut open_editor = None;
                let mut op: Option<(usize, &str)> = None;
                egui::Grid::new("map_grid").num_columns(if editable { 4 } else { 3 }).striped(true).spacing([6.0, 4.0]).show(ui, |ui| {
                    for (i, r) in self.rows.iter_mut().enumerate() {
                        let mut head = r.header.clone();
                        if head.chars().count() > 16 {
                            head = head.chars().take(15).collect::<String>() + "…";
                        }
                        if ui
                            .add(egui::Label::new(format!("{}  {}", col_letter(r.col), head)).sense(egui::Sense::click()))
                            .on_hover_text(format!("{}\nClick to change this column", r.header))
                            .clicked()
                        {
                            open_editor = Some(i);
                        }
                        if builder::field_combo(ui, ("field", i), 190.0, &mut r.field, &mut r.param) {
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
                            ui.add(egui::TextEdit::singleline(&mut r.param).desired_width(60.0)).on_hover_text(def.hint);
                        } else {
                            ui.label("");
                        }
                        if editable {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                if ui.add_enabled(i > 0, egui::Button::new("↑").small()).on_hover_text("Move left").clicked() {
                                    op = Some((i, "up"));
                                }
                                if ui.add_enabled(i + 1 < n, egui::Button::new("↓").small()).on_hover_text("Move right").clicked() {
                                    op = Some((i, "down"));
                                }
                                if ui.add_enabled(n > 1, egui::Button::new("🗑").small()).on_hover_text("Delete this column").clicked() {
                                    op = Some((i, "delete"));
                                }
                            });
                        }
                        ui.end_row();
                    }
                });
                if editable {
                    ui.horizontal(|ui| {
                        if ui.button("+ Add column").clicked() {
                            op = Some((n, "add"));
                        }
                        ui.colored_label(HINT, "Click a name to rename it.");
                    });
                }
                if open_editor.is_some() {
                    self.editing = open_editor;
                }
                if changed {
                    self.remember_mapping();
                    self.refresh_key();
                }
                match op {
                    Some((i, "up")) => self.move_column(i, -1),
                    Some((i, "down")) => self.move_column(i, 1),
                    Some((i, "delete")) => self.delete_column(i),
                    Some((i, "add")) => self.insert_column(i),
                    _ => {}
                }
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("Match files by:").on_hover_text("The column used to recognise files that are already in your sheet");
                let choices = self.key_choices();
                egui::ComboBox::from_id_salt("key").width(130.0).selected_text(self.s.key_header.as_str()).show_ui(ui, |ui| {
                    for c in choices {
                        ui.selectable_value(&mut self.s.key_header, c.clone(), c);
                    }
                });
                ui.label("Name separator:").on_hover_text("Splits file names for “Part of the file name”");
                ui.add(egui::TextEdit::singleline(&mut self.s.delimiter).desired_width(24.0));
            });
        });
        ui.add_space(4.0);

        egui::CollapsingHeader::new(header("Options")).default_open(false).show(ui, |ui| {
            ui.checkbox(&mut self.s.auto_refresh, "Update the grid when files change");
            ui.horizontal_wrapped(|ui| {
                ui.label("Sizes:");
                ui.radio_value(&mut self.s.size_base, 1000, "like Finder (1 KB = 1,000 bytes)");
                ui.radio_value(&mut self.s.size_base, 1024, "like Windows (1,024)");
            });
            ui.add_space(4.0);
            ui.label(RichText::new("When updating a spreadsheet:").color(HINT));
            ui.radio_value(&mut self.s.run_mode, "update".into(), "Add new files, keep existing rows");
            ui.radio_value(&mut self.s.run_mode, "fresh".into(), "Start fresh (clear rows and rebuild)");
            ui.checkbox(&mut self.s.update_existing, "Refresh details of files already listed");
            ui.checkbox(&mut self.s.backup, "Back up the spreadsheet first");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Save setup…").on_hover_text("Keep these settings in a file, e.g. one per project").clicked() {
                    self.pending = Some(Action::SaveSetup);
                }
                if ui.button("Load setup…").clicked() {
                    self.pending = Some(Action::LoadSetup);
                }
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(HINT, format!("File Tracker {}", update::current_version()));
                if ui.small_button("Check for updates").clicked() {
                    self.check_for_updates(true);
                }
                ui.checkbox(&mut self.s.check_updates, "Check at start");
            });
        });
        ui.add_space(8.0);
        ui.colored_label(HINT, self.status.as_str());
    }

    // ------------------------------------------------------------ column window

    fn column_window(&mut self, ctx: &egui::Context) {
        let Some(i) = self.editing else {
            self.rename_buf = None;
            return;
        };
        if i >= self.rows.len() {
            self.editing = None;
            return;
        }
        let editable = self.columns_editable();
        if self.rename_buf.as_ref().map(|(k, _)| *k != i).unwrap_or(true) {
            self.rename_buf = Some((i, self.rows[i].header.clone()));
        }
        let samples: &[FileInfo] = self.grid.data.as_ref().map(|d| d.samples.as_slice()).unwrap_or(&[]);
        let row = &mut self.rows[i];
        let before = (row.field.clone(), row.param.clone());
        let header = row.header.clone();
        let name = self.rename_buf.as_mut().map(|(_, n)| n);
        let open = builder::column_window(
            ctx,
            &col_letter(row.col),
            &header,
            if editable { name } else { None },
            &mut row.field,
            &mut row.param,
            samples,
            &mut self.edit_sample,
        );
        let changed = (row.field.as_str(), row.param.as_str()) != (before.0.as_str(), before.1.as_str());
        if changed {
            self.remember_mapping();
            self.refresh_key();
            ctx.request_repaint();
        }
        // apply the typed name once it's usable (not empty, no commas, not taken)
        if let Some((_, typed)) = self.rename_buf.clone() {
            if editable && typed.trim() != header && !typed.trim().is_empty() && self.rename_column(i, &typed) {
                ctx.request_repaint();
            }
        }
        if !open {
            self.editing = None;
            self.rename_buf = None;
        }
    }

    // ------------------------------------------------------------ shortcuts & drops

    fn global_keys(&mut self, ctx: &egui::Context) {
        use egui::Key;
        let keys = crate::pressed_keys(ctx);
        let pressed = |k: Key, cmd: bool, shift: bool| keys.iter().any(|(key, m)| *key == k && m.command == cmd && m.shift == shift);
        let find = pressed(Key::F, true, false);
        let refresh = pressed(Key::R, true, false) || keys.iter().any(|(k, _)| *k == Key::F5);
        let settings = pressed(Key::Comma, true, false);
        if find {
            ctx.memory_mut(|m| m.request_focus(filter_id()));
        }
        if refresh {
            self.pending = Some(Action::Refresh);
        }
        if settings {
            self.s.show_sidebar = !self.s.show_sidebar;
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let (dropped, hovering) = ctx.input(|i| {
            (
                i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect::<Vec<_>>(),
                !i.raw.hovered_files.is_empty(),
            )
        });
        if !dropped.is_empty() {
            self.dropped(dropped);
        }
        if hovering {
            let screen = ctx.content_rect();
            egui::Area::new(egui::Id::unique("drop_overlay")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
                ui.painter().rect_filled(screen, 0.0, ui.visuals().selection.bg_fill.gamma_multiply(0.25));
                ui.painter().text(
                    screen.center(),
                    egui::Align2::CENTER_CENTER,
                    "Drop a folder to scan it, or a spreadsheet to use its columns",
                    egui::FontId::proportional(20.0),
                    ui.visuals().strong_text_color(),
                );
            });
        }
    }
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

        egui::Panel::top("top_bar").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status_bar").show(ui, |ui| self.status_bar(ui));
        // first launch: just the welcome screen until a folder is chosen
        if self.s.show_sidebar && !self.s.folder.trim().is_empty() {
            egui::Panel::left("settings")
                .resizable(true)
                .default_size(440.0)
                .size_range(360.0..=700.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("side").show(ui, |ui| {
                        ui.add_space(6.0);
                        self.sidebar(ui);
                        ui.add_space(8.0);
                    });
                });
        }
        egui::CentralPanel::default().show(ui, |ui| self.grid_view(ui));

        self.column_window(&ctx);
        self.grid_menu(&ctx);
        self.global_keys(&ctx);
        self.grid_keys(&ctx);
        self.handle_drops(&ctx);
        if self.filter != self.filter_applied {
            self.rebuild_order();
        }
        if let Some(a) = self.pending.take() {
            self.do_action(a);
        }
        self.update_watchers();
        self.follow_row_mode();
        self.schedule_grid();
    }
}
