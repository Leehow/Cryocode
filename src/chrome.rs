//! Window chrome: title bar with tabs and the view switcher, sidebar, status
//! bar, find bar, welcome page, dialogs and toasts.

use crate::app::{App, Modal, SidebarMode};
use crate::editor::{self, EditorSettings};
use crate::formats::{Format, ViewMode};
use crate::palette::{rel_path, Command};
use crate::preview::{self, DataMode};
use crate::theme::{Palette, BOLD_FAMILY};
use egui::{pos2, vec2, Align, Align2, Color32, FontFamily, FontId, Id, Layout, Rect, RichText, Sense, Stroke, UiBuilder};
use std::time::Duration;

const TITLEBAR_H: f32 = 40.0;
const TRAFFIC_LIGHTS_W: f32 = 78.0;

impl App {
    pub fn draw(&mut self, ctx: &egui::Context) {
        let p = Palette::get(self.theme);
        self.title_bar(ctx, &p);
        self.status_bar(ctx, &p);
        if self.sidebar {
            egui::SidePanel::left("sidebar")
                .resizable(true)
                .default_width(240.0)
                .width_range(160.0..=520.0)
                .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin { left: 0, right: 0, top: 8, bottom: 0 }))
                .show_separator_line(false)
                .show(ctx, |ui| {
                    // Never let content (long paths, previews) push the panel wider than the user set it.
                    ui.set_max_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        for (mode, label) in [(SidebarMode::Files, "文件"), (SidebarMode::Search, "搜索")] {
                            if ui.selectable_label(self.sidebar_mode == mode, RichText::new(label).size(12.0)).clicked() {
                                self.sidebar_mode = mode;
                                if mode == SidebarMode::Search {
                                    self.search.focus_request = true;
                                }
                            }
                        }
                    });
                    ui.add_space(4.0);
                    match self.sidebar_mode {
                        SidebarMode::Files => {
                            let active = self.doc().and_then(|d| d.path.clone());
                            if let Some(a) = self.tree.show(ui, &p, active.as_deref()) {
                                self.tree_action(ctx, a);
                            }
                        }
                        SidebarMode::Search => {
                            let files: Vec<std::path::PathBuf> = self.index.as_ref().map(|i| i.files.lock().unwrap().clone()).unwrap_or_default();
                            let root = self.tree.root.clone();
                            if let Some((path, range)) = self.search.show(ui, &p, root.as_deref(), &files) {
                                if let Some(i) = self.open_path(&path, true) {
                                    let d = &mut self.docs[i];
                                    if d.view == ViewMode::Preview {
                                        d.view = ViewMode::Source;
                                    }
                                    d.goto = Some(range);
                                }
                            }
                        }
                    }
                    let r = ui.max_rect();
                    ui.painter().vline(r.right(), r.y_range(), Stroke::new(1.0_f32, p.border));
                });
        }
        if self.terminal.open {
            let mut action = None;
            egui::TopBottomPanel::bottom("terminal")
                .resizable(true)
                .default_height(280.0)
                .height_range(120.0..=900.0)
                .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin { left: 6, right: 6, top: 4, bottom: 4 }))
                .show_separator_line(false)
                .show(ctx, |ui| {
                    let r = ui.max_rect();
                    ui.painter().hline(r.x_range(), r.top() - 4.0, Stroke::new(1.0_f32, p.border));
                    action = self.terminal.show(ui, &p, self.theme, self.font_size);
                });
            if let Some(a) = action {
                self.terminal_action(ctx, a);
            }
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.bg)).show(ctx, |ui| self.central(ui, &p));

        let files: Vec<std::path::PathBuf> = self.index.as_ref().map(|i| i.files.lock().unwrap().clone()).unwrap_or_default();
        let root = self.tree.root.clone();
        let recent = self.recent.clone();
        if let Some(r) = self.palette.show(ctx, &p, &files, root.as_deref(), &recent) {
            self.palette_result(ctx, r);
        }
        self.modal_ui(ctx, &p);
        self.toast_ui(ctx, &p);
    }

    fn title_bar(&mut self, ctx: &egui::Context, p: &Palette) {
        egui::TopBottomPanel::top("titlebar")
            .exact_height(TITLEBAR_H)
            .frame(egui::Frame::new().fill(p.panel))
            .show_separator_line(false)
            .show(ctx, |ui| {
                let bar = ui.max_rect();
                ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0_f32, p.border));
                // Empty title-bar space drags the window; double-click zooms like a native title bar.
                let drag = ui.interact(bar, Id::new("titlebar-drag"), Sense::click_and_drag());
                if drag.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }

                let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                let left = bar.left() + if fullscreen { 10.0 } else { TRAFFIC_LIGHTS_W };
                // Sidebar toggle.
                let tog = Rect::from_center_size(pos2(left + 12.0, bar.center().y), vec2(26.0, 24.0));
                let tr = ui.interact(tog, Id::new("sidebar-toggle"), Sense::click()).on_hover_text("侧边栏 ⌘B");
                if tr.hovered() {
                    ui.painter().rect_filled(tog, 6.0, p.current_line);
                }
                let ic = Rect::from_center_size(tog.center(), vec2(15.0, 12.0));
                ui.painter().rect_stroke(ic, 2.5, Stroke::new(1.3_f32, p.text_dim), egui::StrokeKind::Inside);
                let bar_w = if self.sidebar { 5.0 } else { 0.0 };
                ui.painter().rect_filled(Rect::from_min_size(ic.min, vec2(bar_w, ic.height())), 2.0, p.text_dim);
                ui.painter().vline(ic.left() + 5.0, ic.y_range(), Stroke::new(1.3_f32, p.text_dim));
                if tr.clicked() {
                    self.sidebar = !self.sidebar;
                }

                // Find (magnifier) and terminal buttons.
                let find_r = Rect::from_center_size(pos2(tog.right() + 16.0, bar.center().y), vec2(26.0, 24.0));
                if icon_button(ui, p, find_r, "titlebar-find", Icon::Search, self.find.open).on_hover_text("查找 ⌘F（工作区搜索 ⌘⇧F）").clicked() {
                    if self.find.open {
                        self.find.open = false;
                    } else {
                        self.run(ctx, Command::Find);
                    }
                }
                let term_r = Rect::from_center_size(pos2(find_r.right() + 16.0, bar.center().y), vec2(26.0, 24.0));
                if icon_button(ui, p, term_r, "titlebar-term", Icon::Terminal, self.terminal.open).on_hover_text("终端 ⌃`（在当前文件所在目录打开）").clicked() {
                    self.run(ctx, Command::ToggleTerminal);
                }
                let switch_w = self.view_switch_width();
                let tabs_rect = Rect::from_min_max(pos2(term_r.right() + 8.0, bar.top()), pos2(bar.right() - switch_w - 12.0, bar.bottom()));
                let mut tabs_ui = ui.new_child(UiBuilder::new().max_rect(tabs_rect).layout(Layout::left_to_right(Align::Center)));
                self.tabs(&mut tabs_ui, p);

                let sw_rect = Rect::from_min_max(pos2(bar.right() - switch_w - 8.0, bar.top()), pos2(bar.right() - 8.0, bar.bottom()));
                let mut sw_ui = ui.new_child(UiBuilder::new().max_rect(sw_rect).layout(Layout::right_to_left(Align::Center)));
                self.view_switch(&mut sw_ui, p);
            });
    }

    fn tabs(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let mut close = None;
        let mut activate = None;
        egui::ScrollArea::horizontal().id_salt("tabs").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, d) in self.docs.iter().enumerate() {
                    let title = d.title();
                    let font = FontId::proportional(13.0);
                    let text_w = ui.fonts_mut(|f| f.layout_no_wrap(title.clone(), font.clone(), p.text).size().x);
                    let w = text_w + 50.0;
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
                    let active = i == self.active;
                    let hovered = resp.hovered();
                    if active {
                        ui.painter().rect_filled(rect, 7.0, p.bg);
                        ui.painter().rect_stroke(rect, 7.0, Stroke::new(1.0_f32, p.border), egui::StrokeKind::Inside);
                    } else if hovered {
                        ui.painter().rect_filled(rect, 7.0, p.current_line);
                    }
                    let dot = file_dot(&title, p);
                    ui.painter().circle_filled(pos2(rect.left() + 14.0, rect.center().y), 3.0, dot);
                    ui.painter().text(pos2(rect.left() + 24.0, rect.center().y), Align2::LEFT_CENTER, &title, font, if active { p.text } else { p.text_dim });
                    let xr = Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(18.0, 18.0));
                    let x_resp = ui.interact(xr, Id::new(("tab-close", d.id)), Sense::click());
                    if x_resp.hovered() {
                        ui.painter().rect_filled(xr, 4.0, p.current_line);
                    }
                    if hovered || x_resp.hovered() || (active && !d.is_dirty()) {
                        let c = xr.center();
                        let s = 3.5;
                        let col = if x_resp.hovered() { p.text } else { p.text_faint };
                        ui.painter().line_segment([c - vec2(s, s), c + vec2(s, s)], Stroke::new(1.3_f32, col));
                        ui.painter().line_segment([c + vec2(-s, s), c + vec2(s, -s)], Stroke::new(1.3_f32, col));
                    } else if d.is_dirty() {
                        ui.painter().circle_filled(xr.center(), 3.5, p.text_dim);
                    }
                    if x_resp.clicked() || resp.middle_clicked() {
                        close = Some(i);
                    } else if resp.clicked() {
                        activate = Some(i);
                    }
                    let path = d.path.as_ref().map(|p| p.display().to_string());
                    if let Some(path) = path {
                        resp.on_hover_text_at_pointer(path);
                    }
                }
            });
        });
        if let Some(i) = activate {
            self.active = i;
        }
        if let Some(i) = close {
            self.close_tab(i, false);
        }
    }

    fn view_switch_width(&self) -> f32 {
        match self.doc() {
            Some(d) if d.format.has_preview() && !d.format.is_binary() => {
                let extra = match self.previews.get(&d.id) {
                    Some(s) if s.is_data() && d.view != ViewMode::Source => if s.has_table_mode() { 160.0 } else { 110.0 },
                    _ => 0.0,
                };
                170.0 + extra
            }
            _ => 0.0,
        }
    }

    fn view_switch(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let Some(d) = self.docs.get(self.active) else { return };
        if !d.format.has_preview() || d.format.is_binary() {
            return;
        }
        let current = d.view;
        let label = d.format.preview_label();
        let id = d.id;
        let mut choice = None;
        segmented(ui, p, &[("源码", ViewMode::Source == current), ("分栏", ViewMode::Split == current), (label, ViewMode::Preview == current)], |i| {
            choice = Some([ViewMode::Source, ViewMode::Split, ViewMode::Preview][i]);
        });
        if let Some(v) = choice {
            self.docs[self.active].view = v;
        }
        if current != ViewMode::Source {
            if let Some(state) = self.previews.get_mut(&id) {
                if state.is_data() {
                    ui.add_space(8.0);
                    let mut mode = None;
                    let mut opts = vec![("文档", state.data_mode == DataMode::Doc), ("树", state.data_mode == DataMode::Tree)];
                    if state.has_table_mode() {
                        opts.push(("表格", state.data_mode == DataMode::Table));
                    }
                    segmented(ui, p, &opts, |i| {
                        mode = Some([DataMode::Doc, DataMode::Tree, DataMode::Table][i]);
                    });
                    if let Some(m) = mode {
                        state.data_mode = m;
                    }
                }
            }
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context, p: &Palette) {
        egui::TopBottomPanel::bottom("status")
            .exact_height(26.0)
            .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(12, 0)))
            .show_separator_line(false)
            .show(ctx, |ui| {
                let r = ui.max_rect();
                ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0_f32, p.border));
                let small = |s: &str| RichText::new(s).size(12.0).color(p.text_dim);
                let root = self.tree.root.clone();
                let Some(d) = self.docs.get(self.active) else { return };
                let mut reload = false;
                let mut theme_toggle = false;
                let mut wrap_toggle = false;
                // Right-hand items first; the path then gets whatever width is left and truncates.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    let icon = match self.theme_pref {
                        crate::theme::ThemePref::System => "◐",
                        crate::theme::ThemePref::Light => "☀",
                        crate::theme::ThemePref::Dark => "☾",
                    };
                    if ui.add(egui::Label::new(small(icon)).sense(Sense::click())).on_hover_text(format!("主题：{}（点击切换）", self.theme_pref.label())).clicked() {
                        theme_toggle = true;
                    }
                    let lang = if d.format == Format::Code { d.syntax.clone() } else { d.format.name().to_owned() };
                    ui.label(small(&lang));
                    if !d.format.is_binary() {
                        ui.label(small(if d.line_ending == crate::document::LineEnding::CrLf { "CRLF" } else { "LF" }));
                        ui.label(small("UTF-8"));
                        ui.label(small(&d.indent.label()));
                        if ui.add(egui::Label::new(small(if d.wrap { "换行" } else { "不换行" })).sense(Sense::click())).on_hover_text("⌥Z 切换自动换行").clicked() {
                            wrap_toggle = true;
                        }
                        if d.view != ViewMode::Preview {
                            let sel = if d.selection_chars > 0 { format!("（已选 {}）", d.selection_chars) } else { String::new() };
                            ui.label(small(&format!("行 {}，列 {}{sel}", d.cursor.0, d.cursor.1)));
                        }
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 14.0;
                        if d.disk_changed {
                            ui.label(RichText::new("磁盘上的文件已被修改").size(12.0).color(p.warn));
                            if ui.small_button("重新载入").clicked() {
                                reload = true;
                            }
                        }
                        let path = d.path.as_deref().map(|p| rel_path(p, root.as_deref())).unwrap_or_else(|| "未命名".into());
                        ui.add(egui::Label::new(small(&path)).truncate());
                    });
                });
                if reload {
                    let doc = &mut self.docs[self.active];
                    doc.saved_revision = doc.revision;
                    doc.mtime = None;
                    doc.disk_changed = false;
                    doc.poll_disk();
                }
                if theme_toggle {
                    self.run(ctx, Command::ToggleTheme);
                }
                if wrap_toggle {
                    self.run(ctx, Command::ToggleWrap);
                }
            });
    }

    fn central(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let rect = ui.max_rect();
        let idx = self.active;
        let Some(doc) = self.docs.get(idx) else { return };
        if doc.path.is_none() && doc.text.is_empty() && doc.revision == 0 && self.docs.len() == 1 && !ui.memory(|m| m.has_focus(editor::editor_id(doc))) {
            if self.welcome(ui, p) {
                let id = editor::editor_id(&self.docs[idx]);
                ui.memory_mut(|m| m.request_focus(id));
            }
            return;
        }
        let view = if doc.format.is_binary() { ViewMode::Preview } else { doc.view };
        let settings = EditorSettings { font_size: self.font_size, theme: self.theme };
        if self.find.open {
            self.find.recompute(&self.docs[idx]);
        }
        let doc_id = doc.id;
        match view {
            ViewMode::Source => {
                let doc = &mut self.docs[idx];
                let cache = self.views.entry(doc_id).or_default();
                editor::show(ui, doc, cache, &self.hl, &settings, &self.find, None);
            }
            ViewMode::Preview => {
                let state = self.previews.entry(doc_id).or_default();
                if let Some(link) = preview::show(ui, &self.docs[idx], state, &mut self.res, self.theme, None) {
                    self.open_link(&link);
                }
            }
            ViewMode::Split => {
                let mid = rect.center().x.round();
                let left = Rect::from_min_max(rect.min, pos2(mid, rect.max.y));
                let right = Rect::from_min_max(pos2(mid + 1.0, rect.min.y), rect.max);
                // Whichever pane the pointer is over drives the other one's scroll position.
                let preview_drives = ui.ctx().pointer_hover_pos().is_some_and(|p| right.contains(p));
                let state = self.previews.entry(doc_id).or_default();
                let editor_sync = (preview_drives && state.scrolled).then_some(state.fraction);
                let mut lui = ui.new_child(UiBuilder::new().max_rect(left));
                let doc = &mut self.docs[idx];
                let cache = self.views.entry(doc_id).or_default();
                let out = editor::show(&mut lui, doc, cache, &self.hl, &settings, &self.find, editor_sync);
                ui.painter().vline(mid + 0.5, rect.y_range(), Stroke::new(1.0_f32, p.border));
                let mut rui = ui.new_child(UiBuilder::new().max_rect(right));
                let state = self.previews.entry(doc_id).or_default();
                let sync = (!preview_drives && out.scrolled).then_some(out.scroll_fraction);
                if let Some(link) = preview::show(&mut rui, &self.docs[idx], state, &mut self.res, self.theme, sync) {
                    self.open_link(&link);
                }
            }
        }
        if self.find.open && view != ViewMode::Preview {
            self.find_bar(ui.ctx(), p, rect);
        }
    }

    fn find_bar(&mut self, ctx: &egui::Context, p: &Palette, area: Rect) {
        let mut step = 0isize;
        let mut replace_one = false;
        let mut replace_all = false;
        egui::Area::new(Id::new("find-bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(area.right() - 440.0, area.top() + 8.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(p.panel_alt).corner_radius(8.0).inner_margin(6).show(ui, |ui| {
                    ui.set_width(420.0);
                    ui.horizontal(|ui| {
                        let toggle = if self.find.show_replace { "⌄" } else { "›" };
                        if ui.small_button(toggle).clicked() {
                            self.find.show_replace = !self.find.show_replace;
                        }
                        let resp = ui.add(egui::TextEdit::singleline(&mut self.find.query).id(Id::new("find-input")).hint_text("查找").desired_width(220.0));
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            step = if ui.input(|i| i.modifiers.shift) { -1 } else { 1 };
                            resp.request_focus();
                        }
                        let n = self.find.matches.len();
                        let label = if self.find.query.is_empty() { String::new() } else if n == 0 { "无结果".into() } else { format!("{}/{}", self.find.current + 1, n) };
                        ui.label(RichText::new(label).size(12.0).color(if n == 0 && !self.find.query.is_empty() { p.warn } else { p.text_dim }));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("×").clicked() {
                                self.find.open = false;
                            }
                            if ui.small_button("↓").on_hover_text("下一个 ⌘G").clicked() {
                                step = 1;
                            }
                            if ui.small_button("↑").on_hover_text("上一个 ⌘⇧G").clicked() {
                                step = -1;
                            }
                            let cs = ui.selectable_label(self.find.case_sensitive, "Aa").on_hover_text("区分大小写");
                            if cs.clicked() {
                                self.find.case_sensitive = !self.find.case_sensitive;
                            }
                        });
                    });
                    if self.find.show_replace {
                        ui.horizontal(|ui| {
                            ui.add_space(26.0);
                            ui.add(egui::TextEdit::singleline(&mut self.find.replace).hint_text("替换").desired_width(220.0));
                            if ui.small_button("替换").clicked() {
                                replace_one = true;
                            }
                            if ui.small_button("全部").clicked() {
                                replace_all = true;
                            }
                        });
                    }
                });
            });
        if step != 0 {
            self.find_step(step);
        }
        if replace_one {
            self.replace(false);
            self.find_step(1);
        }
        if replace_all {
            self.replace(true);
        }
    }

    /// Returns true when the user chose to start typing in a new file.
    fn welcome(&mut self, ui: &mut egui::Ui, p: &Palette) -> bool {
        let mut start = false;
        let ctx = ui.ctx().clone();
        // Typing starts a new document only when no other field (search, palette…) has focus.
        let typed = ui.memory(|m| m.focused().is_none()) && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Text(_))));
        let area = ui.max_rect();
        let col_w = 380.0f32.min(area.width() - 40.0);
        let top = area.top() + (area.height() * 0.18).max(30.0);
        let col = Rect::from_min_size(pos2(area.center().x - col_w / 2.0, top), vec2(col_w, area.bottom() - top));
        let mut u = ui.new_child(UiBuilder::new().max_rect(col).layout(Layout::top_down(Align::Min)));
        u.label(RichText::new("Cryocode").font(FontId::new(34.0, FontFamily::Name(BOLD_FAMILY.into()))).color(p.text));
        u.label(RichText::new("快速 · GPU 原生 · 为代码与文档而生").size(14.0).color(p.text_dim));
        u.add_space(28.0);
        u.label(RichText::new("开始").size(12.0).color(p.text_faint));
        u.add_space(4.0);
        let actions = [
            ("新建文件", "直接输入", None),
            ("打开文件…", "⌘O", Some(Command::OpenFile)),
            ("打开文件夹…", "⌘⇧O", Some(Command::OpenFolder)),
            ("快速打开", "⌘P", None),
            ("命令面板", "⌘⇧P", None),
        ];
        for (i, (label, key, cmd)) in actions.iter().enumerate() {
            if link_row(&mut u, p, label, key) {
                match (i, cmd) {
                    (0, _) => start = true,
                    (3, _) => self.palette.toggle(crate::palette::Mode::Files),
                    (4, _) => self.palette.toggle(crate::palette::Mode::Commands),
                    (_, Some(c)) => self.run(&ctx, *c),
                    _ => {}
                }
            }
        }
        let recent: Vec<_> = self.recent.iter().filter(|r| r.exists()).take(8).cloned().collect();
        if !recent.is_empty() {
            u.add_space(22.0);
            u.label(RichText::new("最近打开").size(12.0).color(p.text_faint));
            u.add_space(4.0);
            for r in recent {
                let name = r.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let dir = rel_path(r.parent().unwrap_or(&r), None);
                if link_row(&mut u, p, &name, &dir) {
                    self.open_path(&r, true);
                }
            }
        }
        u.add_space(22.0);
        u.label(RichText::new("终端里输入 code <文件> 即可用 Cryocode 打开").size(12.0).color(p.text_faint));
        start || typed
    }

    fn modal_ui(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(modal) = &self.modal else { return };
        let (title, body) = match modal {
            Modal::CloseDirty(id) => {
                let name = self.docs.iter().find(|d| d.id == *id).map(|d| d.title()).unwrap_or_default();
                (format!("是否保存对「{name}」的修改？"), "如果不保存，修改将会丢失。".to_owned())
            }
            Modal::QuitDirty => {
                let n = self.docs.iter().filter(|d| d.is_dirty()).count();
                (format!("有 {n} 个文件尚未保存"), "退出前要保存它们吗？".to_owned())
            }
        };
        let screen = ctx.content_rect();
        egui::Area::new(Id::new("modal-dim")).order(egui::Order::Middle).fixed_pos(screen.min).show(ctx, |ui| {
            ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(90));
            ui.allocate_rect(screen, Sense::click());
        });
        let mut choice = None;
        egui::Area::new(Id::new("modal"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, -40.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(p.panel_alt).corner_radius(12.0).inner_margin(20).show(ui, |ui| {
                    ui.set_width(360.0);
                    ui.label(RichText::new(title).size(15.0).font(FontId::new(15.0, FontFamily::Name(BOLD_FAMILY.into()))).color(p.text));
                    ui.add_space(6.0);
                    ui.label(RichText::new(body).color(p.text_dim));
                    ui.add_space(16.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let save = egui::Button::new(RichText::new("保存").color(Color32::WHITE)).fill(p.accent).min_size(vec2(72.0, 28.0));
                        if ui.add(save).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            choice = Some(0);
                        }
                        if ui.add(egui::Button::new("不保存").min_size(vec2(72.0, 28.0))).clicked() {
                            choice = Some(1);
                        }
                        if ui.add(egui::Button::new("取消").min_size(vec2(72.0, 28.0))).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            choice = Some(2);
                        }
                    });
                });
            });
        let Some(c) = choice else { return };
        let modal = self.modal.take().unwrap();
        match (modal, c) {
            (Modal::CloseDirty(id), 0) => {
                if let Some(i) = self.docs.iter().position(|d| d.id == id) {
                    if self.save(i, false) {
                        self.close_tab(i, true);
                    }
                }
            }
            (Modal::CloseDirty(id), 1) => {
                if let Some(i) = self.docs.iter().position(|d| d.id == id) {
                    self.close_tab(i, true);
                }
            }
            (Modal::QuitDirty, 0) => {
                let mut ok = true;
                for i in 0..self.docs.len() {
                    if self.docs[i].is_dirty() {
                        ok &= self.save(i, false);
                    }
                }
                if ok {
                    self.quit(ctx);
                }
            }
            (Modal::QuitDirty, 1) => self.quit(ctx),
            _ => {}
        }
    }

    fn toast_ui(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some((msg, at)) = &self.toast else { return };
        let age = at.elapsed().as_secs_f32();
        if age > 2.8 {
            self.toast = None;
            return;
        }
        let alpha = ((2.8 - age) / 0.4).clamp(0.0, 1.0);
        let msg = msg.clone();
        egui::Area::new(Id::new("toast"))
            .order(egui::Order::Tooltip)
            .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -40.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_opacity(alpha);
                egui::Frame::popup(ui.style()).fill(p.panel_alt).corner_radius(8.0).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
                    ui.label(RichText::new(msg).color(p.text));
                });
            });
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}

#[derive(Clone, Copy)]
enum Icon {
    Search,
    Terminal,
}

/// A small title-bar button with a hand-drawn vector icon.
fn icon_button(ui: &mut egui::Ui, p: &Palette, r: Rect, id: &str, icon: Icon, active: bool) -> egui::Response {
    let resp = ui.interact(r, Id::new(id), Sense::click());
    if active {
        ui.painter().rect_filled(r, 6.0, p.accent_soft);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 6.0, p.current_line);
    }
    let c = r.center();
    let col = if active { p.accent } else { p.text_dim };
    let st = Stroke::new(1.4_f32, col);
    match icon {
        Icon::Search => {
            ui.painter().circle_stroke(c + vec2(-1.5, -1.5), 4.6, st);
            ui.painter().line_segment([c + vec2(1.9, 1.9), c + vec2(5.5, 5.5)], Stroke::new(1.8_f32, col));
        }
        Icon::Terminal => {
            let b = Rect::from_center_size(c, vec2(15.0, 12.0));
            ui.painter().rect_stroke(b, 2.5, st, egui::StrokeKind::Inside);
            ui.painter().add(egui::Shape::line(vec![b.left_top() + vec2(3.5, 3.5), b.left_top() + vec2(6.0, 6.0), b.left_top() + vec2(3.5, 8.5)], st));
            ui.painter().line_segment([b.left_top() + vec2(7.5, 8.5), b.left_top() + vec2(11.0, 8.5)], st);
        }
    }
    resp
}

fn link_row(ui: &mut egui::Ui, p: &Palette, label: &str, detail: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect.expand2(vec2(8.0, 0.0)), 6.0, p.current_line);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(14.0), if resp.hovered() { p.accent } else { p.text });
    ui.painter().with_clip_rect(rect).text(rect.right_center(), Align2::RIGHT_CENTER, detail, FontId::proportional(12.0), p.text_faint);
    resp.clicked()
}

/// A pill-shaped segmented control laid out right-to-left; `on_pick` gets the option index.
fn segmented(ui: &mut egui::Ui, p: &Palette, options: &[(&str, bool)], mut on_pick: impl FnMut(usize)) {
    let font = FontId::proportional(12.5);
    let widths: Vec<f32> = options.iter().map(|(l, _)| ui.fonts_mut(|f| f.layout_no_wrap(l.to_string(), font.clone(), p.text).size().x) + 22.0).collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 26.0), Sense::hover());
    ui.painter().rect_filled(rect, 8.0, p.panel_alt);
    let mut x = rect.left() + 2.0;
    for (i, ((label, selected), w)) in options.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(*w, rect.height() - 4.0));
        let resp = ui.interact(r, ui.id().with(("seg", i, *label)), Sense::click());
        if *selected {
            ui.painter().rect_filled(r, 6.0, p.bg);
            ui.painter().rect_stroke(r, 6.0, Stroke::new(1.0_f32, p.border), egui::StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 6.0, p.current_line);
        }
        ui.painter().text(r.center(), Align2::CENTER_CENTER, *label, font.clone(), if *selected { p.accent } else { p.text_dim });
        if resp.clicked() {
            on_pick(i);
        }
        x += w;
    }
}

fn file_dot(name: &str, p: &Palette) -> Color32 {
    crate::sidebar::file_color(name, p)
}
