//! "Document" view for structured data (JSON / YAML / TOML / XML).
//!
//! Instead of drawing every node the same way, each value is laid out by its
//! *shape*, the way good config/API viewers do:
//!
//! * an object's simple fields      → a key/value property sheet
//! * an object's nested objects     → headed sections, like a document
//! * a short list of scalars        → inline chips
//! * a list of similar objects      → a table
//! * an object whose entries share fields (e.g. `[dependencies]`) → a table keyed by name
//! * scalars styled by type: unquoted strings, monospace numbers, boolean
//!   pills, clickable URLs, colour swatches.

use super::tree::Value;
use crate::theme::{Palette, BOLD_FAMILY};
use egui::{vec2, Color32, FontFamily, FontId, RichText, Sense, Stroke};

/// Tables longer than this are cut off with a pointer to the full table view.
const MAX_INLINE_ROWS: usize = 200;
const MAX_CHIPS: usize = 40;

pub struct DocView<'a> {
    pub p: &'a Palette,
    pub clicked_link: Option<String>,
}

fn is_scalar(v: &Value) -> bool {
    matches!(v, Value::Null | Value::Bool(_) | Value::Num(_) | Value::Str(_))
}

/// Values that fit on one line of a property sheet.
fn is_inline(v: &Value) -> bool {
    match v {
        Value::Arr(a) => a.len() <= MAX_CHIPS && a.iter().all(is_scalar) && a.iter().map(|x| x.brief().chars().count()).sum::<usize>() < 400,
        Value::Obj(o) => o.is_empty(),
        _ => true,
    }
}

/// Rows of a list (or keyed map) of objects that share most of their fields → table columns.
fn table_columns<'v>(rows: impl Iterator<Item = &'v Value> + Clone) -> Option<Vec<String>> {
    let mut cols: Vec<String> = Vec::new();
    let mut n = 0usize;
    let mut field_count = 0usize;
    let mut scalar_rows = false;
    for r in rows.clone() {
        n += 1;
        match r {
            Value::Obj(o) => {
                for (k, v) in o {
                    // Deeply nested rows read badly as cells.
                    if !is_inline(v) && !matches!(v, Value::Obj(inner) if inner.iter().all(|(_, x)| is_scalar(x)) && inner.len() <= 3) {
                        return None;
                    }
                    field_count += 1;
                    if !cols.contains(k) {
                        cols.push(k.clone());
                    }
                }
            }
            // A bare scalar row (e.g. `serde = "1"` among `{ version, features }` rows) fills the 值 column.
            v if is_scalar(v) => {
                field_count += 1;
                scalar_rows = true;
            }
            _ => return None,
        }
    }
    let width = cols.len() + usize::from(scalar_rows);
    // Shared shape: the average row fills a fair share of the columns, and columns stay readable.
    if n < 2 || cols.is_empty() || width > 12 || (field_count as f32 / n as f32) < width as f32 * 0.34 {
        return None;
    }
    Some(cols)
}

impl DocView<'_> {
    pub fn show(&mut self, ui: &mut egui::Ui, v: &Value) {
        ui.spacing_mut().item_spacing.y = 8.0;
        match v {
            Value::Obj(o) => self.object(ui, o, 0),
            Value::Arr(a) => self.array(ui, a, 0),
            scalar => {
                self.scalar(ui, scalar);
            }
        }
    }

    fn heading(&self, ui: &mut egui::Ui, text: &str, depth: usize, detail: Option<String>) {
        let size = match depth {
            0 => 19.0,
            1 => 16.0,
            _ => 14.0,
        };
        ui.add_space(if depth == 0 { 10.0 } else { 4.0 });
        ui.horizontal(|ui| {
            ui.label(RichText::new(text).font(FontId::new(size, FontFamily::Name(BOLD_FAMILY.into()))).color(self.p.text));
            if let Some(d) = detail {
                ui.label(RichText::new(d).size(11.5).color(self.p.text_faint));
            }
        });
    }

    fn object(&mut self, ui: &mut egui::Ui, entries: &[(String, Value)], depth: usize) {
        if entries.is_empty() {
            ui.label(RichText::new("（空）").color(self.p.text_faint));
            return;
        }
        // A map whose values share fields reads best as a table keyed by name.
        if entries.len() >= 3 {
            if let Some(cols) = table_columns(entries.iter().map(|(_, v)| v)) {
                if entries.iter().any(|(_, v)| matches!(v, Value::Obj(_))) {
                    let rows: Vec<(String, &Value)> = entries.iter().map(|(k, v)| (k.clone(), v)).collect();
                    self.table(ui, &cols, &rows, true);
                    return;
                }
            }
        }
        let (simple, nested): (Vec<_>, Vec<_>) = entries.iter().partition(|(_, v)| is_inline(v));
        if !simple.is_empty() {
            self.property_sheet(ui, &simple);
        }
        for (k, v) in nested {
            let detail = match v {
                Value::Arr(a) => Some(format!("{} 项", a.len())),
                Value::Obj(o) => Some(format!("{} 个字段", o.len())),
                _ => None,
            };
            self.heading(ui, k, depth, detail);
            let indent = if depth >= 1 { 14.0 } else { 0.0 };
            ui.horizontal_top(|ui| {
                ui.add_space(indent);
                ui.vertical(|ui| {
                    let start = ui.cursor().top();
                    match v {
                        Value::Obj(o) => self.object(ui, o, depth + 1),
                        Value::Arr(a) => self.array(ui, a, depth + 1),
                        other => {
                            self.scalar(ui, other);
                        }
                    }
                    if depth >= 1 {
                        let bottom = ui.min_rect().bottom();
                        let x = ui.min_rect().left() - indent + 3.0;
                        ui.painter().vline(x, start..=bottom, Stroke::new(1.0_f32, self.p.border));
                    }
                });
            });
        }
    }

    fn array(&mut self, ui: &mut egui::Ui, items: &[Value], depth: usize) {
        if items.is_empty() {
            ui.label(RichText::new("（空列表）").color(self.p.text_faint));
            return;
        }
        if items.iter().all(is_scalar) {
            if is_inline(&Value::Arr(items.to_vec())) {
                self.chips(ui, items);
            } else {
                for (i, v) in items.iter().enumerate().take(MAX_INLINE_ROWS) {
                    ui.horizontal_top(|ui| {
                        ui.label(RichText::new(format!("{}.", i + 1)).color(self.p.text_faint).monospace());
                        self.scalar(ui, v);
                    });
                }
                self.more(ui, items.len());
            }
            return;
        }
        // A one-element list of objects (e.g. TOML `[[bin]]`) is just that object.
        if let [Value::Obj(o)] = items {
            return self.object(ui, o, depth);
        }
        if let Some(cols) = table_columns(items.iter()) {
            let rows: Vec<(String, &Value)> = items.iter().enumerate().map(|(i, v)| ((i + 1).to_string(), v)).collect();
            self.table(ui, &cols, &rows, false);
            return;
        }
        // Heterogeneous list: one card per item.
        for (i, v) in items.iter().enumerate().take(MAX_INLINE_ROWS) {
            self.card(ui, |this, ui| {
                ui.label(RichText::new(format!("#{}", i + 1)).size(11.5).color(this.p.text_faint));
                match v {
                    Value::Obj(o) => this.object(ui, o, depth + 1),
                    Value::Arr(a) => this.array(ui, a, depth + 1),
                    other => {
                        this.scalar(ui, other);
                    }
                }
            });
        }
        self.more(ui, items.len());
    }

    fn more(&self, ui: &mut egui::Ui, n: usize) {
        if n > MAX_INLINE_ROWS {
            ui.label(RichText::new(format!("…还有 {} 项（切换到「表格」或「树」查看全部）", n - MAX_INLINE_ROWS)).color(self.p.text_faint).size(12.0));
        }
    }

    fn card(&mut self, ui: &mut egui::Ui, add: impl FnOnce(&mut Self, &mut egui::Ui)) {
        egui::Frame::new()
            .fill(self.p.bg)
            .stroke(Stroke::new(1.0_f32, self.p.border))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                add(self, ui);
            });
    }

    fn property_sheet(&mut self, ui: &mut egui::Ui, rows: &[&(String, Value)]) {
        let key_w = rows.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(4).clamp(4, 28) as f32 * 7.6 + 16.0;
        self.card(ui, |this, ui| {
            egui::Grid::new(ui.next_auto_id()).num_columns(2).spacing(vec2(18.0, 9.0)).min_col_width(key_w.min(220.0)).show(ui, |ui| {
                for (k, v) in rows {
                    ui.label(RichText::new(k).color(this.p.text_dim).font(FontId::monospace(12.5)));
                    ui.horizontal_wrapped(|ui| {
                        ui.set_max_width(560.0);
                        match v {
                            Value::Arr(a) => this.chips(ui, a),
                            Value::Obj(_) => {
                                ui.label(RichText::new("（空）").color(this.p.text_faint));
                            }
                            s => {
                                this.scalar(ui, s);
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        });
    }

    fn chips(&mut self, ui: &mut egui::Ui, items: &[Value]) {
        if items.is_empty() {
            ui.label(RichText::new("（无）").color(self.p.text_faint));
            return;
        }
        // One galley with tag-style backgrounds: wraps between tags, never inside one.
        let mut job = egui::text::LayoutJob::default();
        for (i, v) in items.iter().enumerate() {
            if i > 0 {
                job.append("   ", 0.0, egui::TextFormat::simple(FontId::proportional(12.5), self.p.text));
            }
            let mut f = egui::TextFormat::simple(FontId::proportional(12.5), if matches!(v, Value::Num(_)) { self.p.number } else { self.p.text });
            f.background = self.p.panel_alt;
            f.expand_bg = 3.0;
            // Non-breaking spaces keep each tag on one line.
            job.append(&v.brief().replace(' ', "\u{a0}"), 0.0, f);
        }
        job.wrap.max_width = ui.available_width();
        ui.add(egui::Label::new(job).wrap());
    }

    /// A table cell: single line, long text elided with the full value on hover.
    fn cell(&mut self, ui: &mut egui::Ui, v: &Value) {
        match v {
            Value::Str(s) if s.chars().count() > 48 && !s.starts_with("http") => {
                let short: String = s.chars().take(46).collect();
                ui.add(egui::Label::new(RichText::new(format!("{short}…")).color(self.p.text)).extend()).on_hover_text(s);
            }
            Value::Str(s) if !s.starts_with("http") && !s.starts_with('#') => {
                ui.add(egui::Label::new(RichText::new(s).color(self.p.text)).extend());
            }
            Value::Arr(a) => {
                let mut job = egui::text::LayoutJob::default();
                for (i, x) in a.iter().enumerate() {
                    if i > 0 {
                        job.append("  ", 0.0, egui::TextFormat::simple(FontId::proportional(12.5), self.p.text));
                    }
                    let mut f = egui::TextFormat::simple(FontId::proportional(12.5), self.p.text);
                    f.background = self.p.panel_alt;
                    f.expand_bg = 2.0;
                    job.append(&x.brief(), 0.0, f);
                }
                ui.add(egui::Label::new(job).extend());
            }
            Value::Obj(o) => {
                let s: Vec<String> = o.iter().map(|(k, x)| format!("{k}: {}", x.brief())).collect();
                ui.add(egui::Label::new(RichText::new(s.join(", ")).size(12.5).color(self.p.text_dim)).extend());
            }
            other => {
                self.scalar(ui, other);
            }
        }
    }

    fn table(&mut self, ui: &mut egui::Ui, cols: &[String], rows: &[(String, &Value)], keyed: bool) {
        let p = self.p;
        egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).show(ui, |ui| {
            egui::Frame::new().stroke(Stroke::new(1.0_f32, p.border)).corner_radius(8.0).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                egui::Grid::new(ui.next_auto_id()).striped(true).spacing(vec2(20.0, 8.0)).show(ui, |ui| {
                    let head = |ui: &mut egui::Ui, t: &str| {
                        ui.label(RichText::new(t).font(FontId::new(12.5, FontFamily::Name(BOLD_FAMILY.into()))).color(p.text_dim));
                    };
                    head(ui, if keyed { "名称" } else { "#" });
                    let scalar_rows = rows.iter().any(|(_, v)| is_scalar(v));
                    if scalar_rows {
                        head(ui, "值");
                    }
                    for c in cols {
                        head(ui, c);
                    }
                    ui.end_row();
                    for (key, v) in rows.iter().take(MAX_INLINE_ROWS) {
                        if keyed {
                            ui.label(RichText::new(key).color(p.text).font(FontId::monospace(12.5)));
                        } else {
                            ui.label(RichText::new(key).color(p.text_faint).monospace());
                        }
                        if scalar_rows {
                            if is_scalar(v) {
                                self.cell(ui, v);
                            } else {
                                ui.label("");
                            }
                        }
                        for c in cols {
                            let cell = match v {
                                Value::Obj(o) => o.iter().find(|(k, _)| k == c).map(|(_, x)| x),
                                _ => None,
                            };
                            match cell {
                                Some(x) => self.cell(ui, x),
                                None => {
                                    ui.label("");
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
            });
        });
        self.more(ui, rows.len());
    }

    fn scalar(&mut self, ui: &mut egui::Ui, v: &Value) -> egui::Response {
        let p = self.p;
        match v {
            Value::Null => ui.label(RichText::new("—").color(p.text_faint)),
            Value::Bool(b) => {
                let (fg, bg) = if *b { (p.ok, p.ok.gamma_multiply(0.14)) } else { (p.text_dim, p.panel_alt) };
                egui::Frame::new()
                    .fill(bg)
                    .corner_radius(10.0)
                    .inner_margin(egui::Margin::symmetric(8, 1))
                    .show(ui, |ui| ui.label(RichText::new(if *b { "true" } else { "false" }).size(12.5).color(fg)))
                    .response
            }
            Value::Num(n) => ui.label(RichText::new(n).font(FontId::monospace(13.0)).color(p.number)),
            Value::Str(s) => {
                if s.is_empty() {
                    return ui.label(RichText::new("（空字符串）").color(p.text_faint).size(12.0));
                }
                if (s.starts_with("http://") || s.starts_with("https://")) && !s.contains(char::is_whitespace) {
                    let r = ui.add(egui::Label::new(RichText::new(s).color(p.link).underline()).sense(Sense::click()));
                    if r.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if r.clicked() {
                        self.clicked_link = Some(s.clone());
                    }
                    return r;
                }
                if let Some(c) = hex_color(s) {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        ui.painter().rect_filled(r, 3.0, c);
                        ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0_f32, p.border), egui::StrokeKind::Inside);
                        ui.label(RichText::new(s).monospace().color(p.text));
                    })
                    .response
                } else {
                    ui.add(egui::Label::new(RichText::new(s).color(p.text)).wrap())
                }
            }
            other => ui.label(other.brief()),
        }
    }
}

fn hex_color(s: &str) -> Option<Color32> {
    let h = s.strip_prefix('#')?;
    let v = u32::from_str_radix(h, 16).ok()?;
    match h.len() {
        6 => Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)),
        3 => {
            let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
            Some(Color32::from_rgb((r * 17) as u8, (g * 17) as u8, (b * 17) as u8))
        }
        _ => None,
    }
}
