//! ⌘P quick open, ⌘⇧P command palette and `:line` jumps in one overlay.

use crate::theme::Palette;
use egui::{vec2, Align2, FontId, Key, RichText, Sense};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    NewFile,
    OpenFile,
    OpenFolder,
    Save,
    SaveAs,
    SaveAll,
    CloseTab,
    ToggleSidebar,
    ToggleTheme,
    ToggleWrap,
    ViewSource,
    ViewSplit,
    ViewPreview,
    CycleView,
    Find,
    FormatDocument,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    InstallCli,
    RevealInFinder,
    CopyPath,
    ToggleTerminal,
    NewTerminal,
    SearchWorkspace,
}

impl Command {
    pub const ALL: &'static [Command] = &[
        Command::NewFile,
        Command::OpenFile,
        Command::OpenFolder,
        Command::Save,
        Command::SaveAs,
        Command::SaveAll,
        Command::CloseTab,
        Command::CycleView,
        Command::ViewSource,
        Command::ViewSplit,
        Command::ViewPreview,
        Command::Find,
        Command::SearchWorkspace,
        Command::ToggleTerminal,
        Command::NewTerminal,
        Command::FormatDocument,
        Command::ToggleWrap,
        Command::ToggleSidebar,
        Command::ToggleTheme,
        Command::ZoomIn,
        Command::ZoomOut,
        Command::ZoomReset,
        Command::RevealInFinder,
        Command::CopyPath,
        Command::InstallCli,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Command::NewFile => "新建文件",
            Command::OpenFile => "打开文件…",
            Command::OpenFolder => "打开文件夹…",
            Command::Save => "保存",
            Command::SaveAs => "另存为…",
            Command::SaveAll => "全部保存",
            Command::CloseTab => "关闭标签",
            Command::ToggleSidebar => "显示/隐藏侧边栏",
            Command::ToggleTheme => "切换主题（跟随系统 / 浅色 / 深色）",
            Command::ToggleWrap => "切换自动换行",
            Command::ViewSource => "视图：源码",
            Command::ViewSplit => "视图：分栏（源码 + 渲染）",
            Command::ViewPreview => "视图：渲染",
            Command::CycleView => "切换 源码 / 渲染",
            Command::Find => "查找 / 替换",
            Command::FormatDocument => "格式化文档（JSON）",
            Command::ZoomIn => "放大",
            Command::ZoomOut => "缩小",
            Command::ZoomReset => "重置缩放",
            Command::InstallCli => "安装终端命令 code",
            Command::RevealInFinder => "在 Finder 中显示当前文件",
            Command::CopyPath => "复制当前文件路径",
            Command::ToggleTerminal => "显示/隐藏终端",
            Command::NewTerminal => "新建终端（当前文件所在目录）",
            Command::SearchWorkspace => "在工作区中搜索",
        }
    }

    pub fn shortcut(self) -> &'static str {
        match self {
            Command::NewFile => "⌘N",
            Command::OpenFile => "⌘O",
            Command::OpenFolder => "⌘⇧O",
            Command::Save => "⌘S",
            Command::SaveAs => "⌘⇧S",
            Command::SaveAll => "⌘⌥S",
            Command::CloseTab => "⌘W",
            Command::ToggleSidebar => "⌘B",
            Command::ToggleWrap => "⌥Z",
            Command::CycleView => "⌘E",
            Command::ViewSplit => "⌘\\",
            Command::Find => "⌘F",
            Command::ToggleTerminal => "⌃`",
            Command::SearchWorkspace => "⌘⇧F",
            Command::FormatDocument => "⌥⇧F",
            Command::ZoomIn => "⌘=",
            Command::ZoomOut => "⌘-",
            Command::ZoomReset => "⌘0",
            _ => "",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Files,
    Commands,
}

pub struct PaletteState {
    pub open: bool,
    pub mode: Mode,
    pub query: String,
    pub selected: usize,
    focus_pending: bool,
}

impl Default for PaletteState {
    fn default() -> Self {
        Self { open: false, mode: Mode::Files, query: String::new(), selected: 0, focus_pending: false }
    }
}

pub enum PaletteResult {
    OpenFile(PathBuf),
    Run(Command),
    GotoLine(usize),
}

impl PaletteState {
    pub fn toggle(&mut self, mode: Mode) {
        if self.open && self.mode == mode {
            self.open = false;
            return;
        }
        self.open = true;
        self.mode = mode;
        self.query = if mode == Mode::Commands { ">".into() } else { String::new() };
        self.selected = 0;
        self.focus_pending = true;
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, files: &[PathBuf], root: Option<&Path>, recent: &[PathBuf]) -> Option<PaletteResult> {
        if !self.open {
            return None;
        }
        let mut result = None;
        let screen = ctx.content_rect();
        let width = (screen.width() * 0.5).clamp(420.0, 680.0);
        egui::Area::new(egui::Id::new("palette"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_TOP, vec2(0.0, 64.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(p.panel_alt).corner_radius(10.0).inner_margin(8).show(ui, |ui| {
                    ui.set_width(width);
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text("搜索文件，> 运行命令，: 跳转到行")
                            .font(FontId::proportional(15.0))
                            .desired_width(f32::INFINITY)
                            .frame(false)
                            .margin(vec2(8.0, 6.0)),
                    );
                    if self.focus_pending {
                        edit.request_focus();
                        self.focus_pending = false;
                    }
                    if edit.changed() {
                        self.selected = 0;
                    }
                    ui.separator();

                    let q = self.query.clone();
                    let (down, up, enter, esc) = ui.input_mut(|i| {
                        (
                            i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                            i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                            i.consume_key(egui::Modifiers::NONE, Key::Enter),
                            i.consume_key(egui::Modifiers::NONE, Key::Escape),
                        )
                    });
                    if esc {
                        self.open = false;
                    }
                    if let Some(rest) = q.strip_prefix(':') {
                        let line = rest.trim().parse::<usize>().ok();
                        ui.label(RichText::new(match line {
                            Some(l) => format!("跳转到第 {l} 行"),
                            None => "输入行号".into(),
                        }).color(p.text_dim));
                        if enter {
                            if let Some(l) = line {
                                result = Some(PaletteResult::GotoLine(l));
                            }
                            self.open = false;
                        }
                        return;
                    }
                    let commands = q.starts_with('>');
                    let needle = q.trim_start_matches('>').trim().to_lowercase();
                    let items: Vec<(String, String, PaletteResult)> = if commands {
                        let mut v: Vec<(i64, Command)> = Command::ALL
                            .iter()
                            .filter_map(|c| fuzzy_score(&needle, &c.label().to_lowercase()).map(|s| (s, *c)))
                            .collect();
                        v.sort_by(|a, b| b.0.cmp(&a.0));
                        v.into_iter().map(|(_, c)| (c.label().to_owned(), c.shortcut().to_owned(), PaletteResult::Run(c))).collect()
                    } else {
                        let mut scored: Vec<(i64, &PathBuf)> = Vec::new();
                        let pool: Box<dyn Iterator<Item = &PathBuf>> = if needle.is_empty() { Box::new(recent.iter().chain(files.iter().take(200))) } else { Box::new(recent.iter().chain(files.iter())) };
                        let mut seen = std::collections::HashSet::new();
                        for f in pool {
                            if !seen.insert(f) {
                                continue;
                            }
                            let rel = rel_path(f, root);
                            let name = f.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                            let s = if needle.is_empty() { Some(0) } else {
                                fuzzy_score(&needle, &name).map(|s| s + 1000).or_else(|| fuzzy_score(&needle, &rel.to_lowercase()))
                            };
                            if let Some(s) = s {
                                scored.push((s - rel.len() as i64 / 8, f));
                            }
                            if scored.len() > 5000 {
                                break;
                            }
                        }
                        scored.sort_by(|a, b| b.0.cmp(&a.0));
                        scored
                            .into_iter()
                            .take(60)
                            .map(|(_, f)| {
                                let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                let dir = rel_path(f.parent().unwrap_or(f), root);
                                (name, dir, PaletteResult::OpenFile(f.clone()))
                            })
                            .collect()
                    };
                    if items.is_empty() {
                        ui.label(RichText::new(if commands { "没有匹配的命令" } else { "没有匹配的文件" }).color(p.text_faint));
                        return;
                    }
                    if down {
                        self.selected = (self.selected + 1).min(items.len() - 1);
                    }
                    if up {
                        self.selected = self.selected.saturating_sub(1);
                    }
                    let mut chosen = None;
                    egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, (title, detail, _)) in items.iter().enumerate() {
                            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                            let sel = i == self.selected;
                            if sel {
                                ui.painter().rect_filled(rect, 6.0, p.accent_soft);
                                if down || up {
                                    ui.scroll_to_rect(rect, None);
                                }
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, 6.0, p.current_line);
                            }
                            ui.painter().text(rect.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, title, FontId::proportional(14.0), p.text);
                            ui.painter().with_clip_rect(rect).text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, detail, FontId::proportional(12.0), p.text_faint);
                            if resp.clicked() {
                                chosen = Some(i);
                            }
                        }
                    });
                    if enter {
                        chosen = Some(self.selected);
                    }
                    if let Some(i) = chosen {
                        result = items.into_iter().nth(i).map(|(_, _, r)| r);
                        self.open = false;
                    }
                });
            });
        result
    }
}

pub fn rel_path(p: &Path, root: Option<&Path>) -> String {
    match root.and_then(|r| p.strip_prefix(r).ok()) {
        Some(rel) => rel.to_string_lossy().into_owned(),
        None => {
            let home = dirs::home_dir().unwrap_or_default();
            match p.strip_prefix(&home) {
                Ok(rel) => format!("~/{}", rel.display()),
                Err(_) => p.to_string_lossy().into_owned(),
            }
        }
    }
}

/// Subsequence fuzzy match; consecutive and word-start hits score higher.
pub fn fuzzy_score(needle: &str, hay: &str) -> Option<i64> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = hay.chars().collect();
    let mut score = 0i64;
    let mut hi = 0usize;
    let mut prev_match: Option<usize> = None;
    for nc in needle.chars() {
        if nc == ' ' {
            continue;
        }
        let mut found = None;
        while hi < hay.len() {
            if hay[hi] == nc {
                found = Some(hi);
                hi += 1;
                break;
            }
            hi += 1;
        }
        let i = found?;
        score += 10;
        if prev_match == Some(i.wrapping_sub(1)) {
            score += 15;
        }
        if i == 0 || matches!(hay[i - 1], '/' | '_' | '-' | '.' | ' ') {
            score += 20;
        }
        prev_match = Some(i);
    }
    Some(score - hay.len() as i64 / 4)
}
