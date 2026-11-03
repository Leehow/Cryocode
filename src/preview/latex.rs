//! LaTeX document → Markdown, so `.tex` files reuse the Markdown renderer and
//! the native math engine. Math regions are passed through untouched.

pub fn to_markdown(src: &str) -> String {
    let src = strip_comments(src);
    let title = command_arg(&src, "title");
    let author = command_arg(&src, "author");
    let date = command_arg(&src, "date");
    let body = match (src.find("\\begin{document}"), src.rfind("\\end{document}")) {
        (Some(a), Some(b)) if b > a => &src[a + "\\begin{document}".len()..b],
        (Some(a), None) => &src[a + "\\begin{document}".len()..],
        _ => src.as_str(),
    };
    let mut out = String::with_capacity(body.len() + 256);
    let mut cx = Conv { list_stack: Vec::new(), title, author, date };
    cx.convert(body, &mut out);
    tidy(&out)
}

struct Conv {
    list_stack: Vec<(bool, usize)>,
    title: Option<String>,
    author: Option<String>,
    date: Option<String>,
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let mut prev_backslash = false;
        let mut cut = line.len();
        for (i, c) in line.char_indices() {
            if c == '%' && !prev_backslash {
                cut = i;
                break;
            }
            prev_backslash = c == '\\' && !prev_backslash;
        }
        out.push_str(&line[..cut]);
        out.push('\n');
    }
    out
}

/// `{…}` argument starting at byte `i` (which must point at `{`); returns (content, index after `}`).
fn brace_arg(s: &str, i: usize) -> Option<(String, usize)> {
    let bytes = s.as_bytes();
    let mut j = i;
    while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\n') {
        j += 1;
    }
    if bytes.get(j) != Some(&b'{') {
        return None;
    }
    let mut depth = 0;
    let mut k = j;
    while k < bytes.len() {
        match bytes[k] {
            b'\\' => k += 1,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((s[j + 1..k].to_owned(), k + 1));
                }
            }
            _ => {}
        }
        k += 1;
    }
    Some((s[j + 1..].to_owned(), s.len()))
}

fn bracket_arg(s: &str, i: usize) -> Option<(String, usize)> {
    if s.as_bytes().get(i) != Some(&b'[') {
        return None;
    }
    let end = s[i..].find(']')? + i;
    Some((s[i + 1..end].to_owned(), end + 1))
}

fn command_arg(s: &str, name: &str) -> Option<String> {
    let pat = format!("\\{name}");
    let mut from = 0;
    while let Some(pos) = s[from..].find(&pat) {
        let at = from + pos + pat.len();
        if !s[at..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            let at = bracket_arg(s, at).map(|(_, e)| e).unwrap_or(at);
            return brace_arg(s, at).map(|(a, _)| a);
        }
        from = at;
    }
    None
}

const DISPLAY_ENVS: &[&str] = &[
    "equation", "equation*", "align", "align*", "gather", "gather*", "multline", "multline*", "eqnarray", "eqnarray*",
    "displaymath", "flalign", "flalign*", "alignat", "alignat*", "math",
];

const VERBATIM_ENVS: &[&str] = &["verbatim", "lstlisting", "minted", "Verbatim", "code"];

const THEOREM_ENVS: &[(&str, &str)] = &[
    ("theorem", "Theorem"), ("lemma", "Lemma"), ("proposition", "Proposition"), ("corollary", "Corollary"),
    ("definition", "Definition"), ("remark", "Remark"), ("example", "Example"), ("proof", "Proof"),
    ("claim", "Claim"), ("conjecture", "Conjecture"), ("note", "Note"),
];

impl Conv {
    fn convert(&mut self, s: &str, out: &mut String) {
        let mut i = 0;
        let b = s.as_bytes();
        while i < b.len() {
            let c = b[i];
            match c {
                b'$' => {
                    // Pass math through verbatim.
                    let display = b.get(i + 1) == Some(&b'$');
                    let delim = if display { "$$" } else { "$" };
                    let start = i + delim.len();
                    let end = find_unescaped(s, start, delim).unwrap_or(s.len());
                    let m = &s[start..end];
                    if display {
                        push_display(out, m);
                    } else {
                        out.push('$');
                        out.push_str(m.trim());
                        out.push('$');
                    }
                    i = (end + delim.len()).min(s.len());
                }
                b'\\' => i = self.command(s, i, out),
                b'~' => {
                    out.push('\u{a0}');
                    i += 1;
                }
                b'{' | b'}' => i += 1,
                b'`' if s[i..].starts_with("``") => {
                    out.push('“');
                    i += 2;
                }
                b'\'' if s[i..].starts_with("''") => {
                    out.push('”');
                    i += 2;
                }
                b'-' if s[i..].starts_with("---") => {
                    out.push('—');
                    i += 3;
                }
                b'-' if s[i..].starts_with("--") => {
                    out.push('–');
                    i += 2;
                }
                b'*' | b'_' | b'#' | b'<' | b'>' | b'|' => {
                    // Escape characters that would be Markdown syntax.
                    out.push('\\');
                    out.push(c as char);
                    i += 1;
                }
                b'&' => {
                    out.push_str(" ");
                    i += 1;
                }
                _ => {
                    let ch = s[i..].chars().next().unwrap();
                    out.push(ch);
                    i += ch.len_utf8();
                }
            }
        }
    }

    fn inline(&mut self, s: &str) -> String {
        let mut o = String::new();
        self.convert(s, &mut o);
        o.trim().replace('\n', " ")
    }

    fn command(&mut self, s: &str, i: usize, out: &mut String) -> usize {
        let rest = &s[i + 1..];
        let name: String = rest.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        let mut j = i + 1 + name.len();
        if name.is_empty() {
            // Control symbol: \\ \, \% \[ \( …
            let Some(c) = rest.chars().next() else { return s.len() };
            j = i + 1 + c.len_utf8();
            match c {
                '\\' => {
                    if let Some((_, e)) = bracket_arg(s, j) {
                        j = e;
                    }
                    out.push_str("  \n");
                }
                '[' => {
                    let end = s[j..].find("\\]").map(|e| j + e).unwrap_or(s.len());
                    push_display(out, &s[j..end]);
                    return (end + 2).min(s.len());
                }
                '(' => {
                    let end = s[j..].find("\\)").map(|e| j + e).unwrap_or(s.len());
                    out.push('$');
                    out.push_str(s[j..end].trim());
                    out.push('$');
                    return (end + 2).min(s.len());
                }
                '%' | '&' | '#' | '$' | '{' | '}' => {
                    if c == '#' {
                        out.push('\\');
                    }
                    if c == '$' {
                        out.push('\\');
                    }
                    out.push(c);
                }
                '_' => out.push_str("\\_"),
                ',' | ';' | ' ' | ':' => out.push(' '),
                '-' | '/' | '@' => {}
                _ => out.push(c),
            }
            return j;
        }
        let starred = s[j..].starts_with('*');
        if starred {
            j += 1;
        }
        match name.as_str() {
            "begin" => return self.environment(s, j, out),
            "end" => {
                // Stray \end{…} of an environment we treat as transparent.
                if let Some((env, e)) = brace_arg(s, j) {
                    if env == "itemize" || env == "enumerate" || env == "description" {
                        self.list_stack.pop();
                        out.push_str("\n\n");
                    } else if THEOREM_ENVS.iter().any(|(n, _)| *n == env) || env == "abstract" || env == "quote" || env == "quotation" {
                        out.push_str("\n\n");
                    }
                    return e;
                }
                return j;
            }
            "item" => {
                let label = bracket_arg(s, j);
                if let Some((_, e)) = &label {
                    j = *e;
                }
                let depth = self.list_stack.len().saturating_sub(1);
                let indent = "   ".repeat(depth);
                out.push('\n');
                out.push_str(&indent);
                match self.list_stack.last_mut() {
                    Some((true, n)) => {
                        *n += 1;
                        out.push_str(&format!("{}. ", n));
                    }
                    _ => out.push_str("- "),
                }
                if let Some((l, _)) = label {
                    out.push_str(&format!("**{}** ", self.inline(&l)));
                }
                return j + s[j..].len() - s[j..].trim_start().len();
            }
            "maketitle" => {
                if let Some(t) = self.title.clone() {
                    out.push_str(&format!("\n\n# {}\n\n", self.inline(&t).replace("  \n", " ")));
                }
                let mut meta = Vec::new();
                if let Some(a) = self.author.clone() {
                    meta.push(self.inline(&a.replace("\\and", ", ")).replace("  \n", ", "));
                }
                if let Some(d) = self.date.clone() {
                    let d = self.inline(&d);
                    meta.push(if d.contains("today") || d.is_empty() { String::new() } else { d });
                }
                let meta: Vec<String> = meta.into_iter().filter(|m| !m.is_empty()).collect();
                if !meta.is_empty() {
                    out.push_str(&format!("*{}*\n\n", meta.join(" · ")));
                }
                return j;
            }
            "title" | "author" | "date" | "documentclass" | "usepackage" | "label" | "bibliographystyle" | "bibliography"
            | "newcommand" | "renewcommand" | "providecommand" | "DeclareMathOperator" | "setlength" | "addtolength"
            | "pagestyle" | "thispagestyle" | "include" | "input" | "graphicspath" | "hypersetup" | "geometry"
            | "newtheorem" | "setcounter" | "vspace" | "hspace" | "addbibresource" | "lstset" | "definecolor"
            | "captionsetup" | "theoremstyle" | "numberwithin" | "affil" | "keywords" | "pagenumbering" => {
                // Swallow the command with all its arguments.
                loop {
                    if let Some((_, e)) = bracket_arg(s, j) {
                        j = e;
                    } else if let Some((_, e)) = brace_arg(s, j) {
                        if s[j..e].trim_start().starts_with('{') {
                            j = e;
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                    if !matches!(name.as_str(), "newcommand" | "renewcommand" | "providecommand" | "DeclareMathOperator" | "newtheorem" | "setlength" | "definecolor" | "setcounter" | "addtolength") {
                        break;
                    }
                }
                return j;
            }
            "chapter" | "section" | "subsection" | "subsubsection" | "paragraph" | "subparagraph" | "part" => {
                if let Some((_, e)) = bracket_arg(s, j) {
                    j = e;
                }
                let (arg, e) = brace_arg(s, j).unwrap_or_default();
                let level = match name.as_str() {
                    "part" | "chapter" => "#",
                    "section" => "##",
                    "subsection" => "###",
                    "subsubsection" => "####",
                    _ => "#####",
                };
                let text = self.inline(&arg);
                if name == "paragraph" || name == "subparagraph" {
                    out.push_str(&format!("\n\n**{text}** "));
                } else {
                    out.push_str(&format!("\n\n{level} {text}\n\n"));
                }
                return e.max(j);
            }
            "textbf" | "bf" | "textsc" | "bfseries" | "mathbf" => return self.wrap_arg(s, j, out, "**", "**"),
            "emph" | "textit" | "textsl" | "it" | "itshape" => return self.wrap_arg(s, j, out, "*", "*"),
            "texttt" | "verb" | "path" => {
                if name == "verb" {
                    let d = s[j..].chars().next().unwrap_or('|');
                    let start = j + d.len_utf8();
                    let end = s[start..].find(d).map(|e| start + e).unwrap_or(s.len());
                    out.push_str(&format!("`{}`", &s[start..end]));
                    return (end + d.len_utf8()).min(s.len());
                }
                return self.wrap_arg(s, j, out, "`", "`");
            }
            "underline" | "uline" | "textrm" | "textup" | "textnormal" | "mbox" | "text" | "textsf" | "centering"
            | "small" | "large" | "Large" | "LARGE" | "huge" | "Huge" | "footnotesize" | "normalsize" | "tiny" | "scriptsize" => {
                return self.wrap_arg(s, j, out, "", "");
            }
            "textsuperscript" => return self.wrap_arg(s, j, out, "^", "^"),
            "textsubscript" => return self.wrap_arg(s, j, out, "~", "~"),
            "sout" | "st" => return self.wrap_arg(s, j, out, "~~", "~~"),
            "url" => {
                let (u, e) = brace_arg(s, j).unwrap_or_default();
                out.push_str(&format!("<{u}>"));
                return e.max(j);
            }
            "href" => {
                let (u, e) = brace_arg(s, j).unwrap_or_default();
                let (t, e2) = brace_arg(s, e).unwrap_or_default();
                out.push_str(&format!("[{}]({u})", self.inline(&t)));
                return e2.max(e);
            }
            "footnote" => {
                let (t, e) = brace_arg(s, j).unwrap_or_default();
                out.push_str(&format!("<sup>({})</sup>", self.inline(&t)));
                return e.max(j);
            }
            "cite" | "citep" | "citet" | "parencite" | "textcite" | "autocite" => {
                if let Some((_, e)) = bracket_arg(s, j) {
                    j = e;
                }
                let (k, e) = brace_arg(s, j).unwrap_or_default();
                out.push_str(&format!("[{k}]"));
                return e.max(j);
            }
            "ref" | "eqref" | "autoref" | "cref" | "Cref" | "pageref" => {
                let (k, e) = brace_arg(s, j).unwrap_or_default();
                out.push_str(&if name == "eqref" { format!("({k})") } else { format!("[{k}]") });
                return e.max(j);
            }
            "caption" => {
                let (t, e) = brace_arg(s, j).unwrap_or_default();
                out.push_str(&format!("\n\n*{}*\n\n", self.inline(&t)));
                return e.max(j);
            }
            "includegraphics" => {
                if let Some((_, e)) = bracket_arg(s, j) {
                    j = e;
                }
                let (p, e) = brace_arg(s, j).unwrap_or_default();
                out.push_str(&format!("\n\n![{p}]({p})\n\n"));
                return e.max(j);
            }
            "LaTeX" => out.push_str("LaTeX"),
            "TeX" => out.push_str("TeX"),
            "today" => out.push_str(""),
            "ldots" | "dots" => out.push('…'),
            "par" | "newline" | "linebreak" | "bigskip" | "medskip" | "smallskip" | "newpage" | "clearpage" | "noindent"
            | "indent" | "hfill" | "vfill" | "tableofcontents" | "appendix" | "toprule" | "midrule" | "bottomrule" | "hline" => {
                if matches!(name.as_str(), "par" | "bigskip" | "medskip" | "newpage" | "clearpage") {
                    out.push_str("\n\n");
                }
            }
            _ => {
                // Unknown command: keep its first argument's text, drop the command name.
                if let Some((arg, e)) = brace_arg(s, j) {
                    if s[j..e].trim_start().starts_with('{') {
                        let t = self.inline(&arg);
                        out.push_str(&t);
                        return e;
                    }
                }
            }
        }
        j
    }

    fn wrap_arg(&mut self, s: &str, j: usize, out: &mut String, open: &str, close: &str) -> usize {
        match brace_arg(s, j) {
            Some((arg, e)) if s[j..e].trim_start().starts_with('{') => {
                let t = self.inline(&arg);
                if !t.is_empty() {
                    out.push_str(open);
                    out.push_str(&t);
                    out.push_str(close);
                }
                e
            }
            _ => j,
        }
    }

    fn environment(&mut self, s: &str, j: usize, out: &mut String) -> usize {
        let Some((env, after)) = brace_arg(s, j) else { return j };
        let end_tag = format!("\\end{{{env}}}");
        let body_end = || find_env_end(s, after, &env).unwrap_or(s.len());
        if DISPLAY_ENVS.contains(&env.as_str()) {
            let end = body_end();
            let inner = &s[after..end];
            let math = if env.starts_with("align") || env.starts_with("eqnarray") || env.starts_with("flalign") {
                format!("\\begin{{aligned}}{inner}\\end{{aligned}}")
            } else if env.starts_with("gather") {
                format!("\\begin{{gathered}}{inner}\\end{{gathered}}")
            } else {
                inner.to_owned()
            };
            push_display(out, &math);
            return (end + end_tag.len()).min(s.len());
        }
        if VERBATIM_ENVS.contains(&env.as_str()) {
            let end = s[after..].find(&end_tag).map(|e| after + e).unwrap_or(s.len());
            let mut body_start = after;
            let mut lang = String::new();
            if env == "minted" {
                if let Some((l, e)) = brace_arg(s, bracket_arg(s, after).map(|(_, e)| e).unwrap_or(after)) {
                    lang = l;
                    body_start = e;
                }
            } else if let Some((opts, e)) = bracket_arg(s, after) {
                if let Some(l) = opts.split(',').find_map(|o| o.trim().strip_prefix("language=")) {
                    lang = l.to_ascii_lowercase();
                }
                body_start = e;
            }
            let code = s[body_start..end].trim_matches('\n');
            out.push_str(&format!("\n\n```{lang}\n{code}\n```\n\n"));
            return (end + end_tag.len()).min(s.len());
        }
        match env.as_str() {
            "itemize" | "enumerate" | "description" => {
                self.list_stack.push((env == "enumerate", 0));
                out.push_str("\n\n");
                after
            }
            "abstract" => {
                out.push_str("\n\n> **Abstract.** ");
                let end = body_end();
                let inner = self.inline(&s[after..end]);
                out.push_str(&inner);
                out.push_str("\n\n");
                (end + end_tag.len()).min(s.len())
            }
            "quote" | "quotation" => {
                let end = body_end();
                let inner = self.inline(&s[after..end]);
                out.push_str(&format!("\n\n> {inner}\n\n"));
                (end + end_tag.len()).min(s.len())
            }
            "tabular" | "tabular*" | "tabularx" | "longtable" | "array" => {
                let mut k = after;
                if env == "tabular*" || env == "tabularx" {
                    k = brace_arg(s, k).map(|(_, e)| e).unwrap_or(k);
                }
                let (spec, k) = brace_arg(s, k).unwrap_or_default();
                let k = k.max(after);
                let end = body_end();
                let body = &s[k..end];
                let rows: Vec<Vec<String>> = body
                    .split("\\\\")
                    .map(|r| r.replace("\\hline", "").replace("\\toprule", "").replace("\\midrule", "").replace("\\bottomrule", ""))
                    .filter(|r| !r.trim().is_empty())
                    .map(|r| split_cells(&r).into_iter().map(|c| self.inline(&c).replace('|', "\\|")).collect())
                    .collect();
                let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0).max(spec.chars().filter(|c| "lcrpXmb".contains(*c)).count().min(1));
                if !rows.is_empty() && ncols > 0 {
                    out.push_str("\n\n");
                    for (ri, r) in rows.iter().enumerate() {
                        out.push('|');
                        for ci in 0..ncols {
                            out.push_str(&format!(" {} |", r.get(ci).map(String::as_str).unwrap_or("")));
                        }
                        out.push('\n');
                        if ri == 0 {
                            out.push('|');
                            for _ in 0..ncols {
                                out.push_str(" --- |");
                            }
                            out.push('\n');
                        }
                    }
                    out.push('\n');
                }
                (end + end_tag.len()).min(s.len())
            }
            "figure" | "figure*" | "table" | "table*" | "center" | "flushleft" | "flushright" | "minipage" | "document" | "frame"
            | "multicols" | "wrapfigure" | "subfigure" | "small" => {
                // Transparent: drop placement options, keep the content.
                let mut k = after;
                if let Some((_, e)) = bracket_arg(s, k) {
                    k = e;
                }
                if env == "minipage" || env == "multicols" || env == "wrapfigure" || env == "subfigure" {
                    if let Some((_, e)) = brace_arg(s, k) {
                        k = e;
                    }
                }
                k
            }
            other => {
                if let Some((_, title)) = THEOREM_ENVS.iter().find(|(n, _)| *n == other.trim_end_matches('*')) {
                    let mut k = after;
                    let note = bracket_arg(s, k).map(|(n, e)| {
                        k = e;
                        n
                    });
                    match note {
                        Some(n) => out.push_str(&format!("\n\n**{title}** ({}). ", self.inline(&n))),
                        None => out.push_str(&format!("\n\n**{title}.** ")),
                    }
                    k
                } else {
                    after
                }
            }
        }
    }
}

fn push_display(out: &mut String, m: &str) {
    out.push_str("\n\n$$\n");
    out.push_str(m.trim());
    out.push_str("\n$$\n\n");
}

fn find_unescaped(s: &str, from: usize, pat: &str) -> Option<usize> {
    let mut k = from;
    while let Some(p) = s[k..].find(pat) {
        let at = k + p;
        if at > 0 && s.as_bytes()[at - 1] == b'\\' {
            k = at + pat.len();
            continue;
        }
        return Some(at);
    }
    None
}

/// Find the matching `\end{env}`, honouring nested environments of the same name.
fn find_env_end(s: &str, from: usize, env: &str) -> Option<usize> {
    let open = format!("\\begin{{{env}}}");
    let close = format!("\\end{{{env}}}");
    let mut depth = 0;
    let mut k = from;
    loop {
        let next_open = s[k..].find(&open).map(|p| k + p);
        let next_close = s[k..].find(&close).map(|p| k + p)?;
        match next_open {
            Some(o) if o < next_close => {
                depth += 1;
                k = o + open.len();
            }
            _ => {
                if depth == 0 {
                    return Some(next_close);
                }
                depth -= 1;
                k = next_close + close.len();
            }
        }
    }
}

fn split_cells(row: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut depth = 0;
    let mut in_math = false;
    let mut prev = '\0';
    for c in row.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            '$' if prev != '\\' => in_math = !in_math,
            '&' if depth == 0 && !in_math && prev != '\\' => {
                cells.push(std::mem::take(&mut cur).trim().to_owned());
                prev = c;
                continue;
            }
            _ => {}
        }
        cur.push(c);
        prev = c;
    }
    cells.push(cur.trim().to_owned());
    cells
}

/// Collapse runs of blank lines and trailing spaces the conversion leaves behind.
fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0;
    for line in s.lines() {
        let keep_break = line.ends_with("  ");
        let t = line.trim_end();
        if t.trim().is_empty() {
            blank += 1;
            if blank <= 1 {
                out.push('\n');
            }
            continue;
        }
        blank = 0;
        // Source indentation would turn into Markdown code blocks; keep it only for our own nested lists.
        let body = t.trim_start_matches([' ', '\t', '\u{a0}']);
        let is_list = body.starts_with("- ") || body.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        out.push_str(if is_list { t } else { body });
        if keep_break {
            out.push_str("  ");
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_structure_and_keeps_math() {
        let md = to_markdown(
            "\\documentclass{article}\\title{Hi}\\begin{document}\\maketitle\n\\section{Intro}\nText with $a^2$ and \\textbf{bold}.\n\\begin{align}x&=1\\\\y&=2\\end{align}\n\\begin{itemize}\\item one\\item two\\end{itemize}\\end{document}",
        );
        assert!(md.contains("# Hi"), "{md}");
        assert!(md.contains("## Intro"), "{md}");
        assert!(md.contains("$a^2$"), "{md}");
        assert!(md.contains("**bold**"), "{md}");
        assert!(md.contains("\\begin{aligned}x&=1\\\\y&=2\\end{aligned}"), "{md}");
        assert!(md.contains("- one"), "{md}");
    }
}
