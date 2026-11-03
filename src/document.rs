//! An open file: its text, format, view mode and on-disk bookkeeping.

use crate::formats::{Format, ViewMode};
use crate::highlight::{self, DocId};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineEnding {
    Lf,
    CrLf,
}

pub struct Document {
    pub id: DocId,
    pub path: Option<PathBuf>,
    /// Always LF internally; CRLF files are converted back on save.
    pub text: String,
    /// Last line the editor showed; highlighting is computed up to (a bit past) here.
    pub hl_want: usize,
    pub revision: u64,
    pub saved_revision: u64,
    pub format: Format,
    pub view: ViewMode,
    pub syntax: String,
    pub line_ending: LineEnding,
    pub indent: Indent,
    pub wrap: bool,
    pub mtime: Option<SystemTime>,
    pub disk_changed: bool,
    pub binary: Option<Arc<Vec<u8>>>,
    pub load_error: Option<String>,
    pub scroll_fraction: f32,
    pub edit: crate::editor::EditState,
    /// One-shot request for the editor to select this byte range and scroll there.
    pub goto: Option<(usize, usize)>,
    pub cursor: (usize, usize),
    pub selection_chars: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Indent {
    Spaces(u8),
    Tabs,
}

impl Indent {
    pub fn unit(self) -> String {
        match self {
            Indent::Spaces(n) => " ".repeat(n as usize),
            Indent::Tabs => "\t".into(),
        }
    }

    pub fn label(self) -> String {
        match self {
            Indent::Spaces(n) => format!("空格: {n}"),
            Indent::Tabs => "Tab".into(),
        }
    }
}

impl Document {
    pub fn untitled() -> Self {
        Self::from_text(None, String::new())
    }

    fn from_text(path: Option<PathBuf>, raw: String) -> Self {
        let line_ending = if raw.contains("\r\n") { LineEnding::CrLf } else { LineEnding::Lf };
        let text = if line_ending == LineEnding::CrLf { raw.replace("\r\n", "\n") } else { raw };
        let format = path.as_deref().map(Format::detect).unwrap_or(Format::Markdown);
        let first_line = text.lines().next().unwrap_or("");
        let syntax = match &path {
            Some(p) => highlight::syntax_for(p, first_line),
            None => highlight::syntax_for(Path::new("untitled.md"), first_line),
        };
        let indent = detect_indent(&text);
        // Soft-wrap by default; ⌥Z turns it off per document.
        let wrap = true;
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            path,
            text,
            hl_want: 0,
            revision: 0,
            saved_revision: 0,
            view: initial_view(format),
            format,
            syntax,
            line_ending,
            indent,
            wrap,
            mtime: None,
            disk_changed: false,
            binary: None,
            load_error: None,
            scroll_fraction: 0.0,
            edit: Default::default(),
            goto: None,
            cursor: (1, 1),
            selection_chars: 0,
        }
    }

    /// Open `path`; a missing file becomes an empty document that is created on first save.
    pub fn open(path: &Path) -> Self {
        let format = Format::detect(path);
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if format.is_binary() {
            let mut doc = Self::from_text(Some(path.to_owned()), String::new());
            match std::fs::read(path) {
                Ok(bytes) => doc.binary = Some(Arc::new(bytes)),
                Err(e) => doc.load_error = Some(e.to_string()),
            }
            doc.mtime = mtime;
            return doc;
        }
        let (raw, err) = match std::fs::read(path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(s) => (s, None),
                Err(e) => {
                    let bytes = e.into_bytes();
                    if bytes.iter().take(8192).any(|b| *b == 0) {
                        (String::new(), Some("二进制文件，无法以文本方式显示".to_owned()))
                    } else {
                        (String::from_utf8_lossy(&bytes).into_owned(), Some("非 UTF-8 编码，已按替换字符显示".to_owned()))
                    }
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
            Err(e) => (String::new(), Some(e.to_string())),
        };
        let mut doc = Self::from_text(Some(path.to_owned()), raw);
        doc.load_error = err;
        doc.mtime = mtime;
        doc
    }

    pub fn title(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "未命名".to_owned())
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_changed(&mut self) {
        self.revision += 1;
    }

    /// The text up to the end of 0-based line `line` (or all of it), for the highlighter.
    pub fn prefix_through_line(&self, line: usize) -> Arc<str> {
        let end = self.text.bytes().enumerate().filter(|(_, b)| *b == b'\n').nth(line).map_or(self.text.len(), |(i, _)| i + 1);
        Arc::from(&self.text[..end])
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Err(std::io::Error::other("no path"));
        };
        if self.binary.is_some() {
            return Ok(());
        }
        let out = match self.line_ending {
            LineEnding::Lf => std::borrow::Cow::Borrowed(self.text.as_str()),
            LineEnding::CrLf => std::borrow::Cow::Owned(self.text.replace('\n', "\r\n")),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Write-then-rename so a crash never leaves a half-written file.
        let tmp = path.with_file_name(format!(".{}.cryocode-tmp", self.title()));
        std::fs::write(&tmp, out.as_bytes())?;
        if let Ok(meta) = std::fs::metadata(&path) {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        }
        std::fs::rename(&tmp, &path)?;
        self.saved_revision = self.revision;
        self.mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        self.disk_changed = false;
        Ok(())
    }

    pub fn set_path(&mut self, path: PathBuf) {
        self.format = Format::detect(&path);
        self.syntax = highlight::syntax_for(&path, self.text.lines().next().unwrap_or(""));
        self.path = Some(path);
    }

    /// Reload from disk if the file changed and we have no unsaved edits.
    pub fn poll_disk(&mut self) -> bool {
        let Some(path) = &self.path else { return false };
        let Ok(mtime) = std::fs::metadata(path).and_then(|m| m.modified()) else { return false };
        if Some(mtime) == self.mtime {
            return false;
        }
        if self.is_dirty() {
            self.disk_changed = true;
            return false;
        }
        let fresh = Document::open(path);
        self.text = fresh.text;
        self.edit.reset_history();
        let len = self.text.len();
        self.edit.sel.anchor = self.edit.sel.anchor.min(len);
        self.edit.sel.head = self.edit.sel.head.min(len);
        while !self.text.is_char_boundary(self.edit.sel.head) {
            self.edit.sel.head -= 1;
        }
        self.edit.sel.anchor = self.edit.sel.head;
        self.binary = fresh.binary;
        self.line_ending = fresh.line_ending;
        self.mtime = Some(mtime);
        self.mark_changed();
        self.saved_revision = self.revision;
        true
    }

    pub fn line_count(&self) -> usize {
        self.text.bytes().filter(|b| *b == b'\n').count() + 1
    }
}

fn detect_indent(text: &str) -> Indent {
    let mut tabs = 0;
    let mut spaces = [0usize; 9];
    for line in text.lines().take(2000) {
        if line.starts_with('\t') {
            tabs += 1;
        } else {
            let n = line.chars().take_while(|c| *c == ' ').count();
            if (2..=8).contains(&n) && line.len() > n {
                spaces[n] += 1;
            }
        }
    }
    let space_lines: usize = spaces.iter().sum();
    if tabs > space_lines {
        return Indent::Tabs;
    }
    if spaces[2] > 0 && spaces[2] * 2 >= spaces[4] {
        Indent::Spaces(2)
    } else {
        Indent::Spaces(4)
    }
}

/// `CRYOCODE_VIEW=preview|split` opens documents in that view (handy for scripted screenshots).
fn initial_view(format: Format) -> ViewMode {
    match std::env::var("CRYOCODE_VIEW").as_deref() {
        Ok("preview") if format.has_preview() => ViewMode::Preview,
        Ok("split") if format.has_preview() && !format.is_binary() => ViewMode::Split,
        _ => format.default_view(),
    }
}
