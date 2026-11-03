//! Workspace-wide text search (⌘⇧F), run on a background thread.

use crate::palette::rel_path;
use crate::theme::Palette;
use egui::{vec2, Align2, FontId, RichText, Sense};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const MAX_HITS: usize = 5000;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

pub struct Hit {
    pub line: usize,
    /// Byte range of the match in the file.
    pub range: (usize, usize),
    pub preview: String,
    /// Byte range of the match inside `preview`.
    pub preview_range: (usize, usize),
}

pub struct FileHits {
    pub path: PathBuf,
    pub hits: Vec<Hit>,
}

#[derive(Default)]
struct Results {
    files: Vec<FileHits>,
    total: usize,
    done: bool,
}

#[derive(Default)]
pub struct WorkspaceSearch {
    pub query: String,
    pub case_sensitive: bool,
    results: Arc<Mutex<Results>>,
    generation: Arc<AtomicU64>,
    searched: Option<(String, bool, usize)>,
    pub focus_request: bool,
}

impl WorkspaceSearch {
    fn start(&mut self, ctx: &egui::Context, files: Vec<PathBuf>) {
        let gen = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let results = Arc::new(Mutex::new(Results::default()));
        self.results = results.clone();
        let query = self.query.clone();
        let case = self.case_sensitive;
        let generation = self.generation.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let needle = if case { query.clone() } else { query.to_ascii_lowercase() };
            for path in files {
                if generation.load(Ordering::Relaxed) != gen {
                    return;
                }
                if std::fs::metadata(&path).map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else { continue };
                if bytes.iter().take(4096).any(|b| *b == 0) {
                    continue;
                }
                let Ok(text) = String::from_utf8(bytes) else { continue };
                let hay = if case { std::borrow::Cow::Borrowed(text.as_str()) } else { std::borrow::Cow::Owned(text.to_ascii_lowercase()) };
                let mut hits = Vec::new();
                let mut line = 1usize;
                let mut last = 0usize;
                for (b, m) in hay.match_indices(needle.as_str()) {
                    line += text.as_bytes()[last..b].iter().filter(|c| **c == b'\n').count();
                    last = b;
                    let ls = text[..b].rfind('\n').map_or(0, |i| i + 1);
                    let le = text[b..].find('\n').map_or(text.len(), |i| b + i);
                    // Keep a readable window around the match.
                    // Start the preview shortly before the match so it stays visible in a narrow sidebar.
                    let ps = if b - ls > 24 { text.ceil_char_boundary(b - 16) } else { ls };
                    let pe = text.floor_char_boundary(le.min(b + m.len() + 120));
                    let raw = &text[ps..pe];
                    let trimmed = raw.trim_start();
                    let cut = raw.len() - trimmed.len();
                    hits.push(Hit {
                        line,
                        range: (b, b + m.len()),
                        preview: trimmed.replace('\t', " "),
                        preview_range: (b - ps - cut, b - ps - cut + m.len()),
                    });
                    if hits.len() >= 200 {
                        break;
                    }
                }
                if !hits.is_empty() {
                    let mut r = results.lock().unwrap();
                    r.total += hits.len();
                    r.files.push(FileHits { path, hits });
                    let full = r.total >= MAX_HITS;
                    drop(r);
                    ctx.request_repaint();
                    if full {
                        break;
                    }
                }
            }
            results.lock().unwrap().done = true;
            ctx.request_repaint();
        });
    }

    /// Search panel UI; returns (file, byte range) when a result is clicked.
    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, root: Option<&Path>, files: &[PathBuf]) -> Option<(PathBuf, (usize, usize))> {
        let mut open = None;
        ui.add_space(4.0);
        // Right-to-left so the box fills exactly the remaining width (asking for more makes the panel grow every frame).
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                if ui.selectable_label(self.case_sensitive, "Aa").on_hover_text("区分大小写").clicked() {
                    self.case_sensitive = !self.case_sensitive;
                }
                let edit = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("在工作区中搜索").desired_width(ui.available_width() - 8.0));
                if std::mem::take(&mut self.focus_request) {
                    edit.request_focus();
                }
            });
        });
        if root.is_none() {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("先打开一个文件夹（⌘⇧O）").color(p.text_dim)));
            return None;
        }
        let key = (self.query.clone(), self.case_sensitive, files.len());
        if self.query.chars().count() >= 2 && self.searched.as_ref() != Some(&key) {
            self.searched = Some(key);
            self.start(ui.ctx(), files.to_vec());
        } else if self.query.chars().count() < 2 {
            self.searched = None;
            *self.results.lock().unwrap() = Results::default();
        }
        let r = self.results.lock().unwrap();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let status = if self.query.chars().count() < 2 {
                String::new()
            } else if r.total == 0 && r.done {
                "无结果".into()
            } else {
                format!("{} 个文件中 {} 处结果{}", r.files.len(), r.total, if r.done { "" } else { "…" })
            };
            ui.label(RichText::new(status).size(11.5).color(p.text_dim));
        });
        egui::ScrollArea::vertical().id_salt("ws-search").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for f in &r.files {
                let name = f.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let dir = rel_path(f.path.parent().unwrap_or(&f.path), root);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(RichText::new(name).size(12.5).color(p.text).strong());
                    ui.add(egui::Label::new(RichText::new(dir).size(11.0).color(p.text_faint)).truncate());
                });
                for h in &f.hits {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect.shrink2(vec2(6.0, 0.0)), 4.0, p.current_line);
                    }
                    let font = FontId::monospace(12.0);
                    let (a, b) = h.preview_range;
                    let mut job = egui::text::LayoutJob::default();
                    let fmt = |c| egui::TextFormat::simple(font.clone(), c);
                    job.append(&h.preview[..a.min(h.preview.len())], 0.0, fmt(p.text_dim));
                    let mut m = fmt(p.text);
                    m.background = p.accent_soft;
                    job.append(&h.preview[a.min(h.preview.len())..b.min(h.preview.len())], 0.0, m);
                    job.append(&h.preview[b.min(h.preview.len())..], 0.0, fmt(p.text_dim));
                    let galley = ui.fonts_mut(|f| f.layout_job(job));
                    let painter = ui.painter_at(rect);
                    painter.text(rect.left_center() + vec2(22.0, 0.0), Align2::LEFT_CENTER, h.line.to_string(), FontId::monospace(11.0), p.text_faint);
                    painter.galley(rect.left_center() + vec2(54.0, -galley.size().y / 2.0), galley, p.text);
                    if resp.clicked() {
                        open = Some((f.path.clone(), h.range));
                    }
                }
                ui.add_space(6.0);
            }
        });
        open
    }
}
