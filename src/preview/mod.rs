//! Rendered views of documents: Markdown, LaTeX, data trees, tables, images.

pub mod latex;
pub mod markdown;
pub mod table;
pub mod tree;

use crate::document::Document;
use crate::formats::Format;
use crate::math::MathCache;
use crate::theme::{Palette, ThemeKind};
use egui::{vec2, Id};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Shared, long-lived rendering resources.
#[derive(Default)]
pub struct PreviewResources {
    pub math: MathCache,
    pub code_jobs: markdown::CodeJobCache,
    pub images: ImageCache,
}

/// Borrowed bundle handed to every renderer.
pub struct PreviewCtx<'a> {
    pub palette: Palette,
    pub theme: ThemeKind,
    pub math: &'a mut MathCache,
    pub code_jobs: &'a mut markdown::CodeJobCache,
    pub images: &'a mut ImageCache,
    pub base_dir: Option<PathBuf>,
    pub clicked_link: Option<String>,
}

enum Parsed {
    None,
    Markdown(Vec<markdown::Block>),
    Data(Result<tree::Value, String>),
    Xml(Result<tree::XmlNode, String>),
    Table(Result<table::Table, String>),
    Image(Result<egui::TextureHandle, String>),
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum DataMode {
    #[default]
    Tree,
    Table,
}

/// Per-document preview state.
pub struct PreviewState {
    revision: Option<u64>,
    parsed: Parsed,
    json_table: Option<table::Table>,
    pub data_mode: DataMode,
    heights: Vec<f32>,
    width_key: f32,
    parsed_at: Instant,
    sheet: usize,
    last_synced_fraction: f32,
    /// Scroll position of the rendered view (0..1), for driving the editor in split view.
    pub fraction: f32,
    /// The rendered view moved last frame (user scroll, or a sync from the editor).
    pub scrolled: bool,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            revision: None,
            parsed: Parsed::None,
            json_table: None,
            data_mode: DataMode::Tree,
            heights: Vec::new(),
            width_key: 0.0,
            parsed_at: Instant::now() - Duration::from_secs(10),
            sheet: 0,
            last_synced_fraction: -1.0,
            fraction: 0.0,
            scrolled: false,
        }
    }
}

impl PreviewState {
    fn refresh(&mut self, ctx: &egui::Context, doc: &Document) {
        if self.revision == Some(doc.revision) {
            return;
        }
        // Debounce re-parsing of large documents while typing.
        if self.revision.is_some() && doc.text.len() > 256 * 1024 && self.parsed_at.elapsed() < Duration::from_millis(250) {
            ctx.request_repaint_after(Duration::from_millis(260));
            return;
        }
        self.parsed = match doc.format {
            Format::Markdown => Parsed::Markdown(markdown::parse(&doc.text)),
            Format::Latex => Parsed::Markdown(markdown::parse(&latex::to_markdown(&doc.text))),
            Format::Json | Format::Yaml | Format::Toml => Parsed::Data(tree::parse_data(doc.format, &doc.text)),
            Format::Xml => Parsed::Xml(tree::parse_xml(&doc.text)),
            Format::Csv => Parsed::Table(table::parse_delimited(&doc.text, b',')),
            Format::Tsv => Parsed::Table(table::parse_delimited(&doc.text, b'\t')),
            Format::Spreadsheet => Parsed::Table(match &doc.binary {
                Some(b) => table::parse_spreadsheet(b, self.sheet),
                None => Err(doc.load_error.clone().unwrap_or_else(|| "无法读取".into())),
            }),
            Format::Image => Parsed::Image(match &doc.binary {
                Some(b) => load_texture(ctx, b, &format!("doc-img-{}", doc.id)),
                None => Err(doc.load_error.clone().unwrap_or_else(|| "无法读取".into())),
            }),
            Format::Code => Parsed::None,
        };
        self.json_table = match &self.parsed {
            Parsed::Data(Ok(v)) => tree::as_table(v).map(|(h, r)| table::Table::new(h, r)),
            Parsed::Xml(Ok(n)) => tree::xml_table(n).map(|(h, r)| table::Table::new(h, r)),
            _ => None,
        };
        if self.json_table.is_none() {
            self.data_mode = DataMode::Tree;
        }
        self.revision = Some(doc.revision);
        self.parsed_at = Instant::now();
    }

    pub fn has_table_mode(&self) -> bool {
        self.json_table.is_some()
    }
}

/// Draw the preview for `doc`. `sync` carries the source editor's scroll fraction in split mode.
pub fn show(ui: &mut egui::Ui, doc: &Document, state: &mut PreviewState, res: &mut PreviewResources, theme: ThemeKind, sync: Option<f32>) -> Option<String> {
    // `sync`: in split view, the editor's scroll fraction when the editor is the one being scrolled.
    state.refresh(ui.ctx(), doc);
    let palette = Palette::get(theme);
    let mut copied: Option<String> = None;
    let mut cx = PreviewCtx {
        palette,
        theme,
        math: &mut res.math,
        code_jobs: &mut res.code_jobs,
        images: &mut res.images,
        base_dir: doc.path.as_deref().and_then(Path::parent).map(Path::to_owned),
        clicked_link: None,
    };
    let id = Id::new(("preview", doc.id));
    match &mut state.parsed {
        Parsed::Markdown(blocks) => {
            let mut area = egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]);
            let frac = sync.filter(|f| (f - state.last_synced_fraction).abs() > 0.0005);
            if let Some(f) = frac {
                let content_h = ui.data(|d| d.get_temp::<f32>(id.with("content_h"))).unwrap_or(0.0);
                area = area.vertical_scroll_offset((content_h - ui.available_height()).max(0.0) * f);
                state.last_synced_fraction = f;
            }
            let out = area.show(ui, |ui| {
                // A centred reading column.
                let max_w = 860.0f32;
                let full = ui.available_width();
                let col = (full - 56.0).clamp(200.0, max_w);
                let side = ((full - col) / 2.0).max(28.0);
                ui.add_space(24.0);
                ui.horizontal_top(|ui| {
                    ui.add_space(side);
                    ui.vertical(|ui| {
                        ui.set_width(col);
                        ui.spacing_mut().item_spacing.y = 10.0;
                        markdown::show_blocks(ui, &mut cx, blocks, &mut state.heights, &mut state.width_key);
                        ui.add_space(120.0);
                    });
                });
            });
            let max = (out.content_size.y - out.inner_rect.height()).max(1.0);
            let f = (out.state.offset.y / max).clamp(0.0, 1.0);
            state.scrolled = (f - state.fraction).abs() > 0.0005;
            state.fraction = f;
            ui.data_mut(|d| d.insert_temp(id.with("content_h"), out.content_size.y));
        }
        Parsed::Data(result) => padded(ui, |ui| match result {
            Ok(v) => {
                if state.data_mode == DataMode::Table {
                    if let Some(t) = &state.json_table {
                        table::show(ui, t, &palette, id);
                        return;
                    }
                }
                egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
                    tree::TreeUi { palette, copied: &mut copied }.show_root(ui, v, id.with("root"));
                });
            }
            Err(e) => tree::error_box(ui, &palette, e),
        }),
        Parsed::Xml(result) => padded(ui, |ui| match result {
            Ok(_) if state.data_mode == DataMode::Table && state.json_table.is_some() => {
                table::show(ui, state.json_table.as_ref().unwrap(), &palette, id);
            }
            Ok(n) => {
                egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
                    tree::show_xml(ui, n, &palette, id.with("root"), 0);
                });
            }
            Err(e) => tree::error_box(ui, &palette, e),
        }),
        Parsed::Table(result) => {
            let mut switch = None;
            padded(ui, |ui| match result {
                Ok(t) => switch = table::show(ui, t, &palette, id),
                Err(e) => tree::error_box(ui, &palette, e),
            });
            if let Some(s) = switch {
                state.sheet = s;
                state.revision = None;
            }
        }
        Parsed::Image(result) => padded(ui, |ui| match result {
            Ok(tex) => {
                let size = tex.size_vec2();
                ui.label(egui::RichText::new(format!("{} × {} px", size.x, size.y)).color(palette.text_dim).size(12.0));
                egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
                    let avail = ui.available_size() - vec2(8.0, 8.0);
                    let scale = (avail.x / size.x).min(avail.y / size.y).min(1.0).max(0.05);
                    ui.centered_and_justified(|ui| {
                        ui.add(egui::Image::new(&*tex).fit_to_exact_size(size * scale).corner_radius(4.0));
                    });
                });
            }
            Err(e) => tree::error_box(ui, &palette, e),
        }),
        Parsed::None => {}
    }
    if let Some(c) = copied {
        ui.ctx().copy_text(c);
    }
    cx.clicked_link
}

fn padded(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().inner_margin(egui::Margin { left: 20, right: 12, top: 14, bottom: 8 }).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        f(ui)
    });
}

fn load_texture(ctx: &egui::Context, bytes: &[u8], name: &str) -> Result<egui::TextureHandle, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("无法解码图片：{e}"))?;
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    Ok(ctx.load_texture(name, color, egui::TextureOptions::LINEAR))
}

/// Local images referenced from Markdown, decoded once.
#[derive(Default)]
pub struct ImageCache {
    textures: HashMap<PathBuf, Option<egui::TextureHandle>>,
}

impl ImageCache {
    pub fn texture(&mut self, ctx: &egui::Context, base: Option<&Path>, url: &str) -> Option<egui::TextureHandle> {
        if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("data:") {
            return None;
        }
        let rel = url.strip_prefix("file://").unwrap_or(url);
        let path = match base {
            Some(b) if !Path::new(rel).is_absolute() => b.join(rel),
            _ => PathBuf::from(rel),
        };
        self.textures
            .entry(path.clone())
            .or_insert_with(|| std::fs::read(&path).ok().and_then(|b| load_texture(ctx, &b, &path.to_string_lossy()).ok()))
            .clone()
    }
}
