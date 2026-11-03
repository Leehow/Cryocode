//! Per-document editing state: selection, undo history and the editing
//! commands that operate on whole lines.

use super::text;
use crate::document::{Document, Indent};
use std::ops::Range;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(i: usize) -> Self {
        Self { anchor: i, head: i }
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditKind {
    Typing,
    Delete,
    Other,
}

#[derive(Clone)]
struct Edit {
    at: usize,
    removed: String,
    inserted: String,
}

struct Step {
    edits: Vec<Edit>,
    sel_before: Selection,
    sel_after: Selection,
    kind: EditKind,
    at: Instant,
}

#[derive(Default)]
pub struct EditState {
    pub sel: Selection,
    /// Horizontal position remembered across vertical moves.
    pub preferred_x: Option<f32>,
    pub preedit: Option<String>,
    pub scroll_to_caret: bool,
    pub last_activity: Option<Instant>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// While set, every edit joins the current undo step (multi-part commands).
    grouping: bool,
}

impl EditState {
    pub fn reset_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn begin_group(&mut self) {
        self.grouping = true;
        self.undo.push(Step { edits: Vec::new(), sel_before: self.sel, sel_after: self.sel, kind: EditKind::Other, at: Instant::now() });
    }

    pub fn end_group(&mut self) {
        self.grouping = false;
        if self.undo.last().is_some_and(|s| s.edits.is_empty()) {
            self.undo.pop();
        }
    }
}

impl Document {
    /// Replace `range` with `insert`, record undo, and put the caret after the insertion.
    pub fn replace(&mut self, range: Range<usize>, insert: &str, kind: EditKind) {
        let range = range.start.min(self.text.len())..range.end.min(self.text.len());
        let removed = self.text[range.clone()].to_owned();
        if removed.is_empty() && insert.is_empty() {
            return;
        }
        let before = self.edit.sel;
        self.text.replace_range(range.clone(), insert);
        let after = Selection::caret(range.start + insert.len());
        self.edit.sel = after;
        self.edit.redo.clear();
        let edit = Edit { at: range.start, removed, inserted: insert.to_owned() };
        let now = Instant::now();
        let merge = self.edit.grouping
            || self.edit.undo.last().is_some_and(|s| {
                s.kind == kind
                    && kind != EditKind::Other
                    && now.duration_since(s.at) < Duration::from_millis(1200)
                    && s.sel_after == before
                    && !insert.contains('\n')
            });
        if merge {
            if let Some(step) = self.edit.undo.last_mut() {
                step.edits.push(edit);
                step.sel_after = after;
                step.at = now;
            }
        } else {
            self.edit.undo.push(Step { edits: vec![edit], sel_before: before, sel_after: after, kind, at: now });
            if self.edit.undo.len() > 2000 {
                self.edit.undo.remove(0);
            }
        }
        self.edit.preferred_x = None;
        self.edit.scroll_to_caret = true;
        self.mark_changed();
    }

    pub fn undo(&mut self) {
        let Some(step) = self.edit.undo.pop() else { return };
        for e in step.edits.iter().rev() {
            self.text.replace_range(e.at..e.at + e.inserted.len(), &e.removed);
        }
        self.edit.sel = step.sel_before;
        self.edit.redo.push(step);
        self.after_history_move();
    }

    pub fn redo(&mut self) {
        let Some(step) = self.edit.redo.pop() else { return };
        for e in &step.edits {
            self.text.replace_range(e.at..e.at + e.removed.len(), &e.inserted);
        }
        self.edit.sel = step.sel_after;
        self.edit.undo.push(step);
        self.after_history_move();
    }

    fn after_history_move(&mut self) {
        let len = self.text.len();
        self.edit.sel.anchor = self.edit.sel.anchor.min(len);
        self.edit.sel.head = self.edit.sel.head.min(len);
        self.edit.preferred_x = None;
        self.edit.scroll_to_caret = true;
        self.mark_changed();
    }

    pub fn insert_text(&mut self, s: &str) {
        let r = self.edit.sel.range();
        let kind = if s.chars().count() <= 2 { EditKind::Typing } else { EditKind::Other };
        self.replace(r, s, kind);
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.edit.sel.range()]
    }

    /// Lines touched by the selection, as (first line start, end of last line without '\n').
    fn selected_lines(&self) -> Range<usize> {
        let r = self.edit.sel.range();
        let start = self.text[..r.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let mut end_probe = r.end;
        if r.end > r.start && self.text[..r.end].ends_with('\n') {
            end_probe -= 1;
        }
        let end = self.text[end_probe..].find('\n').map(|i| end_probe + i).unwrap_or(self.text.len());
        start..end
    }

    /// Replace a whole-line block and select the result (keeps multi-line selections intact).
    fn replace_lines(&mut self, block: Range<usize>, new: &str, keep_caret_delta: Option<isize>) {
        let sel = self.edit.sel;
        self.replace(block.clone(), new, EditKind::Other);
        self.edit.sel = match keep_caret_delta {
            Some(d) if sel.is_empty() => Selection::caret(((sel.head as isize + d).max(block.start as isize)) as usize),
            _ => Selection { anchor: block.start, head: block.start + new.len() },
        };
    }

    pub fn newline_with_indent(&mut self) {
        let r = self.edit.sel.range();
        let line_start = self.text[..r.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let mut indent = text::indent_of_line(&self.text, line_start).to_owned();
        if indent.len() > r.start - line_start {
            indent.truncate(r.start - line_start);
        }
        let before = self.text[line_start..r.start].trim_end();
        let after = self.text[r.end..].chars().next();
        let opens = before.ends_with(['{', '(', '[']) || (before.ends_with(':') && !before.ends_with("::"));
        if opens {
            let unit = self.indent.unit();
            // Between a bracket pair: put the closer on its own line.
            if matches!((before.chars().last(), after), (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']'))) {
                let s = format!("\n{indent}{unit}\n{indent}");
                self.replace(r.clone(), &s, EditKind::Other);
                self.edit.sel = Selection::caret(r.start + 1 + indent.len() + unit.len());
                return;
            }
            indent.push_str(&unit);
        }
        self.replace(r, &format!("\n{indent}"), EditKind::Other);
    }

    pub fn tab(&mut self) {
        let r = self.edit.sel.range();
        if self.text[r.clone()].contains('\n') {
            return self.indent_lines(true);
        }
        let s = match self.indent {
            Indent::Tabs => "\t".to_owned(),
            Indent::Spaces(n) => {
                let line_start = self.text[..r.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = self.text[line_start..r.start].chars().count();
                " ".repeat(n as usize - col % n as usize)
            }
        };
        self.replace(r, &s, EditKind::Typing);
    }

    pub fn indent_lines(&mut self, indent: bool) {
        let block = self.selected_lines();
        let unit = self.indent.unit();
        let old = self.text[block.clone()].to_owned();
        let new = old
            .split('\n')
            .map(|l| {
                if indent {
                    if l.is_empty() { l.to_owned() } else { format!("{unit}{l}") }
                } else if let Some(r) = l.strip_prefix('\t') {
                    r.to_owned()
                } else {
                    let n = l.chars().take_while(|c| *c == ' ').count().min(unit.len().max(1));
                    l[n..].to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let first_delta = new.split('\n').next().map_or(0, str::len) as isize - old.split('\n').next().map_or(0, str::len) as isize;
        self.replace_lines(block, &new, Some(first_delta));
    }

    /// Smart backspace: inside leading spaces, delete back to the previous indent stop.
    pub fn backspace(&mut self) {
        let sel = self.edit.sel;
        if !sel.is_empty() {
            return self.replace(sel.range(), "", EditKind::Delete);
        }
        let i = sel.head;
        if i == 0 {
            return;
        }
        let line_start = self.text[..i].rfind('\n').map(|j| j + 1).unwrap_or(0);
        let before = &self.text[line_start..i];
        if let Indent::Spaces(n) = self.indent {
            if !before.is_empty() && before.bytes().all(|b| b == b' ') {
                let n = n as usize;
                let cut = match before.len() % n {
                    0 => n,
                    r => r,
                };
                return self.replace(i - cut..i, "", EditKind::Delete);
            }
        }
        // Delete an auto-closed pair as a unit.
        let prev = text::prev_char(&self.text, i);
        let pair = matches!((&self.text[prev..i], self.text[i..].chars().next()), ("(", Some(')')) | ("[", Some(']')) | ("{", Some('}')) | ("\"", Some('"')) | ("'", Some('\'')));
        let end = if pair { i + 1 } else { i };
        self.replace(prev..end, "", EditKind::Delete);
    }

    pub fn toggle_comment(&mut self, token: &str) {
        let block = self.selected_lines();
        let old = self.text[block.clone()].to_owned();
        let lines: Vec<&str> = old.split('\n').collect();
        let code = lines.iter().filter(|l| !l.trim().is_empty());
        let all_commented = code.clone().all(|l| l.trim_start().starts_with(token));
        let min_indent = code.map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
        let new = lines
            .iter()
            .map(|l| {
                if l.trim().is_empty() {
                    l.to_string()
                } else if all_commented {
                    let i = l.find(token).unwrap_or(0);
                    let rest = &l[i + token.len()..];
                    format!("{}{}", &l[..i], rest.strip_prefix(' ').unwrap_or(rest))
                } else {
                    format!("{}{token} {}", &l[..min_indent], &l[min_indent..])
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let delta = new.len() as isize - old.len() as isize;
        self.replace_lines(block, &new, Some(delta.signum() * (token.len() as isize + 1).min(delta.abs())));
    }

    pub fn duplicate_lines(&mut self) {
        let block = self.selected_lines();
        let s = self.text[block.clone()].to_owned();
        let sel = self.edit.sel;
        self.replace(block.end..block.end, &format!("\n{s}"), EditKind::Other);
        let shift = s.len() + 1;
        self.edit.sel = Selection { anchor: sel.anchor + shift, head: sel.head + shift };
    }

    pub fn move_lines(&mut self, up: bool) {
        let block = self.selected_lines();
        let sel = self.edit.sel;
        let s = self.text[block.clone()].to_owned();
        if up {
            if block.start == 0 {
                return;
            }
            let prev_start = self.text[..block.start - 1].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let prev = self.text[prev_start..block.start - 1].to_owned();
            self.replace(prev_start..block.end, &format!("{s}\n{prev}"), EditKind::Other);
            let d = prev.len() + 1;
            self.edit.sel = Selection { anchor: sel.anchor - d, head: sel.head - d };
        } else {
            if block.end >= self.text.len() {
                return;
            }
            let next_end = self.text[block.end + 1..].find('\n').map(|i| block.end + 1 + i).unwrap_or(self.text.len());
            let next = self.text[block.end + 1..next_end].to_owned();
            self.replace(block.start..next_end, &format!("{next}\n{s}"), EditKind::Other);
            let d = next.len() + 1;
            self.edit.sel = Selection { anchor: sel.anchor + d, head: sel.head + d };
        }
    }

    /// Select the current line including its newline (⌘L), extending on repeat.
    pub fn select_line(&mut self) {
        let r = self.edit.sel.range();
        let start = self.text[..r.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let probe = if !self.edit.sel.is_empty() && self.text[..r.end].ends_with('\n') { r.end } else { r.end };
        let end = self.text[probe..].find('\n').map(|i| probe + i + 1).unwrap_or(self.text.len());
        self.edit.sel = Selection { anchor: start, head: end };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        let mut d = Document::untitled();
        d.text = text.to_owned();
        d.indent = Indent::Spaces(4);
        d
    }

    #[test]
    fn typing_groups_and_undoes() {
        let mut d = doc("");
        for c in ["a", "b", "c"] {
            d.insert_text(c);
        }
        assert_eq!(d.text, "abc");
        d.undo();
        assert_eq!(d.text, "");
        d.redo();
        assert_eq!(d.text, "abc");
        assert_eq!(d.edit.sel, Selection::caret(3));
    }

    #[test]
    fn enter_keeps_indent_and_splits_braces() {
        let mut d = doc("    fn x() {}");
        d.edit.sel = Selection::caret(12);
        d.newline_with_indent();
        assert_eq!(d.text, "    fn x() {\n        \n    }");
        assert_eq!(d.edit.sel, Selection::caret(21));
    }

    #[test]
    fn indent_comment_move() {
        let mut d = doc("a\nb\nc");
        d.edit.sel = Selection { anchor: 0, head: 3 };
        d.indent_lines(true);
        assert_eq!(d.text, "    a\n    b\nc");
        d.indent_lines(false);
        assert_eq!(d.text, "a\nb\nc");
        d.edit.sel = Selection::caret(2);
        d.toggle_comment("//");
        assert_eq!(d.text, "a\n// b\nc");
        d.toggle_comment("//");
        assert_eq!(d.text, "a\nb\nc");
        d.edit.sel = Selection::caret(2);
        d.move_lines(true);
        assert_eq!(d.text, "b\na\nc");
        assert_eq!(d.edit.sel, Selection::caret(0));
        d.move_lines(false);
        assert_eq!(d.text, "a\nb\nc");
    }

    #[test]
    fn smart_backspace() {
        let mut d = doc("        x");
        d.edit.sel = Selection::caret(8);
        d.backspace();
        assert_eq!(d.text, "    x");
    }
}
