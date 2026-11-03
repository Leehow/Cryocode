//! Workspace file tree.

use crate::theme::Palette;
use egui::{pos2, vec2, Align2, FontId, Sense, Stroke};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

pub struct FileTree {
    pub root: Option<PathBuf>,
    expanded: HashSet<PathBuf>,
    listing: HashMap<PathBuf, Vec<Entry>>,
    refreshed: Instant,
}

pub enum TreeAction {
    Open(PathBuf),
    Reveal(PathBuf),
    CopyPath(PathBuf),
}

impl FileTree {
    pub fn new(root: Option<PathBuf>) -> Self {
        let mut t = Self { root: None, expanded: HashSet::new(), listing: HashMap::new(), refreshed: Instant::now() };
        if let Some(r) = root {
            t.set_root(r);
        }
        t
    }

    pub fn set_root(&mut self, root: PathBuf) {
        self.expanded.clear();
        self.listing.clear();
        self.expanded.insert(root.clone());
        self.root = Some(root);
    }

    pub fn expanded_dirs(&self) -> Vec<PathBuf> {
        self.expanded.iter().cloned().collect()
    }

    pub fn restore_expanded(&mut self, dirs: Vec<PathBuf>) {
        self.expanded.extend(dirs.into_iter().filter(|d| d.is_dir()));
    }

    /// Expand the folders leading to `path` so the active file is visible.
    pub fn reveal(&mut self, path: &Path) {
        let Some(root) = &self.root else { return };
        if !path.starts_with(root) {
            return;
        }
        for a in path.ancestors().skip(1) {
            self.expanded.insert(a.to_owned());
            if a == root {
                break;
            }
        }
    }

    fn entries(&mut self, dir: &Path) -> &[Entry] {
        if !self.listing.contains_key(dir) {
            let mut v: Vec<Entry> = std::fs::read_dir(dir)
                .map(|rd| {
                    rd.flatten()
                        .filter_map(|e| {
                            let name = e.file_name().to_string_lossy().into_owned();
                            if name == ".DS_Store" || name == ".git" {
                                return None;
                            }
                            let path = e.path();
                            let is_dir = e.file_type().map(|t| t.is_dir() || (t.is_symlink() && path.is_dir())).unwrap_or(false);
                            Some(Entry { path, name, is_dir })
                        })
                        .collect()
                })
                .unwrap_or_default();
            v.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| natural_key(&a.name).cmp(&natural_key(&b.name))));
            self.listing.insert(dir.to_owned(), v);
        }
        &self.listing[dir]
    }

    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, active: Option<&Path>) -> Option<TreeAction> {
        if self.refreshed.elapsed() > Duration::from_secs(3) {
            self.listing.clear();
            self.refreshed = Instant::now();
        }
        let Some(root) = self.root.clone() else {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("没有打开的文件夹").color(p.text_dim));
                ui.add_space(4.0);
                ui.label(egui::RichText::new("⌘O 打开文件夹").color(p.text_faint).size(12.0));
            });
            return None;
        };
        let mut action = None;
        let name = root.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_else(|| "/".into());
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(egui::RichText::new(name).size(11.0).color(p.text_dim).strong());
        });
        ui.add_space(2.0);
        egui::ScrollArea::vertical().id_salt("file-tree").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            self.rows(ui, p, &root, 0, active, &mut action);
            ui.add_space(24.0);
        });
        action
    }

    fn rows(&mut self, ui: &mut egui::Ui, p: &Palette, dir: &Path, depth: usize, active: Option<&Path>, action: &mut Option<TreeAction>) {
        let items: Vec<(PathBuf, String, bool)> = self.entries(dir).iter().map(|e| (e.path.clone(), e.name.clone(), e.is_dir)).collect();
        for (path, name, is_dir) in items {
            let open = is_dir && self.expanded.contains(&path);
            let w = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
            if ui.is_rect_visible(rect) {
                let painter = ui.painter();
                let is_active = active == Some(path.as_path());
                if is_active {
                    painter.rect_filled(rect.shrink2(vec2(6.0, 1.0)), 5.0, p.accent_soft);
                } else if resp.hovered() {
                    painter.rect_filled(rect.shrink2(vec2(6.0, 1.0)), 5.0, p.current_line);
                }
                let x = rect.left() + 14.0 + depth as f32 * 14.0;
                let cy = rect.center().y;
                if is_dir {
                    chevron(painter, pos2(x + 4.0, cy), open, p.text_dim);
                } else {
                    let c = file_color(&name, p);
                    painter.circle_filled(pos2(x + 4.0, cy), 3.0, c);
                }
                let color = if is_active { p.text } else if name.starts_with('.') { p.text_faint } else { p.text_dim.lerp_to_gamma(p.text, 0.55) };
                painter.with_clip_rect(rect.shrink(2.0)).text(pos2(x + 16.0, cy), Align2::LEFT_CENTER, &name, FontId::proportional(13.0), color);
            }
            if resp.clicked() {
                if is_dir {
                    if open {
                        self.expanded.remove(&path);
                    } else {
                        self.expanded.insert(path.clone());
                    }
                } else {
                    *action = Some(TreeAction::Open(path.clone()));
                }
            }
            resp.context_menu(|ui| {
                if !is_dir && ui.button("打开").clicked() {
                    *action = Some(TreeAction::Open(path.clone()));
                    ui.close();
                }
                if ui.button("在 Finder 中显示").clicked() {
                    *action = Some(TreeAction::Reveal(path.clone()));
                    ui.close();
                }
                if ui.button("复制路径").clicked() {
                    *action = Some(TreeAction::CopyPath(path.clone()));
                    ui.close();
                }
            });
            if open {
                self.rows(ui, p, &path, depth + 1, active, action);
            }
        }
    }
}

fn chevron(painter: &egui::Painter, c: egui::Pos2, open: bool, color: egui::Color32) {
    let s = 3.5;
    let pts = if open {
        vec![c + vec2(-s, -s * 0.5), c + vec2(0.0, s * 0.6), c + vec2(s, -s * 0.5)]
    } else {
        vec![c + vec2(-s * 0.5, -s), c + vec2(s * 0.6, 0.0), c + vec2(-s * 0.5, s)]
    };
    painter.add(egui::Shape::line(pts, Stroke::new(1.4_f32, color)));
}

/// Colour dot per file family, purely decorative.
pub fn file_color(name: &str, p: &Palette) -> egui::Color32 {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "rs" => egui::Color32::from_rgb(0xde, 0x8a, 0x5a),
        "py" => egui::Color32::from_rgb(0x6a, 0xa8, 0xe0),
        "js" | "mjs" | "cjs" | "jsx" => egui::Color32::from_rgb(0xe6, 0xc8, 0x5a),
        "ts" | "tsx" | "mts" => egui::Color32::from_rgb(0x4f, 0x8f, 0xd8),
        "go" => egui::Color32::from_rgb(0x5a, 0xc8, 0xd8),
        "c" | "h" | "cpp" | "hpp" | "cc" | "m" | "mm" => egui::Color32::from_rgb(0x8a, 0x9a, 0xd8),
        "md" | "markdown" | "mdx" => p.accent,
        "tex" | "bib" | "sty" => egui::Color32::from_rgb(0x5a, 0xb8, 0x8a),
        "json" | "yaml" | "yml" | "toml" | "xml" | "csv" | "tsv" | "xlsx" | "xsl" => p.number,
        "html" | "css" | "scss" | "vue" | "svelte" => egui::Color32::from_rgb(0xd8, 0x6a, 0x8a),
        "sh" | "zsh" | "bash" | "fish" => p.ok,
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" => p.keyword,
        _ => p.text_faint,
    }
}

/// Sort key that orders "file2" before "file10".
fn natural_key(s: &str) -> Vec<(u8, u64, String)> {
    let mut out = Vec::new();
    let mut num = String::new();
    let mut txt = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !txt.is_empty() {
                out.push((1, 0, std::mem::take(&mut txt).to_lowercase()));
            }
            num.push(c);
        } else {
            if !num.is_empty() {
                out.push((0, num.parse().unwrap_or(u64::MAX), String::new()));
                num.clear();
            }
            txt.push(c);
        }
    }
    if !num.is_empty() {
        out.push((0, num.parse().unwrap_or(u64::MAX), String::new()));
    }
    if !txt.is_empty() {
        out.push((1, 0, txt.to_lowercase()));
    }
    out
}

/// Background index of all files under the workspace (respects .gitignore), for quick open.
pub struct FileIndex {
    pub files: std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>>,
}

impl FileIndex {
    pub fn build(root: PathBuf) -> Self {
        let files: std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>> = Default::default();
        let f = files.clone();
        let r = root;
        std::thread::spawn(move || {
            let walker = ignore::WalkBuilder::new(&r).hidden(false).filter_entry(|e| e.file_name() != ".git" && e.file_name() != "node_modules" && e.file_name() != "target").build();
            let mut batch = Vec::new();
            for entry in walker.flatten() {
                if entry.file_type().is_some_and(|t| t.is_file()) {
                    batch.push(entry.into_path());
                    if batch.len() >= 2000 {
                        f.lock().unwrap().append(&mut batch);
                    }
                    if f.lock().unwrap().len() > 200_000 {
                        break;
                    }
                }
            }
            f.lock().unwrap().append(&mut batch);
        });
        Self { files }
    }
}
