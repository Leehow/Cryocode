//! Application state and actions. Window chrome lives in `chrome.rs`.

use crate::document::Document;
use crate::editor::{self, FindState, ViewCache};
use crate::formats::{Format, ViewMode};
use crate::highlight::{DocId, HighlightService};
use crate::ipc::{Inbox, OpenRequest};
use crate::palette::{Command, Mode, PaletteResult, PaletteState};
use crate::preview::{PreviewResources, PreviewState};
use crate::search::WorkspaceSearch;
use crate::sidebar::{FileIndex, FileTree, TreeAction};
use crate::terminal::{PanelAction, TerminalPanel};
use crate::theme::{self, ThemeKind, ThemePref};
use egui::{Key, KeyboardShortcut, Modifiers};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Session {
    pub root: Option<PathBuf>,
    pub files: Vec<PathBuf>,
    pub active: Option<PathBuf>,
    #[serde(default)]
    pub theme_pref: ThemePref,
    pub font_size: f32,
    pub sidebar: bool,
    pub recent: Vec<PathBuf>,
    #[serde(default)]
    pub expanded: Vec<PathBuf>,
}

impl Default for Session {
    fn default() -> Self {
        Self { root: None, files: vec![], active: None, theme_pref: ThemePref::System, font_size: 13.5, sidebar: false, recent: vec![], expanded: vec![] }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SidebarMode {
    Files,
    Search,
}

pub enum Modal {
    CloseDirty(DocId),
    QuitDirty,
}

pub struct App {
    pub docs: Vec<Document>,
    pub active: usize,
    pub views: HashMap<DocId, ViewCache>,
    pub previews: HashMap<DocId, PreviewState>,
    pub res: PreviewResources,
    pub hl: HighlightService,
    hl_requested: HashMap<DocId, (u64, ThemeKind, usize)>,
    pub tree: FileTree,
    pub index: Option<FileIndex>,
    pub palette: PaletteState,
    pub find: FindState,
    pub theme: ThemeKind,
    pub theme_pref: ThemePref,
    pub font_size: f32,
    pub sidebar: bool,
    pub recent: Vec<PathBuf>,
    pub inbox: Inbox,
    pub toast: Option<(String, Instant)>,
    pub modal: Option<Modal>,
    pub terminal: TerminalPanel,
    pub search: WorkspaceSearch,
    pub sidebar_mode: SidebarMode,
    quitting: bool,
    last_poll: Instant,
    last_title: String,
    applied_theme: Option<ThemeKind>,
    styled: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, inbox: Inbox, initial: Vec<OpenRequest>) -> Self {
        let ctx = cc.egui_ctx.clone();
        inbox.set_ctx(ctx.clone());
        theme::install_fonts(&ctx);
        let session: Session = cc.storage.and_then(|s| eframe::get_value(s, "session")).unwrap_or_default();
        let mut app = Self {
            docs: Vec::new(),
            active: 0,
            views: HashMap::new(),
            previews: HashMap::new(),
            res: PreviewResources::default(),
            hl: HighlightService::start(ctx.clone()),
            hl_requested: HashMap::new(),
            tree: FileTree::new(None),
            index: None,
            palette: PaletteState::default(),
            find: FindState::default(),
            theme: ThemeKind::Light,
            theme_pref: session.theme_pref,
            font_size: session.font_size.clamp(9.0, 32.0),
            sidebar: session.sidebar,
            recent: session.recent.clone(),
            inbox,
            toast: None,
            modal: None,
            terminal: TerminalPanel::default(),
            search: WorkspaceSearch::default(),
            sidebar_mode: SidebarMode::Files,
            quitting: false,
            last_poll: Instant::now(),
            last_title: String::new(),
            applied_theme: None,
            styled: false,
        };
        if let Some(root) = session.root.clone().filter(|r| r.is_dir()) {
            app.set_root(root);
            app.tree.restore_expanded(session.expanded.clone());
        }
        for f in session.files.iter().filter(|f| f.is_file()) {
            app.open_path(f, false);
        }
        if let Some(a) = &session.active {
            if let Some(i) = app.docs.iter().position(|d| d.path.as_deref() == Some(a)) {
                app.active = i;
            }
        }
        for req in initial {
            app.handle_open(req);
        }
        if app.docs.is_empty() {
            app.docs.push(Document::untitled());
        }
        app
    }

    pub fn session(&self) -> Session {
        Session {
            root: self.tree.root.clone(),
            files: self.docs.iter().filter_map(|d| d.path.clone()).collect(),
            active: self.doc().and_then(|d| d.path.clone()),
            theme_pref: self.theme_pref,
            font_size: self.font_size,
            sidebar: self.sidebar,
            recent: self.recent.clone(),
            expanded: self.tree.expanded_dirs(),
        }
    }

    pub fn doc(&self) -> Option<&Document> {
        self.docs.get(self.active)
    }

    pub fn doc_mut(&mut self) -> Option<&mut Document> {
        self.docs.get_mut(self.active)
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    pub fn set_root(&mut self, root: PathBuf) {
        self.tree.set_root(root.clone());
        self.index = Some(FileIndex::build(root));
    }

    /// Open a file (or folder), focusing an existing tab when it is already open.
    pub fn open_path(&mut self, path: &Path, activate: bool) -> Option<usize> {
        if path.is_dir() {
            self.set_root(path.to_owned());
            self.sidebar = true;
            return None;
        }
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        if let Some(i) = self.docs.iter().position(|d| d.path.as_deref() == Some(canon.as_path())) {
            if activate {
                self.active = i;
            }
            return Some(i);
        }
        let doc = Document::open(&canon);
        if let Some(err) = &doc.load_error {
            self.toast(format!("{}：{err}", doc.title()));
        }
        // Replace a pristine untitled tab instead of stacking empty ones.
        let replace = self.docs.len() == 1 && self.docs[0].path.is_none() && self.docs[0].text.is_empty() && !self.docs[0].is_dirty();
        let idx = if replace {
            self.docs[0] = doc;
            0
        } else {
            self.docs.push(doc);
            self.docs.len() - 1
        };
        if activate || replace {
            self.active = idx;
        }
        self.recent.retain(|r| r != &canon);
        self.recent.insert(0, canon.clone());
        self.recent.truncate(30);
        if self.tree.root.is_none() {
            if let Some(root) = project_root(&canon) {
                self.set_root(root);
            }
        }
        self.tree.reveal(&canon);
        Some(idx)
    }

    pub fn handle_open(&mut self, req: OpenRequest) {
        if let Some(i) = self.open_path(&req.path, true) {
            if let Some(line) = req.line {
                let doc = &mut self.docs[i];
                let start = editor::text::line_to_byte(&doc.text, line);
                let c = editor::text::advance_chars_in_line(&doc.text, start, req.col.unwrap_or(1).saturating_sub(1));
                doc.goto = Some((c, c));
                if doc.view == ViewMode::Preview {
                    doc.view = ViewMode::Source;
                }
            }
        }
    }

    pub fn close_tab(&mut self, i: usize, force: bool) {
        if i >= self.docs.len() {
            return;
        }
        if !force && self.docs[i].is_dirty() {
            self.active = i;
            self.modal = Some(Modal::CloseDirty(self.docs[i].id));
            return;
        }
        let d = self.docs.remove(i);
        self.views.remove(&d.id);
        self.previews.remove(&d.id);
        self.hl.forget(d.id);
        self.hl_requested.remove(&d.id);
        if self.active >= i && self.active > 0 {
            self.active -= 1;
        }
        if self.docs.is_empty() {
            self.docs.push(Document::untitled());
            self.active = 0;
        }
        self.release_waiters();
    }

    fn release_waiters(&self) {
        let open: Vec<PathBuf> = self.docs.iter().filter_map(|d| d.path.clone()).collect();
        self.inbox.release_waiters(|p| open.iter().any(|o| o == p || o.canonicalize().ok().as_deref() == p.canonicalize().ok().as_deref()));
    }

    pub fn save(&mut self, i: usize, save_as: bool) -> bool {
        let Some(doc) = self.docs.get_mut(i) else { return false };
        if doc.path.is_none() || save_as {
            let mut dialog = rfd::FileDialog::new().set_file_name(doc.title());
            if let Some(dir) = doc.path.as_deref().and_then(Path::parent).or(self.tree.root.as_deref()) {
                dialog = dialog.set_directory(dir);
            }
            match dialog.save_file() {
                Some(p) => doc.set_path(p),
                None => return false,
            }
        }
        match doc.save() {
            Ok(()) => {
                let t = doc.title();
                self.hl_requested.remove(&self.docs[i].id);
                self.toast(format!("已保存 {t}"));
                true
            }
            Err(e) => {
                self.toast(format!("保存失败：{e}"));
                false
            }
        }
    }

    pub fn run(&mut self, ctx: &egui::Context, cmd: Command) {
        match cmd {
            Command::NewFile => {
                self.docs.push(Document::untitled());
                self.active = self.docs.len() - 1;
            }
            Command::OpenFile => {
                let mut d = rfd::FileDialog::new();
                if let Some(r) = &self.tree.root {
                    d = d.set_directory(r);
                }
                if let Some(files) = d.pick_files() {
                    for f in files {
                        self.open_path(&f, true);
                    }
                }
            }
            Command::OpenFolder => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.set_root(dir);
                    self.sidebar = true;
                }
            }
            Command::Save => {
                self.save(self.active, false);
            }
            Command::SaveAs => {
                self.save(self.active, true);
            }
            Command::SaveAll => {
                for i in 0..self.docs.len() {
                    if self.docs[i].is_dirty() {
                        self.save(i, false);
                    }
                }
            }
            Command::CloseTab => self.close_tab(self.active, false),
            Command::ToggleSidebar => self.sidebar = !self.sidebar,
            Command::ToggleTheme => {
                self.theme_pref = self.theme_pref.next();
                ctx.options_mut(|o| o.theme_preference = self.theme_pref.egui());
                self.toast(format!("主题：{}", self.theme_pref.label()));
            }
            Command::ToggleWrap => {
                if let Some(d) = self.doc_mut() {
                    d.wrap = !d.wrap;
                }
            }
            Command::ViewSource => self.set_view(ViewMode::Source),
            Command::ViewSplit => {
                let v = if self.doc().is_some_and(|d| d.view == ViewMode::Split) { ViewMode::Source } else { ViewMode::Split };
                self.set_view(v);
            }
            Command::ViewPreview => self.set_view(ViewMode::Preview),
            Command::CycleView => {
                let v = match self.doc().map(|d| d.view) {
                    Some(ViewMode::Source) => ViewMode::Preview,
                    _ => ViewMode::Source,
                };
                self.set_view(v);
            }
            Command::Find => {
                self.find.open = true;
                if let Some(d) = self.doc_mut() {
                    if d.view == ViewMode::Preview && !d.format.is_binary() {
                        d.view = ViewMode::Source;
                    }
                }
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("find-input")));
            }
            Command::FormatDocument => self.format_document(),
            Command::ZoomIn => self.font_size = (self.font_size + 1.0).min(32.0),
            Command::ZoomOut => self.font_size = (self.font_size - 1.0).max(9.0),
            Command::ZoomReset => self.font_size = 13.5,
            Command::InstallCli => match crate::install_cli() {
                Ok(p) => self.toast(format!("已安装 {}", p.display())),
                Err(e) => self.toast(format!("安装失败：{e}")),
            },
            Command::ToggleTerminal => {
                if self.terminal.toggled_at.is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
                    return;
                }
                self.terminal.toggled_at = Some(Instant::now());
                if self.terminal.open {
                    self.terminal.open = false;
                } else if self.terminal.sessions.is_empty() {
                    self.new_terminal(ctx);
                } else {
                    self.terminal.open = true;
                    self.terminal.focus_request = true;
                }
            }
            Command::NewTerminal => self.new_terminal(ctx),
            Command::SearchWorkspace => {
                self.sidebar = true;
                self.sidebar_mode = SidebarMode::Search;
                self.search.focus_request = true;
            }
            Command::RevealInFinder => {
                if let Some(p) = self.doc().and_then(|d| d.path.clone()) {
                    let _ = std::process::Command::new("/usr/bin/open").arg("-R").arg(p).spawn();
                }
            }
            Command::CopyPath => {
                if let Some(p) = self.doc().and_then(|d| d.path.clone()) {
                    ctx.copy_text(p.display().to_string());
                    self.toast("已复制路径");
                }
            }
        }
    }

    /// Directory for a new terminal: the active file's folder, else the workspace, else home.
    pub fn terminal_cwd(&self) -> PathBuf {
        self.doc()
            .and_then(|d| d.path.as_deref())
            .and_then(Path::parent)
            .filter(|p| p.is_dir())
            .map(Path::to_owned)
            .or_else(|| self.tree.root.clone())
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    pub fn new_terminal(&mut self, ctx: &egui::Context) {
        let cwd = self.terminal_cwd();
        if let Err(e) = self.terminal.new_session(ctx, &cwd) {
            self.toast(e);
        }
    }

    pub fn terminal_action(&mut self, ctx: &egui::Context, a: PanelAction) {
        match a {
            PanelAction::New => self.new_terminal(ctx),
            PanelAction::OpenExternal(dir) => {
                let _ = std::process::Command::new("/usr/bin/open").arg("-a").arg("Terminal").arg(dir).spawn();
            }
        }
    }

    fn set_view(&mut self, v: ViewMode) {
        if let Some(d) = self.doc_mut() {
            if d.format.is_binary() {
                return;
            }
            if v != ViewMode::Source && !d.format.has_preview() {
                return;
            }
            d.view = v;
        }
    }

    fn format_document(&mut self) {
        let Some(d) = self.doc_mut() else { return };
        if d.format != Format::Json {
            return;
        }
        let indent = d.indent.unit();
        match serde_json::from_str::<serde_json::Value>(&d.text) {
            Ok(v) => {
                let mut buf = Vec::new();
                let fmt = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
                let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
                if serde::Serialize::serialize(&v, &mut ser).is_ok() {
                    let mut s = String::from_utf8(buf).unwrap_or_default();
                    s.push('\n');
                    if s != d.text {
                        let len = d.text.len();
                        d.replace(0..len, &s, editor::EditKind::Other);
                        d.edit.sel = editor::Selection::caret(0);
                    }
                }
            }
            Err(e) => {
                let msg = format!("JSON 有错误，无法格式化：{e}");
                self.toast(msg);
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = Modifiers::COMMAND;
        let cs = cmd | Modifiers::SHIFT;
        let ca = cmd | Modifiers::ALT;
        let table: &[(Modifiers, Key, Command)] = &[
            (cmd, Key::S, Command::Save),
            (cs, Key::S, Command::SaveAs),
            (ca, Key::S, Command::SaveAll),
            (cmd, Key::O, Command::OpenFile),
            (cs, Key::O, Command::OpenFolder),
            (cmd, Key::N, Command::NewFile),
            (cmd, Key::W, Command::CloseTab),
            (cmd, Key::B, Command::ToggleSidebar),
            (Modifiers::ALT, Key::Z, Command::ToggleWrap),
            (cmd, Key::E, Command::CycleView),
            (cmd, Key::Backslash, Command::ViewSplit),
            (cmd, Key::F, Command::Find),
            (Modifiers::CTRL, Key::F, Command::Find),
            (cs, Key::F, Command::SearchWorkspace),
            (Modifiers::CTRL, Key::Backtick, Command::ToggleTerminal),
            (cmd, Key::J, Command::ToggleTerminal),
            (Modifiers::ALT | Modifiers::SHIFT, Key::F, Command::FormatDocument),
        ];
        // egui matches shortcuts "logically" (⌘⇧F also satisfies ⌘F), so try the most specific first.
        let mut table = table.to_vec();
        table.sort_by_key(|(m, _, _)| std::cmp::Reverse(m.shift as u8 + m.alt as u8 + m.ctrl as u8 + m.command as u8));
        let mut fire = Vec::new();
        ctx.input_mut(|i| {
            for (m, k, c) in &table {
                if i.consume_shortcut(&KeyboardShortcut::new(*m, *k)) {
                    fire.push(*c);
                }
            }
        });
        for c in fire {
            self.run(ctx, c);
        }
        let mut find_prev_slot = false;
        let mut prev_slot = false;
        let (quick, commands, next, prev, find_next, find_prev, esc) = ctx.input_mut(|i| {
            let commands = i.consume_shortcut(&KeyboardShortcut::new(cs, Key::P));
            (
                i.consume_shortcut(&KeyboardShortcut::new(cmd, Key::P)),
                commands,
                {
                    // Shifted variants first (see above).
                    let p = i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab)) || i.consume_shortcut(&KeyboardShortcut::new(cs, Key::OpenBracket));
                    prev_slot = p;
                    i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::Tab)) || i.consume_shortcut(&KeyboardShortcut::new(cs, Key::CloseBracket))
                },
                prev_slot,
                {
                    let prev = i.consume_shortcut(&KeyboardShortcut::new(cs, Key::G));
                    find_prev_slot = prev;
                    i.consume_shortcut(&KeyboardShortcut::new(cmd, Key::G))
                },
                find_prev_slot,
                self.find.open && !self.palette.open && i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if commands {
            self.palette.toggle(Mode::Commands);
        } else if quick {
            self.palette.toggle(Mode::Files);
        }
        let n = self.docs.len();
        if next && n > 0 {
            self.active = (self.active + 1) % n;
        }
        if prev && n > 0 {
            self.active = (self.active + n - 1) % n;
        }
        for (k, key) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9].iter().enumerate() {
            if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(cmd, *key))) {
                self.active = if *key == Key::Num9 { n - 1 } else { k.min(n - 1) };
            }
        }
        if find_next {
            self.find_step(1);
        }
        if find_prev {
            self.find_step(-1);
        }
        if esc {
            self.find.open = false;
            if let Some(d) = self.doc() {
                let id = editor::editor_id(d);
                ctx.memory_mut(|m| m.request_focus(id));
            }
        }
    }

    pub fn find_step(&mut self, dir: isize) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        self.find.recompute(doc);
        let n = self.find.matches.len();
        if n == 0 {
            return;
        }
        let sel = doc.edit.sel.range();
        let idx = if dir > 0 {
            // First match after the current selection (wrapping).
            self.find.matches.iter().position(|m| m.0 >= sel.end && (m.0, m.1) != (sel.start, sel.end)).unwrap_or(0)
        } else {
            self.find.matches.iter().rposition(|m| m.1 <= sel.start).unwrap_or(n - 1)
        };
        self.find.current = idx;
        doc.goto = Some(self.find.matches[idx]);
    }

    pub fn replace(&mut self, all: bool) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        self.find.recompute(doc);
        if self.find.matches.is_empty() {
            return;
        }
        let ranges: Vec<(usize, usize)> = if all {
            self.find.matches.clone()
        } else {
            vec![self.find.matches[self.find.current.min(self.find.matches.len() - 1)]]
        };
        let count = ranges.len();
        doc.edit.begin_group();
        for &(a, b) in ranges.iter().rev() {
            doc.replace(a..b, &self.find.replace, editor::EditKind::Other);
        }
        doc.edit.end_group();
        if all {
            self.toast(format!("已替换 {count} 处"));
        }
    }

    fn process_inbox(&mut self, ctx: &egui::Context) {
        let reqs = self.inbox.take();
        if reqs.is_empty() {
            return;
        }
        for r in reqs {
            self.handle_open(r);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn request_highlighting(&mut self) {
        let theme = self.theme;
        for i in 0..self.docs.len() {
            let visible = i == self.active;
            let doc = &mut self.docs[i];
            if !visible || doc.view == ViewMode::Preview || doc.format.is_binary() {
                continue;
            }
            // Colour a margin below the viewport, in coarse steps so scrolling doesn't spam requests.
            let want = (doc.hl_want / 500 + 2) * 500;
            let key = (doc.revision, theme, want);
            if self.hl_requested.get(&doc.id) == Some(&key) {
                continue;
            }
            let text = doc.prefix_through_line(want);
            self.hl.request(doc.id, text, &doc.syntax, theme);
            self.hl_requested.insert(doc.id, key);
        }
    }

    fn poll_disk(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.focused);
        if !focused || self.last_poll.elapsed() < Duration::from_millis(1200) {
            return;
        }
        self.last_poll = Instant::now();
        for d in &mut self.docs {
            d.poll_disk();
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        for p in dropped {
            self.open_path(&p, true);
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && self.docs.iter().any(Document::is_dirty) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.modal = Some(Modal::QuitDirty);
        }
    }

    pub fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    pub fn tree_action(&mut self, ctx: &egui::Context, a: TreeAction) {
        match a {
            TreeAction::Open(p) => {
                self.open_path(&p, true);
            }
            TreeAction::Reveal(p) => {
                let _ = std::process::Command::new("/usr/bin/open").arg("-R").arg(p).spawn();
            }
            TreeAction::CopyPath(p) => {
                ctx.copy_text(p.display().to_string());
                self.toast("已复制路径");
            }
        }
    }

    pub fn palette_result(&mut self, ctx: &egui::Context, r: PaletteResult) {
        match r {
            PaletteResult::OpenFile(p) => {
                self.open_path(&p, true);
            }
            PaletteResult::Run(c) => self.run(ctx, c),
            PaletteResult::GotoLine(l) => {
                if let Some(d) = self.doc_mut() {
                    let c = editor::text::line_to_byte(&d.text, l);
                    d.goto = Some((c, c));
                    if d.view == ViewMode::Preview {
                        d.view = ViewMode::Source;
                    }
                }
            }
        }
    }

    pub fn open_link(&mut self, url: &str) {
        if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
            let _ = std::process::Command::new("/usr/bin/open").arg(url).spawn();
            return;
        }
        if url.starts_with('#') {
            return;
        }
        let base = self.doc().and_then(|d| d.path.as_deref()).and_then(Path::parent).map(Path::to_owned);
        let target = url.split('#').next().unwrap_or(url);
        let p = match base {
            Some(b) if !Path::new(target).is_absolute() => b.join(target),
            _ => PathBuf::from(target),
        };
        if p.exists() {
            self.open_path(&p, true);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.styled {
            theme::apply_style(ctx, self.theme_pref);
            self.styled = true;
        }
        self.theme = theme::effective(ctx);
        if self.applied_theme != Some(self.theme) {
            self.applied_theme = Some(self.theme);
            self.views.clear();
            self.res.code_jobs = Default::default();
        }
        self.process_inbox(ctx);
        self.handle_drops(ctx);
        self.handle_close_request(ctx);
        if self.modal.is_none() {
            self.shortcuts(ctx);
        }
        self.active = self.active.min(self.docs.len().saturating_sub(1));
        self.request_highlighting();
        self.poll_disk(ctx);

        let title = self.doc().map(|d| format!("{}{} — Cryocode", if d.is_dirty() { "● " } else { "" }, d.title())).unwrap_or_else(|| "Cryocode".into());
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        self.draw(ctx);
        self.release_waiters();
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "session", &self.session());
    }

    fn auto_save_interval(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Nearest ancestor that looks like a project root (VCS or build manifest).
fn project_root(file: &Path) -> Option<PathBuf> {
    const MARKERS: &[&str] = &[".git", "Cargo.toml", "package.json", "pyproject.toml", "go.mod", ".hg"];
    let home = dirs::home_dir();
    for dir in file.ancestors().skip(1) {
        if Some(dir) == home.as_deref() || dir.parent().is_none() {
            break;
        }
        if MARKERS.iter().any(|m| dir.join(m).exists()) {
            return Some(dir.to_owned());
        }
    }
    None
}
