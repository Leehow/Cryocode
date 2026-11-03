//! Native TeX math typesetting (the KaTeX subset), drawn straight onto the GPU canvas.

mod layout;
mod parse;

pub use layout::MBox;
use std::collections::HashMap;
use std::sync::Arc;

/// Memoises laid-out formulas; layout needs font metrics so it lives on the UI thread.
#[derive(Default)]
pub struct MathCache {
    boxes: HashMap<(String, u32, bool), Arc<MBox>>,
}

impl MathCache {
    pub fn get(&mut self, ctx: &egui::Context, tex: &str, size: f32, display: bool) -> Arc<MBox> {
        let key = (tex.to_owned(), size.to_bits(), display);
        if let Some(b) = self.boxes.get(&key) {
            return b.clone();
        }
        let nodes = parse::parse(tex);
        let b = Arc::new(layout::Layouter::new(ctx).layout_root(&nodes, size, display));
        if self.boxes.len() > 4096 {
            self.boxes.clear();
        }
        self.boxes.insert(key, b.clone());
        b
    }
}

/// Allocate and paint one formula. Inline formulas sit on the text baseline.
pub fn show(ui: &mut egui::Ui, cache: &mut MathCache, tex: &str, size: f32, display: bool, color: egui::Color32) -> egui::Response {
    let b = cache.get(ui.ctx(), tex, size, display);
    let pad = if display { 0.0 } else { 1.0 };
    // Inline: reserve a row-height box and centre the formula's baseline on the text baseline.
    let row_h = ui.text_style_height(&egui::TextStyle::Body).max(size * 1.2);
    let (asc, desc) = if display { (b.asc, b.desc) } else { (b.asc.max(size * 0.9), b.desc.max(size * 0.3)) };
    let h = if display { asc + desc } else { (asc + desc).max(row_h) };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(b.w + 2.0 * pad, h), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let baseline_y = if display { rect.top() + asc } else { rect.bottom() - desc.max(size * 0.3) + (h - asc - desc).min(0.0) };
        let baseline_y = if display { baseline_y } else { rect.top() + asc + (h - asc - desc) * 0.5 };
        layout::paint(ui.ctx(), ui.painter(), egui::pos2(rect.left() + pad, baseline_y), &b, color);
    }
    resp
}
