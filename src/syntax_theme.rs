//! Syntax colour themes, ported from GitHub's official VS Code themes
//! ("GitHub Light Default" / "GitHub Dark Default", primer/github-vscode-theme),
//! which are tuned for contrast on white and near-black canvases.

use crate::theme::ThemeKind;
use std::str::FromStr;
use std::sync::LazyLock;
use syntect::highlighting::{Color, FontStyle, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings};

struct Tones {
    fg: u32,
    bg: u32,
    comment: u32,
    keyword: u32,
    constant: u32,
    entity: u32,
    function: u32,
    string: u32,
    tag: u32,
    invalid: u32,
    heading: u32,
}

const LIGHT: Tones = Tones {
    fg: 0x1f2328,
    bg: 0xffffff,
    comment: 0x6e7781,
    keyword: 0xcf222e,
    constant: 0x0550ae,
    entity: 0x953800,
    function: 0x8250df,
    string: 0x0a3069,
    tag: 0x116329,
    invalid: 0x82071e,
    heading: 0x0550ae,
};

const DARK: Tones = Tones {
    fg: 0xe6edf3,
    bg: 0x0d1117,
    comment: 0x8b949e,
    keyword: 0xff7b72,
    constant: 0x79c0ff,
    entity: 0xffa657,
    function: 0xd2a8ff,
    string: 0xa5d6ff,
    tag: 0x7ee787,
    invalid: 0xffa198,
    heading: 0x79c0ff,
};

fn rgb(v: u32) -> Color {
    Color { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8, a: 0xff }
}

fn build(name: &str, t: &Tones) -> Theme {
    // (scope selectors, colour, font style) in GitHub's rule order; later rules win ties.
    let rules: Vec<(&str, u32, Option<FontStyle>)> = vec![
        ("comment, punctuation.definition.comment, string.comment", t.comment, None),
        ("constant.other.placeholder, constant.character", t.keyword, None),
        ("constant, entity.name.constant, variable.other.constant, variable.other.enummember, variable.language, entity", t.constant, None),
        ("entity.name, meta.export.default, meta.definition.variable", t.entity, None),
        ("variable.parameter.function, meta.jsx.children, meta.block, meta.tag.attributes, entity.name.constant, meta.object.member, meta.embedded.expression", t.fg, None),
        ("entity.name.function, support.function", t.function, None),
        ("entity.name.tag, support.class.component", t.tag, None),
        ("keyword, storage, storage.type", t.keyword, None),
        ("storage.modifier.package, storage.modifier.import, storage.type.java", t.fg, None),
        ("string, punctuation.definition.string, string punctuation.section.embedded source", t.string, None),
        ("support, meta.property-name, support.type.property-name", t.constant, None),
        ("variable", t.entity, None),
        ("variable.other, variable.parameter", t.fg, None),
        ("invalid.broken, invalid.deprecated, invalid.illegal, invalid.unimplemented", t.invalid, Some(FontStyle::ITALIC)),
        ("string.regexp, source.regexp", t.string, None),
        ("string.regexp constant.character.escape", t.tag, None),
        ("support.constant, constant.numeric, constant.language", t.constant, None),
        ("meta.module-reference, punctuation.definition.list.begin.markdown", t.constant, None),
        ("markup.heading, markup.heading entity.name, punctuation.definition.heading", t.heading, Some(FontStyle::BOLD)),
        ("markup.quote", t.tag, None),
        ("markup.italic", t.fg, Some(FontStyle::ITALIC)),
        ("markup.bold", t.fg, Some(FontStyle::BOLD)),
        ("markup.raw, markup.inline.raw", t.constant, None),
        ("markup.inserted, meta.diff.header.to-file, punctuation.definition.inserted", t.tag, None),
        ("markup.deleted, meta.diff.header.from-file, punctuation.definition.deleted", t.invalid, None),
        ("markup.underline.link, string.other.link", t.string, None),
        ("meta.diff.range, meta.diff.header", t.function, None),
        ("keyword.operator, punctuation.accessor", t.keyword, None),
        ("punctuation, meta.brace, keyword.operator.assignment", t.fg, None),
    ];
    let scopes = rules
        .into_iter()
        .filter_map(|(sel, c, style)| {
            Some(ThemeItem {
                scope: ScopeSelectors::from_str(sel).ok()?,
                style: StyleModifier { foreground: Some(rgb(c)), background: None, font_style: style },
            })
        })
        .collect();
    Theme {
        name: Some(name.to_owned()),
        author: Some("GitHub (ported)".into()),
        settings: ThemeSettings { foreground: Some(rgb(t.fg)), background: Some(rgb(t.bg)), ..Default::default() },
        scopes,
    }
}

static LIGHT_THEME: LazyLock<Theme> = LazyLock::new(|| build("GitHub Light Default", &LIGHT));
static DARK_THEME: LazyLock<Theme> = LazyLock::new(|| build("GitHub Dark Default", &DARK));

pub fn get(kind: ThemeKind) -> &'static Theme {
    match kind {
        ThemeKind::Light => &LIGHT_THEME,
        ThemeKind::Dark => &DARK_THEME,
    }
}
