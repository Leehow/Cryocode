//! Visual identity: palette, egui style and font stack.

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle};
use std::sync::Arc;

pub const MATH_FAMILY: &str = "math";
pub const MATH_ITALIC_FAMILY: &str = "math-italic";
pub const SERIF_FAMILY: &str = "serif";
pub const BOLD_FAMILY: &str = "bold";
const CJK_MONO_OFFSET: f32 = 0.2;
/// Same correction for the proportional (UI / prose) stack.
const CJK_OFFSET: f32 = 0.2;

/// The theme actually in effect.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum ThemeKind {
    Dark,
    Light,
}

/// What the user chose; `System` follows macOS appearance live.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePref {
    pub fn next(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "跟随系统",
            Self::Light => "浅色",
            Self::Dark => "深色",
        }
    }

    pub fn egui(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        }
    }
}

pub fn effective(ctx: &egui::Context) -> ThemeKind {
    match ctx.theme() {
        egui::Theme::Dark => ThemeKind::Dark,
        egui::Theme::Light => ThemeKind::Light,
    }
}

/// Semantic colours shared by every view.
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub panel_alt: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub selection: Color32,
    pub current_line: Color32,
    pub code_bg: Color32,
    pub link: Color32,
    pub string: Color32,
    pub number: Color32,
    pub keyword: Color32,
    pub warn: Color32,
    pub ok: Color32,
}

impl Palette {
    pub fn get(kind: ThemeKind) -> Self {
        match kind {
            // GitHub Dark Default.
            ThemeKind::Dark => Self {
                bg: Color32::from_rgb(0x0d, 0x11, 0x17),
                panel: Color32::from_rgb(0x01, 0x04, 0x09),
                panel_alt: Color32::from_rgb(0x16, 0x1b, 0x22),
                border: Color32::from_rgb(0x30, 0x36, 0x3d),
                text: Color32::from_rgb(0xe6, 0xed, 0xf3),
                text_dim: Color32::from_rgb(0x91, 0x98, 0xa1),
                text_faint: Color32::from_rgb(0x6e, 0x76, 0x81),
                accent: Color32::from_rgb(0x44, 0x93, 0xf8),
                accent_soft: Color32::from_rgba_unmultiplied(0x44, 0x93, 0xf8, 0x26),
                selection: Color32::from_rgba_unmultiplied(0x38, 0x8b, 0xfd, 0x40),
                current_line: Color32::from_rgba_unmultiplied(0x6e, 0x76, 0x81, 0x1a),
                code_bg: Color32::from_rgb(0x16, 0x1b, 0x22),
                link: Color32::from_rgb(0x44, 0x93, 0xf8),
                string: Color32::from_rgb(0xa5, 0xd6, 0xff),
                number: Color32::from_rgb(0x79, 0xc0, 0xff),
                keyword: Color32::from_rgb(0xff, 0x7b, 0x72),
                warn: Color32::from_rgb(0xf8, 0x51, 0x49),
                ok: Color32::from_rgb(0x3f, 0xb9, 0x50),
            },
            // GitHub Light Default.
            ThemeKind::Light => Self {
                bg: Color32::from_rgb(0xff, 0xff, 0xff),
                panel: Color32::from_rgb(0xf6, 0xf8, 0xfa),
                panel_alt: Color32::from_rgb(0xea, 0xee, 0xf2),
                border: Color32::from_rgb(0xd0, 0xd7, 0xde),
                text: Color32::from_rgb(0x1f, 0x23, 0x28),
                text_dim: Color32::from_rgb(0x59, 0x63, 0x6e),
                text_faint: Color32::from_rgb(0x81, 0x8b, 0x98),
                accent: Color32::from_rgb(0x09, 0x69, 0xda),
                accent_soft: Color32::from_rgba_unmultiplied(0x09, 0x69, 0xda, 0x1a),
                selection: Color32::from_rgba_unmultiplied(0x09, 0x69, 0xda, 0x33),
                current_line: Color32::from_rgba_unmultiplied(0xea, 0xee, 0xf2, 0xc0),
                code_bg: Color32::from_rgb(0xf6, 0xf8, 0xfa),
                link: Color32::from_rgb(0x09, 0x69, 0xda),
                string: Color32::from_rgb(0x0a, 0x30, 0x69),
                number: Color32::from_rgb(0x05, 0x50, 0xae),
                keyword: Color32::from_rgb(0xcf, 0x22, 0x2e),
                warn: Color32::from_rgb(0xd1, 0x24, 0x2f),
                ok: Color32::from_rgb(0x1a, 0x7f, 0x37),
            },
        }
    }
}

/// Install the customised dark and light styles; egui switches between them with the system.
pub fn apply_style(ctx: &egui::Context, pref: ThemePref) {
    for (kind, theme) in [(ThemeKind::Dark, egui::Theme::Dark), (ThemeKind::Light, egui::Theme::Light)] {
        ctx.set_visuals_of(theme, visuals(kind));
    }
    ctx.options_mut(|o| o.theme_preference = pref.egui());
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.scroll = egui::style::ScrollStyle::floating();
        style.spacing.scroll.bar_width = 8.0;
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(13.5)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Heading, FontId::proportional(20.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        style.interaction.tooltip_delay = 0.4;
    });
}

fn visuals(kind: ThemeKind) -> egui::Visuals {
    let p = Palette::get(kind);
    let mut visuals = match kind {
        ThemeKind::Dark => egui::Visuals::dark(),
        ThemeKind::Light => egui::Visuals::light(),
    };
    visuals.panel_fill = p.panel;
    visuals.window_fill = p.panel_alt;
    visuals.extreme_bg_color = p.bg;
    visuals.faint_bg_color = p.panel_alt;
    visuals.code_bg_color = p.code_bg;
    visuals.hyperlink_color = p.link;
    visuals.selection.bg_fill = p.selection;
    visuals.selection.stroke = Stroke::new(1.0_f32, p.accent);
    visuals.window_stroke = Stroke::new(1.0_f32, p.border);
    visuals.window_corner_radius = 10.into();
    visuals.menu_corner_radius = 8.into();
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(if kind == ThemeKind::Dark { 120 } else { 40 }),
    };
    visuals.popup_shadow = visuals.window_shadow;
    visuals.text_cursor.stroke = Stroke::new(2.0_f32, p.accent);
    visuals.override_text_color = Some(p.text);
    // Slightly heavier glyph coverage: egui's light-mode default renders dark-on-light text too thin.
    visuals.text_alpha_from_coverage = egui::epaint::AlphaFromCoverage::TwoCoverageMinusCoverageSq;
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.corner_radius = 6.into();
    }
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text_dim);
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_fill = p.panel_alt;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.text_dim);
    visuals.widgets.hovered.weak_bg_fill = p.accent_soft;
    visuals.widgets.hovered.bg_fill = p.accent_soft;
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
    visuals.widgets.active.weak_bg_fill = p.accent_soft;
    visuals.widgets.active.bg_fill = p.accent_soft;
    visuals.widgets.active.bg_stroke = Stroke::NONE;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, p.accent);
    visuals
}

/// Load the system fonts used by the UI, the code editor and the math renderer.
///
/// Every file is optional: missing fonts fall back to egui's built-in faces.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    // Each font file is read once and shared by every face that uses it (the CJK
    // collection alone is 23 MB and backs three faces). Fonts live for the whole process.
    let mut files: std::collections::HashMap<std::path::PathBuf, &'static [u8]> = Default::default();
    let mut load = |name: &str, paths: &[&str], index: u32, y_offset: f32| -> bool {
        for path in paths {
            let path = expand_home(path);
            let bytes = match files.get(&path) {
                Some(b) => Some(*b),
                None => std::fs::read(&path).ok().map(|b| {
                    let leaked: &'static [u8] = Box::leak(b.into_boxed_slice());
                    files.insert(path.clone(), leaked);
                    leaked
                }),
            };
            if let Some(bytes) = bytes {
                let mut data = FontData::from_static(bytes);
                data.index = index;
                data.tweak.y_offset_factor = y_offset;
                fonts.font_data.insert(name.to_owned(), Arc::new(data));
                return true;
            }
        }
        false
    };

    let has_ui = load("ui", &["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/HelveticaNeue.ttc"], 0, 0.0);
    let has_mono = load(
        "mono",
        &[
            "~/Library/Fonts/JetBrainsMono-Regular.ttf",
            // Static SF Mono (the system copy is a variable font whose default instance renders thin).
            "/System/Applications/Utilities/Terminal.app/Contents/Resources/Fonts/SF-Mono-Regular.otf",
            "~/Library/Fonts/SF-Mono-Regular.otf",
            "/Library/Fonts/SF-Mono-Regular.otf",
            "/Library/Fonts/JetBrainsMono-Regular.ttf",
            "/System/Library/Fonts/SFNSMono.ttf",
            "/System/Library/Fonts/Menlo.ttc",
        ],
        0,
        0.0,
    );
    let has_cjk = load(
        "cjk",
        &["/System/Library/Fonts/Hiragino Sans GB.ttc", "/System/Library/Fonts/STHeiti Light.ttc"],
        0,
        CJK_OFFSET,
    );
    // Hiragino sits higher than the Latin monospace faces; nudge it onto their baseline.
    let has_cjk_mono = load("cjk-mono", &["/System/Library/Fonts/Hiragino Sans GB.ttc"], 0, CJK_MONO_OFFSET);
    let has_math = load("stix-math", &["/System/Library/Fonts/Supplemental/STIXTwoMath.otf"], 0, 0.0);
    let has_serif = load("stix-text", &["/System/Library/Fonts/Supplemental/STIXTwoText.ttf"], 0, 0.0);
    let has_serif_it =
        load("stix-text-it", &["/System/Library/Fonts/Supplemental/STIXTwoText-Italic.ttf"], 0, 0.0);
    let has_symbols = load("symbols", &["/System/Library/Fonts/Apple Symbols.ttf"], 0, 0.0);
    let has_bold = load("ui-bold", &["/System/Library/Fonts/HelveticaNeue.ttc"], 1, 0.0);
    let has_cjk_bold = load("cjk-bold", &["/System/Library/Fonts/Hiragino Sans GB.ttc"], 2, CJK_OFFSET);

    let fams = &mut fonts.families;
    let prop = fams.entry(FontFamily::Proportional).or_default();
    if has_ui {
        prop.insert(0, "ui".into());
    }
    if has_cjk {
        prop.push("cjk".into());
    }
    if has_math {
        prop.push("stix-math".into());
    }
    if has_symbols {
        prop.push("symbols".into());
    }

    let mono = fams.entry(FontFamily::Monospace).or_default();
    if has_mono {
        mono.insert(0, "mono".into());
    }
    if has_cjk_mono {
        mono.push("cjk-mono".into());
    }
    if has_symbols {
        mono.push("symbols".into());
    }

    let mut math = Vec::new();
    if has_math {
        math.push("stix-math".to_owned());
    }
    if has_serif {
        math.push("stix-text".to_owned());
    }
    if has_cjk {
        math.push("cjk".to_owned());
    }
    math.extend(fams[&FontFamily::Proportional].iter().cloned());
    fams.insert(FontFamily::Name(MATH_FAMILY.into()), math.clone());

    let mut math_it = Vec::new();
    if has_serif_it {
        math_it.push("stix-text-it".to_owned());
    }
    math_it.extend(math.iter().cloned());
    fams.insert(FontFamily::Name(MATH_ITALIC_FAMILY.into()), math_it);

    let mut bold = Vec::new();
    if has_bold {
        bold.push("ui-bold".to_owned());
    }
    if has_cjk_bold {
        bold.push("cjk-bold".to_owned());
    }
    bold.extend(fams[&FontFamily::Proportional].iter().cloned());
    fams.insert(FontFamily::Name(BOLD_FAMILY.into()), bold);

    let mut serif = Vec::new();
    if has_serif {
        serif.push("stix-text".to_owned());
    }
    serif.extend(math.iter().cloned());
    fams.insert(FontFamily::Name(SERIF_FAMILY.into()), serif);

    ctx.set_fonts(fonts);
}

fn expand_home(path: &str) -> std::path::PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => path.into(),
    }
}
