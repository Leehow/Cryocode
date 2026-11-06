//! Parsing structured data with positioned problems.
//!
//! Broken files are the normal state while editing, so instead of a bare
//! "parse failed" this reports *where* (byte offset → line/column) and, for the
//! most common JSON slip — a missing comma — repairs a working copy so the
//! preview keeps rendering and offers a one-click fix.

use super::tree::{Value, XmlNode};
use crate::formats::Format;

#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    /// Byte offset in the original text.
    pub offset: usize,
    /// 1-based line and column (in characters).
    pub line: usize,
    pub col: usize,
    pub message: String,
    /// Insert a comma at this byte offset to fix it.
    pub insert_comma_at: Option<usize>,
}

pub struct DataParse<T> {
    /// `Some` when the text parsed, possibly after repairs listed in `problems`.
    pub value: Option<T>,
    pub problems: Vec<Problem>,
}

impl Problem {
    fn at(text: &str, offset: usize, message: String, insert_comma_at: Option<usize>) -> Self {
        let offset = floor_boundary(text, offset.min(text.len()));
        let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
        let line = text[..offset].bytes().filter(|b| *b == b'\n').count() + 1;
        let col = text[line_start..offset].chars().count() + 1;
        Self { offset, line, col, message, insert_comma_at }
    }

    pub fn is_fixable(&self) -> bool {
        self.insert_comma_at.is_some()
    }
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Byte offset of a 1-based (line, column) position, where `column` counts bytes like serde_json does.
fn offset_of(text: &str, line: usize, column: usize) -> usize {
    let start = if line <= 1 { 0 } else { text.match_indices('\n').nth(line - 2).map_or(text.len(), |(i, _)| i + 1) };
    floor_boundary(text, (start + column.saturating_sub(1)).min(text.len()))
}

pub fn parse_data(format: Format, text: &str) -> DataParse<Value> {
    match format {
        Format::Json => parse_json(text),
        Format::Yaml => {
            let mut docs = Vec::new();
            for d in serde_yaml::Deserializer::from_str(text) {
                match <serde_yaml::Value as serde::Deserialize>::deserialize(d) {
                    Ok(v) => docs.push(Value::from_yaml(v)),
                    Err(e) => {
                        let offset = e.location().map_or(0, |l| l.index());
                        return DataParse { value: None, problems: vec![Problem::at(text, offset, clean_message(&e.to_string()), None)] };
                    }
                }
            }
            DataParse { value: Some(if docs.len() == 1 { docs.pop().unwrap() } else { Value::Arr(docs) }), problems: vec![] }
        }
        Format::Toml => match toml::from_str::<toml::Value>(text) {
            Ok(v) => DataParse { value: Some(Value::from_toml(v)), problems: vec![] },
            Err(e) => {
                let offset = e.span().map_or(0, |s| s.start);
                DataParse { value: None, problems: vec![Problem::at(text, offset, e.message().to_owned(), None)] }
            }
        },
        _ => DataParse { value: None, problems: vec![] },
    }
}

pub fn parse_xml(text: &str) -> DataParse<XmlNode> {
    match super::tree::parse_xml_doc(text) {
        Ok(n) => DataParse { value: Some(n), problems: vec![] },
        Err(e) => {
            let pos = e.pos();
            let offset = text.lines().take(pos.row.saturating_sub(1) as usize).map(|l| l.len() + 1).sum::<usize>()
                + text.lines().nth(pos.row.saturating_sub(1) as usize).map_or(0, |l| l.chars().take(pos.col.saturating_sub(1) as usize).map(char::len_utf8).sum());
            DataParse { value: None, problems: vec![Problem::at(text, offset, clean_message(&e.to_string()), None)] }
        }
    }
}

/// Drop "at line X column Y" tails — the position is shown separately.
fn clean_message(m: &str) -> String {
    let m = m.split(" at line ").next().unwrap_or(m);
    let m = m.split(" at position ").next().unwrap_or(m);
    m.trim().to_owned()
}

fn parse_json(text: &str) -> DataParse<Value> {
    // Comments and trailing commas (JSONC) are blanked out in place, keeping byte offsets.
    let mut work = neutralize_jsonc(text);
    let mut problems = Vec::new();
    // Positions (in `work`) of commas we inserted, ascending.
    let mut inserted: Vec<usize> = Vec::new();
    for _ in 0..64 {
        match serde_json::from_str::<serde_json::Value>(&work) {
            Ok(v) => return DataParse { value: Some(Value::from_json(v)), problems },
            Err(e) => {
                let msg = e.to_string();
                let at = offset_of(&work, e.line(), e.column());
                let expected_comma = msg.starts_with("expected `,` or `]`") || msg.starts_with("expected `,` or `}`");
                // Only a missing comma if another value really starts here; otherwise it's a stray character.
                let next = work[at..].chars().next();
                let value_starts = next.is_some_and(|c| matches!(c, '"' | '{' | '[' | '-' | '0'..='9' | 't' | 'f' | 'n'));
                let missing_comma = expected_comma && value_starts;
                if expected_comma && !value_starts {
                    let orig = at - inserted.iter().filter(|&&p| p < at).count();
                    let what = next.map_or("多余的内容".to_owned(), |c| format!("多余的字符「{c}」"));
                    problems.push(Problem::at(text, orig, what, None));
                    return DataParse { value: None, problems };
                }
                // The comma belongs right after the previous value, not where the parser noticed.
                let prev_end = work[..at].trim_end().len();
                if missing_comma && prev_end > 0 {
                    let orig = prev_end - inserted.iter().filter(|&&p| p < prev_end).count();
                    let next_line = Problem::at(text, orig, "缺少逗号".into(), Some(orig));
                    problems.push(next_line);
                    work.insert(prev_end, ',');
                    inserted.push(prev_end);
                    inserted.sort_unstable();
                    continue;
                }
                let orig = at - inserted.iter().filter(|&&p| p < at).count();
                problems.push(Problem::at(text, orig, translate_json_error(&clean_message(&msg)), None));
                return DataParse { value: None, problems };
            }
        }
    }
    DataParse { value: None, problems }
}

/// Short Chinese wording for the frequent serde_json messages; anything else is shown as is.
fn translate_json_error(m: &str) -> String {
    match m {
        "EOF while parsing an object" | "EOF while parsing a list" => "文件在括号闭合前就结束了（缺少 } 或 ]）".into(),
        "EOF while parsing a string" => "字符串没有闭合的引号".into(),
        "key must be a string" => "键名必须用双引号括起来".into(),
        "expected `:`" => "键名后缺少冒号".into(),
        "expected value" => "这里需要一个值".into(),
        "trailing characters" => "JSON 结束后还有多余内容".into(),
        _ => m.to_owned(),
    }
}

/// Replace `//` / `/* */` comments and trailing commas with spaces (same byte length).
fn neutralize_jsonc(s: &str) -> String {
    let mut out = s.as_bytes().to_vec();
    // Pass 1: comments.
    scan_outside_strings(&out.clone(), |i, bytes| {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            let end = bytes[i..].iter().position(|b| *b == b'\n').map_or(bytes.len(), |e| i + e);
            out[i..end].fill(b' ');
            return end;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let end = s[i + 2..].find("*/").map_or(bytes.len(), |e| i + 2 + e + 2);
            for b in &mut out[i..end] {
                if *b != b'\n' {
                    *b = b' ';
                }
            }
            return end;
        }
        i + 1
    });
    // Pass 2: trailing commas, now that comments are gone.
    let snapshot = out.clone();
    scan_outside_strings(&snapshot, |i, bytes| {
        if bytes[i] == b',' {
            let next = bytes[i + 1..].iter().find(|b| !b.is_ascii_whitespace());
            if matches!(next, Some(b'}') | Some(b']')) {
                out[i] = b' ';
            }
        }
        i + 1
    });
    // Only ASCII bytes were replaced by ASCII spaces, so this stays valid UTF-8.
    String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
}

/// Walk `bytes`, skipping string literals; `visit` handles position `i` and returns the next one.
fn scan_outside_strings(bytes: &[u8], mut visit: impl FnMut(usize, &[u8]) -> usize) {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
            continue;
        }
        i = visit(i, bytes).max(i + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_missing_comma_and_reports_position() {
        let src = "{\n  \"a\": [\n    \"x\",\n    \"y\"\n    \"z\"\n  ]\n}";
        let r = parse_json(src);
        assert!(r.value.is_some());
        assert_eq!(r.problems.len(), 1);
        let p = &r.problems[0];
        assert_eq!(p.line, 4);
        assert_eq!(&src[..p.insert_comma_at.unwrap()], "{\n  \"a\": [\n    \"x\",\n    \"y\"");
    }

    #[test]
    fn jsonc_comments_and_trailing_commas_keep_offsets() {
        let src = "{ // c\n \"a\": 1, /* x */ \"b\": [1,2,],\n}";
        let r = parse_json(src);
        assert!(r.value.is_some(), "{:?}", r.problems);
        assert!(r.problems.is_empty());
    }

    #[test]
    fn stray_character_is_not_a_missing_comma() {
        let r = parse_json("{\n  \"a\": \"b\"@\n}");
        assert!(r.value.is_none());
        assert_eq!(r.problems.len(), 1);
        assert!(r.problems[0].insert_comma_at.is_none());
        assert_eq!(r.problems[0].message, "多余的字符「@」");
    }

    #[test]
    fn unrecoverable_error_has_position() {
        let src = "{\n  \"a\": \n}";
        let r = parse_json(src);
        assert!(r.value.is_none());
        assert_eq!(r.problems[0].line, 3);
    }

    #[test]
    fn yaml_and_toml_positions() {
        let y = parse_data(Format::Yaml, "a: 1\nb: [1, 2\n");
        assert!(y.value.is_none());
        assert!(y.problems[0].line >= 2);
        let t = parse_data(Format::Toml, "a = 1\nb = \n");
        assert_eq!(t.problems[0].line, 2);
    }
}
