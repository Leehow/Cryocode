//! Markdown → block tree → native widgets.
//!
//! The parsed tree is cached per document revision, and blocks scrolled out
//! of view are skipped using last frame's measured heights, so long documents
//! stay cheap to render every frame.

use super::PreviewCtx;
use crate::highlight;
use crate::math;
use crate::theme::Palette;
use egui::text::{LayoutJob, TextFormat};
use egui::{vec2, Color32, FontFamily, FontId, RichText, Sense, Stroke};
use pulldown_cmark::{Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::iter::Peekable;
use std::sync::Arc;

#[derive(Clone, Default, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub sup: bool,
    pub sub: bool,
    pub link: Option<Arc<str>>,
}

#[derive(Clone)]
pub enum Inline {
    Text(String, Style),
    Math(String),
    DisplayMath(String),
    Image { url: String, alt: String },
    FootnoteRef(String),
    Break,
}

#[derive(Clone)]
pub enum Block {
    Heading(u8, Vec<Inline>),
    Paragraph(Vec<Inline>),
    Code { lang: String, code: String },
    Math(String),
    Quote(Option<BlockQuoteKind>, Vec<Block>),
    List { start: Option<u64>, items: Vec<ListItem> },
    Table { aligns: Vec<Alignment>, head: Vec<Vec<Inline>>, rows: Vec<Vec<Vec<Inline>>> },
    Rule,
    Html(String),
    Footnote(String, Vec<Block>),
    FrontMatter(String),
}

#[derive(Clone)]
pub struct ListItem {
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

pub fn parse(src: &str) -> Vec<Block> {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_SUPERSCRIPT
        | Options::ENABLE_SUBSCRIPT;
    let mut ev = Parser::new_ext(src, opts).peekable();
    blocks(&mut ev, None)
}

type Events<'a> = Peekable<Parser<'a>>;

fn is_inline_start(e: &Event) -> bool {
    match e {
        Event::Text(_) | Event::Code(_) | Event::InlineMath(_) | Event::DisplayMath(_) | Event::InlineHtml(_)
        | Event::SoftBreak | Event::HardBreak | Event::FootnoteReference(_) => true,
        Event::Start(t) => matches!(
            t,
            Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Superscript | Tag::Subscript | Tag::Link { .. } | Tag::Image { .. }
        ),
        _ => false,
    }
}

fn blocks(ev: &mut Events, end: Option<TagEnd>) -> Vec<Block> {
    let mut out = Vec::new();
    let mut loose: Vec<Inline> = Vec::new();
    loop {
        let Some(peeked) = ev.peek() else { break };
        if is_inline_start(peeked) {
            inline_event(ev, &mut loose, &mut Style::default());
            continue;
        }
        let e = ev.next().unwrap();
        if !loose.is_empty() {
            out.push(Block::Paragraph(std::mem::take(&mut loose)));
        }
        match e {
            Event::End(t) if Some(t) == end => break,
            Event::End(_) => {}
            Event::Start(Tag::Paragraph) => {
                let inl = inlines(ev, TagEnd::Paragraph);
                // A paragraph that is only `$$…$$` is a display equation block.
                if let [Inline::DisplayMath(m)] = inl.as_slice() {
                    out.push(Block::Math(m.clone()));
                } else {
                    out.push(Block::Paragraph(inl));
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let n = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                out.push(Block::Heading(n, inlines(ev, TagEnd::Heading(level))));
            }
            Event::Start(Tag::BlockQuote(kind)) => {
                let inner = blocks(ev, Some(TagEnd::BlockQuote(kind)));
                out.push(Block::Quote(kind, inner));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.split([' ', ',', '{']).next().unwrap_or("").to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                let mut code = String::new();
                for e in ev.by_ref() {
                    match e {
                        Event::Text(t) => code.push_str(&t),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                if code.ends_with('\n') {
                    code.pop();
                }
                if matches!(lang.as_str(), "math" | "latex" | "katex" | "tex-math") {
                    out.push(Block::Math(code));
                } else {
                    out.push(Block::Code { lang, code });
                }
            }
            Event::Start(Tag::List(start)) => {
                let mut items = Vec::new();
                loop {
                    match ev.next() {
                        Some(Event::Start(Tag::Item)) => {
                            let mut task = None;
                            if let Some(Event::TaskListMarker(b)) = ev.peek() {
                                task = Some(*b);
                                ev.next();
                            }
                            let blocks = blocks(ev, Some(TagEnd::Item));
                            items.push(ListItem { task, blocks });
                        }
                        Some(Event::End(TagEnd::List(_))) | None => break,
                        _ => {}
                    }
                }
                out.push(Block::List { start, items });
            }
            Event::Start(Tag::Table(aligns)) => {
                let mut head = Vec::new();
                let mut rows = Vec::new();
                let mut row: Vec<Vec<Inline>> = Vec::new();
                loop {
                    match ev.next() {
                        Some(Event::Start(Tag::TableCell)) => row.push(inlines(ev, TagEnd::TableCell)),
                        Some(Event::End(TagEnd::TableHead)) => head = std::mem::take(&mut row),
                        Some(Event::End(TagEnd::TableRow)) => rows.push(std::mem::take(&mut row)),
                        Some(Event::End(TagEnd::Table)) | None => break,
                        _ => {}
                    }
                }
                out.push(Block::Table { aligns, head, rows });
            }
            Event::Start(Tag::HtmlBlock) => {
                let mut html = String::new();
                for e in ev.by_ref() {
                    match e {
                        Event::Html(t) | Event::Text(t) => html.push_str(&t),
                        Event::End(TagEnd::HtmlBlock) => break,
                        _ => {}
                    }
                }
                let trimmed = html.trim();
                if !(trimmed.starts_with("<!--") && trimmed.ends_with("-->")) {
                    out.push(Block::Html(trimmed.to_owned()));
                }
            }
            Event::Start(Tag::FootnoteDefinition(label)) => {
                let inner = blocks(ev, Some(TagEnd::FootnoteDefinition));
                out.push(Block::Footnote(label.to_string(), inner));
            }
            Event::Start(Tag::MetadataBlock(kind)) => {
                let mut meta = String::new();
                for e in ev.by_ref() {
                    match e {
                        Event::Text(t) => meta.push_str(&t),
                        Event::End(TagEnd::MetadataBlock(k)) if k == kind => break,
                        _ => {}
                    }
                }
                out.push(Block::FrontMatter(meta.trim_end().to_owned()));
            }
            Event::Start(other) => {
                let inner = blocks(ev, Some(other.to_end()));
                out.extend(inner);
            }
            Event::Rule => out.push(Block::Rule),
            Event::Html(h) => out.push(Block::Html(h.to_string())),
            _ => {}
        }
    }
    if !loose.is_empty() {
        out.push(Block::Paragraph(loose));
    }
    out
}

fn inlines(ev: &mut Events, end: TagEnd) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut style = Style::default();
    while let Some(e) = ev.peek() {
        if matches!(e, Event::End(t) if *t == end) {
            ev.next();
            break;
        }
        inline_event(ev, &mut out, &mut style);
    }
    out
}

fn push_text(out: &mut Vec<Inline>, s: &str, style: &Style) {
    if let Some(Inline::Text(prev, st)) = out.last_mut() {
        if st == style {
            prev.push_str(s);
            return;
        }
    }
    out.push(Inline::Text(s.to_owned(), style.clone()));
}

fn inline_event(ev: &mut Events, out: &mut Vec<Inline>, style: &mut Style) {
    let Some(e) = ev.next() else { return };
    match e {
        Event::Text(t) => push_text(out, &t, style),
        Event::Code(t) => {
            let mut s = style.clone();
            s.code = true;
            out.push(Inline::Text(t.to_string(), s));
        }
        Event::InlineMath(m) => out.push(Inline::Math(m.to_string())),
        Event::DisplayMath(m) => out.push(Inline::DisplayMath(m.to_string())),
        Event::SoftBreak => push_text(out, " ", style),
        Event::HardBreak => out.push(Inline::Break),
        Event::InlineHtml(h) => {
            let tag = h.trim().to_ascii_lowercase();
            match tag.as_str() {
                "<br>" | "<br/>" | "<br />" => out.push(Inline::Break),
                "<b>" | "<strong>" => style.bold = true,
                "</b>" | "</strong>" => style.bold = false,
                "<i>" | "<em>" => style.italic = true,
                "</i>" | "</em>" => style.italic = false,
                "<code>" | "<kbd>" => style.code = true,
                "</code>" | "</kbd>" => style.code = false,
                "<sup>" => style.sup = true,
                "</sup>" => style.sup = false,
                "<sub>" => style.sub = true,
                "</sub>" => style.sub = false,
                _ => {}
            }
        }
        Event::FootnoteReference(l) => out.push(Inline::FootnoteRef(l.to_string())),
        Event::Start(Tag::Emphasis) => style.italic = true,
        Event::End(TagEnd::Emphasis) => style.italic = false,
        Event::Start(Tag::Strong) => style.bold = true,
        Event::End(TagEnd::Strong) => style.bold = false,
        Event::Start(Tag::Strikethrough) => style.strike = true,
        Event::End(TagEnd::Strikethrough) => style.strike = false,
        Event::Start(Tag::Superscript) => style.sup = true,
        Event::End(TagEnd::Superscript) => style.sup = false,
        Event::Start(Tag::Subscript) => style.sub = true,
        Event::End(TagEnd::Subscript) => style.sub = false,
        Event::Start(Tag::Link { dest_url, .. }) => style.link = Some(Arc::from(dest_url.as_ref())),
        Event::End(TagEnd::Link) => style.link = None,
        Event::Start(Tag::Image { dest_url, .. }) => {
            let mut alt = String::new();
            for e in ev.by_ref() {
                match e {
                    Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                    Event::End(TagEnd::Image) => break,
                    _ => {}
                }
            }
            out.push(Inline::Image { url: dest_url.to_string(), alt });
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Rendering

pub const BODY: f32 = 15.0;

fn heading_size(level: u8) -> f32 {
    match level {
        1 => 30.0,
        2 => 24.0,
        3 => 19.5,
        4 => 17.0,
        _ => 15.0,
    }
}

/// Render the whole document; returns nothing, scroll is handled by the caller.
pub fn show_blocks(ui: &mut egui::Ui, cx: &mut PreviewCtx, blocks: &[Block], heights: &mut Vec<f32>, width_key: &mut f32) {
    let width = ui.available_width();
    if (*width_key - width).abs() > 0.5 || heights.len() != blocks.len() {
        heights.clear();
        heights.resize(blocks.len(), 0.0);
        *width_key = width;
    }
    let clip = ui.clip_rect();
    for (i, b) in blocks.iter().enumerate() {
        let top = ui.cursor().top();
        let known = heights[i];
        // Skip blocks far outside the viewport once we know how tall they are.
        if known > 0.0 && (top > clip.bottom() + 800.0 || top + known < clip.top() - 800.0) {
            ui.allocate_space(vec2(width, known));
            ui.add_space(ui.spacing().item_spacing.y);
            continue;
        }
        block(ui, cx, b, 0);
        heights[i] = (ui.cursor().top() - top - ui.spacing().item_spacing.y).max(1.0);
    }
}

fn block(ui: &mut egui::Ui, cx: &mut PreviewCtx, b: &Block, depth: usize) {
    let p = cx.palette;
    match b {
        Block::Heading(level, inl) => {
            ui.add_space(if *level <= 2 { 14.0 } else { 8.0 });
            let size = heading_size(*level);
            let top = ui.cursor().top();
            paragraph(ui, cx, inl, size, true, if *level == 6 { p.text_dim } else { p.text });
            if *level <= 2 {
                let bottom = ui.min_rect().bottom().max(top);
                let r = ui.available_rect_before_wrap();
                ui.painter().hline(r.x_range(), bottom + 4.0, Stroke::new(1.0_f32, p.border));
                ui.add_space(4.0);
            }
        }
        Block::Paragraph(inl) => paragraph(ui, cx, inl, BODY, false, p.text),
        Block::Math(m) => {
            ui.add_space(4.0);
            ui.vertical_centered(|ui| {
                egui::ScrollArea::horizontal().id_salt(("dmath", m.as_str())).show(ui, |ui| {
                    math::show(ui, cx.math, m, BODY * 1.15, true, p.text);
                });
            });
            ui.add_space(4.0);
        }
        Block::Code { lang, code } => code_block(ui, cx, lang, code),
        Block::FrontMatter(meta) => code_block(ui, cx, "yaml", meta),
        Block::Quote(kind, inner) => {
            let (accent, title) = match kind {
                Some(BlockQuoteKind::Note) => (p.accent, Some("ⓘ  Note")),
                Some(BlockQuoteKind::Tip) => (p.ok, Some("💡 Tip")),
                Some(BlockQuoteKind::Important) => (p.keyword, Some("❗ Important")),
                Some(BlockQuoteKind::Warning) => (p.number, Some("⚠  Warning")),
                Some(BlockQuoteKind::Caution) => (p.warn, Some("⛔ Caution")),
                None => (p.border, None),
            };
            let resp = egui::Frame::new()
                .inner_margin(egui::Margin { left: 16, right: 8, top: 4, bottom: 4 })
                .fill(if kind.is_some() { accent.gamma_multiply(0.06) } else { Color32::TRANSPARENT })
                .show(ui, |ui| {
                    if let Some(t) = title {
                        ui.label(RichText::new(t).color(accent).strong().size(BODY - 1.0));
                    }
                    for b in inner {
                        block(ui, cx, b, depth + 1);
                    }
                });
            let r = resp.response.rect;
            ui.painter().rect_filled(egui::Rect::from_min_size(r.min, vec2(3.0, r.height())), 1.5, if kind.is_some() { accent } else { p.text_faint });
        }
        Block::List { start, items } => ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for (i, item) in items.iter().enumerate() {
                ui.horizontal_top(|ui| {
                    let marker_w = 22.0;
                    let (rect, _) = ui.allocate_exact_size(vec2(marker_w, BODY * 1.45), Sense::hover());
                    let painter = ui.painter();
                    match (item.task, start) {
                        (Some(done), _) => {
                            let r = egui::Rect::from_center_size(rect.center() - vec2(3.0, 0.0), vec2(13.0, 13.0));
                            if done {
                                painter.rect_filled(r, 3.0, p.accent);
                                painter.add(egui::Shape::line(
                                    vec![r.left_center() + vec2(3.0, 0.0), r.center_bottom() + vec2(-1.0, -3.5), r.right_top() + vec2(-3.0, 3.5)],
                                    Stroke::new(1.6_f32, p.bg),
                                ));
                            } else {
                                painter.rect_stroke(r, 3.0, Stroke::new(1.2_f32, p.text_dim), egui::StrokeKind::Inside);
                            }
                        }
                        (None, Some(n)) => {
                            painter.text(rect.right_center() - vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER, format!("{}.", n + i as u64), FontId::proportional(BODY), p.text_dim);
                        }
                        (None, None) => {
                            let c = rect.center() - vec2(3.0, 0.0);
                            if depth % 2 == 0 {
                                painter.circle_filled(c, 2.6, p.text_dim);
                            } else {
                                painter.circle_stroke(c, 2.6, Stroke::new(1.1_f32, p.text_dim));
                            }
                        }
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for b in &item.blocks {
                            block(ui, cx, b, depth + 1);
                        }
                    });
                });
            }
        }).inner,
        Block::Table { aligns, head, rows } => {
            egui::ScrollArea::horizontal().id_salt(("md-table", head.len(), rows.len(), ui.cursor().top() as i32)).show(ui, |ui| {
                egui::Frame::new().stroke(Stroke::new(1.0_f32, p.border)).corner_radius(6.0).inner_margin(0).show(ui, |ui| {
                    egui::Grid::new(ui.next_auto_id()).striped(true).spacing(vec2(18.0, 8.0)).min_col_width(40.0).show(ui, |ui| {
                        for (j, cell) in head.iter().enumerate() {
                            cell_ui(ui, cx, cell, aligns.get(j), true);
                        }
                        ui.end_row();
                        for row in rows {
                            for (j, cell) in row.iter().enumerate() {
                                cell_ui(ui, cx, cell, aligns.get(j), false);
                            }
                            ui.end_row();
                        }
                    });
                });
            });
        }
        Block::Rule => {
            ui.add_space(6.0);
            let r = ui.available_rect_before_wrap();
            ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0_f32, p.border));
            ui.add_space(8.0);
        }
        Block::Html(h) => {
            ui.label(RichText::new(h).monospace().color(p.text_faint).size(12.0));
        }
        Block::Footnote(label, inner) => {
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(format!("[{label}]")).color(p.accent).size(12.5));
                ui.vertical(|ui| {
                    for b in inner {
                        block(ui, cx, b, depth + 1);
                    }
                });
            });
        }
    }
}

fn cell_ui(ui: &mut egui::Ui, cx: &mut PreviewCtx, cell: &[Inline], align: Option<&Alignment>, header: bool) {
    let layout = match align {
        Some(Alignment::Center) => egui::Layout::top_down(egui::Align::Center),
        Some(Alignment::Right) => egui::Layout::top_down(egui::Align::Max),
        _ => egui::Layout::top_down(egui::Align::Min),
    };
    ui.with_layout(layout, |ui| {
        ui.set_max_width(420.0);
        let c = cx.palette.text;
        paragraph(ui, cx, cell, BODY - 1.0, header, c);
    });
}

fn code_block(ui: &mut egui::Ui, cx: &mut PreviewCtx, lang: &str, code: &str) {
    let p = cx.palette;
    let font = FontId::monospace(13.0);
    let job = cx.code_jobs.get(lang, code, cx.theme, &font, p.text);
    egui::Frame::new()
        .fill(p.code_bg)
        .corner_radius(8.0)
        .stroke(Stroke::new(1.0_f32, p.border))
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let top_right = ui.max_rect().right_top();
            egui::ScrollArea::horizontal().id_salt(("code", lang, code.len(), ui.cursor().top() as i32)).show(ui, |ui| {
                ui.add(egui::Label::new(job).extend());
            });
            let btn = egui::Rect::from_min_size(top_right + vec2(-70.0, -4.0), vec2(70.0, 18.0));
            let resp = ui.interact(btn, ui.id().with(("copy", code.len(), lang)), Sense::click());
            let label = if resp.hovered() { "复制" } else if lang.is_empty() { "" } else { lang };
            ui.painter().text(btn.right_center(), egui::Align2::RIGHT_CENTER, label, FontId::proportional(11.0), if resp.hovered() { p.accent } else { p.text_faint });
            if resp.clicked() {
                ui.ctx().copy_text(code.to_owned());
            }
        });
}

fn text_format(style: &Style, size: f32, heading: bool, color: Color32, p: &Palette) -> TextFormat {
    let mut size = size;
    if style.sup || style.sub {
        size *= 0.75;
    }
    let family = if style.code {
        FontFamily::Monospace
    } else if style.bold || heading {
        FontFamily::Name(crate::theme::BOLD_FAMILY.into())
    } else {
        FontFamily::Proportional
    };
    let mut f = TextFormat::simple(FontId::new(if style.code { size * 0.9 } else { size }, family), color);
    if style.code {
        f.background = p.code_bg;
        f.color = p.keyword;
    }
    if style.italic {
        f.italics = true;
    }
    if style.strike {
        f.strikethrough = Stroke::new(1.0_f32, p.text_dim);
        f.color = p.text_dim;
    }
    if style.link.is_some() {
        f.color = p.link;
        f.underline = Stroke::new(0.8_f32, p.link.gamma_multiply(0.5));
    }
    if style.sup {
        f.valign = egui::Align::TOP;
    }
    if style.sub {
        f.valign = egui::Align::BOTTOM;
    }
    f
}

/// Lay out a run of inlines. Plain text and styled spans merge into one galley so wrapping and
/// CJK line breaking stay natural; links, math and images are separate widgets in the flow.
fn paragraph(ui: &mut egui::Ui, cx: &mut PreviewCtx, inl: &[Inline], size: f32, heading: bool, color: Color32) {
    let p = cx.palette;
    let simple = inl.iter().all(|i| matches!(i, Inline::Text(_, s) if s.link.is_none()));
    let row_h = size * 1.55;
    if simple {
        let mut job = LayoutJob::default();
        for i in inl {
            if let Inline::Text(t, s) = i {
                let mut f = text_format(s, size, heading, color, &p);
                f.line_height = Some(row_h);
                job.append(t, 0.0, f);
            }
        }
        job.wrap.max_width = ui.available_width();
        ui.add(egui::Label::new(job).wrap());
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.set_row_height(row_h);
        let mut job = LayoutJob::default();
        let flush = |ui: &mut egui::Ui, job: &mut LayoutJob| {
            if !job.text.is_empty() {
                let j = std::mem::take(job);
                ui.add(egui::Label::new(j).wrap());
            }
        };
        for i in inl {
            match i {
                Inline::Text(t, s) if s.link.is_none() => {
                    let mut f = text_format(s, size, heading, color, &p);
                    f.line_height = Some(row_h);
                    job.append(t, 0.0, f);
                }
                Inline::Text(t, s) => {
                    flush(ui, &mut job);
                    let mut j = LayoutJob::default();
                    let mut f = text_format(s, size, heading, color, &p);
                    f.line_height = Some(row_h);
                    j.append(t, 0.0, f);
                    let url = s.link.clone().unwrap_or_default();
                    let resp = ui.add(egui::Label::new(j).wrap().sense(Sense::click())).on_hover_text(url.as_ref());
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if resp.clicked() {
                        cx.clicked_link = Some(url.to_string());
                    }
                }
                Inline::Math(m) => {
                    flush(ui, &mut job);
                    math::show(ui, cx.math, m, size * 1.05, false, color);
                }
                Inline::DisplayMath(m) => {
                    flush(ui, &mut job);
                    ui.end_row();
                    ui.vertical_centered(|ui| {
                        math::show(ui, cx.math, m, size * 1.15, true, color);
                    });
                    ui.end_row();
                }
                Inline::Image { url, alt } => {
                    flush(ui, &mut job);
                    image(ui, cx, url, alt);
                }
                Inline::FootnoteRef(l) => {
                    let mut f = TextFormat::simple(FontId::proportional(size * 0.72), p.accent);
                    f.valign = egui::Align::TOP;
                    job.append(&format!("[{l}]"), 0.0, f);
                }
                Inline::Break => {
                    flush(ui, &mut job);
                    ui.end_row();
                }
            }
        }
        flush(ui, &mut job);
    });
}

fn image(ui: &mut egui::Ui, cx: &mut PreviewCtx, url: &str, alt: &str) {
    let p = cx.palette;
    match cx.images.texture(ui.ctx(), cx.base_dir.as_deref(), url) {
        Some(tex) => {
            let size = tex.size_vec2();
            let max_w = ui.available_width().min(size.x);
            let scale = max_w / size.x.max(1.0);
            ui.add(egui::Image::new(&tex).fit_to_exact_size(size * scale).corner_radius(6.0)).on_hover_text(alt);
        }
        None => {
            ui.label(RichText::new(format!("🖼 {}", if alt.is_empty() { url } else { alt })).color(p.text_dim));
        }
    }
}

/// Highlighted code-block layout jobs, cached by content.
#[derive(Default)]
pub struct CodeJobCache {
    jobs: std::collections::HashMap<(u64, crate::theme::ThemeKind), Arc<LayoutJob>>,
}

impl CodeJobCache {
    pub fn get(&mut self, lang: &str, code: &str, theme: crate::theme::ThemeKind, font: &FontId, fallback: Color32) -> Arc<LayoutJob> {
        let key = (highlight::line_hash(&format!("{lang}\u{0}{code}")), theme);
        if let Some(j) = self.jobs.get(&key) {
            return j.clone();
        }
        let job = Arc::new(highlight::highlight_snippet(code, lang, theme, font.clone(), fallback));
        if self.jobs.len() > 2048 {
            self.jobs.clear();
        }
        self.jobs.insert(key, job.clone());
        job
    }
}
