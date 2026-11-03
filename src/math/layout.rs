//! Node tree → positioned boxes, following TeX's layout rules loosely
//! (math axis, script styles, inter-atom spacing). Delimiters, radicals and
//! accents are drawn as vector strokes so they stay crisp at any height.

use super::parse::*;
use crate::theme::{MATH_FAMILY, SERIF_FAMILY};
use egui::{pos2, vec2, Color32, FontFamily, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2};

#[derive(Clone, Debug)]
pub enum Item {
    Text { pos: Vec2, text: String, size: f32, glyphs: Glyphs, color: Option<Color32> },
    Rule { rect: Rect, color: Option<Color32> },
    Polyline { pts: Vec<Pos2>, width: f32, color: Option<Color32> },
    Bezier { pts: [Pos2; 4], width: f32, color: Option<Color32> },
    Dot { center: Pos2, radius: f32, color: Option<Color32> },
    Frame { rect: Rect, width: f32, color: Option<Color32> },
}

impl Item {
    fn translate(&mut self, d: Vec2) {
        match self {
            Item::Text { pos, .. } => *pos += d,
            Item::Rule { rect, .. } | Item::Frame { rect, .. } => *rect = rect.translate(d),
            Item::Polyline { pts, .. } => pts.iter_mut().for_each(|p| *p += d),
            Item::Bezier { pts, .. } => pts.iter_mut().for_each(|p| *p += d),
            Item::Dot { center, .. } => *center += d,
        }
    }

    fn set_color_if_none(&mut self, c: Color32) {
        let slot = match self {
            Item::Text { color, .. }
            | Item::Rule { color, .. }
            | Item::Polyline { color, .. }
            | Item::Bezier { color, .. }
            | Item::Dot { color, .. }
            | Item::Frame { color, .. } => color,
        };
        if slot.is_none() {
            *slot = Some(c);
        }
    }
}

/// A laid-out box. Coordinates: x grows right from the box's left edge,
/// y grows *down* from the baseline (so ascenders have negative y).
#[derive(Clone, Debug, Default)]
pub struct MBox {
    pub w: f32,
    pub asc: f32,
    pub desc: f32,
    pub items: Vec<Item>,
}

impl MBox {
    fn empty() -> Self {
        Self::default()
    }

    fn place(&mut self, mut other: MBox, dx: f32, dy: f32) {
        for it in &mut other.items {
            it.translate(vec2(dx, dy));
        }
        self.items.extend(other.items);
        self.asc = self.asc.max(other.asc - dy);
        self.desc = self.desc.max(other.desc + dy);
        self.w = self.w.max(dx + other.w);
    }

    fn hcat(&mut self, other: MBox) {
        let x = self.w;
        self.place(other, x, 0.0);
    }
}

pub struct Layouter<'a> {
    ctx: &'a egui::Context,
}

fn scale(style: MathStyle) -> f32 {
    match style {
        MathStyle::Display | MathStyle::Text => 1.0,
        MathStyle::Script => 0.72,
        MathStyle::ScriptScript => 0.55,
    }
}

fn sup_style(style: MathStyle) -> MathStyle {
    match style {
        MathStyle::Display | MathStyle::Text => MathStyle::Script,
        _ => MathStyle::ScriptScript,
    }
}

fn frac_style(style: MathStyle) -> MathStyle {
    match style {
        MathStyle::Display => MathStyle::Text,
        MathStyle::Text => MathStyle::Script,
        _ => MathStyle::ScriptScript,
    }
}

fn family(g: Glyphs) -> FontFamily {
    match g {
        Glyphs::Math => FontFamily::Name(MATH_FAMILY.into()),
        Glyphs::Text => FontFamily::Name(SERIF_FAMILY.into()),
    }
}

const AXIS: f32 = 0.25;

/// Spacing in em between two atom classes (TeX's table, simplified).
fn spacing(l: Class, r: Class, tight: bool) -> f32 {
    use Class::*;
    const THIN: f32 = 3.0 / 18.0;
    const MED: f32 = 4.0 / 18.0;
    const THICK: f32 = 5.0 / 18.0;
    let s = match (l, r) {
        (Ord, Op) | (Op, Ord) | (Op, Op) | (Close, Op) | (Inner, Op) | (Op, Inner) | (Op, Open) => return THIN,
        (Ord, Bin) | (Bin, _) | (Close, Bin) | (Inner, Bin) => MED,
        (_, Rel) | (Rel, _) if l != r && l != Open && r != Close && r != Punct => THICK,
        (Punct, _) => THIN,
        (Ord, Inner) | (Inner, Ord) | (Close, Inner) | (Inner, Open) | (Inner, Punct) | (Inner, Inner) => THIN,
        _ => 0.0,
    };
    if tight {
        0.0
    } else {
        s
    }
}

fn glyph_extent(text: &str, size: f32) -> (f32, f32) {
    let mut asc: f32 = 0.0;
    let mut desc: f32 = 0.0;
    for c in text.chars() {
        let base = demath(c);
        let (a, d) = match base {
            '(' | ')' | '[' | ']' | '{' | '}' | '|' | '‖' | '⟨' | '⟩' | '⌊' | '⌋' | '⌈' | '⌉' | '/' | '∖' => (0.75, 0.25),
            'g' | 'j' | 'p' | 'q' | 'y' | 'β' | 'γ' | 'ζ' | 'η' | 'μ' | 'ξ' | 'ρ' | 'φ' | 'χ' | 'ψ' | 'ς' | ',' | ';' | 'ϕ' | 'ϱ' => (0.46, 0.21),
            'a' | 'c' | 'e' | 'm' | 'n' | 'o' | 'r' | 's' | 'u' | 'v' | 'w' | 'x' | 'z' | 'α' | 'ε' | 'ι' | 'κ' | 'ν' | 'ο' | 'π' | 'σ' | 'τ' | 'υ' | 'ω' | 'ϵ' | '+' | '−' | '=' | '×' | '⋅' | '∗' | '<' | '>' | '≤' | '≥' | '∼' | '≈' | '→' | '←' | '.' | ':' | '∘' | '∙' => (0.48, 0.0),
            '∫' | '∮' | '∬' | '∭' => (0.85, 0.3),
            '∑' | '∏' | '∐' | '⋃' | '⋂' => (0.75, 0.25),
            _ => (0.70, 0.02),
        };
        asc = asc.max(a);
        desc = desc.max(d);
    }
    (asc * size, desc * size)
}

/// Map a math-alphabet character back to its plain letter for metrics lookups.
fn demath(c: char) -> char {
    let u = c as u32;
    match u {
        0x210E => 'h',
        0x1D400..=0x1D6A3 => {
            let i = (u - 0x1D400) % 52;
            if i < 26 {
                char::from_u32('A' as u32 + i).unwrap_or(c)
            } else {
                char::from_u32('a' as u32 + i - 26).unwrap_or(c)
            }
        }
        0x1D6A8..=0x1D7C9 => {
            let i = (u - 0x1D6A8) % 58;
            if (26..51).contains(&i) {
                char::from_u32(0x3B1 + i - 26).unwrap_or(c)
            } else {
                'A'
            }
        }
        _ => c,
    }
}

fn is_italic_letter(text: &str) -> bool {
    let mut it = text.chars();
    matches!((it.next(), it.next()), (Some(c), None) if matches!(c as u32, 0x1D434..=0x1D467 | 0x1D6FC..=0x1D71B | 0x210E))
}

impl<'a> Layouter<'a> {
    pub fn new(ctx: &'a egui::Context) -> Self {
        Self { ctx }
    }

    fn text_width(&self, text: &str, size: f32, glyphs: Glyphs) -> f32 {
        self.ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), FontId::new(size, family(glyphs)), Color32::WHITE).size().x)
    }

    fn text_box(&self, text: &str, size: f32, glyphs: Glyphs) -> MBox {
        let w = self.text_width(text, size, glyphs);
        let (asc, desc) = if glyphs == Glyphs::Text { (0.70 * size, 0.2 * size) } else { glyph_extent(text, size) };
        MBox { w, asc, desc, items: vec![Item::Text { pos: Vec2::ZERO, text: text.to_owned(), size, glyphs, color: None }] }
    }

    pub fn layout_root(&self, nodes: &[Node], size: f32, display: bool) -> MBox {
        let style = if display { MathStyle::Display } else { MathStyle::Text };
        self.list(nodes, style, size, None)
    }

    /// Lay out a math list with TeX inter-atom spacing.
    fn list(&self, nodes: &[Node], mut style: MathStyle, base: f32, lead: Option<Class>) -> MBox {
        let mut atoms: Vec<(MBox, Class, MathStyle)> = Vec::new();
        for n in nodes {
            if let Node::StyleSwitch(s) = n {
                style = *s;
                continue;
            }
            let (b, c) = self.node(n, style, base);
            atoms.push((b, c, style));
        }
        // TeX rule 5/6: a Bin in a non-binary position becomes Ord.
        let n = atoms.len();
        for i in 0..n {
            if atoms[i].1 != Class::Bin {
                continue;
            }
            let prev = if i == 0 { lead } else { Some(atoms[i - 1].1) };
            let next = atoms.get(i + 1).map(|a| a.1);
            let bad_prev = matches!(prev, None | Some(Class::Bin | Class::Op | Class::Rel | Class::Open | Class::Punct));
            let bad_next = matches!(next, None | Some(Class::Rel | Class::Close | Class::Punct));
            if bad_prev || bad_next {
                atoms[i].1 = Class::Ord;
            }
        }
        let mut out = MBox::empty();
        let mut prev = lead;
        for (b, c, st) in atoms {
            if let Some(p) = prev {
                let tight = matches!(st, MathStyle::Script | MathStyle::ScriptScript);
                let sp = spacing(p, c, tight && !(p == Class::Op || c == Class::Op));
                out.w += sp * base * scale(st);
            }
            out.hcat(b);
            prev = Some(c);
        }
        out
    }

    fn node(&self, node: &Node, style: MathStyle, base: f32) -> (MBox, Class) {
        let s = base * scale(style);
        match node {
            Node::Glyph { text, class, glyphs } => (self.text_box(text, s, *glyphs), *class),
            Node::Op { text, glyphs, integral, .. } => (self.op_symbol(text, *glyphs, *integral, style, s), Class::Op),
            Node::Group(nodes, class) => (self.list(nodes, style, base, None), *class),
            Node::Space(em) => (MBox { w: em * s, ..MBox::empty() }, Class::Ord),
            Node::Frac { num, den, bar, style: forced, delims } => {
                let st = forced.unwrap_or(style);
                let b = self.frac(num, den, *bar, st, base);
                match delims {
                    Some((l, r)) => (self.wrap_delims(b, l, r, base * scale(st)), Class::Inner),
                    None => (b, Class::Inner),
                }
            }
            Node::Sqrt { body, index } => (self.sqrt(body, index.as_deref(), style, base), Class::Ord),
            Node::Scripts { base: b, sup, sub } => self.scripts(b, sup.as_deref(), sub.as_deref(), style, base),
            Node::Accent { body, kind, wide } => (self.accent(body, *kind, *wide, style, base), Class::Ord),
            Node::Delimited { left, right, body } => {
                let inner = self.list(body, style, base, None);
                (self.wrap_delims(inner, left, right, s), Class::Inner)
            }
            Node::BigDelim { delim, scale: k, class } => {
                let half = k * s * 0.5;
                (self.delimiter(delim, -AXIS * s - half, -AXIS * s + half, s), *class)
            }
            Node::Matrix { rows, kind, aligns, left, right } => {
                let m = self.matrix(rows, *kind, aligns, style, base);
                (self.wrap_delims(m, left, right, s), Class::Inner)
            }
            Node::Color(c, nodes) => {
                let mut b = self.list(nodes, style, base, None);
                b.items.iter_mut().for_each(|i| i.set_color_if_none(*c));
                (b, Class::Ord)
            }
            Node::Boxed(inner) => {
                let (b, _) = self.node(inner, style, base);
                let pad = 0.25 * s;
                let mut out = MBox { w: b.w + 2.0 * pad, asc: b.asc + pad, desc: b.desc + pad, items: vec![] };
                out.items.push(Item::Frame {
                    rect: Rect::from_min_max(pos2(0.0, -b.asc - pad), pos2(b.w + 2.0 * pad, b.desc + pad)),
                    width: 0.04 * s,
                    color: None,
                });
                out.place(b, pad, 0.0);
                (out, Class::Ord)
            }
            Node::Stack { body, over, under } => {
                let (b, c) = self.node(body, style, base);
                (self.stack(b, over.as_deref(), under.as_deref(), style, base), c)
            }
            Node::Phantom(inner) => {
                let (mut b, _) = self.node(inner, style, base);
                b.items.clear();
                (b, Class::Ord)
            }
            Node::StyleSwitch(_) => (MBox::empty(), Class::Ord),
            Node::Error(e) => {
                let mut b = self.text_box(e, s, Glyphs::Text);
                b.items.iter_mut().for_each(|i| i.set_color_if_none(Color32::from_rgb(0xe0, 0x4f, 0x4f)));
                (b, Class::Ord)
            }
        }
    }

    fn op_symbol(&self, text: &str, glyphs: Glyphs, integral: bool, style: MathStyle, s: f32) -> MBox {
        if glyphs == Glyphs::Text {
            return self.text_box(text, s, Glyphs::Text);
        }
        let display = style == MathStyle::Display;
        let k = match (integral, display) {
            (true, true) => 2.0,
            (true, false) => 1.3,
            (false, true) => 1.55,
            (false, false) => 1.1,
        };
        let f = s * k;
        let mut b = self.text_box(text, f, Glyphs::Math);
        let (a, d) = glyph_extent(text, f);
        // Centre the glyph on the math axis of the surrounding size.
        let centre = (a - d) / 2.0;
        let dy = centre - AXIS * s;
        b.items.iter_mut().for_each(|i| i.translate(vec2(0.0, dy)));
        b.asc = a - dy;
        b.desc = d + dy;
        if integral {
            b.w += 0.08 * s;
        }
        b
    }

    fn frac(&self, num: &Node, den: &Node, bar: bool, style: MathStyle, base: f32) -> MBox {
        let s = base * scale(style);
        let inner = frac_style(style);
        let (n, _) = self.node(num, inner, base);
        let (d, _) = self.node(den, inner, base);
        let t = (0.045 * s).max(1.0);
        let gap = if style == MathStyle::Display { 0.18 * s } else { 0.1 * s };
        let pad = 0.12 * s;
        let w = n.w.max(d.w) + 2.0 * pad;
        let axis = AXIS * s;
        let num_y = -(axis + t / 2.0 + gap + n.desc);
        let den_y = -axis + t / 2.0 + gap + d.asc;
        let mut out = MBox { w, ..MBox::empty() };
        let (nw, dw) = (n.w, d.w);
        out.place(n, (w - nw) / 2.0, num_y);
        out.place(d, (w - dw) / 2.0, den_y);
        if bar {
            out.items.push(Item::Rule {
                rect: Rect::from_min_max(pos2(pad * 0.4, -axis - t / 2.0), pos2(w - pad * 0.4, -axis + t / 2.0)),
                color: None,
            });
        }
        out
    }

    fn sqrt(&self, body: &Node, index: Option<&Node>, style: MathStyle, base: f32) -> MBox {
        let s = base * scale(style);
        let (b, _) = self.node(body, style, base);
        let t = (0.05 * s).max(1.0);
        let gap = 0.12 * s;
        let asc = b.asc.max(0.65 * s);
        let desc = b.desc.max(0.1 * s);
        let top = -(asc + gap);
        let bottom = desc + 0.04 * s;
        let h = bottom - top;
        let surd_w = 0.42 * s + 0.06 * h;
        let mut shift = 0.0;
        let mut index_box = None;
        if let Some(ix) = index {
            let (ib, _) = self.node(ix, MathStyle::ScriptScript, base);
            shift = (ib.w - surd_w * 0.55).max(0.0);
            index_box = Some(ib);
        }
        let x0 = shift;
        let mid = top + h * 0.62;
        let pts = vec![
            pos2(x0, mid),
            pos2(x0 + surd_w * 0.2, mid - 0.06 * s),
            pos2(x0 + surd_w * 0.5, bottom),
            pos2(x0 + surd_w, top),
            pos2(x0 + surd_w + b.w + 0.12 * s, top),
        ];
        let mut out = MBox { w: x0 + surd_w + b.w + 0.16 * s, asc: -top + t, desc: bottom, items: vec![] };
        out.items.push(Item::Polyline { pts, width: t, color: None });
        if let Some(ib) = index_box {
            let y = mid - 0.08 * s - ib.desc;
            out.place(ib, 0.0, y);
        }
        out.place(b, x0 + surd_w + 0.06 * s, 0.0);
        out
    }

    fn scripts(&self, base_node: &Node, sup: Option<&Node>, sub: Option<&Node>, style: MathStyle, base: f32) -> (MBox, Class) {
        let s = base * scale(style);
        let (b, class) = self.node(base_node, style, base);
        let limits = match base_node {
            Node::Op { limits, integral, .. } => limits.unwrap_or(style == MathStyle::Display && !*integral),
            Node::Accent { kind: AccentKind::OverBrace | AccentKind::UnderBrace, .. } => true,
            _ => false,
        };
        let ss = sup_style(style);
        let sup_b = sup.map(|n| self.node(n, ss, base).0);
        let sub_b = sub.map(|n| self.node(n, ss, base).0);
        if limits {
            return (self.stack_boxes(b, sup_b, sub_b, s), class);
        }
        let italic = matches!(base_node, Node::Glyph { text, .. } if is_italic_letter(text));
        let integral = matches!(base_node, Node::Op { integral: true, .. });
        let cramped_sup = 0.38 * s;
        let mut sup_y = -(cramped_sup.max(b.asc - 0.32 * s));
        let mut sub_y = (0.18 * s).max(b.desc - 0.05 * s);
        if let (Some(p), Some(q)) = (&sup_b, &sub_b) {
            sub_y = sub_y.max(0.26 * s);
            let gap = (sub_y - q.asc) - (sup_y + p.desc);
            if gap < 0.12 * s {
                let fix = 0.12 * s - gap;
                sup_y -= fix / 2.0;
                sub_y += fix / 2.0;
            }
        }
        let bw = b.w;
        let mut out = b;
        let sup_x = bw + if italic { 0.06 * s } else if integral { 0.02 * s } else { 0.02 * s };
        let sub_x = if integral { bw - 0.22 * s } else { bw + 0.01 * s };
        if let Some(p) = sup_b {
            out.place(p, sup_x, sup_y);
        }
        if let Some(q) = sub_b {
            out.place(q, sub_x.max(0.0), sub_y);
        }
        out.w += 0.04 * s;
        (out, class)
    }

    fn stack_boxes(&self, b: MBox, over: Option<MBox>, under: Option<MBox>, s: f32) -> MBox {
        let w = b.w.max(over.as_ref().map_or(0.0, |o| o.w)).max(under.as_ref().map_or(0.0, |u| u.w));
        let gap = 0.12 * s;
        let mut out = MBox { w, ..MBox::empty() };
        let (basc, bdesc, bw) = (b.asc, b.desc, b.w);
        out.place(b, (w - bw) / 2.0, 0.0);
        if let Some(o) = over {
            let y = -(basc + gap + o.desc);
            let ow = o.w;
            out.place(o, (w - ow) / 2.0, y);
        }
        if let Some(u) = under {
            let y = bdesc + gap + u.asc;
            let uw = u.w;
            out.place(u, (w - uw) / 2.0, y);
        }
        out
    }

    fn stack(&self, b: MBox, over: Option<&Node>, under: Option<&Node>, style: MathStyle, base: f32) -> MBox {
        let s = base * scale(style);
        let ss = sup_style(style);
        let o = over.map(|n| self.node(n, ss, base).0);
        let u = under.map(|n| self.node(n, ss, base).0);
        self.stack_boxes(b, o, u, s)
    }

    fn accent(&self, body: &Node, kind: AccentKind, wide: bool, style: MathStyle, base: f32) -> MBox {
        let s = base * scale(style);
        let (b, _) = self.node(body, style, base);
        let italic = matches!(body, Node::Glyph { text, .. } if is_italic_letter(text));
        let skew = if italic { 0.08 * s } else { 0.0 };
        let t = (0.045 * s).max(1.0);
        let top = -(b.asc.max(0.46 * s) + 0.08 * s);
        let (x0, x1) = if wide || matches!(kind, AccentKind::Overline | AccentKind::Underline | AccentKind::OverArrow | AccentKind::OverLeftArrow | AccentKind::OverBrace | AccentKind::UnderBrace) {
            (0.02 * s, b.w - 0.02 * s)
        } else {
            let aw = (b.w * 0.8).clamp(0.28 * s, 0.42 * s);
            let c = b.w / 2.0 + skew;
            (c - aw / 2.0, c + aw / 2.0)
        };
        let xm = (x0 + x1) / 2.0;
        let ah = 0.12 * s;
        let mut items = Vec::new();
        let mut extra_above = 0.18 * s;
        let mut extra_below = 0.0;
        match kind {
            AccentKind::Hat => items.push(Item::Polyline { pts: vec![pos2(x0, top), pos2(xm, top - ah), pos2(x1, top)], width: t, color: None }),
            AccentKind::Check => items.push(Item::Polyline { pts: vec![pos2(x0, top - ah), pos2(xm, top), pos2(x1, top - ah)], width: t, color: None }),
            AccentKind::Tilde => {
                let y = top - ah * 0.5;
                let d = (x1 - x0) / 3.0;
                items.push(Item::Bezier { pts: [pos2(x0, y + ah * 0.3), pos2(x0 + d, y - ah * 0.9), pos2(x1 - d, y + ah * 0.9), pos2(x1, y - ah * 0.3)], width: t, color: None });
            }
            AccentKind::Bar | AccentKind::Overline => {
                let y = top - 0.02 * s;
                items.push(Item::Rule { rect: Rect::from_min_max(pos2(x0, y - t / 2.0), pos2(x1, y + t / 2.0)), color: None });
                extra_above = 0.1 * s;
            }
            AccentKind::Underline => {
                let y = b.desc + 0.1 * s;
                items.push(Item::Rule { rect: Rect::from_min_max(pos2(x0, y - t / 2.0), pos2(x1, y + t / 2.0)), color: None });
                extra_above = 0.0;
                extra_below = 0.16 * s;
            }
            AccentKind::Vec | AccentKind::OverArrow | AccentKind::OverLeftArrow => {
                let y = top - 0.06 * s;
                let head = 0.1 * s;
                let left = kind == AccentKind::OverLeftArrow;
                let (tip, tail) = if left { (x0, x1) } else { (x1, x0) };
                let dir = if left { 1.0 } else { -1.0 };
                items.push(Item::Polyline { pts: vec![pos2(tail, y), pos2(tip, y)], width: t, color: None });
                items.push(Item::Polyline { pts: vec![pos2(tip + dir * head, y - head * 0.7), pos2(tip, y), pos2(tip + dir * head, y + head * 0.7)], width: t, color: None });
            }
            AccentKind::Dot => items.push(Item::Dot { center: pos2(xm, top - 0.04 * s), radius: 0.055 * s, color: None }),
            AccentKind::Ddot => {
                items.push(Item::Dot { center: pos2(xm - 0.1 * s, top - 0.04 * s), radius: 0.05 * s, color: None });
                items.push(Item::Dot { center: pos2(xm + 0.1 * s, top - 0.04 * s), radius: 0.05 * s, color: None });
            }
            AccentKind::Breve => items.push(Item::Bezier { pts: [pos2(x0, top - ah), pos2(x0, top + ah * 0.2), pos2(x1, top + ah * 0.2), pos2(x1, top - ah)], width: t, color: None }),
            AccentKind::Acute => items.push(Item::Polyline { pts: vec![pos2(xm - 0.04 * s, top), pos2(xm + 0.08 * s, top - ah * 1.1)], width: t * 1.2, color: None }),
            AccentKind::Grave => items.push(Item::Polyline { pts: vec![pos2(xm + 0.04 * s, top), pos2(xm - 0.08 * s, top - ah * 1.1)], width: t * 1.2, color: None }),
            AccentKind::OverBrace | AccentKind::UnderBrace => {
                let up = kind == AccentKind::OverBrace;
                let bh = 0.22 * s;
                let y_edge = if up { top + 0.02 * s } else { b.desc + 0.08 * s };
                items.extend(hbrace(x0, x1, y_edge, bh, up, t));
                if up {
                    extra_above = bh + 0.06 * s;
                } else {
                    extra_above = 0.0;
                    extra_below = bh + 0.14 * s;
                }
            }
        }
        let asc = if extra_above > 0.0 { (-top + extra_above).max(b.asc) } else { b.asc };
        let desc = b.desc + extra_below;
        let mut out = MBox { w: b.w, asc, desc, items };
        out.place(b, 0.0, 0.0);
        out
    }

    fn wrap_delims(&self, inner: MBox, left: &str, right: &str, s: f32) -> MBox {
        if left.is_empty() && right.is_empty() {
            return inner;
        }
        let axis = AXIS * s;
        let half = (inner.asc - axis).max(inner.desc + axis).max(0.5 * s) + 0.08 * s;
        let (top, bottom) = (-axis - half, -axis + half);
        let mut out = MBox::empty();
        if !left.is_empty() {
            out.hcat(self.delimiter(left, top, bottom, s));
            out.w += 0.04 * s;
        }
        out.hcat(inner);
        if !right.is_empty() {
            out.w += 0.04 * s;
            out.hcat(self.delimiter(right, top, bottom, s));
        }
        out
    }

    /// A delimiter spanning `top..bottom` (baseline-relative y).
    fn delimiter(&self, d: &str, top: f32, bottom: f32, s: f32) -> MBox {
        if d.is_empty() {
            return MBox { w: 0.1 * s, ..MBox::empty() };
        }
        let h = bottom - top;
        if h <= 1.1 * s {
            return self.text_box(d, s, Glyphs::Math);
        }
        let t = (0.05 * s).max(1.0);
        let mid = (top + bottom) / 2.0;
        let w = (0.32 + 0.04 * (h / s).min(4.0)) * s;
        let mut items = Vec::new();
        let mirror = |p: Pos2| pos2(w - p.x, p.y);
        let paren = |items: &mut Vec<Item>, flip: bool| {
            let f = |p: Pos2| if flip { mirror(p) } else { p };
            let bulge = 0.18 * w;
            items.push(Item::Bezier { pts: [f(pos2(0.85 * w, top)), f(pos2(bulge, top + h * 0.22)), f(pos2(bulge, bottom - h * 0.22)), f(pos2(0.85 * w, bottom))], width: t * 0.8, color: None });
            items.push(Item::Bezier { pts: [f(pos2(0.85 * w, top)), f(pos2(bulge + 0.12 * s, top + h * 0.25)), f(pos2(bulge + 0.12 * s, bottom - h * 0.25)), f(pos2(0.85 * w, bottom))], width: t * 0.8, color: None });
        };
        let bracket = |items: &mut Vec<Item>, flip: bool, top_bar: bool, bottom_bar: bool| {
            let f = |p: Pos2| if flip { mirror(p) } else { p };
            let mut pts = Vec::new();
            if top_bar {
                pts.push(f(pos2(0.85 * w, top)));
            }
            pts.push(f(pos2(0.3 * w, top)));
            pts.push(f(pos2(0.3 * w, bottom)));
            if bottom_bar {
                pts.push(f(pos2(0.85 * w, bottom)));
            }
            items.push(Item::Polyline { pts, width: t, color: None });
        };
        let brace = |items: &mut Vec<Item>, flip: bool| {
            let f = |p: Pos2| if flip { mirror(p) } else { p };
            let x = 0.5 * w;
            let c = (0.2 * h).min(0.35 * s);
            for (a, sign) in [(top, 1.0f32), (bottom, -1.0)] {
                let stem_end = mid - sign * c;
                items.push(Item::Bezier { pts: [f(pos2(0.9 * w, a)), f(pos2(x, a)), f(pos2(x, a + sign * c * 0.3)), f(pos2(x, a + sign * c))], width: t, color: None });
                items.push(Item::Polyline { pts: vec![f(pos2(x, a + sign * c)), f(pos2(x, stem_end))], width: t, color: None });
                items.push(Item::Bezier { pts: [f(pos2(x, stem_end)), f(pos2(x, mid - sign * c * 0.3)), f(pos2(x - 0.1 * w, mid)), f(pos2(0.1 * w, mid))], width: t, color: None });
            }
        };
        match d {
            "(" => paren(&mut items, false),
            ")" => paren(&mut items, true),
            "[" => bracket(&mut items, false, true, true),
            "]" => bracket(&mut items, true, true, true),
            "⌊" => bracket(&mut items, false, false, true),
            "⌋" => bracket(&mut items, true, false, true),
            "⌈" => bracket(&mut items, false, true, false),
            "⌉" => bracket(&mut items, true, true, false),
            "{" => brace(&mut items, false),
            "}" => brace(&mut items, true),
            "|" => items.push(Item::Polyline { pts: vec![pos2(w / 2.0, top), pos2(w / 2.0, bottom)], width: t, color: None }),
            "‖" => {
                items.push(Item::Polyline { pts: vec![pos2(w * 0.35, top), pos2(w * 0.35, bottom)], width: t, color: None });
                items.push(Item::Polyline { pts: vec![pos2(w * 0.65, top), pos2(w * 0.65, bottom)], width: t, color: None });
            }
            "⟨" => items.push(Item::Polyline { pts: vec![pos2(0.85 * w, top), pos2(0.2 * w, mid), pos2(0.85 * w, bottom)], width: t, color: None }),
            "⟩" => items.push(Item::Polyline { pts: vec![pos2(0.15 * w, top), pos2(0.8 * w, mid), pos2(0.15 * w, bottom)], width: t, color: None }),
            "/" => items.push(Item::Polyline { pts: vec![pos2(0.85 * w, top), pos2(0.15 * w, bottom)], width: t, color: None }),
            other => {
                let mut b = self.text_box(other, h, Glyphs::Math);
                let (a, de) = glyph_extent(other, h);
                let dy = bottom - de;
                b.items.iter_mut().for_each(|i| i.translate(vec2(0.0, dy - 0.0)));
                b.asc = a - dy;
                b.desc = de + dy;
                return b;
            }
        }
        MBox { w, asc: -top, desc: bottom, items }
    }

    fn matrix(&self, rows: &[Vec<Node>], kind: MatrixKind, aligns: &[ColAlign], style: MathStyle, base: f32) -> MBox {
        let s = base * scale(style);
        let cell_style = match kind {
            MatrixKind::Aligned | MatrixKind::Gathered => if style == MathStyle::Display { MathStyle::Display } else { style },
            _ => if style == MathStyle::Display { MathStyle::Text } else { style },
        };
        let cells: Vec<Vec<MBox>> = rows
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(j, cell)| {
                        let lead = (kind == MatrixKind::Aligned && j % 2 == 1).then_some(Class::Ord);
                        match cell {
                            Node::Group(nodes, _) => self.list(nodes, cell_style, base, lead),
                            other => self.node(other, cell_style, base).0,
                        }
                    })
                    .collect()
            })
            .collect();
        let ncols = cells.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut col_w = vec![0.0f32; ncols];
        for row in &cells {
            for (j, c) in row.iter().enumerate() {
                col_w[j] = col_w[j].max(c.w);
            }
        }
        let col_gap = |j: usize| -> f32 {
            match kind {
                MatrixKind::Aligned => if j % 2 == 0 { 0.0 } else { 2.0 * s },
                MatrixKind::Cases => 1.0 * s,
                MatrixKind::Gathered => 0.0,
                _ => 0.9 * s,
            }
        };
        let align = |j: usize| -> ColAlign {
            match kind {
                MatrixKind::Aligned => if j % 2 == 0 { ColAlign::Right } else { ColAlign::Left },
                MatrixKind::Cases => ColAlign::Left,
                MatrixKind::Array => aligns.get(j).copied().unwrap_or(ColAlign::Center),
                _ => ColAlign::Center,
            }
        };
        let row_gap = match kind {
            MatrixKind::Aligned | MatrixKind::Gathered => 0.28 * s,
            _ => 0.22 * s,
        };
        let strut_a = 0.72 * s;
        let strut_d = 0.28 * s;
        let heights: Vec<(f32, f32)> = cells
            .iter()
            .map(|r| {
                let a = r.iter().map(|c| c.asc).fold(strut_a, f32::max);
                let d = r.iter().map(|c| c.desc).fold(strut_d, f32::max);
                (a, d)
            })
            .collect();
        let total_h: f32 = heights.iter().map(|(a, d)| a + d).sum::<f32>() + row_gap * (heights.len().saturating_sub(1)) as f32;
        let total_w: f32 = col_w.iter().sum::<f32>() + (0..ncols.saturating_sub(1)).map(col_gap).sum::<f32>();
        let axis = AXIS * s;
        let mut y = -axis - total_h / 2.0;
        let pad = if matches!(kind, MatrixKind::Matrix | MatrixKind::Array) { 0.1 * s } else { 0.0 };
        let mut out = MBox { w: total_w + 2.0 * pad, asc: axis + total_h / 2.0, desc: total_h / 2.0 - axis, items: vec![] };
        for (row, (a, d)) in cells.into_iter().zip(heights) {
            let baseline = y + a;
            let mut x = pad;
            for (j, c) in row.into_iter().enumerate() {
                let slack = col_w[j] - c.w;
                let dx = match align(j) {
                    ColAlign::Left => 0.0,
                    ColAlign::Center => slack / 2.0,
                    ColAlign::Right => slack,
                };
                out.place(c, x + dx, baseline);
                x += col_w[j] + col_gap(j);
            }
            y += a + d + row_gap;
        }
        out
    }
}

/// A horizontal curly brace between x0 and x1, with its flat edge at `y_edge`.
fn hbrace(x0: f32, x1: f32, y_edge: f32, h: f32, up: bool, t: f32) -> Vec<Item> {
    let sign = if up { -1.0 } else { 1.0 };
    let tip_y = y_edge + sign * h;
    let mid_y = y_edge + sign * h * 0.5;
    let xm = (x0 + x1) / 2.0;
    let c = ((x1 - x0) * 0.1).min(h * 1.2);
    let mut items = Vec::new();
    for (a, dir) in [(x0, 1.0f32), (x1, -1.0)] {
        items.push(Item::Bezier { pts: [pos2(a, y_edge), pos2(a, mid_y), pos2(a + dir * c * 0.3, mid_y), pos2(a + dir * c, mid_y)], width: t, color: None });
        items.push(Item::Polyline { pts: vec![pos2(a + dir * c, mid_y), pos2(xm - dir * c, mid_y)], width: t, color: None });
        items.push(Item::Bezier { pts: [pos2(xm - dir * c, mid_y), pos2(xm - dir * c * 0.3, mid_y), pos2(xm, mid_y), pos2(xm, tip_y)], width: t, color: None });
    }
    items
}

/// Paint a laid-out formula with its baseline-left corner at `origin`.
pub fn paint(ctx: &egui::Context, painter: &Painter, origin: Pos2, b: &MBox, color: Color32) {
    for item in &b.items {
        match item {
            Item::Text { pos, text, size, glyphs, color: c } => {
                let col = c.unwrap_or(color);
                let galley = ctx.fonts_mut(|f| f.layout_no_wrap(text.clone(), FontId::new(*size, family(*glyphs)), col));
                let baseline = galley
                    .rows
                    .first()
                    .and_then(|r| r.row.glyphs.first().map(|g| r.pos.y + g.pos.y))
                    .unwrap_or(size * 0.8);
                painter.galley(origin + *pos - vec2(0.0, baseline), galley, col);
            }
            Item::Rule { rect, color: c } => {
                painter.rect_filled(rect.translate(origin.to_vec2()), 0.0, c.unwrap_or(color));
            }
            Item::Frame { rect, width, color: c } => {
                painter.rect_stroke(rect.translate(origin.to_vec2()), 2.0, Stroke::new(*width, c.unwrap_or(color)), egui::StrokeKind::Middle);
            }
            Item::Polyline { pts, width, color: c } => {
                let pts: Vec<Pos2> = pts.iter().map(|p| *p + origin.to_vec2()).collect();
                painter.add(Shape::line(pts, Stroke::new(*width, c.unwrap_or(color))));
            }
            Item::Bezier { pts, width, color: c } => {
                let pts = pts.map(|p| p + origin.to_vec2());
                painter.add(egui::epaint::CubicBezierShape::from_points_stroke(pts, false, Color32::TRANSPARENT, Stroke::new(*width, c.unwrap_or(color))));
            }
            Item::Dot { center, radius, color: c } => {
                painter.circle_filled(*center + origin.to_vec2(), *radius, c.unwrap_or(color));
            }
        }
    }
}
