//! Syntax highlighting, done the way VS Code / Sublime keep it cheap:
//!
//! * **Lazy** — the UI asks for colours up to the last visible line (plus a
//!   margin); nothing below that is parsed until it scrolls into view.
//! * **Checkpointed** — the parser state is kept only every
//!   [`CHECKPOINT`] lines; resuming from the nearest checkpoint costs at most
//!   that many lines.
//! * **Compact** — a line's colours are 12-byte tokens indexing an interned
//!   style table, shared with the UI by `Arc` instead of being copied.
//!
//! Work happens on a background thread in slices, publishing partial results
//! so the viewport is coloured almost immediately even in huge files.

use crate::theme::ThemeKind;
use egui::text::{LayoutJob, LayoutSection, TextFormat};
use egui::{Color32, FontId};
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;
use syntect::highlighting::{FontStyle, HighlightState, Highlighter as ThemeHighlighter, RangedHighlightIterator, Theme};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

pub static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

/// Parser state is remembered after every CHECKPOINT-th line.
const CHECKPOINT: usize = 32;
/// Lines longer than this are left uncoloured (minified files would stall the regex engine).
const MAX_LINE_BYTES: usize = 20_000;

pub fn theme(kind: ThemeKind) -> &'static Theme {
    crate::syntax_theme::get(kind)
}

pub fn line_hash(line: &str) -> u64 {
    static STATE: LazyLock<ahash::RandomState> = LazyLock::new(|| ahash::RandomState::with_seeds(7, 11, 13, 17));
    STATE.hash_one(line)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpanStyle {
    pub color: Color32,
    pub italic: bool,
    pub underline: bool,
}

/// A coloured byte range within one line.
#[derive(Clone, Copy)]
pub struct Token {
    pub start: u32,
    pub end: u32,
    pub style: u16,
}

#[derive(Clone)]
pub struct LineSpans {
    pub hash: u64,
    pub tokens: Arc<[Token]>,
}

/// The newest published colours for a document (possibly only a prefix of it).
pub struct Highlighted {
    pub lines: Vec<LineSpans>,
    pub styles: Arc<Vec<SpanStyle>>,
}

pub type DocId = u64;

struct Job {
    doc: DocId,
    /// Text up to the last line the UI wants coloured.
    text: Arc<str>,
    syntax: String,
    theme: ThemeKind,
}

#[derive(Clone)]
pub struct HighlightService {
    tx: Sender<Job>,
    results: Arc<Mutex<HashMap<DocId, Arc<Highlighted>>>>,
}

impl HighlightService {
    pub fn start(ctx: egui::Context) -> Self {
        let (tx, rx) = channel::<Job>();
        let results: Arc<Mutex<HashMap<DocId, Arc<Highlighted>>>> = Default::default();
        let out = results.clone();
        std::thread::Builder::new().name("highlighter".into()).spawn(move || Worker::new(out, ctx).run(rx)).expect("spawn highlighter");
        Self { tx, results }
    }

    pub fn request(&self, doc: DocId, text: Arc<str>, syntax: &str, theme: ThemeKind) {
        let _ = self.tx.send(Job { doc, text, syntax: syntax.to_owned(), theme });
    }

    pub fn latest(&self, doc: DocId) -> Option<Arc<Highlighted>> {
        self.results.lock().unwrap().get(&doc).cloned()
    }

    pub fn forget(&self, doc: DocId) {
        self.results.lock().unwrap().remove(&doc);
    }
}

struct CachedLine {
    hash: u64,
    tokens: Arc<[Token]>,
    /// Parser state after this line; kept only at checkpoints and at the frontier.
    state: Option<Box<(ParseState, HighlightState)>>,
}

struct DocCache {
    syntax: String,
    theme: ThemeKind,
    lines: Vec<CachedLine>,
    styles: Vec<SpanStyle>,
    style_ids: HashMap<SpanStyle, u16>,
}

impl DocCache {
    fn new(syntax: &str, theme: ThemeKind) -> Self {
        Self { syntax: syntax.to_owned(), theme, lines: Vec::new(), styles: Vec::new(), style_ids: HashMap::new() }
    }

    fn intern(&mut self, s: SpanStyle) -> u16 {
        if let Some(&id) = self.style_ids.get(&s) {
            return id;
        }
        let id = self.styles.len().min(u16::MAX as usize) as u16;
        self.styles.push(s);
        self.style_ids.insert(s, id);
        id
    }
}

struct Worker {
    out: Arc<Mutex<HashMap<DocId, Arc<Highlighted>>>>,
    ctx: egui::Context,
    caches: HashMap<DocId, DocCache>,
    pending: HashMap<DocId, Job>,
}

impl Worker {
    fn new(out: Arc<Mutex<HashMap<DocId, Arc<Highlighted>>>>, ctx: egui::Context) -> Self {
        Self { out, ctx, caches: HashMap::new(), pending: HashMap::new() }
    }

    fn run(mut self, rx: Receiver<Job>) {
        loop {
            if self.pending.is_empty() {
                match rx.recv() {
                    Ok(j) => {
                        self.pending.insert(j.doc, j);
                    }
                    Err(_) => return,
                }
            }
            while let Ok(j) = rx.try_recv() {
                self.pending.insert(j.doc, j);
            }
            let Some(&doc) = self.pending.keys().next() else { continue };
            let job = self.pending.remove(&doc).unwrap();
            self.process(job, &rx);
        }
    }

    fn publish(&self, doc: DocId, cache: &DocCache) {
        let lines = cache.lines.iter().map(|l| LineSpans { hash: l.hash, tokens: l.tokens.clone() }).collect();
        let h = Highlighted { lines, styles: Arc::new(cache.styles.clone()) };
        self.out.lock().unwrap().insert(doc, Arc::new(h));
        self.ctx.request_repaint();
    }

    fn process(&mut self, job: Job, rx: &Receiver<Job>) {
        let Some(syntax) = SYNTAXES.find_syntax_by_name(&job.syntax) else { return };
        let mut cache = self.caches.remove(&job.doc).unwrap_or_else(|| DocCache::new(&job.syntax, job.theme));
        if cache.syntax != job.syntax || cache.theme != job.theme {
            cache = DocCache::new(&job.syntax, job.theme);
        }
        let highlighter = ThemeHighlighter::new(theme(job.theme));
        let new_lines: Vec<&str> = job.text.split_inclusive('\n').collect();
        let hashes: Vec<u64> = new_lines.iter().map(|l| line_hash(l)).collect();

        // Old lines are reusable up to the first changed one…
        let prefix = cache.lines.iter().zip(&hashes).take_while(|(c, h)| c.hash == **h).count();
        // …and the unchanged tail is reusable once the parser state converges again.
        let max_suffix = cache.lines.len().min(new_lines.len()) - prefix;
        let suffix = cache.lines.iter().rev().zip(hashes.iter().rev()).take(max_suffix).take_while(|(c, h)| c.hash == **h).count();
        let old_len = cache.lines.len();
        let old_suffix_start = old_len - suffix;
        let new_suffix_start = new_lines.len() - suffix;
        let mut old_suffix: Vec<CachedLine> = cache.lines.drain(old_suffix_start..).collect();
        let before_suffix: Option<(ParseState, HighlightState)> = match old_suffix_start.checked_sub(1) {
            Some(j) => cache.lines.get(j).and_then(|l| l.state.as_deref().cloned()),
            None => Some((ParseState::new(syntax), HighlightState::new(&highlighter, ScopeStack::new()))),
        };
        cache.lines.truncate(prefix);
        // Resume from the last checkpoint inside the reusable prefix.
        let resume = cache.lines.iter().rposition(|l| l.state.is_some()).map_or(0, |i| i + 1);
        let (mut parse, mut hl) = match resume.checked_sub(1) {
            Some(i) => cache.lines[i].state.as_deref().cloned().unwrap(),
            None => (ParseState::new(syntax), HighlightState::new(&highlighter, ScopeStack::new())),
        };
        cache.lines.truncate(resume);
        // Drop frontier states that are not checkpoints.
        for (i, l) in cache.lines.iter_mut().enumerate() {
            if (i + 1) % CHECKPOINT != 0 && i + 1 != resume {
                l.state = None;
            }
        }

        let mut slice_start = Instant::now();
        let mut i = resume;
        while i < new_lines.len() {
            if i >= new_suffix_start {
                let k = i - new_suffix_start;
                let prev_state = match k.checked_sub(1) {
                    Some(j) => old_suffix[j].state.as_deref().map(|s| (&s.0, &s.1)),
                    None => before_suffix.as_ref().map(|s| (&s.0, &s.1)),
                };
                if prev_state.is_some_and(|(p, h)| *p == parse && *h == hl) {
                    cache.lines.extend(old_suffix.drain(k..));
                    break;
                }
            }
            let tokens = highlight_tokens(new_lines[i], &mut parse, &mut hl, &highlighter, &mut cache);
            let keep = (i + 1) % CHECKPOINT == 0 || i + 1 == new_lines.len();
            cache.lines.push(CachedLine { hash: hashes[i], tokens, state: keep.then(|| Box::new((parse.clone(), hl.clone()))) });
            i += 1;
            // Time-slice: show progress, and yield to a newer request for this document.
            if slice_start.elapsed().as_millis() > 40 {
                if let Some(last) = cache.lines.last_mut() {
                    if last.state.is_none() {
                        last.state = Some(Box::new((parse.clone(), hl.clone())));
                    }
                }
                self.publish(job.doc, &cache);
                while let Ok(j) = rx.try_recv() {
                    self.pending.insert(j.doc, j);
                }
                if self.pending.contains_key(&job.doc) {
                    self.caches.insert(job.doc, cache);
                    return;
                }
                slice_start = Instant::now();
            }
        }
        self.publish(job.doc, &cache);
        self.caches.insert(job.doc, cache);
    }
}

fn highlight_tokens(line: &str, parse: &mut ParseState, hl: &mut HighlightState, highlighter: &ThemeHighlighter, cache: &mut DocCache) -> Arc<[Token]> {
    if line.len() > MAX_LINE_BYTES {
        return Arc::from([]);
    }
    let spans = highlight_line(line, parse, hl, highlighter);
    spans.into_iter().map(|(start, end, s)| Token { start, end, style: cache.intern(s) }).collect()
}

fn highlight_line(line: &str, parse: &mut ParseState, hl: &mut HighlightState, highlighter: &ThemeHighlighter) -> Vec<(u32, u32, SpanStyle)> {
    let ops = match parse.parse_line(line, &SYNTAXES) {
        Ok(ops) => ops,
        Err(_) => return Vec::new(),
    };
    let mut spans: Vec<(u32, u32, SpanStyle)> = Vec::new();
    for (style, _, range) in RangedHighlightIterator::new(hl, &ops, line, highlighter) {
        let s = SpanStyle {
            color: Color32::from_rgb(style.foreground.r, style.foreground.g, style.foreground.b),
            italic: style.font_style.contains(FontStyle::ITALIC),
            underline: style.font_style.contains(FontStyle::UNDERLINE),
        };
        let (start, end) = (range.start as u32, range.end as u32);
        match spans.last_mut() {
            Some(last) if last.2 == s && last.1 == start => last.1 = end,
            _ => spans.push((start, end, s)),
        }
    }
    spans
}

/// Pick the syntect syntax for a file: extension, then full file name, then shebang / first line.
pub fn syntax_for(path: &Path, first_line: &str) -> String {
    let ss = &*SYNTAXES;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let found = ss
        .find_syntax_by_extension(ext)
        .or_else(|| ss.find_syntax_by_extension(&ext.to_ascii_lowercase()))
        .or_else(|| ss.find_syntax_by_extension(name))
        .or_else(|| ss.find_syntax_by_first_line(first_line))
        .or_else(|| match ext {
            "jsonc" | "json5" | "ipynb" | "geojson" => ss.find_syntax_by_extension("json"),
            "xsl" | "xslt" | "xsd" | "plist" | "svg" => ss.find_syntax_by_extension("xml"),
            "mdx" | "qmd" | "rmd" => ss.find_syntax_by_extension("md"),
            "katex" => ss.find_syntax_by_extension("tex"),
            _ => None,
        });
    found.unwrap_or_else(|| ss.find_syntax_plain_text()).name.clone()
}

pub fn syntax_for_token(token: &str) -> Option<&'static SyntaxReference> {
    let ss = &*SYNTAXES;
    let t = token.trim().to_ascii_lowercase();
    let alias = match t.as_str() {
        "js" | "javascript" | "node" => "js",
        "ts" | "typescript" => "ts",
        "py" | "python" | "python3" => "py",
        "rs" | "rust" => "rs",
        "sh" | "bash" | "shell" | "zsh" | "console" => "sh",
        "c++" | "cpp" => "cpp",
        "c#" | "csharp" | "cs" => "cs",
        "yml" | "yaml" => "yaml",
        "md" | "markdown" => "md",
        "golang" | "go" => "go",
        "tex" | "latex" => "tex",
        "html" | "xml" | "json" | "toml" | "sql" | "java" | "kotlin" | "swift" | "ruby" | "rb"
        | "php" | "lua" | "css" | "scss" | "c" | "h" | "hs" | "dockerfile" | "makefile" => t.as_str(),
        other => other,
    };
    ss.find_syntax_by_token(alias).or_else(|| ss.find_syntax_by_extension(alias))
}

/// Synchronous highlighting for small snippets (markdown code blocks).
pub fn highlight_snippet(code: &str, lang: &str, kind: ThemeKind, font: FontId, fallback: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let Some(syntax) = syntax_for_token(lang) else {
        job.append(code, 0.0, TextFormat::simple(font, fallback));
        return job;
    };
    let highlighter = ThemeHighlighter::new(theme(kind));
    let mut parse = ParseState::new(syntax);
    let mut hl = HighlightState::new(&highlighter, ScopeStack::new());
    for line in code.split_inclusive('\n') {
        let spans = highlight_line(line, &mut parse, &mut hl, &highlighter);
        push_line(&mut job, line, Some(&spans), &font, fallback);
    }
    job
}

fn push_line(job: &mut LayoutJob, line: &str, spans: Option<&[(u32, u32, SpanStyle)]>, font: &FontId, fallback: Color32) {
    let base = job.text.len();
    job.text.push_str(line);
    let plain = TextFormat::simple(font.clone(), fallback);
    let mut covered = 0usize;
    if let Some(spans) = spans {
        for &(s, e, style) in spans {
            let (s, e) = (s as usize, (e as usize).min(line.len()));
            if s >= e || e > line.len() {
                continue;
            }
            if s > covered {
                job.sections.push(LayoutSection {
                    leading_space: 0.0,
                    byte_range: base + covered..base + s,
                    format: plain.clone(),
                });
            }
            let mut format = TextFormat::simple(font.clone(), style.color);
            format.italics = style.italic;
            if style.underline {
                format.underline = egui::Stroke::new(1.0_f32, style.color);
            }
            job.sections.push(LayoutSection { leading_space: 0.0, byte_range: base + s..base + e, format });
            covered = e;
        }
    }
    if covered < line.len() {
        job.sections.push(LayoutSection { leading_space: 0.0, byte_range: base + covered..base + line.len(), format: plain });
    }
}
