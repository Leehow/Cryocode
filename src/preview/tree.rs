//! Collapsible tree views for structured data: JSON, YAML, TOML and XML.

use crate::formats::Format;
use crate::theme::Palette;
use egui::collapsing_header::CollapsingState;
use egui::{vec2, Color32, FontId, Id, RichText, Sense};

/// Format-neutral data tree for JSON / YAML / TOML.
#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

impl Value {
    fn from_json(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(b) => Self::Bool(b),
            serde_json::Value::Number(n) => Self::Num(n.to_string()),
            serde_json::Value::String(s) => Self::Str(s),
            serde_json::Value::Array(a) => Self::Arr(a.into_iter().map(Self::from_json).collect()),
            serde_json::Value::Object(o) => Self::Obj(o.into_iter().map(|(k, v)| (k, Self::from_json(v))).collect()),
        }
    }

    fn from_yaml(v: serde_yaml::Value) -> Self {
        use serde_yaml::Value as Y;
        match v {
            Y::Null => Self::Null,
            Y::Bool(b) => Self::Bool(b),
            Y::Number(n) => Self::Num(n.to_string()),
            Y::String(s) => Self::Str(s),
            Y::Sequence(a) => Self::Arr(a.into_iter().map(Self::from_yaml).collect()),
            Y::Mapping(m) => Self::Obj(
                m.into_iter()
                    .map(|(k, v)| {
                        let key = match k {
                            Y::String(s) => s,
                            other => serde_yaml::to_string(&other).unwrap_or_default().trim().to_owned(),
                        };
                        (key, Self::from_yaml(v))
                    })
                    .collect(),
            ),
            Y::Tagged(t) => Self::from_yaml(t.value),
        }
    }

    fn from_toml(v: toml::Value) -> Self {
        match v {
            toml::Value::String(s) => Self::Str(s),
            toml::Value::Integer(i) => Self::Num(i.to_string()),
            toml::Value::Float(f) => Self::Num(f.to_string()),
            toml::Value::Boolean(b) => Self::Bool(b),
            toml::Value::Datetime(d) => Self::Str(d.to_string()),
            toml::Value::Array(a) => Self::Arr(a.into_iter().map(Self::from_toml).collect()),
            toml::Value::Table(t) => Self::Obj(t.into_iter().map(|(k, v)| (k, Self::from_toml(v))).collect()),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Num(n) => n.parse::<serde_json::Number>().map(serde_json::Value::Number).unwrap_or_else(|_| serde_json::Value::String(n.clone())),
            Value::Str(s) => serde_json::Value::String(s.clone()),
            Value::Arr(a) => serde_json::Value::Array(a.iter().map(Value::to_json).collect()),
            Value::Obj(o) => serde_json::Value::Object(o.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()),
        }
    }

    /// Short single-line rendering used in table cells and collapsed rows.
    pub fn brief(&self) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Bool(b) => b.to_string(),
            Value::Num(n) => n.clone(),
            Value::Str(s) => s.clone(),
            Value::Arr(a) => format!("[{}]", a.len()),
            Value::Obj(o) => format!("{{{}}}", o.len()),
        }
    }
}

pub fn parse_data(format: Format, text: &str) -> Result<Value, String> {
    match format {
        Format::Json => {
            let cleaned;
            let src = if text.contains("//") || text.contains("/*") {
                cleaned = strip_json_comments(text);
                cleaned.as_str()
            } else {
                text
            };
            serde_json::from_str::<serde_json::Value>(src).map(Value::from_json).map_err(|e| format!("JSON 解析失败：第 {} 行第 {} 列：{e}", e.line(), e.column()))
        }
        Format::Yaml => {
            let mut docs = Vec::new();
            for d in serde_yaml::Deserializer::from_str(text) {
                let v = <serde_yaml::Value as serde::Deserialize>::deserialize(d).map_err(|e| format!("YAML 解析失败：{e}"))?;
                docs.push(Value::from_yaml(v));
            }
            Ok(if docs.len() == 1 { docs.pop().unwrap() } else { Value::Arr(docs) })
        }
        Format::Toml => toml::from_str::<toml::Value>(text).map(Value::from_toml).map_err(|e| format!("TOML 解析失败：{e}")),
        _ => Err("不支持的格式".into()),
    }
}

/// JSONC support: drop `//` and `/* */` comments and trailing commas outside strings.
fn strip_json_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                    }
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }
    // Trailing commas.
    let mut result = String::with_capacity(out.len());
    let bytes: Vec<char> = out.chars().collect();
    let mut in_str = false;
    for (i, &c) in bytes.iter().enumerate() {
        if c == '"' && (i == 0 || bytes[i - 1] != '\\') {
            in_str = !in_str;
        }
        if c == ',' && !in_str {
            let next = bytes[i + 1..].iter().find(|c| !c.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        result.push(c);
    }
    result
}

const PAGE: usize = 200;

pub struct TreeUi<'a> {
    pub palette: Palette,
    pub copied: &'a mut Option<String>,
}

impl TreeUi<'_> {
    pub fn show_root(&mut self, ui: &mut egui::Ui, v: &Value, id: Id) {
        ui.spacing_mut().item_spacing.y = 2.0;
        self.node(ui, None, v, id, 0, "$".into());
    }

    fn key_label(&self, key: Option<&str>) -> Option<RichText> {
        key.map(|k| RichText::new(k).color(self.palette.keyword).monospace())
    }

    fn value_text(&self, v: &Value) -> RichText {
        let p = &self.palette;
        let (t, c) = match v {
            Value::Null => ("null".to_owned(), p.text_faint),
            Value::Bool(b) => (b.to_string(), p.number),
            Value::Num(n) => (n.clone(), p.number),
            Value::Str(s) => {
                let shown = if s.chars().count() > 300 { format!("\"{}…\"", s.chars().take(300).collect::<String>()) } else { format!("\"{s}\"") };
                (shown, p.string)
            }
            _ => (v.brief(), p.text_dim),
        };
        RichText::new(t).color(c).monospace()
    }

    fn node(&mut self, ui: &mut egui::Ui, key: Option<&str>, v: &Value, id: Id, depth: usize, path: String) {
        let children = match v {
            Value::Arr(a) => a.len(),
            Value::Obj(o) => o.len(),
            _ => 0,
        };
        let container = matches!(v, Value::Arr(_) | Value::Obj(_));
        if !container || children == 0 {
            let resp = ui
                .horizontal(|ui| {
                    ui.add_space(18.0);
                    if let Some(k) = self.key_label(key) {
                        ui.label(k);
                        ui.label(RichText::new(":").color(self.palette.text_faint).monospace());
                    }
                    ui.add(egui::Label::new(self.value_text(v)).sense(Sense::click()))
                })
                .inner;
            self.context_menu(&resp, &path, v);
            return;
        }
        let state = CollapsingState::load_with_default_open(ui.ctx(), id, depth < 2);
        let (open_char, close_char) = if matches!(v, Value::Arr(_)) { ("[", "]") } else { ("{", "}") };
        let header = state.show_header(ui, |ui| {
            if let Some(k) = self.key_label(key) {
                ui.label(k);
                ui.label(RichText::new(":").color(self.palette.text_faint).monospace());
            }
            let resp = ui.add(egui::Label::new(RichText::new(format!("{open_char}{children}{close_char}")).color(self.palette.text_faint).monospace()).sense(Sense::click()));
            self.context_menu(&resp, &path, v);
        });
        header.body(|ui| {
            let shown_id = id.with("shown");
            let shown = ui.data(|d| d.get_temp::<usize>(shown_id)).unwrap_or(PAGE);
            match v {
                Value::Arr(a) => {
                    for (i, c) in a.iter().enumerate().take(shown) {
                        self.node(ui, Some(&i.to_string()), c, id.with(i), depth + 1, format!("{path}[{i}]"));
                    }
                }
                Value::Obj(o) => {
                    for (i, (k, c)) in o.iter().enumerate().take(shown) {
                        let p = if k.chars().all(|c| c.is_alphanumeric() || c == '_') { format!("{path}.{k}") } else { format!("{path}[{k:?}]") };
                        self.node(ui, Some(k), c, id.with(i), depth + 1, p);
                    }
                }
                _ => {}
            }
            if children > shown {
                let more = ui.button(RichText::new(format!("显示更多（还有 {} 项）", children - shown)).color(self.palette.accent).size(12.0));
                if more.clicked() {
                    ui.data_mut(|d| d.insert_temp(shown_id, shown + PAGE * 5));
                }
            }
        });
    }

    fn context_menu(&mut self, resp: &egui::Response, path: &str, v: &Value) {
        resp.context_menu(|ui| {
            if ui.button("复制路径").clicked() {
                *self.copied = Some(path.to_owned());
                ui.close();
            }
            if ui.button("复制值 (JSON)").clicked() {
                *self.copied = Some(serde_json::to_string_pretty(&v.to_json()).unwrap_or_default());
                ui.close();
            }
        });
        if resp.hovered() {
            resp.ctx.set_cursor_icon(egui::CursorIcon::Default);
        }
    }
}

/// Tabular projection of an array of objects (rows) — used by the JSON "表格" mode.
pub fn as_table(v: &Value) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let Value::Arr(items) = v else { return None };
    if items.is_empty() || !items.iter().all(|i| matches!(i, Value::Obj(_))) {
        return None;
    }
    let mut cols: Vec<String> = Vec::new();
    for it in items.iter().take(2000) {
        if let Value::Obj(o) = it {
            for (k, _) in o {
                if !cols.contains(k) {
                    cols.push(k.clone());
                }
            }
        }
    }
    let rows = items
        .iter()
        .map(|it| {
            let Value::Obj(o) = it else { unreachable!() };
            cols.iter().map(|c| o.iter().find(|(k, _)| k == c).map(|(_, v)| v.brief()).unwrap_or_default()).collect()
        })
        .collect();
    Some((cols, rows))
}

// ---------------------------------------------------------------------------
// XML

pub enum XmlNode {
    Element { name: String, attrs: Vec<(String, String)>, children: Vec<XmlNode> },
    Text(String),
    Comment(String),
    Pi(String),
}

pub fn parse_xml(text: &str) -> Result<XmlNode, String> {
    let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(text, opts).map_err(|e| format!("XML 解析失败：{e}"))?;
    fn conv(n: roxmltree::Node) -> Option<XmlNode> {
        if n.is_element() {
            let tag = n.tag_name();
            let name = match n.lookup_prefix(tag.namespace().unwrap_or("")) {
                Some(p) if !p.is_empty() => format!("{p}:{}", tag.name()),
                _ => tag.name().to_owned(),
            };
            let attrs = n
                .attributes()
                .map(|a| {
                    let k = match a.namespace().and_then(|ns| n.lookup_prefix(ns)) {
                        Some(p) if !p.is_empty() => format!("{p}:{}", a.name()),
                        _ => a.name().to_owned(),
                    };
                    (k, a.value().to_owned())
                })
                .collect();
            Some(XmlNode::Element { name, attrs, children: n.children().filter_map(conv).collect() })
        } else if n.is_text() {
            let t = n.text().unwrap_or("").trim();
            (!t.is_empty()).then(|| XmlNode::Text(t.to_owned()))
        } else if n.is_comment() {
            Some(XmlNode::Comment(n.text().unwrap_or("").trim().to_owned()))
        } else if n.is_pi() {
            n.pi().map(|pi| XmlNode::Pi(format!("{} {}", pi.target, pi.value.unwrap_or(""))))
        } else {
            None
        }
    }
    conv(doc.root_element()).ok_or_else(|| "空文档".into())
}

/// Tabular projection of XML: the shallowest element whose child elements repeat one tag
/// (`<catalog><cd/><cd/>…`) becomes the rows; attributes (`@name`) and child elements are columns.
pub fn xml_table(root: &XmlNode) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let mut queue: std::collections::VecDeque<&XmlNode> = [root].into();
    while let Some(node) = queue.pop_front() {
        let XmlNode::Element { children, .. } = node else { continue };
        let mut counts: Vec<(&str, usize)> = Vec::new();
        for c in children {
            if let XmlNode::Element { name, .. } = c {
                match counts.iter_mut().find(|(n, _)| n == name) {
                    Some(e) => e.1 += 1,
                    None => counts.push((name, 1)),
                }
            }
        }
        if let Some(&(tag, n)) = counts.iter().max_by_key(|(_, n)| *n) {
            if n >= 2 {
                let records: Vec<&XmlNode> = children.iter().filter(|c| matches!(c, XmlNode::Element { name, .. } if name == tag)).collect();
                return Some(records_table(&records));
            }
        }
        queue.extend(children.iter());
    }
    None
}

fn element_text(n: &XmlNode) -> String {
    match n {
        XmlNode::Text(t) => t.clone(),
        XmlNode::Element { children, .. } => {
            let parts: Vec<String> = children.iter().map(element_text).filter(|s| !s.is_empty()).collect();
            parts.join(" ")
        }
        _ => String::new(),
    }
}

fn records_table(records: &[&XmlNode]) -> (Vec<String>, Vec<Vec<String>>) {
    let mut cols: Vec<String> = Vec::new();
    let add = |c: String, cols: &mut Vec<String>| {
        if !cols.contains(&c) {
            cols.push(c);
        }
    };
    for r in records.iter().take(2000) {
        if let XmlNode::Element { attrs, children, .. } = r {
            for (k, _) in attrs {
                add(format!("@{k}"), &mut cols);
            }
            for c in children {
                match c {
                    XmlNode::Element { name, .. } => add(name.clone(), &mut cols),
                    XmlNode::Text(_) => add("#text".into(), &mut cols),
                    _ => {}
                }
            }
        }
    }
    let rows = records
        .iter()
        .map(|r| {
            let XmlNode::Element { attrs, children, .. } = r else { return Vec::new() };
            cols.iter()
                .map(|c| {
                    if let Some(a) = c.strip_prefix('@') {
                        attrs.iter().find(|(k, _)| k == a).map(|(_, v)| v.clone()).unwrap_or_default()
                    } else if c == "#text" {
                        children.iter().filter_map(|n| if let XmlNode::Text(t) = n { Some(t.as_str()) } else { None }).collect::<Vec<_>>().join(" ")
                    } else {
                        let vals: Vec<String> = children
                            .iter()
                            .filter(|n| matches!(n, XmlNode::Element { name, .. } if name == c))
                            .map(element_text)
                            .collect();
                        vals.join(" | ")
                    }
                })
                .collect()
        })
        .collect();
    (cols, rows)
}

pub fn show_xml(ui: &mut egui::Ui, node: &XmlNode, p: &Palette, id: Id, depth: usize) {
    ui.spacing_mut().item_spacing.y = 2.0;
    let mono = |s: &str, c: Color32| RichText::new(s).monospace().color(c);
    match node {
        XmlNode::Text(t) => {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(18.0);
                ui.label(RichText::new(t).color(p.text));
            });
        }
        XmlNode::Comment(c) => {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(18.0);
                ui.label(mono(&format!("<!-- {c} -->"), p.text_faint));
            });
        }
        XmlNode::Pi(s) => {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                ui.label(mono(&format!("<?{s}?>"), p.text_faint));
            });
        }
        XmlNode::Element { name, attrs, children } => {
            let head = |ui: &mut egui::Ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label(mono("<", p.text_faint));
                ui.label(mono(name, p.keyword));
                for (k, v) in attrs.iter().take(12) {
                    ui.label(mono(&format!(" {k}"), p.number));
                    ui.label(mono("=", p.text_faint));
                    let v = if v.chars().count() > 80 { format!("\"{}…\"", v.chars().take(80).collect::<String>()) } else { format!("\"{v}\"") };
                    ui.label(mono(&v, p.string));
                }
                if attrs.len() > 12 {
                    ui.label(mono(&format!(" …+{}", attrs.len() - 12), p.text_faint));
                }
                ui.label(mono(if children.is_empty() { " />" } else { ">" }, p.text_faint));
            };
            // An element holding only short text renders on one line: <name>text</name>.
            if let [XmlNode::Text(t)] = children.as_slice() {
                if t.chars().count() < 120 {
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        head(ui);
                        ui.label(RichText::new(t).color(p.text));
                        ui.label(mono(&format!("</{name}>"), p.text_faint));
                    });
                    return;
                }
            }
            if children.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    head(ui);
                });
                return;
            }
            let state = CollapsingState::load_with_default_open(ui.ctx(), id, depth < 3);
            state
                .show_header(ui, head)
                .body(|ui| {
                    let shown_id = id.with("shown");
                    let shown = ui.data(|d| d.get_temp::<usize>(shown_id)).unwrap_or(PAGE);
                    for (i, c) in children.iter().enumerate().take(shown) {
                        show_xml(ui, c, p, id.with(i), depth + 1);
                    }
                    if children.len() > shown && ui.button(format!("显示更多（还有 {} 个节点）", children.len() - shown)).clicked() {
                        ui.data_mut(|d| d.insert_temp(shown_id, shown + PAGE * 5));
                    }
                });
        }
    }
}

pub fn error_box(ui: &mut egui::Ui, p: &Palette, msg: &str) {
    egui::Frame::new()
        .fill(p.warn.gamma_multiply(0.08))
        .stroke(egui::Stroke::new(1.0_f32, p.warn.gamma_multiply(0.5)))
        .corner_radius(8.0)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.label(RichText::new("无法渲染").color(p.warn).strong());
            ui.add_space(4.0);
            ui.label(RichText::new(msg).color(p.text).font(FontId::monospace(12.5)));
            ui.add_space(4.0);
            ui.label(RichText::new("切换到「源码」修改后会自动重新渲染。").color(p.text_dim).size(12.0));
        });
    ui.add_space(vec2(0.0, 4.0).y);
}
