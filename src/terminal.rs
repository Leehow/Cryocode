//! Integrated terminal: a login shell on a PTY, emulated by `alacritty_terminal`
//! (the core of Alacritty/Zed's terminals) and painted cell-by-cell with egui.

use crate::theme::{Palette, ThemeKind};
use alacritty_terminal::event::{Event as TermEvent, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Point};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Rgb};
use egui::{pos2, vec2, Align2, Color32, FontId, Id, Key, Rect, Sense, Stroke};
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct Listener {
    ctx: egui::Context,
    /// Colours and cell size reported back to programs that query them (OSC 10/11/4, CSI 14/18 t).
    fg: Arc<Mutex<Rgb>>,
    bg: Arc<Mutex<Rgb>>,
    size: Arc<Mutex<WindowSize>>,
    title: Arc<Mutex<String>>,
    exited: Arc<AtomicBool>,
    sender: Arc<Mutex<Option<EventLoopSender>>>,
}

impl Listener {
    fn reply(&self, s: String) {
        if let Some(tx) = self.sender.lock().unwrap().as_ref() {
            let _ = tx.send(Msg::Input(Cow::Owned(s.into_bytes())));
        }
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        match event {
            TermEvent::Wakeup | TermEvent::Bell | TermEvent::CursorBlinkingChange | TermEvent::MouseCursorDirty => self.ctx.request_repaint(),
            TermEvent::Title(t) => *self.title.lock().unwrap() = t,
            TermEvent::ResetTitle => self.title.lock().unwrap().clear(),
            TermEvent::PtyWrite(s) => self.reply(s),
            TermEvent::ClipboardStore(_, s) => self.ctx.copy_text(s),
            // Shells and prompts block on these replies (e.g. background-colour detection).
            TermEvent::ColorRequest(index, format) => {
                let rgb = match index {
                    11 | 257 => *self.bg.lock().unwrap(),
                    i if i < 16 => {
                        let c = ansi(ThemeKind::Light, i);
                        Rgb { r: c.r(), g: c.g(), b: c.b() }
                    }
                    _ => *self.fg.lock().unwrap(),
                };
                self.reply(format(rgb));
            }
            TermEvent::TextAreaSizeRequest(format) => {
                let size = *self.size.lock().unwrap();
                self.reply(format(size));
            }
            TermEvent::ClipboardLoad(_, format) => {
                // Never hand the clipboard to programs silently.
                self.reply(format(""));
            }
            TermEvent::Exit | TermEvent::ChildExit(_) => {
                self.exited.store(true, Ordering::Relaxed);
                self.ctx.request_repaint();
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Dims {
    cols: usize,
    lines: usize,
}

impl Dimensions for Dims {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct Session {
    pub id: u64,
    pub cwd: PathBuf,
    term: Arc<FairMutex<Term<Listener>>>,
    notifier: Notifier,
    title: Arc<Mutex<String>>,
    exited: Arc<AtomicBool>,
    listener: Listener,
    dims: Dims,
    scroll_acc: f32,
}

impl Session {
    pub fn spawn(ctx: &egui::Context, cwd: &Path) -> std::io::Result<Self> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let dims = Dims { cols: 100, lines: 24 };
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let mut env = std::collections::HashMap::new();
        env.insert("TERM".to_owned(), "xterm-256color".to_owned());
        env.insert("COLORTERM".to_owned(), "truecolor".to_owned());
        env.insert("TERM_PROGRAM".to_owned(), "Cryocode".to_owned());
        let opts = tty::Options {
            shell: Some(tty::Shell::new(shell, vec!["-l".into()])),
            working_directory: Some(cwd.to_owned()),
            drain_on_exit: false,
            env,
        };
        let ws = WindowSize { num_lines: dims.lines as u16, num_cols: dims.cols as u16, cell_width: 8, cell_height: 16 };
        let pty = tty::new(&opts, ws, id)?;
        let listener = Listener {
            ctx: ctx.clone(),
            fg: Arc::new(Mutex::new(Rgb { r: 0x1f, g: 0x23, b: 0x28 })),
            bg: Arc::new(Mutex::new(Rgb { r: 0xff, g: 0xff, b: 0xff })),
            size: Arc::new(Mutex::new(ws)),
            title: Default::default(),
            exited: Default::default(),
            sender: Default::default(),
        };
        let config = Config { scrolling_history: 10_000, ..Default::default() };
        let term = Arc::new(FairMutex::new(Term::new(config, &dims, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener.clone(), pty, false, false)?;
        let sender = event_loop.channel();
        *listener.sender.lock().unwrap() = Some(sender.clone());
        event_loop.spawn();
        Ok(Self { id, cwd: cwd.to_owned(), term, notifier: Notifier(sender), title: listener.title.clone(), exited: listener.exited.clone(), listener, dims, scroll_acc: 0.0 })
    }

    pub fn title(&self) -> String {
        let t = self.title.lock().unwrap().clone();
        if !t.is_empty() {
            return t;
        }
        self.cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "~".into())
    }

    pub fn exited(&self) -> bool {
        self.exited.load(Ordering::Relaxed)
    }

    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        self.notifier.notify(bytes);
    }

    fn resize(&mut self, cols: usize, lines: usize, cell: egui::Vec2) {
        let (cols, lines) = (cols.max(2), lines.max(1));
        if cols == self.dims.cols && lines == self.dims.lines {
            return;
        }
        self.dims = Dims { cols, lines };
        self.term.lock().resize(self.dims);
        let ws = WindowSize { num_lines: lines as u16, num_cols: cols as u16, cell_width: cell.x as u16, cell_height: cell.y as u16 };
        *self.listener.size.lock().unwrap() = ws;
        self.notifier.on_resize(ws);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}

/// ANSI palette per theme (from GitHub's terminal colours).
fn ansi(kind: ThemeKind, i: usize) -> Color32 {
    const LIGHT: [u32; 16] = [
        0x24292f, 0xcf222e, 0x116329, 0x4d2d00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781, 0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f,
    ];
    const DARK: [u32; 16] = [
        0x484f58, 0xff7b72, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xb1bac4, 0x6e7681, 0xffa198, 0x56d364, 0xe3b341, 0x79c0ff, 0xd2a8ff, 0x56d4dd, 0xffffff,
    ];
    let v = match kind {
        ThemeKind::Light => LIGHT[i],
        ThemeKind::Dark => DARK[i],
    };
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn indexed(kind: ThemeKind, i: u8) -> Color32 {
    match i {
        0..=15 => ansi(kind, i as usize),
        16..=231 => {
            let i = i - 16;
            let step = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color32::from_rgb(step(i / 36), step((i / 6) % 6), step(i % 6))
        }
        _ => {
            let g = 8 + (i - 232) * 10;
            Color32::from_rgb(g, g, g)
        }
    }
}

fn resolve(c: AnsiColor, overrides: &alacritty_terminal::term::color::Colors, kind: ThemeKind, p: &Palette, fg: bool) -> Color32 {
    let rgb = |r: Rgb| Color32::from_rgb(r.r, r.g, r.b);
    match c {
        AnsiColor::Spec(r) => rgb(r),
        AnsiColor::Indexed(i) => overrides[i as usize].map(rgb).unwrap_or_else(|| indexed(kind, i)),
        AnsiColor::Named(n) => {
            if let Some(o) = overrides[n as usize] {
                return rgb(o);
            }
            match n {
                NamedColor::Foreground | NamedColor::BrightForeground => p.text,
                NamedColor::Background => {
                    if fg {
                        p.text
                    } else {
                        Color32::TRANSPARENT
                    }
                }
                NamedColor::Cursor => p.accent,
                NamedColor::DimForeground => p.text_dim,
                n if (n as usize) < 16 => ansi(kind, n as usize),
                n => {
                    // Dim variants map onto the normal colours, faded.
                    let base = (n as usize).saturating_sub(NamedColor::DimBlack as usize).min(7);
                    ansi(kind, base).gamma_multiply(0.7)
                }
            }
        }
    }
}

/// Encode a key press for the PTY (xterm conventions).
fn key_bytes(key: Key, m: egui::Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    let arrow = |c: u8| -> Vec<u8> {
        if m.shift || m.alt || m.ctrl {
            let modi = 1 + (m.shift as u8) + 2 * (m.alt as u8) + 4 * (m.ctrl as u8);
            format!("\x1b[1;{}{}", modi, c as char).into_bytes()
        } else if app_cursor {
            vec![0x1b, b'O', c]
        } else {
            vec![0x1b, b'[', c]
        }
    };
    Some(match key {
        Key::Enter => b"\r".to_vec(),
        Key::Tab if m.shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Backspace if m.alt => b"\x1b\x7f".to_vec(),
        Key::Backspace if m.command => b"\x15".to_vec(),
        Key::Backspace => b"\x7f".to_vec(),
        Key::Escape => b"\x1b".to_vec(),
        Key::ArrowUp => arrow(b'A'),
        Key::ArrowDown => arrow(b'B'),
        Key::ArrowRight if m.alt => b"\x1bf".to_vec(),
        Key::ArrowLeft if m.alt => b"\x1bb".to_vec(),
        Key::ArrowRight if m.command => b"\x05".to_vec(),
        Key::ArrowLeft if m.command => b"\x01".to_vec(),
        Key::ArrowRight => arrow(b'C'),
        Key::ArrowLeft => arrow(b'D'),
        Key::Home => b"\x1b[H".to_vec(),
        Key::End => b"\x1b[F".to_vec(),
        Key::PageUp => b"\x1b[5~".to_vec(),
        Key::PageDown => b"\x1b[6~".to_vec(),
        Key::Delete => b"\x1b[3~".to_vec(),
        k if m.ctrl && !m.command => {
            let name = k.name();
            let c = name.chars().next()?;
            if name.len() == 1 && c.is_ascii_alphabetic() {
                vec![(c.to_ascii_lowercase() as u8) & 0x1f]
            } else {
                match k {
                    Key::OpenBracket => vec![0x1b],
                    Key::Backslash => vec![0x1c],
                    Key::CloseBracket => vec![0x1d],
                    _ => return None,
                }
            }
        }
        _ => return None,
    })
}

/// The terminal panel: tabs of sessions plus the active session's grid.
#[derive(Default)]
pub struct TerminalPanel {
    pub open: bool,
    /// When the panel was last shown/hidden, to ignore a doubled toggle (e.g. a repeated click event).
    pub toggled_at: Option<std::time::Instant>,
    pub sessions: Vec<Session>,
    pub active: usize,
    pub focus_request: bool,
}

pub enum PanelAction {
    New,
    OpenExternal(PathBuf),
}

impl TerminalPanel {
    pub fn new_session(&mut self, ctx: &egui::Context, cwd: &Path) -> Result<(), String> {
        let s = Session::spawn(ctx, cwd).map_err(|e| format!("无法启动终端：{e}"))?;
        self.sessions.push(s);
        self.active = self.sessions.len() - 1;
        self.open = true;
        self.focus_request = true;
        Ok(())
    }

    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, kind: ThemeKind, font_size: f32) -> Option<PanelAction> {
        self.sessions.retain(|s| !s.exited());
        if self.sessions.is_empty() {
            self.open = false;
            return None;
        }
        self.active = self.active.min(self.sessions.len() - 1);
        let mut action = None;
        // Header: session tabs + actions.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(egui::RichText::new("终端").size(11.5).color(p.text_dim).strong());
            ui.add_space(6.0);
            let mut close = None;
            for (i, s) in self.sessions.iter().enumerate() {
                let sel = i == self.active;
                let r = ui.selectable_label(sel, egui::RichText::new(s.title()).size(12.0));
                if r.clicked() {
                    self.active = i;
                    self.focus_request = true;
                }
                if r.middle_clicked() {
                    close = Some(i);
                }
                r.on_hover_text(s.cwd.display().to_string());
            }
            if ui.small_button("＋").on_hover_text("新建终端（当前文件所在目录）").clicked() {
                action = Some(PanelAction::New);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("×").on_hover_text("关闭此终端").clicked() {
                    close = Some(self.active);
                }
                if ui.small_button("⌄").on_hover_text("隐藏面板 ⌃`").clicked() {
                    self.open = false;
                }
                if ui.small_button("↗").on_hover_text("在系统终端中打开此目录").clicked() {
                    action = Some(PanelAction::OpenExternal(self.sessions[self.active].cwd.clone()));
                }
            });
            if let Some(i) = close {
                self.sessions.remove(i);
                self.active = self.active.saturating_sub(usize::from(i <= self.active && self.active > 0));
            }
        });
        if self.sessions.is_empty() {
            self.open = false;
            return action;
        }
        let focus_request = std::mem::take(&mut self.focus_request);
        let s = &mut self.sessions[self.active];
        grid(ui, s, p, kind, font_size, focus_request);
        action
    }
}

fn grid(ui: &mut egui::Ui, s: &mut Session, p: &Palette, kind: ThemeKind, font_size: f32, focus_request: bool) {
    let font = FontId::monospace(font_size);
    let cell_w = ui.fonts_mut(|f| f.glyph_width(&font, 'M'));
    let cell_h = (font_size * 1.4).round();
    let area = ui.available_rect_before_wrap().shrink2(vec2(10.0, 4.0));
    let id = Id::new(("terminal", s.id));
    let resp = ui.interact(area, id, Sense::click_and_drag());
    ui.allocate_rect(area, Sense::hover());
    if focus_request || resp.clicked() || resp.drag_started() {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let focused = ui.memory(|m| m.has_focus(id));
    let filter = egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true };
    if focused {
        ui.memory_mut(|m| m.set_focus_lock_filter(id, filter));
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    let rgb = |c: Color32| Rgb { r: c.r(), g: c.g(), b: c.b() };
    *s.listener.fg.lock().unwrap() = rgb(p.text);
    *s.listener.bg.lock().unwrap() = rgb(p.bg);
    let cols = ((area.width() / cell_w).floor() as usize).max(2);
    let lines = ((area.height() / cell_h).floor() as usize).max(1);
    s.resize(cols, lines, vec2(cell_w, cell_h));

    let mode = *s.term.lock().mode();
    // Keyboard.
    if focused {
        let events = ui.input(|i| i.filtered_events(&filter));
        for ev in events {
            match ev {
                egui::Event::Text(t) => {
                    s.term.lock().scroll_display(Scroll::Bottom);
                    s.write(t.into_bytes());
                }
                egui::Event::Paste(t) => {
                    let t = t.replace("\r\n", "\r").replace('\n', "\r");
                    if mode.contains(TermMode::BRACKETED_PASTE) {
                        s.write(format!("\x1b[200~{t}\x1b[201~").into_bytes());
                    } else {
                        s.write(t.into_bytes());
                    }
                }
                egui::Event::Copy => {
                    if let Some(text) = s.term.lock().selection_to_string() {
                        ui.ctx().copy_text(text);
                    }
                }
                egui::Event::Ime(egui::ImeEvent::Commit(t)) => s.write(t.into_bytes()),
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    if modifiers.command && key == Key::K {
                        s.write(b"\x0c".to_vec());
                        continue;
                    }
                    if let Some(b) = key_bytes(key, modifiers, mode.contains(TermMode::APP_CURSOR)) {
                        s.term.lock().scroll_display(Scroll::Bottom);
                        s.write(b);
                    }
                }
                _ => {}
            }
        }
        let caret = Rect::from_min_size(area.min, vec2(1.0, cell_h));
        ui.ctx().output_mut(|o| o.ime = Some(egui::output::IMEOutput { rect: area, cursor_rect: caret }));
    }
    // Scrollback.
    if resp.hovered() {
        let dy = ui.input(|i| i.smooth_scroll_delta.y);
        if dy != 0.0 {
            s.scroll_acc += dy / cell_h;
            let n = s.scroll_acc.trunc() as i32;
            if n != 0 {
                s.scroll_acc -= n as f32;
                if mode.contains(TermMode::ALT_SCREEN) {
                    let k = if n > 0 { b"\x1b[A" } else { b"\x1b[B" };
                    for _ in 0..n.unsigned_abs().min(20) {
                        s.write(k.to_vec());
                    }
                } else {
                    s.term.lock().scroll_display(Scroll::Delta(n));
                }
            }
        }
    }
    // Mouse selection.
    let to_point = |pos: egui::Pos2, offset: usize| -> (Point, Direction) {
        let col = ((pos.x - area.left()) / cell_w).max(0.0);
        let row = (((pos.y - area.top()) / cell_h).max(0.0) as usize).min(lines - 1);
        let side = if col.fract() < 0.5 { Direction::Left } else { Direction::Right };
        let vp = Point::new(row, Column((col as usize).min(cols - 1)));
        (alacritty_terminal::term::viewport_to_point(offset, vp), side)
    };
    if let Some(pos) = resp.interact_pointer_pos() {
        let mut term = s.term.lock();
        let offset = term.grid().display_offset();
        let (pt, side) = to_point(pos, offset);
        if resp.drag_started() {
            term.selection = Some(Selection::new(SelectionType::Simple, pt, side));
        } else if resp.dragged() {
            if let Some(sel) = term.selection.as_mut() {
                sel.update(pt, side);
            }
        } else if resp.double_clicked() {
            term.selection = Some(Selection::new(SelectionType::Semantic, pt, side));
        } else if resp.clicked() {
            term.selection = None;
        }
    }

    // Paint.
    let painter = ui.painter_at(area.expand(4.0));
    let term = s.term.lock();
    let content = term.renderable_content();
    let offset = content.display_offset as i32;
    let colors = content.colors;
    let sel = content.selection;
    let cursor = content.cursor;
    let show_cursor = content.mode.contains(TermMode::SHOW_CURSOR) && offset == 0;
    let mut run = String::new();
    let mut run_start = (0usize, 0usize, Color32::TRANSPARENT);
    let flush = |run: &mut String, start: (usize, usize, Color32)| {
        if !run.is_empty() {
            let pos = pos2(area.left() + start.1 as f32 * cell_w, area.top() + start.0 as f32 * cell_h + cell_h * 0.5);
            painter.text(pos, Align2::LEFT_CENTER, std::mem::take(run), font.clone(), start.2);
        }
    };
    for cell in content.display_iter {
        let row = (cell.point.line.0 + offset) as usize;
        let col = cell.point.column.0;
        if row >= lines {
            continue;
        }
        let flags = cell.flags;
        let mut fg = resolve(cell.fg, colors, kind, p, true);
        let mut bg = resolve(cell.bg, colors, kind, p, false);
        if flags.contains(Flags::BOLD) {
            if let AnsiColor::Named(n) = cell.fg {
                if (n as usize) < 8 {
                    fg = ansi(kind, n as usize + 8);
                }
            }
        }
        if flags.contains(Flags::DIM) {
            fg = fg.gamma_multiply(0.65);
        }
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
            if fg == Color32::TRANSPARENT {
                fg = p.bg;
            }
            if bg == Color32::TRANSPARENT {
                bg = p.text;
            }
        }
        let width = if flags.contains(Flags::WIDE_CHAR) { 2.0 } else { 1.0 };
        let cell_rect = Rect::from_min_size(pos2(area.left() + col as f32 * cell_w, area.top() + row as f32 * cell_h), vec2(cell_w * width, cell_h));
        let selected = sel.is_some_and(|r| r.contains(cell.point));
        if selected {
            painter.rect_filled(cell_rect, 0.0, p.selection);
        } else if bg != Color32::TRANSPARENT {
            painter.rect_filled(cell_rect, 0.0, bg);
        }
        let is_cursor = show_cursor && cell.point == cursor.point;
        if is_cursor {
            if focused {
                painter.rect_filled(cell_rect, 1.0, p.accent);
                fg = p.bg;
            } else {
                painter.rect_stroke(cell_rect.shrink(0.5), 1.0, Stroke::new(1.0_f32, p.accent), egui::StrokeKind::Inside);
            }
        }
        if flags.contains(Flags::UNDERLINE) {
            painter.hline(cell_rect.x_range(), cell_rect.bottom() - 2.0, Stroke::new(1.0_f32, fg));
        }
        if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER | Flags::HIDDEN) {
            continue;
        }
        let c = cell.c;
        // ASCII runs of one colour share a galley; anything else is placed cell by cell.
        let contiguous = run_start.0 == row && run_start.1 + run.len() == col && run_start.2 == fg;
        if c.is_ascii() && !is_cursor && width == 1.0 && contiguous && !run.is_empty() {
            run.push(c);
            continue;
        }
        flush(&mut run, run_start);
        if c == ' ' {
            continue;
        }
        if c.is_ascii() && !is_cursor {
            run.push(c);
            run_start = (row, col, fg);
        } else {
            painter.text(cell_rect.center(), Align2::CENTER_CENTER, c.to_string(), font.clone(), fg);
        }
    }
    flush(&mut run, run_start);
}
