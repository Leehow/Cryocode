//! Pure text-navigation helpers over a `&str` with byte offsets.

use std::ops::Range;

/// Byte offset of the start of every line (always contains 0).
pub fn line_starts(text: &str) -> Vec<usize> {
    let mut v = Vec::with_capacity(text.len() / 32 + 1);
    v.push(0);
    v.extend(text.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i + 1));
    v
}

/// Zero-based line containing `byte`.
pub fn line_of(starts: &[usize], byte: usize) -> usize {
    starts.partition_point(|&s| s <= byte).saturating_sub(1)
}

/// Byte range of `line` without its trailing newline.
pub fn line_range(text: &str, starts: &[usize], line: usize) -> Range<usize> {
    let start = starts[line];
    let end = starts.get(line + 1).map(|&n| n - 1).unwrap_or(text.len());
    start..end
}

pub fn prev_char(text: &str, i: usize) -> usize {
    text[..i].char_indices().next_back().map(|(b, _)| b).unwrap_or(0)
}

pub fn next_char(text: &str, i: usize) -> usize {
    text[i..].chars().next().map(|c| i + c.len_utf8()).unwrap_or(text.len())
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Space,
    Word,
    Punct,
    Newline,
}

fn class(c: char) -> Class {
    if c == '\n' {
        Class::Newline
    } else if c.is_whitespace() {
        Class::Space
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// Start of the word left of `i` (Option+←).
pub fn word_left(text: &str, i: usize) -> usize {
    let mut it = text[..i].char_indices().rev().peekable();
    while let Some(&(_, c)) = it.peek() {
        if class(c) == Class::Space {
            it.next();
        } else {
            break;
        }
    }
    let Some(&(_, first)) = it.peek() else { return 0 };
    let k = class(first);
    let mut pos = i;
    for (b, c) in it {
        if class(c) != k || k == Class::Newline {
            if k == Class::Newline {
                pos = b;
            }
            break;
        }
        pos = b;
    }
    pos.min(i)
}

/// End of the word right of `i` (Option+→).
pub fn word_right(text: &str, i: usize) -> usize {
    let mut it = text[i..].char_indices().peekable();
    while let Some(&(_, c)) = it.peek() {
        if class(c) == Class::Space {
            it.next();
        } else {
            break;
        }
    }
    let Some(&(_, first)) = it.peek() else { return text.len() };
    let k = class(first);
    let mut pos = text.len();
    for (b, c) in it {
        if class(c) != k {
            pos = i + b;
            break;
        }
        if k == Class::Newline {
            pos = i + b + 1;
            break;
        }
    }
    pos
}

/// The word (or run of punctuation/space) under `i`, for double-click.
pub fn word_at(text: &str, i: usize) -> Range<usize> {
    let c = text[i..].chars().next().or_else(|| text[..i].chars().next_back());
    let Some(c) = c else { return i..i };
    let k = class(c);
    if k == Class::Newline {
        return i..i;
    }
    let start = text[..i].char_indices().rev().take_while(|(_, c)| class(*c) == k).last().map(|(b, _)| b).unwrap_or(i);
    let end = text[i..].char_indices().find(|(_, c)| class(*c) != k).map(|(b, _)| i + b).unwrap_or(text.len());
    start..end
}

/// Leading whitespace of the line containing `i`.
pub fn indent_of_line(text: &str, line_start: usize) -> &str {
    let rest = &text[line_start..];
    let n = rest.find(|c: char| c != ' ' && c != '\t').unwrap_or(rest.len());
    &rest[..n]
}

/// Byte offset of 1-based `line`, clamped to the document.
pub fn line_to_byte(text: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    text.bytes().enumerate().filter(|(_, b)| *b == b'\n').nth(line - 2).map(|(i, _)| i + 1).unwrap_or(text.len())
}

/// Advance `chars` characters from `byte`, not crossing the end of the line.
pub fn advance_chars_in_line(text: &str, byte: usize, chars: usize) -> usize {
    let mut b = byte;
    for c in text[byte..].chars().take(chars) {
        if c == '\n' {
            break;
        }
        b += c.len_utf8();
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines() {
        let t = "ab\ncd\n\nef";
        let s = line_starts(t);
        assert_eq!(s, vec![0, 3, 6, 7]);
        assert_eq!(line_of(&s, 4), 1);
        assert_eq!(line_of(&s, 6), 2);
        assert_eq!(line_range(t, &s, 1), 3..5);
        assert_eq!(line_range(t, &s, 3), 7..9);
        assert_eq!(line_to_byte(t, 3), 6);
    }

    #[test]
    fn words() {
        let t = "foo_bar  (baz) 中文";
        assert_eq!(word_right(t, 0), 7);
        assert_eq!(word_right(t, 7), 10);
        assert_eq!(word_left(t, 13), 10);
        assert_eq!(word_left(t, 9), 0);
        assert_eq!(word_at(t, 2), 0..7);
        let cjk = t.find('中').unwrap();
        assert_eq!(word_at(t, cjk), cjk..t.len());
    }
}
