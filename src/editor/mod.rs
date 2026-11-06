//! The source editor: a virtualised code view.
//!
//! Only the rows inside the viewport are laid out and painted, so a 100k-line
//! file costs the same per frame as a 100-line one. Text is stored as a plain
//! `String`; a line index and the visual-row table are rebuilt when the
//! document revision, wrap width or font size changes.

pub mod state;
pub mod text;

use crate::document::Document;
use crate::highlight::{self, HighlightService, Highlighted, LineSpans};
use crate::theme::{Palette, ThemeKind};
use egui::text::{CCursor, LayoutJob, LayoutSection, TextFormat};
use egui::{pos2, vec2, Color32, Event, FontId, Galley, Id, Key, Modifiers, Rect, Sense, Stroke};
pub use state::{EditKind, EditState, Selection};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

#[derive(Default, Clone)]
pub struct FindState {
    pub open: bool,
    pub query: String,
    pub replace: String,
    pub case_sensitive: bool,
    pub show_replace: bool,
    /// Byte ranges of matches in the active document.
    pub matches: Vec<(usize, usize)>,
    pub current: usize,
    computed_for: Option<(u64, u64, String, bool)>,
}

impl FindState {
    pub fn recompute(&mut self, doc: &Document) {
        let key = (doc.id, doc.revision, self.query.clone(), self.case_sensitive);
        if self.computed_for.as_ref() == Some(&key) {
            return;
        }
        self.computed_for = Some(key);
        self.matches.clear();
        if self.query.is_empty() {
            return;
        }
        // ASCII case folding keeps byte offsets identical to the original text.
        let (hay, needle) = if self.case_sensitive {
            (std::borrow::Cow::Borrowed(doc.text.as_str()), std::borrow::Cow::Borrowed(self.query.as_str()))
        } else {
            (std::borrow::Cow::Owned(doc.text.to_ascii_lowercase()), std::borrow::Cow::Owned(self.query.to_ascii_lowercase()))
        };
        for (b, m) in hay.match_indices(needle.as_ref()) {
            self.matches.push((b, b + m.len()));
            if self.matches.len() >= 100_000 {
                break;
            }
        }
        if self.current >= self.matches.len() {
            self.current = 0;
        }
    }
}

pub struct EditorSettings {
    pub font_size: f32,
    pub theme: ThemeKind,
}

pub struct EditorOutput {
    pub scroll_fraction: f32,
    pub scrolled: bool,
}

#[derive(Clone, Copy)]
struct Row {
    line: u32,
    start: usize,
    end: usize,
}

/// Line index and visual rows for one document, rebuilt lazily.
#[derive(Default)]
pub struct ViewCache {
    key: Option<(u64, u32, u32, bool)>,
    starts: Vec<usize>,
    rows: Vec<Row>,
    /// First row of every line.
    line_row: Vec<u32>,
    max_cols: usize,
    char_w: HashMap<char, f32>,
    last_scroll: f32,
}

impl ViewCache {
    fn rebuild(&mut self, ui: &egui::Ui, doc: &Document, font: &FontId, wrap_width: Option<f32>) {
        let key = (doc.revision, font.size.to_bits(), wrap_width.unwrap_or(0.0).to_bits(), wrap_width.is_some());
        if self.key == Some(key) && !self.starts.is_empty() {
            return;
        }
        if self.key.is_some_and(|k| k.1 != key.1) {
            self.char_w.clear();
        }
        self.key = Some(key);
        let text = &doc.text;
        self.starts = text::line_starts(text);
        self.rows.clear();
        self.line_row.clear();
        self.max_cols = 0;
        let n = self.starts.len();
        for line in 0..n {
            let r = text::line_range(text, &self.starts, line);
            self.line_row.push(self.rows.len() as u32);
            let s = &text[r.clone()];
            match wrap_width {
                None => {
                    let cols = if s.is_ascii() { s.len() } else { s.chars().map(|c| if (c as u32) < 0x1100 { 1 } else { 2 }).sum() };
                    self.max_cols = self.max_cols.max(cols);
                    self.rows.push(Row { line: line as u32, start: r.start, end: r.end });
                }
                Some(w) => {
                    // Greedy wrap by measured glyph advances, preferring to break after a space.
                    let mut row_start = r.start;
                    let mut x = 0.0;
                    let mut last_space: Option<usize> = None;
                    for (b, c) in s.char_indices() {
                        let cw = *self.char_w.entry(c).or_insert_with(|| ui.fonts_mut(|f| f.glyph_width(font, c)));
                        let abs = r.start + b;
                        if x + cw > w && abs > row_start {
                            let brk = match last_space {
                                Some(sp) if sp > row_start => sp,
                                _ => abs,
                            };
                            self.rows.push(Row { line: line as u32, start: row_start, end: brk });
                            x = text[brk..abs].chars().map(|c| *self.char_w.get(&c).unwrap_or(&cw)).sum();
                            row_start = brk;
                            last_space = None;
                        }
                        x += cw;
                        if c == ' ' {
                            last_space = Some(abs + 1);
                        }
                    }
                    self.rows.push(Row { line: line as u32, start: row_start, end: r.end });
                }
            }
        }
    }

    fn row_of(&self, byte: usize) -> usize {
        let line = text::line_of(&self.starts, byte);
        let first = self.line_row[line] as usize;
        let last = self.line_row.get(line + 1).map(|&r| r as usize).unwrap_or(self.rows.len());
        let mut r = first;
        while r + 1 < last && self.rows[r + 1].start <= byte {
            r += 1;
        }
        r
    }
}

pub fn editor_id(doc: &Document) -> Id {
    Id::new(("cryocode-editor", doc.id))
}

fn line_spans<'a>(hl: Option<&'a Highlighted>, line: usize, line_text_with_nl: &str, delta: isize) -> Option<&'a LineSpans> {
    let lines = &hl?.lines;
    let h = highlight::line_hash(line_text_with_nl);
    lines.get(line).filter(|l| l.hash == h).or_else(|| {
        let j = line as isize - delta;
        (j >= 0).then(|| lines.get(j as usize)).flatten().filter(|l| l.hash == h)
    })
}

/// Layout job for one visual row, coloured from the line's highlight spans.
fn row_job(doc: &Document, row: Row, line_start: usize, spans: Option<(&LineSpans, &[highlight::SpanStyle])>, font: &FontId, fallback: Color32) -> LayoutJob {
    let text = &doc.text[row.start..row.end];
    let mut job = LayoutJob { text: text.to_owned(), ..Default::default() };
    job.wrap.max_width = f32::INFINITY;
    let off = row.start - line_start;
    let mut covered = 0usize;
    let push = |job: &mut LayoutJob, a: usize, b: usize, color: Color32, italic: bool| {
        let mut f = TextFormat::simple(font.clone(), color);
        f.italics = italic;
        job.sections.push(LayoutSection { leading_space: 0.0, byte_range: a..b, format: f });
    };
    if let Some((sp, styles)) = spans {
        for t in sp.tokens.iter() {
            let Some(style) = styles.get(t.style as usize) else { continue };
            let (s, e) = ((t.start as usize).saturating_sub(off), (t.end as usize).saturating_sub(off).min(text.len()));
            if e <= s || s >= text.len() || !text.is_char_boundary(s) || !text.is_char_boundary(e) {
                continue;
            }
            if s > covered {
                push(&mut job, covered, s, fallback, false);
            }
            push(&mut job, s.max(covered), e, style.color, style.italic);
            covered = e;
        }
    }
    if covered < text.len() {
        push(&mut job, covered, text.len(), fallback, false);
    }
    job
}

struct Geometry<'a> {
    doc: &'a mut Document,
    cache: &'a ViewCache,
    hl: Option<&'a Highlighted>,
    delta: isize,
    font: FontId,
    fallback: Color32,
    galleys: HashMap<usize, Arc<Galley>>,
}

impl Geometry<'_> {
    fn galley(&mut self, ui: &egui::Ui, row_idx: usize) -> Arc<Galley> {
        if let Some(g) = self.galleys.get(&row_idx) {
            return g.clone();
        }
        let row = self.cache.rows[row_idx];
        let line = row.line as usize;
        let ls = self.cache.starts[line];
        let le = self.cache.starts.get(line + 1).copied().unwrap_or(self.doc.text.len());
        let spans = line_spans(self.hl, line, &self.doc.text[ls..le], self.delta).zip(self.hl.map(|h| h.styles.as_slice()));
        let job = row_job(self.doc, row, ls, spans, &self.font, self.fallback);
        let g = ui.fonts_mut(|f| f.layout_job(job));
        self.galleys.insert(row_idx, g.clone());
        g
    }

    fn x_of(&mut self, ui: &egui::Ui, row_idx: usize, byte: usize) -> f32 {
        let row = self.cache.rows[row_idx];
        let byte = byte.clamp(row.start, row.end);
        let n = self.doc.text[row.start..byte].chars().count();
        self.galley(ui, row_idx).pos_from_cursor(CCursor::new(n)).min.x
    }

    fn byte_at(&mut self, ui: &egui::Ui, row_idx: usize, x: f32) -> usize {
        let row = self.cache.rows[row_idx];
        let g = self.galley(ui, row_idx);
        let n = g.cursor_from_pos(vec2(x, g.size().y * 0.5)).index;
        text::advance_chars_in_line(&self.doc.text, row.start, n).min(row.end)
    }
}

const MARGIN_X: f32 = 8.0;
const MARGIN_TOP: f32 = 8.0;

pub fn show(
    ui: &mut egui::Ui,
    doc: &mut Document,
    cache: &mut ViewCache,
    hl: &HighlightService,
    settings: &EditorSettings,
    find: &FindState,
    sync: Option<f32>,
) -> EditorOutput {
    let p = Palette::get(settings.theme);
    let font = FontId::monospace(settings.font_size);
    let id = editor_id(doc);
    let row_h = (settings.font_size * 1.6).round();
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let digits = ((doc.line_count().max(100) as f32).log10().floor() as usize) + 1;
    let digit_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
    let gutter_w = digits as f32 * digit_w + 30.0;
    let gutter = Rect::from_min_size(full.min, vec2(gutter_w, full.height()));
    let content = Rect::from_min_max(pos2(full.min.x + gutter_w, full.min.y), full.max);

    if let Some((a, b)) = doc.goto.take() {
        let len = doc.text.len();
        doc.edit.sel = Selection { anchor: a.min(len), head: b.min(len) };
        doc.edit.scroll_to_caret = true;
        // Keep focus where it is when something else holds it (e.g. Enter in the find field
        // must keep searching, not type into the document).
        if ui.memory(|m| m.focused().is_none()) {
            ui.memory_mut(|m| m.request_focus(id));
        }
    }

    let focused = ui.memory(|m| m.has_focus(id));
    if focused {
        ui.memory_mut(|m| m.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false }));
    }

    let wrap_width = doc.wrap.then(|| (content.width() - 2.0 * MARGIN_X - 16.0).max(80.0));
    cache.rebuild(ui, doc, &font, wrap_width);

    // Keyboard input first, so this frame paints the result.
    if focused {
        let page_rows = ((content.height() / row_h) as usize).saturating_sub(2).max(1);
        let hl_now = hl.latest(doc.id);
        handle_keyboard(ui, doc, cache, hl_now.as_deref(), &font, p.text, row_h, page_rows, &settings.font_size);
        cache.rebuild(ui, doc, &font, wrap_width);
    }

    let hl_result = hl.latest(doc.id);
    let delta = 0;
    let total_rows = cache.rows.len();
    let content_w = if doc.wrap { content.width() - 12.0 } else { (cache.max_cols as f32 * digit_w * 1.02 + 2.0 * MARGIN_X + 80.0).max(content.width() - 12.0) };
    let content_h = total_rows as f32 * row_h + MARGIN_TOP + content.height() * 0.5;

    let mut gutter_rows: Vec<(usize, f32, bool)> = Vec::new();
    let diagnostics = doc.diagnostics.clone();
    let mut diag_lines: std::collections::HashSet<usize> = Default::default();
    let mut out = EditorOutput { scroll_fraction: doc.scroll_fraction, scrolled: false };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content));
    let caret_line = text::line_of(&cache.starts, doc.edit.sel.head);
    let mut visible_last_line = doc.hl_want;
    let mut area = egui::ScrollArea::both().id_salt(("editor-scroll", doc.id)).auto_shrink([false, false]);
    if let Some(f) = sync {
        area = area.vertical_scroll_offset((content_h - content.height()).max(0.0) * f);
    }
    let scroll = area.show_viewport(&mut child, |ui, viewport| {
        let (rect, _) = ui.allocate_exact_size(vec2(content_w, content_h), Sense::hover());
        let origin = rect.min + vec2(MARGIN_X, MARGIN_TOP);
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        let mut geo = Geometry { doc, cache, hl: hl_result.as_deref(), delta, font: font.clone(), fallback: p.text, galleys: HashMap::new() };

        // Mouse.
        let pointer_row = |y: f32| -> usize { (((y - origin.y) / row_h).floor().max(0.0) as usize).min(total_rows.saturating_sub(1)) };
        let mut new_sel: Option<Selection> = None;
        if let Some(pos) = resp.interact_pointer_pos() {
            let r = pointer_row(pos.y);
            let b = geo.byte_at(ui, r, pos.x - origin.x);
            let shift = ui.input(|i| i.modifiers.shift);
            let pressed = ui.input(|i| i.pointer.primary_pressed());
            let sel = geo.doc.edit.sel;
            if resp.triple_clicked() {
                let line = geo.cache.rows[r].line as usize;
                let lr = text::line_range(&geo.doc.text, &geo.cache.starts, line);
                new_sel = Some(Selection { anchor: lr.start, head: (lr.end + 1).min(geo.doc.text.len()) });
            } else if resp.double_clicked() {
                let w = text::word_at(&geo.doc.text, b);
                new_sel = Some(Selection { anchor: w.start, head: w.end });
            } else if pressed && resp.hovered() {
                new_sel = Some(if shift { Selection { anchor: sel.anchor, head: b } } else { Selection::caret(b) });
                ui.memory_mut(|m| m.request_focus(id));
            } else if resp.dragged() {
                new_sel = Some(Selection { anchor: sel.anchor, head: b });
                geo.galleys.clear();
            }
        }

        // Paint visible rows.
        let first = ((viewport.min.y - MARGIN_TOP) / row_h).floor().max(0.0) as usize;
        let last = (((viewport.max.y - MARGIN_TOP) / row_h).ceil() as usize + 1).min(total_rows);
        let painter = ui.painter().clone();
        let sel = new_sel.unwrap_or(geo.doc.edit.sel);
        let sel_range = sel.range();
        let find_start = find.matches.partition_point(|m| m.1 < geo.cache.rows.get(first).map_or(0, |r| r.start));
        let caret_row = geo.cache.row_of(sel.head.min(geo.doc.text.len()));
        for ri in first..last {
            let row = geo.cache.rows[ri];
            let y = origin.y + ri as f32 * row_h;
            let row_rect = Rect::from_min_max(pos2(viewport.min.x + rect.min.x, y), pos2(viewport.max.x + rect.min.x, y + row_h));
            if row.line as usize == caret_line && sel.is_empty() {
                painter.rect_filled(row_rect, 0.0, p.current_line);
            }
            // Find matches.
            if find.open {
                for (k, &(a, b)) in find.matches.iter().enumerate().skip(find_start) {
                    if a > row.end {
                        break;
                    }
                    if b < row.start {
                        continue;
                    }
                    let x0 = geo.x_of(ui, ri, a.max(row.start));
                    let x1 = geo.x_of(ui, ri, b.min(row.end));
                    let r = Rect::from_min_max(pos2(origin.x + x0, y + 2.0), pos2(origin.x + x1.max(x0 + 2.0), y + row_h - 2.0));
                    painter.rect_filled(r, 3.0, if k == find.current { p.accent.gamma_multiply(0.45) } else { p.accent_soft });
                }
            }
            // Selection.
            if !sel.is_empty() && sel_range.start <= row.end + 1 && sel_range.end > row.start {
                let x0 = geo.x_of(ui, ri, sel_range.start.max(row.start));
                let x1 = if sel_range.end > row.end { geo.x_of(ui, ri, row.end) + digit_w * 0.6 } else { geo.x_of(ui, ri, sel_range.end) };
                painter.rect_filled(Rect::from_min_max(pos2(origin.x + x0, y), pos2(origin.x + x1, y + row_h)), 2.0, p.selection);
            }
            let g = geo.galley(ui, ri);
            painter.galley(pos2(origin.x, y + (row_h - g.size().y) * 0.5), g, p.text);
            // Syntax problems: a red wavy underline under the offending character.
            for (off, msg) in &diagnostics {
                if *off < row.start || *off > row.end || (*off == row.end && row.end != row.start && geo.doc.text.as_bytes().get(row.end) != Some(&b'\n')) {
                    continue;
                }
                let x0 = origin.x + geo.x_of(ui, ri, *off);
                let next = text::next_char(&geo.doc.text, *off).min(row.end);
                let x1 = (origin.x + geo.x_of(ui, ri, next)).max(x0 + digit_w);
                let base = y + row_h - 4.0;
                let mut pts = Vec::new();
                let mut x = x0;
                let mut up = true;
                while x <= x1 {
                    pts.push(pos2(x, if up { base - 1.5 } else { base + 1.5 }));
                    x += 2.5;
                    up = !up;
                }
                painter.add(egui::Shape::line(pts, Stroke::new(1.3_f32, p.warn)));
                diag_lines.insert(row.line as usize);
                let hot = Rect::from_min_max(pos2(x0 - 2.0, y), pos2(x1 + 2.0, y + row_h));
                if resp.hover_pos().is_some_and(|hp| hot.contains(hp)) {
                    resp.show_tooltip_text(msg.as_str());
                }
            }
            let first_of_line = geo.cache.line_row[row.line as usize] as usize == ri;
            if first_of_line {
                gutter_rows.push((row.line as usize + 1, y, row.line as usize == caret_line));
            }
        }

        visible_last_line = geo.cache.rows.get(last.saturating_sub(1)).map_or(0, |r| r.line as usize);

        // Caret + IME preedit.
        let cx = geo.x_of(ui, caret_row, sel.head);
        let cy = origin.y + caret_row as f32 * row_h;
        let caret = Rect::from_min_size(pos2(origin.x + cx - 1.0, cy + 2.0), vec2(2.0, row_h - 4.0));
        if focused {
            if let Some(pre) = &geo.doc.edit.preedit {
                let g = ui.fonts_mut(|f| f.layout_no_wrap(pre.clone(), font.clone(), p.text));
                let r = Rect::from_min_size(pos2(origin.x + cx, cy), vec2(g.size().x, row_h));
                painter.rect_filled(r, 2.0, p.panel_alt);
                painter.galley(pos2(r.left(), cy + (row_h - g.size().y) * 0.5), g, p.text);
                painter.hline(r.x_range(), r.bottom() - 3.0, Stroke::new(1.0_f32, p.accent));
            } else {
                let since = geo.doc.edit.last_activity.map_or(10.0, |t| t.elapsed().as_secs_f32());
                if since < 0.6 || (since * 1.6).fract() < 0.6 {
                    painter.rect_filled(caret, 1.0, p.accent);
                }
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
            }
            ui.ctx().output_mut(|o| o.ime = Some(egui::output::IMEOutput { rect: content, cursor_rect: caret }));
        }
        if geo.doc.edit.scroll_to_caret {
            ui.scroll_to_rect(caret.expand2(vec2(40.0, row_h * 2.0)), None);
        }
        if let Some(s) = new_sel {
            if s != geo.doc.edit.sel {
                geo.doc.edit.sel = s;
                geo.doc.edit.preferred_x = None;
                geo.doc.edit.last_activity = Some(Instant::now());
                if resp.dragged() {
                    ui.scroll_to_rect(caret, None);
                }
            }
        }
        if resp.clicked() || resp.drag_started() {
            ui.memory_mut(|m| m.request_focus(id));
        }
    });
    doc.edit.scroll_to_caret = false;
    doc.hl_want = visible_last_line;

    let max_scroll = (scroll.content_size.y - scroll.inner_rect.height()).max(1.0);
    let frac = (scroll.state.offset.y / max_scroll).clamp(0.0, 1.0);
    out.scrolled = (frac - cache.last_scroll).abs() > 0.0005;
    cache.last_scroll = frac;
    out.scroll_fraction = frac;
    doc.scroll_fraction = frac;

    // Status bar info.
    let head = doc.edit.sel.head.min(doc.text.len());
    let ls = cache.starts[caret_line.min(cache.starts.len() - 1)];
    doc.cursor = (caret_line + 1, doc.text[ls.min(head)..head].chars().count() + 1);
    let r = doc.edit.sel.range();
    doc.selection_chars = if r.len() > 4_000_000 { r.len() } else { doc.text[r].chars().count() };

    // Gutter.
    let painter = ui.painter_at(gutter);
    painter.rect_filled(gutter, 0.0, p.bg);
    let num_font = FontId::monospace(settings.font_size * 0.86);
    for (n, y, active) in gutter_rows {
        if diag_lines.contains(&(n - 1)) {
            painter.circle_filled(pos2(gutter.left() + 9.0, y + row_h * 0.5), 3.5, p.warn);
        }
        painter.text(pos2(gutter.right() - 14.0, y + row_h * 0.5), egui::Align2::RIGHT_CENTER, n.to_string(), num_font.clone(), if active { p.text_dim } else { p.text_faint });
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn handle_keyboard(
    ui: &egui::Ui,
    doc: &mut Document,
    cache: &ViewCache,
    hl: Option<&Highlighted>,
    font: &FontId,
    fallback: Color32,
    _row_h: f32,
    page_rows: usize,
    _font_size: &f32,
) {
    let filter = egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false };
    let events = ui.input(|i| i.filtered_events(&filter));
    if events.is_empty() {
        return;
    }
    let comment = comment_token(&doc.syntax);
    let mut rows_dirty = false;
    let mut local_cache: Option<ViewCache> = None;
    for ev in events {
        // After an edit, vertical moves need fresh rows; rebuild a private copy on demand.
        let vc: &ViewCache = match (&local_cache, rows_dirty) {
            (Some(c), false) => c,
            (_, true) => {
                let mut c = ViewCache::default();
                c.rebuild(ui, doc, font, None);
                rows_dirty = false;
                local_cache = Some(c);
                local_cache.as_ref().unwrap()
            }
            (None, false) => cache,
        };
        let before_rev = doc.revision;
        match ev {
            Event::Text(t) => {
                if doc.edit.preedit.is_none() {
                    insert_typed(doc, &t);
                }
            }
            Event::Paste(t) => doc.insert_text(&t.replace("\r\n", "\n")),
            Event::Copy => {
                let s = if doc.edit.sel.is_empty() { current_line_with_nl(doc) } else { doc.selected_text().to_owned() };
                ui.ctx().copy_text(s);
            }
            Event::Cut => {
                if doc.edit.sel.is_empty() {
                    let s = current_line_with_nl(doc);
                    ui.ctx().copy_text(s);
                    let head = doc.edit.sel.head;
                    let start = doc.text[..head].rfind('\n').map(|i| i + 1).unwrap_or(0);
                    let end = doc.text[head..].find('\n').map(|i| head + i + 1).unwrap_or(doc.text.len());
                    doc.replace(start..end, "", EditKind::Other);
                } else {
                    ui.ctx().copy_text(doc.selected_text().to_owned());
                    let r = doc.edit.sel.range();
                    doc.replace(r, "", EditKind::Other);
                }
            }
            Event::Ime(ime) => match ime {
                egui::ImeEvent::Preedit(s) => doc.edit.preedit = (!s.is_empty()).then_some(s),
                egui::ImeEvent::Commit(s) => {
                    doc.edit.preedit = None;
                    if !s.is_empty() {
                        doc.insert_text(&s);
                    }
                }
                egui::ImeEvent::Disabled => doc.edit.preedit = None,
                egui::ImeEvent::Enabled => {}
            },
            Event::Key { key, pressed: true, modifiers, .. } => {
                if doc.edit.preedit.is_some() {
                    continue;
                }
                let mut geo = Geometry { doc, cache: vc, hl, delta: 0, font: font.clone(), fallback, galleys: HashMap::new() };
                key_command(ui, &mut geo, key, modifiers, page_rows, comment);
            }
            _ => {}
        }
        if doc.revision != before_rev {
            rows_dirty = true;
        }
        doc.edit.last_activity = Some(Instant::now());
    }
}

fn current_line_with_nl(doc: &Document) -> String {
    let head = doc.edit.sel.head;
    let start = doc.text[..head].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = doc.text[head..].find('\n').map(|i| head + i + 1).unwrap_or(doc.text.len());
    let mut s = doc.text[start..end].to_owned();
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Typing with bracket/quote auto-pairing and over-typing of the closer.
fn insert_typed(doc: &mut Document, t: &str) {
    let sel = doc.edit.sel;
    let next = doc.text[sel.head..].chars().next();
    let mut chars = t.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if sel.is_empty() && next == Some(c) && matches!(c, ')' | ']' | '}' | '"' | '\'' | '`') {
            doc.edit.sel = Selection::caret(sel.head + c.len_utf8());
            return;
        }
        let close = match c {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' | '\'' | '`' => {
                let prev = doc.text[..sel.head].chars().next_back();
                (!prev.is_some_and(|p| p.is_alphanumeric()) && !next.is_some_and(|n| n.is_alphanumeric())).then_some(c)
            }
            _ => None,
        };
        if let Some(close) = close {
            if !sel.is_empty() {
                let inner = doc.selected_text().to_owned();
                let r = sel.range();
                doc.replace(r.clone(), &format!("{c}{inner}{close}"), EditKind::Other);
                doc.edit.sel = Selection { anchor: r.start + 1, head: r.start + 1 + inner.len() };
                return;
            }
            if next.is_none_or(|n| n.is_whitespace() || matches!(n, ')' | ']' | '}' | ',' | ';')) {
                doc.replace(sel.range(), &format!("{c}{close}"), EditKind::Typing);
                doc.edit.sel = Selection::caret(sel.head + c.len_utf8());
                return;
            }
        }
    }
    doc.insert_text(t);
}

fn key_command(ui: &egui::Ui, geo: &mut Geometry, key: Key, m: Modifiers, page_rows: usize, comment: Option<&str>) {
    let cmd = m.command;
    let shift = m.shift;
    let alt = m.alt;
    let len = geo.doc.text.len();
    let sel = geo.doc.edit.sel;
    let set_head = |geo: &mut Geometry, head: usize, keep_x: bool| {
        let head = head.min(geo.doc.text.len());
        let anchor = if shift { geo.doc.edit.sel.anchor } else { head };
        geo.doc.edit.sel = Selection { anchor, head };
        if !keep_x {
            geo.doc.edit.preferred_x = None;
        }
        geo.doc.edit.scroll_to_caret = true;
    };
    match key {
        Key::ArrowLeft | Key::ArrowRight if !shift && !sel.is_empty() && !cmd && !alt => {
            let r = sel.range();
            set_head(geo, if key == Key::ArrowLeft { r.start } else { r.end }, false);
        }
        Key::ArrowLeft => {
            let text = geo.doc.text.as_str();
            let h = if cmd {
                smart_home(text, sel.head)
            } else if alt {
                text::word_left(text, sel.head)
            } else {
                text::prev_char(text, sel.head)
            };
            set_head(geo, h, false);
        }
        Key::ArrowRight => {
            let text = geo.doc.text.as_str();
            let h = if cmd {
                text[sel.head..].find('\n').map(|i| sel.head + i).unwrap_or(len)
            } else if alt {
                text::word_right(text, sel.head)
            } else {
                text::next_char(text, sel.head)
            };
            set_head(geo, h, false);
        }
        Key::ArrowUp | Key::ArrowDown if alt && !cmd => geo.doc.move_lines(key == Key::ArrowUp),
        Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown => {
            if cmd {
                set_head(geo, if key == Key::ArrowUp || key == Key::PageUp { 0 } else { len }, false);
                return;
            }
            let rows = geo.cache.rows.len();
            let cur = geo.cache.row_of(sel.head);
            let step = if matches!(key, Key::PageUp | Key::PageDown) { page_rows } else { 1 };
            let target = if matches!(key, Key::ArrowUp | Key::PageUp) {
                if cur == 0 {
                    return set_head(geo, 0, false);
                }
                cur.saturating_sub(step)
            } else {
                if cur + 1 >= rows {
                    return set_head(geo, len, false);
                }
                (cur + step).min(rows - 1)
            };
            let x = match geo.doc.edit.preferred_x {
                Some(x) => x,
                None => geo.x_of(ui, cur, sel.head),
            };
            let b = geo.byte_at(ui, target, x);
            set_head(geo, b, true);
            geo.doc.edit.preferred_x = Some(x);
        }
        Key::Home => {
            let h = smart_home(&geo.doc.text, sel.head);
            set_head(geo, h, false);
        }
        Key::End => {
            let h = geo.doc.text[sel.head..].find('\n').map(|i| sel.head + i).unwrap_or(len);
            set_head(geo, h, false);
        }
        Key::Backspace => {
            if !sel.is_empty() {
                geo.doc.replace(sel.range(), "", EditKind::Delete);
            } else if cmd {
                let start = geo.doc.text[..sel.head].rfind('\n').map(|i| i + 1).unwrap_or(0);
                geo.doc.replace(start..sel.head, "", EditKind::Delete);
            } else if alt {
                let w = text::word_left(&geo.doc.text, sel.head);
                geo.doc.replace(w..sel.head, "", EditKind::Delete);
            } else {
                geo.doc.backspace();
            }
        }
        Key::Delete => {
            if !sel.is_empty() {
                geo.doc.replace(sel.range(), "", EditKind::Delete);
            } else {
                let end = if alt { text::word_right(&geo.doc.text, sel.head) } else { text::next_char(&geo.doc.text, sel.head) };
                geo.doc.replace(sel.head..end, "", EditKind::Delete);
            }
        }
        Key::Enter => {
            if cmd {
                let end = geo.doc.text[sel.head..].find('\n').map(|i| sel.head + i).unwrap_or(len);
                geo.doc.edit.sel = Selection::caret(end);
            }
            geo.doc.newline_with_indent();
        }
        Key::Tab => {
            if shift {
                geo.doc.indent_lines(false);
            } else {
                geo.doc.tab();
            }
        }
        Key::A if cmd => geo.doc.edit.sel = Selection { anchor: 0, head: len },
        Key::Z if cmd && shift => geo.doc.redo(),
        Key::Z if cmd => geo.doc.undo(),
        Key::Y if cmd => geo.doc.redo(),
        Key::L if cmd => geo.doc.select_line(),
        Key::D if cmd && shift => geo.doc.duplicate_lines(),
        Key::Slash if cmd => {
            if let Some(tok) = comment {
                geo.doc.toggle_comment(tok);
            }
        }
        Key::CloseBracket if cmd => geo.doc.indent_lines(true),
        Key::OpenBracket if cmd => geo.doc.indent_lines(false),
        Key::Escape => {
            if !sel.is_empty() {
                geo.doc.edit.sel = Selection::caret(sel.head);
            }
        }
        _ => {}
    }
}

/// ⌘← / Home: first non-blank character, or column 0 if already there.
fn smart_home(text: &str, i: usize) -> usize {
    let start = text[..i].rfind('\n').map(|j| j + 1).unwrap_or(0);
    let first = start + text::indent_of_line(text, start).len();
    if i == first {
        start
    } else {
        first
    }
}

fn comment_token(syntax: &str) -> Option<&'static str> {
    let s = syntax.to_ascii_lowercase();
    Some(match s.as_str() {
        "python" | "ruby" | "perl" | "r" | "yaml" | "toml" | "makefile" | "dockerfile" | "cmake" | "elixir" | "julia" | "nim"
        | "powershell" | "nix" | "fish" | "tcl" | "coffeescript" | "graphql" | "crystal" => "#",
        _ if s.contains("shell") || s.contains("bash") || s.contains("zsh") || s.contains("ini") || s.contains("git") || s.contains("properties") => "#",
        "sql" | "lua" | "haskell" | "elm" | "ada" | "vhdl" => "--",
        "latex" | "tex" | "matlab" | "erlang" | "bibtex" | "postscript" => "%",
        "lisp" | "clojure" | "scheme" | "racket" | "nasm" => ";",
        "vimscript" | "vim l" => "\"",
        "markdown" | "html" | "xml" | "plain text" | "json" | "csv" => return None,
        _ => "//",
    })
}
