//! The column builder window: choose what fills a column, including picking
//! a piece of any folder or file name by clicking it.

use crate::core::{field, FileInfo, FIELDS};
use crate::extract::{self, Case, Method, Rule, Source};
use eframe::egui::{self, text::LayoutJob, Color32, RichText, TextFormat};

const HINT: Color32 = Color32::from_rgb(128, 128, 128);
const STD_SEPS: [(char, &str); 4] = [('_', "_"), ('-', "-"), (' ', "space"), ('.', ".")];

fn to_int(s: &str, d: i64) -> i64 {
    s.trim().parse().unwrap_or(d)
}

/// Change what fills a column. Switching to a custom rule starts from whatever
/// the column did before, so nothing has to be set up twice.
pub fn switch_field(field_key: &mut String, param: &mut String, new: &str) {
    if field_key == new {
        return;
    }
    let (old, old_param) = (field_key.clone(), param.clone());
    *field_key = new.to_string();
    *param = if new == "extract" {
        let mut r = Rule::default();
        let whole = |r: &mut Rule, s: Source| {
            r.source = s;
            r.method = Method::Whole;
        };
        match old.as_str() {
            "part" => r.part_from = to_int(&old_param, 1),
            "level" => {
                whole(&mut r, Source::Folder);
                r.level = to_int(&old_param, 1);
            }
            "parent" => {
                whole(&mut r, Source::Folder);
                r.level = -1;
            }
            "name" => whole(&mut r, Source::FileName),
            "stem" => whole(&mut r, Source::Stem),
            "ext" => whole(&mut r, Source::Extension),
            "ext_upper" => {
                whole(&mut r, Source::Extension);
                r.case = Case::Upper;
            }
            "rel_folder" => whole(&mut r, Source::RelFolder),
            "rel_path" => whole(&mut r, Source::RelPath),
            _ => {}
        }
        extract::to_param(&r)
    } else {
        field(new).default.unwrap_or("").to_string()
    };
}

/// The label shown in "Fill with" boxes.
pub fn field_label(field_key: &str, param: &str, max_chars: usize) -> String {
    let s = if field_key == "extract" {
        let d = extract::describe(&extract::parse(param));
        let mut c = d.chars();
        format!("Custom: {}{}", c.by_ref().take(1).flat_map(char::to_uppercase).collect::<String>(), c.as_str())
    } else {
        field(field_key).label.to_string()
    };
    if s.chars().count() > max_chars {
        s.chars().take(max_chars.saturating_sub(1)).collect::<String>() + "…"
    } else {
        s
    }
}

/// The "Fill with" dropdown. Returns true when the choice changed.
pub fn field_combo(ui: &mut egui::Ui, id: impl egui::AsIdSalt, width: f32, field_key: &mut String, param: &mut String) -> bool {
    let mut choice = field_key.clone();
    egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(field_label(field_key, param, (width / 7.0) as usize))
        .show_ui(ui, |ui| {
            ui.set_min_width(270.0);
            for f in FIELDS {
                ui.selectable_value(&mut choice, f.key.to_string(), f.label);
            }
        });
    if choice != *field_key {
        switch_field(field_key, param, &choice);
        true
    } else {
        false
    }
}

fn highlighted(ui: &egui::Ui, text: &str, span: Option<(usize, usize)>, size: f32) -> LayoutJob {
    let font = egui::FontId::monospace(size);
    let normal = TextFormat { font_id: font.clone(), color: ui.visuals().text_color(), ..Default::default() };
    let marked = TextFormat {
        font_id: font,
        color: ui.visuals().strong_text_color(),
        background: ui.visuals().selection.bg_fill,
        ..Default::default()
    };
    let chars: Vec<char> = text.chars().collect();
    let mut job = LayoutJob::default();
    match span {
        Some((a, b)) if a < b => {
            let seg = |x: usize, y: usize| chars[x..y].iter().collect::<String>();
            job.append(&seg(0, a), 0.0, normal.clone());
            job.append(&seg(a, b), 0.0, marked);
            job.append(&seg(b, chars.len()), 0.0, normal);
        }
        _ => job.append(if text.is_empty() { "(empty)" } else { text }, 0.0, normal),
    }
    job
}

fn chip(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).monospace()).selected(selected))
}

fn step(ui: &mut egui::Ui, n: &str, title: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(format!("{n}  {title}")).strong().size(14.5));
    ui.add_space(2.0);
}

/// Shows the column window. Returns false once the user closes it.
pub fn column_window(
    ctx: &egui::Context,
    letter: &str,
    header: &str,
    field_key: &mut String,
    param: &mut String,
    samples: &[FileInfo],
    sample_idx: &mut usize,
) -> bool {
    let mut open = true;
    let mut done = false;
    egui::Window::new(format!("Column {letter} — {header}"))
        .id(egui::Id::unique("column_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(600.0)
        // over the settings sidebar, so the live grid stays visible
        .default_pos(ctx.content_rect().min + egui::vec2(10.0, 50.0))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height() - 140.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Fill with:");
                    field_combo(ui, "colwin_field", 330.0, field_key, param);
                });
                if field_key == "extract" {
                    let mut rule = extract::parse(param);
                    rule_editor(ui, &mut rule, samples, sample_idx);
                    let new = extract::to_param(&rule);
                    if new != *param {
                        *param = new;
                    }
                } else {
                    let def = field(field_key);
                    if def.default.is_some() {
                        ui.horizontal(|ui| {
                            ui.label("Setting:");
                            ui.add(egui::TextEdit::singleline(param).desired_width(120.0));
                            ui.colored_label(HINT, def.hint);
                        });
                    }
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(HINT, "Need a particular piece of a folder or file name?");
                        if ui.button("Build a custom rule…").clicked() {
                            switch_field(field_key, param, "extract");
                        }
                    });
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.colored_label(HINT, "The grid updates as you change this.");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new("Done").strong()).clicked() {
                        done = true;
                    }
                });
            });
        });
    if !open || done {
        return false;
    }
    true
}

fn rule_editor(ui: &mut egui::Ui, r: &mut Rule, samples: &[FileInfo], sample_idx: &mut usize) {
    let shift = ui.input(|i| i.modifiers.shift);
    if !samples.is_empty() {
        *sample_idx = (*sample_idx).min(samples.len() - 1);
    }
    let sample = samples.get(*sample_idx);

    // ---- example picker
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Example file:");
        if samples.is_empty() {
            ui.colored_label(HINT, "choose a folder (and spreadsheet) to see real examples here");
            return;
        }
        if ui.small_button("◀").on_hover_text("Previous example").clicked() {
            *sample_idx = (*sample_idx + samples.len() - 1) % samples.len();
        }
        ui.label(format!("{} of {}", *sample_idx + 1, samples.len()));
        if ui.small_button("▶").on_hover_text("Next example").clicked() {
            *sample_idx = (*sample_idx + 1) % samples.len();
        }
    });

    // ---- step 1: source
    step(ui, "1", "Take text from — click a folder or the file name");
    if let Some(s) = sample {
        let n = s.folders.len() as i64;
        let cur_folder = if r.source == Source::Folder {
            let idx = if r.level >= 0 { r.level } else { n + r.level };
            Some(idx)
        } else {
            None
        };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (k, name) in s.folders.iter().enumerate() {
                let k = k as i64;
                let tip = if k == 0 { "The folder you scan".to_string() } else { format!("Folder level {k}") };
                if chip(ui, &format!("📁 {name}"), cur_folder == Some(k)).on_hover_text(tip).clicked() {
                    r.source = Source::Folder;
                    r.level = if r.level < 0 { k - n } else { k };
                }
                ui.colored_label(HINT, "/");
            }
            let file_sel = matches!(r.source, Source::FileName | Source::Stem | Source::Extension);
            let (icon, tip) = if s.is_folder { ("📁", "This row's folder name") } else { ("📄", "The file name") };
            if chip(ui, &format!("{icon} {}", s.name), file_sel).on_hover_text(tip).clicked() && !file_sel {
                r.source = Source::Stem;
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        ui.colored_label(HINT, "or:");
        for (src, label) in [
            (Source::Stem, "Name without extension"),
            (Source::FileName, "Whole file name"),
            (Source::Extension, "Extension"),
            (Source::RelFolder, "Folder path"),
            (Source::RelPath, "File path"),
        ] {
            ui.radio_value(&mut r.source, src, label);
        }
    });
    if r.source == Source::Folder {
        ui.horizontal_wrapped(|ui| {
            ui.label("Count folders:");
            let n = sample.map(|s| s.folders.len() as i64);
            let mut from_top = r.level >= 0;
            let top_label = format!("from the top (level {})", if r.level >= 0 { r.level } else { n.map(|n| n + r.level).unwrap_or(1) });
            let up_label = format!(
                "up from the file ({} up)",
                if r.level < 0 { -r.level } else { n.map(|n| n - r.level).unwrap_or(1) }
            );
            let a = ui.radio_value(&mut from_top, true, top_label).changed();
            let b = ui.radio_value(&mut from_top, false, up_label).changed();
            if a || b {
                let n = n.unwrap_or(r.level.abs() + 1);
                r.level = if from_top { (n + r.level).max(0) } else { (r.level - n).min(-1) };
            }
        });
        ui.colored_label(
            HINT,
            "Use “from the top” for things like the project folder, and “up from the file” when folders are nested to different depths.",
        );
    }

    // ---- step 2: which part
    step(ui, "2", "Keep");
    ui.horizontal_wrapped(|ui| {
        ui.radio_value(&mut r.method, Method::Whole, "All of it");
        ui.radio_value(&mut r.method, Method::Split, "A section between separators");
        ui.radio_value(&mut r.method, Method::Chars, "Characters by position");
        ui.radio_value(&mut r.method, Method::Between, "Text between markers");
        ui.radio_value(&mut r.method, Method::Pattern, "Pattern (advanced)");
    });
    let src_text = sample.map(|s| extract::source_text(r, s)).unwrap_or_default();
    match r.method {
        Method::Whole => {}
        Method::Split => {
            ui.horizontal_wrapped(|ui| {
                ui.label("Split at:");
                for (ch, label) in STD_SEPS {
                    let mut on = r.seps.contains(ch);
                    if ui.checkbox(&mut on, label).changed() {
                        if on {
                            r.seps.push(ch);
                        } else {
                            r.seps.retain(|c| c != ch);
                        }
                    }
                }
                ui.label("other:");
                let mut other: String = r.seps.chars().filter(|c| !STD_SEPS.iter().any(|(s, _)| s == c)).collect();
                if ui.add(egui::TextEdit::singleline(&mut other).desired_width(50.0).hint_text("e.g. ,+")).changed() {
                    let std: String = r.seps.chars().filter(|c| STD_SEPS.iter().any(|(s, _)| s == c)).collect();
                    let mut seen = std.clone();
                    for ch in other.chars() {
                        if !seen.contains(ch) {
                            seen.push(ch);
                        }
                    }
                    r.seps = seen;
                }
            });
            let parts = extract::split_spans(&src_text, &r.seps);
            let chars: Vec<char> = src_text.chars().collect();
            let from_end = r.part_from < 0;
            if !parts.is_empty() {
                let np = parts.len() as i64;
                let resolve = |k: i64| if k > 0 { k - 1 } else { np + k };
                let a = resolve(r.part_from);
                let b = if r.to_end { np - 1 } else if r.part_to == 0 { a } else { resolve(r.part_to) };
                let (lo, hi) = (a.min(b), a.max(b));
                ui.horizontal_wrapped(|ui| {
                    ui.label("Click a section:");
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (i, (x, y)) in parts.iter().enumerate() {
                        let i = i as i64;
                        let text: String = chars[*x..*y].iter().collect();
                        let num = if from_end { i - np } else { i + 1 };
                        let resp = chip(ui, &text, (lo..=hi).contains(&i)).on_hover_text(format!(
                            "Section {num}{}. Shift-click to take several sections.",
                            if from_end { " (counting from the end)" } else { "" }
                        ));
                        if resp.clicked() {
                            if shift {
                                r.part_to = num;
                                r.to_end = false;
                            } else {
                                r.part_from = num;
                                r.part_to = 0;
                                r.to_end = false;
                            }
                        }
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Section");
                ui.add(egui::DragValue::new(&mut r.part_from).range(-50..=50));
                ui.label("to");
                ui.add_enabled(!r.to_end, egui::DragValue::new(&mut r.part_to).range(-50..=50))
                    .on_hover_text("0 = just one section");
                ui.checkbox(&mut r.to_end, "to the end");
                let mut fe = from_end;
                if ui
                    .checkbox(&mut fe, "count from the end")
                    .on_hover_text("Use this when names have different numbers of sections but the one you want is always near the end")
                    .changed()
                {
                    let np = parts.len().max(1) as i64;
                    let flip = |k: i64| if k == 0 { 0 } else if fe && k > 0 { k - np - 1 } else if !fe && k < 0 { np + k + 1 } else { k };
                    r.part_from = flip(r.part_from);
                    r.part_to = flip(r.part_to);
                    if r.part_from == 0 {
                        r.part_from = if fe { -1 } else { 1 };
                    }
                }
            });
        }
        Method::Chars => {
            ui.horizontal_wrapped(|ui| {
                ui.label("Start at character");
                ui.add(egui::DragValue::new(&mut r.char_from).range(-500..=500));
                ui.label("and take");
                ui.add(egui::DragValue::new(&mut r.char_len).range(0..=500));
                ui.label("characters");
            });
            ui.colored_label(HINT, "0 characters = all the rest. A negative start counts from the end: −3 = the last three characters.");
        }
        Method::Between => {
            ui.horizontal_wrapped(|ui| {
                ui.label("After");
                ui.add(egui::TextEdit::singleline(&mut r.after).desired_width(110.0).hint_text("start"));
                ui.label("and before");
                ui.add(egui::TextEdit::singleline(&mut r.before).desired_width(110.0).hint_text("end"));
                ui.checkbox(&mut r.ignore_case, "ignore upper/lower case");
            });
            ui.colored_label(HINT, "Leave “After” blank to start at the beginning, “before” blank to go to the end. E.g. after “Rev” and before “_”.");
        }
        Method::Pattern => {
            ui.horizontal(|ui| {
                ui.label("Pattern:");
                ui.add(egui::TextEdit::singleline(&mut r.pattern).desired_width(260.0).font(egui::TextStyle::Monospace).hint_text(r"e.g. (\d{4}-\d{2})"));
                ui.checkbox(&mut r.ignore_case, "ignore case");
            });
            if let Some(e) = extract::regex_error(&r.pattern).filter(|_| !r.pattern.is_empty()) {
                ui.colored_label(Color32::from_rgb(220, 90, 80), format!("Pattern problem: {e}"));
            }
            ui.colored_label(HINT, r"A regular expression. If it has a part in (brackets), only that part is kept. \d = digit, \w = letter or digit, + = one or more.");
        }
    }

    // ---- step 3: tidy
    step(ui, "3", "Tidy up");
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut r.trim, "Trim spaces");
        ui.label("  Case:");
        egui::ComboBox::from_id_salt("rule_case")
            .width(110.0)
            .selected_text(match r.case {
                Case::AsIs => "As is",
                Case::Upper => "UPPER",
                Case::Lower => "lower",
                Case::Title => "Title Case",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut r.case, Case::AsIs, "As is");
                ui.selectable_value(&mut r.case, Case::Upper, "UPPER");
                ui.selectable_value(&mut r.case, Case::Lower, "lower");
                ui.selectable_value(&mut r.case, Case::Title, "Title Case");
            });
        ui.label("  If nothing is found, use:");
        ui.add(egui::TextEdit::singleline(&mut r.if_empty).desired_width(80.0).hint_text("blank"));
    });

    // ---- results
    ui.add_space(10.0);
    ui.separator();
    let Some(s) = sample else { return };
    let e = extract::apply(r, s);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Result:").strong());
        let shown = if e.text.is_empty() { "(nothing)".to_string() } else { e.text.clone() };
        ui.label(RichText::new(shown).monospace().size(17.0).strong().color(ui.visuals().strong_text_color()));
    });
    ui.horizontal_wrapped(|ui| {
        ui.colored_label(HINT, "from");
        ui.label(highlighted(ui, &e.source, e.span, 13.0));
    });

    ui.add_space(8.0);
    ui.label(RichText::new("Check it on other files").strong());
    ui.colored_label(HINT, "Click a row to use it as the example above.");
    let others: Vec<usize> = {
        // a spread of other files, never repeating the example shown above
        let n = samples.len();
        let want = 8.min(n.saturating_sub(1));
        let mut v: Vec<usize> = (0..want).map(|k| (*sample_idx + 1 + k * (n - 1) / want.max(1)) % n).collect();
        v.dedup();
        v
    };
    egui::Grid::new("rule_examples").striped(true).num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
        ui.colored_label(HINT, "Taken from");
        ui.colored_label(HINT, "Gives");
        ui.end_row();
        for idx in others {
            let ex = extract::apply(r, &samples[idx]);
            let src = ui.add(egui::Label::new(highlighted(ui, &ex.source, ex.span, 12.0)).sense(egui::Sense::click()));
            let res = ui.add(
                egui::Label::new(RichText::new(if ex.text.is_empty() { "(nothing)" } else { &ex.text }).monospace().strong())
                    .sense(egui::Sense::click()),
            );
            if src.clicked() || res.clicked() {
                *sample_idx = idx;
            }
            ui.end_row();
        }
    });
}
