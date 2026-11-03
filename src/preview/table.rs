//! Virtualised grid for tabular data (CSV, TSV, spreadsheets, JSON arrays).
//! Only visible rows are painted, so million-row files scroll smoothly.

use crate::theme::Palette;
use egui::{pos2, vec2, Align2, FontId, Rect, Sense, Stroke};

pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub widths: Vec<f32>,
    pub sheets: Vec<String>,
    pub sheet: usize,
}

impl Table {
    pub fn new(header: Vec<String>, rows: Vec<Vec<String>>) -> Self {
        let ncols = rows.iter().take(1000).map(|r| r.len()).max().unwrap_or(0).max(header.len());
        let mut chars = vec![3usize; ncols];
        for (j, h) in header.iter().enumerate() {
            chars[j] = chars[j].max(display_len(h));
        }
        for r in rows.iter().take(500) {
            for (j, c) in r.iter().enumerate() {
                chars[j] = chars[j].max(display_len(c));
            }
        }
        let widths = chars.iter().map(|&n| (n as f32 * 7.6 + 24.0).clamp(56.0, 380.0)).collect();
        Self { header, rows, widths, sheets: Vec::new(), sheet: 0 }
    }
}

fn display_len(s: &str) -> usize {
    s.chars().take(60).map(|c| if c.is_ascii() { 1 } else { 2 }).sum()
}

pub fn parse_delimited(text: &str, delimiter: u8) -> Result<Table, String> {
    let mut rdr = csv::ReaderBuilder::new().delimiter(delimiter).has_headers(false).flexible(true).from_reader(text.as_bytes());
    let mut rows: Vec<Vec<String>> = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| format!("解析失败：{e}"))?;
        rows.push(rec.iter().map(str::to_owned).collect());
    }
    let header = if rows.is_empty() { Vec::new() } else { rows.remove(0) };
    Ok(Table::new(header, rows))
}

pub fn parse_spreadsheet(bytes: &[u8], sheet: usize) -> Result<Table, String> {
    use calamine::{Data, Reader};
    let mut wb = calamine::open_workbook_auto_from_rs(std::io::Cursor::new(bytes.to_vec())).map_err(|e| format!("无法读取表格：{e}"))?;
    let names = wb.sheet_names().to_vec();
    let Some(name) = names.get(sheet).or(names.first()).cloned() else { return Err("工作簿没有工作表".into()) };
    let range = wb.worksheet_range(&name).map_err(|e| format!("无法读取工作表 {name}：{e}"))?;
    let mut rows: Vec<Vec<String>> = range
        .rows()
        .map(|r| {
            r.iter()
                .map(|c| match c {
                    Data::Empty => String::new(),
                    Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", *f as i64),
                    other => other.to_string(),
                })
                .collect()
        })
        .collect();
    let header = if rows.is_empty() { Vec::new() } else { rows.remove(0) };
    let mut t = Table::new(header, rows);
    t.sheets = names;
    t.sheet = sheet.min(t.sheets.len().saturating_sub(1));
    Ok(t)
}

/// Returns the sheet index the user switched to, if any.
pub fn show(ui: &mut egui::Ui, t: &Table, p: &Palette, id: egui::Id) -> Option<usize> {
    let mut switch = None;
    if t.sheets.len() > 1 {
        ui.horizontal(|ui| {
            for (i, s) in t.sheets.iter().enumerate() {
                if ui.selectable_label(i == t.sheet, s).clicked() && i != t.sheet {
                    switch = Some(i);
                }
            }
        });
        ui.add_space(4.0);
    }
    let row_h = 26.0;
    let gutter = (t.rows.len().max(1) as f32).log10().floor() * 8.0 + 30.0;
    let total_w: f32 = gutter + t.widths.iter().sum::<f32>();
    let font = FontId::proportional(13.0);
    let mono = FontId::monospace(12.0);
    ui.label(egui::RichText::new(format!("{} 行 × {} 列", t.rows.len(), t.widths.len())).color(p.text_dim).size(12.0));
    egui::ScrollArea::horizontal().id_salt(id.with("h")).auto_shrink([false, false]).show(ui, |ui| {
        ui.set_min_width(total_w);
        // Sticky header.
        let (hr, _) = ui.allocate_exact_size(vec2(total_w, row_h + 2.0), Sense::hover());
        let painter = ui.painter_at(hr);
        painter.rect_filled(hr, 6.0, p.panel_alt);
        let mut x = hr.left() + gutter;
        for (j, w) in t.widths.iter().enumerate() {
            let cell = Rect::from_min_size(pos2(x, hr.top()), vec2(*w, row_h + 2.0));
            let text = t.header.get(j).map(String::as_str).unwrap_or("");
            let shown = if text.is_empty() { column_name(j) } else { text.to_owned() };
            painter.with_clip_rect(cell.shrink2(vec2(8.0, 0.0))).text(cell.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, shown, FontId::new(13.0, egui::FontFamily::Name(crate::theme::BOLD_FAMILY.into())), p.text);
            x += w;
            painter.vline(x, hr.y_range(), Stroke::new(1.0_f32, p.border));
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().id_salt(id.with("v")).auto_shrink([false, false]).show_rows(ui, row_h, t.rows.len(), |ui, range| {
            let start = range.start;
            let (rect, _) = ui.allocate_exact_size(vec2(total_w, row_h * range.len() as f32), Sense::hover());
            let painter = ui.painter_at(rect);
            for (k, row) in t.rows[range].iter().enumerate() {
                let i = start + k;
                let y = rect.top() + k as f32 * row_h;
                let row_rect = Rect::from_min_size(pos2(rect.left(), y), vec2(total_w, row_h));
                if i % 2 == 1 {
                    painter.rect_filled(row_rect, 0.0, p.current_line);
                }
                painter.text(pos2(rect.left() + gutter - 10.0, y + row_h / 2.0), Align2::RIGHT_CENTER, (i + 1).to_string(), mono.clone(), p.text_faint);
                let mut x = rect.left() + gutter;
                for (j, w) in t.widths.iter().enumerate() {
                    if let Some(v) = row.get(j) {
                        let cell = Rect::from_min_size(pos2(x, y), vec2(*w, row_h));
                        let numeric = !v.is_empty() && v.trim().parse::<f64>().is_ok();
                        let (pos, align) = if numeric { (cell.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER) } else { (cell.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER) };
                        let shown: String = v.chars().take(200).map(|c| if c == '\n' { '⏎' } else { c }).collect();
                        painter.with_clip_rect(cell.shrink2(vec2(6.0, 0.0))).text(pos, align, shown, font.clone(), if numeric { p.number } else { p.text });
                    }
                    x += w;
                }
            }
        });
    });
    switch
}

fn column_name(mut j: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (j % 26) as u8) as char);
        if j < 26 {
            break;
        }
        j = j / 26 - 1;
    }
    s
}
