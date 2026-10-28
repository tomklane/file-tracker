//! The grid: drawing it, selecting with mouse and keyboard, and its
//! right-click menus.

use crate::core::file_manager_name;
use crate::{sorting, App, DragKind, GridMenu, Hit, Sel, HINT};
use eframe::egui::{self, Align, Color32, Layout, Pos2, Rect, RichText, Stroke, StrokeKind};
use egui_extras::{Column, TableBuilder};

const ROW_H: f32 = 22.0;
const GUTTER_W: f32 = 54.0;

/// Something chosen from a right-click menu.
enum Cmd {
    Reveal,
    Open,
    Copy,
    CopyWithHeaders,
    CopyPaths,
    CopyNames,
    Sort(usize, Option<bool>),
    FilterTo(String),
    ClearFilter,
    EditColumn(usize),
    LeaveAsIs(usize),
    InsertColumn(usize),
    DeleteColumn(usize),
    MoveColumn(usize, i64),
    SkipName(String),
    SelectAll,
}

fn cmd_key(k: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{k}")
    } else {
        format!("Ctrl+{k}")
    }
}

fn cmd_shift_key(k: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⇧⌘{k}")
    } else {
        format!("Ctrl+Shift+{k}")
    }
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        s.chars().take(n - 1).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

/// A menu line; returns true when chosen.
fn item(ui: &mut egui::Ui, label: impl Into<String>, shortcut: Option<String>, enabled: bool) -> bool {
    let mut b = egui::Button::new(label.into()).frame(false);
    if let Some(s) = shortcut {
        b = b.shortcut_text(RichText::new(s).color(HINT));
    }
    ui.add_enabled(enabled, b).clicked()
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

impl App {
    pub(crate) fn grid_view(&mut self, ui: &mut egui::Ui) {
        let Some(d) = &self.grid.data else {
            self.empty_state(ui);
            return;
        };
        if self.grid.order.is_empty() {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                let msg = if !self.filter.trim().is_empty() {
                    format!("No rows contain “{}”.", self.filter.trim())
                } else if d.skipped > 0 {
                    format!(
                        "Everything in the folder is already in your sheet ({} items).\nNew files will appear here automatically.",
                        d.skipped
                    )
                } else {
                    "Nothing found. Check the folder and the filters in Settings.".to_string()
                };
                ui.label(RichText::new(msg).size(16.0).color(HINT));
                if !self.filter.trim().is_empty() && ui.button("Clear the filter").clicked() {
                    self.filter.clear();
                }
            });
            return;
        }

        let nrows = self.grid.order.len();
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

        // column widths follow the content; a big change re-lays out the table
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
        let scroll_to = self.scroll_to.take();

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
                if let Some(r) = scroll_to {
                    tb = tb.scroll_to_row(r, None);
                }
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
                                    "Click to select the column · double-click or right-click to change it · arrows sort"
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

    fn hit_at(&self, pos: Pos2) -> Option<Hit> {
        self.grid.hits.iter().find(|(r, _)| r.contains(pos)).map(|(_, h)| *h)
    }

    /// Double-clicking a heading (or choosing Edit column) opens the column window.
    pub(crate) fn edit_grid_column(&mut self, c: usize) {
        if let Some(i) = self.row_for_grid_col(c) {
            self.editing = Some(i);
            self.grid.drag = None;
        }
    }

    /// Excel-style mouse selection, plus the right-click menu.
    fn grid_mouse(&mut self, ctx: &egui::Context, nrows: usize, ncols: usize) {
        let (primary, secondary, down, pos, shift) = ctx.input(|i| {
            // read presses from the events: a quick click or trackpad tap can
            // press and release within one frame
            let mut primary = None;
            let mut secondary = None;
            for e in &i.events {
                if let egui::Event::PointerButton { pos, button, pressed: true, modifiers } = e {
                    match button {
                        // Control-click is a right-click on a Mac
                        egui::PointerButton::Primary if cfg!(target_os = "macos") && modifiers.ctrl => secondary = Some(*pos),
                        egui::PointerButton::Primary => primary = Some((*pos, modifiers.shift)),
                        egui::PointerButton::Secondary => secondary = Some(*pos),
                        _ => {}
                    }
                }
            }
            (primary, secondary, i.pointer.primary_down(), i.pointer.latest_pos(), i.modifiers.shift)
        });
        let double = ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
        let on_grid = |p: Pos2| ctx.layer_id_at(p).map(|l| l.order == egui::Order::Background).unwrap_or(false);
        let last_c = ncols - 1;

        // right-click: select what's under the pointer (unless already selected) and open the menu
        if let Some(p) = secondary.filter(|p| on_grid(*p)) {
            if let Some(hit) = self.hit_at(p) {
                let cur = self.grid.sel;
                let keep = match (hit, cur) {
                    (Hit::Cell(r, c), Some(s)) => s.contains(r, c),
                    (Hit::Row(r), Some(s)) => s.has_row(r),
                    (Hit::Col(c), Some(s)) => s.has_col(c) && s.bounds().0 == 0 && s.bounds().1 == nrows - 1,
                    _ => false,
                };
                if !keep {
                    self.grid.sel = match hit {
                        Hit::Cell(r, c) => Some(Sel { a: (r, c), b: (r, c) }),
                        Hit::Row(r) => Some(Sel { a: (r, 0), b: (r, last_c) }),
                        Hit::Col(c) | Hit::Sort(c) => Some(Sel { a: (0, c), b: (nrows - 1, c) }),
                        Hit::Corner => Some(Sel { a: (0, 0), b: (nrows - 1, last_c) }),
                    };
                }
                let target = if let Hit::Sort(c) = hit { Hit::Col(c) } else { hit };
                self.menu = Some(GridMenu { pos: p, target, fresh: true });
                ctx.memory_mut(|m| m.stop_text_input());
                return;
            }
        }

        if let Some((p, click_shift)) = primary.filter(|(p, _)| on_grid(*p)) {
            let shift = click_shift || shift;
            if let Some(Hit::Sort(c)) = self.hit_at(p) {
                self.toggle_sort(c);
                return;
            }
            if let Some(hit) = self.hit_at(p) {
                ctx.memory_mut(|m| m.stop_text_input());
                self.grid.copy_msg.clear();
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
                    self.edit_grid_column(c);
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
                            sel.b = (r, last_c)
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

    /// Keyboard: arrows move (Shift extends), ⌘A all, ⌘C copy, Esc clears,
    /// ⌘O open, ⇧⌘R show in Finder. Only when no text box has the cursor.
    pub(crate) fn grid_keys(&mut self, ctx: &egui::Context) {
        // typing in a text box takes the keys
        if ctx.text_edit_focused() {
            return;
        }
        let (nr, nc) = match &self.grid.data {
            Some(d) if !self.grid.order.is_empty() => (self.grid.order.len(), d.columns.len()),
            _ => return,
        };
        use egui::Key;
        let keys = crate::pressed_keys(ctx);
        let copy_event = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
        let copy_all = keys.iter().any(|(k, m)| *k == Key::C && m.command && m.shift);
        if copy_all {
            self.copy_all();
        } else if copy_event {
            self.copy_selection();
        }
        for (key, m) in keys {
            match key {
                Key::A if m.command => self.grid.sel = Some(Sel { a: (0, 0), b: (nr - 1, nc - 1) }),
                Key::Escape => {
                    if self.menu.take().is_none() {
                        self.grid.sel = None;
                        self.grid.copy_msg.clear();
                    }
                }
                Key::O if m.command && !m.shift => self.open_selected(),
                Key::R if m.command && m.shift => self.reveal_selected(),
                Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight | Key::PageUp | Key::PageDown => {
                    let (dr, dc): (i64, i64) = match key {
                        Key::ArrowUp => (-1, 0),
                        Key::ArrowDown => (1, 0),
                        Key::ArrowLeft => (0, -1),
                        Key::ArrowRight => (0, 1),
                        Key::PageUp => (-20, 0),
                        _ => (20, 0),
                    };
                    self.menu = None;
                    let cur = self.grid.sel.map(|s| s.b).unwrap_or((0, 0));
                    // ⌘+arrow jumps to the edge, like a spreadsheet
                    let (dr, dc) = if m.command { (dr * nr as i64, dc * nc as i64) } else { (dr, dc) };
                    let r = (cur.0 as i64 + dr).clamp(0, nr as i64 - 1) as usize;
                    let c = (cur.1 as i64 + dc).clamp(0, nc as i64 - 1) as usize;
                    self.grid.sel = Some(match (m.shift, self.grid.sel) {
                        (true, Some(s)) => Sel { a: s.a, b: (r, c) },
                        _ => Sel { a: (r, c), b: (r, c) },
                    });
                    self.scroll_to = Some(r);
                    self.grid.copy_msg.clear();
                }
                _ => {}
            }
        }
    }

    /// The right-click menu, drawn above everything.
    pub(crate) fn grid_menu(&mut self, ctx: &egui::Context) {
        let Some(menu) = &self.menu else { return };
        let (pos, target, fresh) = (menu.pos, menu.target, menu.fresh);
        let Some(d) = &self.grid.data else {
            self.menu = None;
            return;
        };
        let editable = self.columns_editable();
        let fm = file_manager_name();
        let n_sel = self.selected_paths().len();
        let many = |verb: &str| if n_sel > 1 { format!("{verb} {n_sel} items") } else { verb.to_string() };
        let sorted_by = self.sort_index();
        let filtering = !self.filter.trim().is_empty();
        let folder_rows = self.s.row_mode != "files";
        // the column and value under the pointer
        let (col, value) = match target {
            Hit::Cell(r, c) => (Some(c), Some(sorting::cell(d, &self.grid.order, self.renumber(), r, c).0)),
            Hit::Col(c) => (Some(c), None),
            _ => (None, None),
        };
        let col_name = col.map(|c| short(&d.columns[c], 22));
        let row_path = match target {
            Hit::Cell(r, _) | Hit::Row(r) => self.grid.order.get(r).and_then(|&i| d.paths.get(i)).cloned(),
            _ => None,
        };
        let row_has_col = col.and_then(|c| self.row_for_grid_col(c)).is_some();
        let ncols = d.columns.len();

        let mut chosen: Option<Cmd> = None;
        let area = egui::Area::new(egui::Id::unique("grid_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_min_width(250.0);
                    ui.with_layout(Layout::top_down_justified(Align::LEFT), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let mut pick = |ui: &mut egui::Ui, label: String, sc: Option<String>, enabled: bool, cmd: Cmd| {
                            if item(ui, label, sc, enabled) {
                                chosen = Some(cmd);
                            }
                        };
                        let on_rows = matches!(target, Hit::Cell(..) | Hit::Row(_));
                        if on_rows {
                            pick(ui, many(&format!("Show in {fm}")), Some(cmd_shift_key("R")), true, Cmd::Reveal);
                            pick(ui, many("Open"), Some(cmd_key("O")), true, Cmd::Open);
                            ui.separator();
                        }
                        if !matches!(target, Hit::Corner) {
                            pick(ui, "Copy".into(), Some(cmd_key("C")), true, Cmd::Copy);
                            pick(ui, "Copy with header row".into(), None, true, Cmd::CopyWithHeaders);
                        }
                        if on_rows {
                            pick(ui, if n_sel > 1 { "Copy paths".into() } else { "Copy path".into() }, None, true, Cmd::CopyPaths);
                            pick(ui, if n_sel > 1 { "Copy names".into() } else { "Copy name".into() }, None, true, Cmd::CopyNames);
                        }
                        if matches!(target, Hit::Corner) {
                            pick(ui, "Select all".into(), Some(cmd_key("A")), true, Cmd::SelectAll);
                            pick(ui, "Copy everything".into(), None, true, Cmd::Copy);
                        }
                        if let (Some(c), Some(name)) = (col, &col_name) {
                            ui.separator();
                            if let Some(v) = value.as_ref().filter(|v| !v.trim().is_empty()) {
                                pick(ui, format!("Show only rows with “{}”", short(v, 24)), None, true, Cmd::FilterTo(v.clone()));
                            }
                            if filtering {
                                pick(ui, "Show all rows (clear filter)".into(), None, true, Cmd::ClearFilter);
                            }
                            pick(ui, format!("Sort by {name}, A → Z"), None, true, Cmd::Sort(c, Some(false)));
                            pick(ui, format!("Sort by {name}, Z → A"), None, true, Cmd::Sort(c, Some(true)));
                            if sorted_by.is_some() {
                                pick(ui, "Unsort (folder order)".into(), None, true, Cmd::Sort(c, None));
                            }
                            ui.separator();
                            pick(ui, format!("Change column {name}…"), None, row_has_col, Cmd::EditColumn(c));
                            pick(ui, "Leave this column blank".into(), None, row_has_col && d.filled[c], Cmd::LeaveAsIs(c));
                            if editable {
                                pick(ui, "Insert column left".into(), None, true, Cmd::InsertColumn(c));
                                pick(ui, "Insert column right".into(), None, true, Cmd::InsertColumn(c + 1));
                                if matches!(target, Hit::Col(_)) {
                                    pick(ui, "Move column left".into(), None, c > 0, Cmd::MoveColumn(c, -1));
                                    pick(ui, "Move column right".into(), None, c + 1 < ncols, Cmd::MoveColumn(c, 1));
                                }
                                pick(ui, "Delete column".into(), None, ncols > 1, Cmd::DeleteColumn(c));
                            } else if matches!(target, Hit::Col(_)) {
                                ui.add_enabled(false, egui::Button::new("Add or rename columns in your spreadsheet").frame(false));
                            }
                        }
                        if let Some(p) = &row_path {
                            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                            let parent = p.parent().and_then(|q| q.file_name()).map(|n| n.to_string_lossy().to_string());
                            ui.separator();
                            if folder_rows {
                                pick(ui, format!("Skip folders named “{}”", short(&name, 26)), None, true, Cmd::SkipName(name));
                            } else {
                                pick(ui, format!("Skip files named “{}”", short(&name, 26)), None, true, Cmd::SkipName(name));
                                if let Some(parent) = parent.filter(|_| n_sel <= 1) {
                                    pick(ui, format!("Skip folders named “{}”", short(&parent, 26)), None, true, Cmd::SkipName(parent));
                                }
                            }
                        }
                    });
                });
            });

        // close on a choice, a click elsewhere, or after one frame without a menu
        let clicked_outside = !fresh
            && ctx.input(|i| {
                i.events.iter().any(|e| {
                    matches!(e, egui::Event::PointerButton { pressed: true, pos, .. } if !area.response.rect.contains(*pos))
                })
            });
        if let Some(m) = &mut self.menu {
            m.fresh = false;
        }
        if clicked_outside {
            self.menu = None;
        }
        if let Some(cmd) = chosen {
            self.menu = None;
            self.run_cmd(cmd);
        }
    }

    fn run_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Reveal => self.reveal_selected(),
            Cmd::Open => self.open_selected(),
            Cmd::Copy => self.copy_selection(),
            Cmd::CopyWithHeaders => self.copy_with_headers(),
            Cmd::CopyPaths => self.copy_paths(false),
            Cmd::CopyNames => self.copy_paths(true),
            Cmd::Sort(c, dir) => self.set_sort(c, dir),
            Cmd::FilterTo(v) => {
                self.filter = v;
                self.grid.sel = None;
            }
            Cmd::ClearFilter => self.filter.clear(),
            Cmd::EditColumn(c) => self.edit_grid_column(c),
            Cmd::LeaveAsIs(c) => {
                if let Some(i) = self.row_for_grid_col(c) {
                    self.rows[i].field.clear();
                    self.rows[i].param.clear();
                    self.remember_mapping();
                    self.refresh_key();
                }
            }
            Cmd::InsertColumn(at) => self.insert_column(at),
            Cmd::DeleteColumn(c) => self.delete_column(c),
            Cmd::MoveColumn(c, delta) => self.move_column(c, delta),
            Cmd::SkipName(n) => self.skip_name(&n),
            Cmd::SelectAll => {
                if let Some(d) = &self.grid.data {
                    let (nr, nc) = (self.grid.order.len(), d.columns.len());
                    if nr > 0 {
                        self.grid.sel = Some(Sel { a: (0, 0), b: (nr - 1, nc - 1) });
                    }
                }
            }
        }
    }

    /// Shown when there's nothing to put in the grid yet.
    fn empty_state(&mut self, ui: &mut egui::Ui) {
        ui.add_space(70.0);
        ui.vertical_centered(|ui| {
            if self.grid.busy {
                ui.spinner();
                ui.colored_label(HINT, "Scanning…");
                return;
            }
            if self.s.folder.trim().is_empty() {
                ui.label(RichText::new("Track the files in a folder").size(24.0).strong());
                ui.add_space(8.0);
                ui.label(RichText::new("Drop a folder onto this window, or").size(15.0).color(HINT));
                ui.add_space(6.0);
                if ui.add(egui::Button::new(RichText::new("📁  Choose a folder…").size(16.0)).min_size(egui::vec2(220.0, 38.0))).clicked() {
                    self.pending = Some(crate::Action::BrowseFolder);
                }
                ui.add_space(26.0);
                ui.label(RichText::new("Have a tracking spreadsheet? Its columns are used as they are.").color(HINT));
                ui.add_space(4.0);
                if ui.button("📊  Choose a spreadsheet… (optional)").clicked() {
                    self.pending = Some(crate::Action::BrowseSheet);
                }
                ui.add_space(4.0);
                ui.colored_label(HINT, "Otherwise you start with standard columns and can change them any time.");
            } else {
                ui.label(
                    RichText::new(self.grid.message.as_deref().unwrap_or("Choose a folder to begin."))
                        .size(16.0)
                        .color(HINT),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let w = 420.0;
                    ui.add_space((ui.available_width() - w).max(0.0) / 2.0);
                    if ui.button("Choose a folder…").clicked() {
                        self.pending = Some(crate::Action::BrowseFolder);
                    }
                    if ui.button("Choose a spreadsheet…").clicked() {
                        self.pending = Some(crate::Action::BrowseSheet);
                    }
                    if self.s.sheet_mode != "new" && ui.button("Start from new columns").clicked() {
                        self.pending = Some(crate::Action::UseNewColumns);
                    }
                });
            }
        });
    }
}
