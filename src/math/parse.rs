//! TeX math source → node tree.
//!
//! Covers the KaTeX subset people actually write in notes and papers:
//! scripts, fractions, roots, big operators, accents, font switches,
//! `\left…\right`, matrices/cases/aligned environments, colours and spacing.

use egui::Color32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Glyphs {
    /// STIX Two Math, characters already mapped to their Unicode math alphabet.
    Math,
    /// Upright text face (`\text`, `\mathrm`, function names).
    Text,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AccentKind {
    Hat,
    Check,
    Tilde,
    Bar,
    Vec,
    Dot,
    Ddot,
    Breve,
    Acute,
    Grave,
    Overline,
    Underline,
    OverArrow,
    OverLeftArrow,
    OverBrace,
    UnderBrace,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MathStyle {
    Display,
    Text,
    Script,
    ScriptScript,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatrixKind {
    Matrix,
    Cases,
    /// `aligned`: alternating right/left columns with no gap inside a pair.
    Aligned,
    Gathered,
    Array,
}

#[derive(Clone, Debug)]
pub enum Node {
    Glyph { text: String, class: Class, glyphs: Glyphs },
    /// A large operator (∑, ∫, lim…). `limits: None` means "TeX default for this operator".
    Op { text: String, glyphs: Glyphs, limits: Option<bool>, integral: bool },
    Group(Vec<Node>, Class),
    Frac { num: Box<Node>, den: Box<Node>, bar: bool, style: Option<MathStyle>, delims: Option<(String, String)> },
    Sqrt { body: Box<Node>, index: Option<Box<Node>> },
    Scripts { base: Box<Node>, sup: Option<Box<Node>>, sub: Option<Box<Node>> },
    Accent { body: Box<Node>, kind: AccentKind, wide: bool },
    Delimited { left: String, right: String, body: Vec<Node> },
    /// Fixed-size delimiter from `\big`, `\Bigl`…, scale relative to the font size.
    BigDelim { delim: String, scale: f32, class: Class },
    Matrix { rows: Vec<Vec<Node>>, kind: MatrixKind, aligns: Vec<ColAlign>, left: String, right: String },
    Space(f32),
    Color(Color32, Vec<Node>),
    Boxed(Box<Node>),
    Stack { body: Box<Node>, over: Option<Box<Node>>, under: Option<Box<Node>> },
    Phantom(Box<Node>),
    StyleSwitch(MathStyle),
    Error(String),
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Cmd(String),
    Char(char),
    Open,
    Close,
    Sup,
    Sub,
    Amp,
    Prime,
}

fn tokenize(src: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut it = src.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => {
                let mut name = String::new();
                match it.peek().copied() {
                    Some(n) if n.is_ascii_alphabetic() => {
                        while let Some(&n) = it.peek() {
                            if n.is_ascii_alphabetic() {
                                name.push(n);
                                it.next();
                            } else {
                                break;
                            }
                        }
                        // `\operatorname*`, `\begin{align*}` handled by the parser; `*` after a
                        // command name is only meaningful for a few commands.
                        if name == "operatorname" && it.peek() == Some(&'*') {
                            it.next();
                            name.push('*');
                        }
                    }
                    Some(n) => {
                        name.push(n);
                        it.next();
                    }
                    None => name.push('\\'),
                }
                out.push(Tok::Cmd(name));
            }
            '{' => out.push(Tok::Open),
            '}' => out.push(Tok::Close),
            '^' => out.push(Tok::Sup),
            '_' => out.push(Tok::Sub),
            '&' => out.push(Tok::Amp),
            '\'' => out.push(Tok::Prime),
            '%' => {
                // Comment to end of line.
                for n in it.by_ref() {
                    if n == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            '~' => out.push(Tok::Cmd(" ".into())),
            c => out.push(Tok::Char(c)),
        }
    }
    out
}

pub fn parse(src: &str) -> Vec<Node> {
    let toks = tokenize(src);
    let mut p = Parser { toks, pos: 0 };
    p.list(&|_| false)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

type Stop<'a> = &'a dyn Fn(&Tok) -> bool;

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn list(&mut self, stop: Stop) -> Vec<Node> {
        let mut out: Vec<Node> = Vec::new();
        while let Some(t) = self.peek() {
            if stop(t) || *t == Tok::Close {
                break;
            }
            match t.clone() {
                Tok::Sup | Tok::Sub | Tok::Prime => {
                    let base = out.pop().unwrap_or(Node::Group(vec![], Class::Ord));
                    out.push(self.scripts(base));
                }
                Tok::Cmd(c) if c == "limits" || c == "nolimits" => {
                    self.next();
                    if let Some(Node::Op { limits, .. }) = out.last_mut() {
                        *limits = Some(c == "limits");
                    }
                }
                Tok::Cmd(c) if c == "over" || c == "choose" => {
                    // `{a \over b}`: everything so far is the numerator.
                    self.next();
                    let num = Node::Group(std::mem::take(&mut out), Class::Ord);
                    let den = Node::Group(self.list(stop), Class::Ord);
                    let delims = (c == "choose").then(|| ("(".to_owned(), ")".to_owned()));
                    out.push(Node::Frac { num: Box::new(num), den: Box::new(den), bar: c == "over", style: None, delims });
                }
                _ => {
                    if let Some(n) = self.atom() {
                        out.push(n);
                    }
                }
            }
        }
        out
    }

    fn scripts(&mut self, base: Node) -> Node {
        let (mut base, mut sup, mut sub) = match base {
            Node::Scripts { base, sup, sub } => (*base, sup, sub),
            other => (other, None, None),
        };
        loop {
            match self.peek() {
                Some(Tok::Sup) => {
                    self.next();
                    let arg = self.arg_node();
                    sup = Some(Box::new(match sup.take() {
                        Some(prev) => Node::Group(vec![*prev, arg], Class::Ord),
                        None => arg,
                    }));
                }
                Some(Tok::Sub) => {
                    self.next();
                    sub = Some(Box::new(self.arg_node()));
                }
                Some(Tok::Prime) => {
                    self.next();
                    let prime = glyph("′", Class::Ord);
                    sup = Some(Box::new(match sup.take() {
                        Some(prev) => Node::Group(vec![*prev, prime], Class::Ord),
                        None => prime,
                    }));
                }
                _ => break,
            }
        }
        if let Node::Scripts { .. } = base {
            base = Node::Group(vec![base], Class::Ord);
        }
        Node::Scripts { base: Box::new(base), sup, sub }
    }

    /// A single argument: `{…}` group or one atom.
    fn arg_node(&mut self) -> Node {
        match self.peek() {
            Some(Tok::Open) => {
                self.next();
                let inner = self.list(&|_| false);
                self.expect_close();
                Node::Group(inner, Class::Ord)
            }
            Some(_) => self.atom().unwrap_or(Node::Group(vec![], Class::Ord)),
            None => Node::Group(vec![], Class::Ord),
        }
    }

    fn expect_close(&mut self) {
        if self.peek() == Some(&Tok::Close) {
            self.next();
        }
    }

    /// Raw text of a `{…}` argument (for `\text`, `\begin`, `\color`…).
    fn raw_arg(&mut self) -> String {
        let mut s = String::new();
        match self.peek() {
            Some(Tok::Open) => {
                self.next();
                let mut depth = 0;
                while let Some(t) = self.next() {
                    match t {
                        Tok::Open => {
                            depth += 1;
                            s.push('{');
                        }
                        Tok::Close if depth == 0 => break,
                        Tok::Close => {
                            depth -= 1;
                            s.push('}');
                        }
                        Tok::Char(c) => s.push(c),
                        Tok::Cmd(c) if c == " " => s.push(' '),
                        Tok::Cmd(c) if c.len() == 1 && !c.chars().all(|c| c.is_ascii_alphabetic()) => s.push_str(&c),
                        Tok::Cmd(c) => {
                            s.push('\\');
                            s.push_str(&c);
                        }
                        Tok::Sup => s.push('^'),
                        Tok::Sub => s.push('_'),
                        Tok::Amp => s.push('&'),
                        Tok::Prime => s.push('\''),
                    }
                }
            }
            Some(Tok::Char(c)) => {
                s.push(*c);
                self.next();
            }
            _ => {}
        }
        s
    }

    fn optional_arg(&mut self) -> Option<Vec<Node>> {
        if self.peek() == Some(&Tok::Char('[')) {
            self.next();
            let inner = self.list(&|t| *t == Tok::Char(']'));
            if self.peek() == Some(&Tok::Char(']')) {
                self.next();
            }
            Some(inner)
        } else {
            None
        }
    }

    fn delim_token(&mut self) -> String {
        match self.next() {
            Some(Tok::Char('.')) => String::new(),
            Some(Tok::Char(c)) => c.to_string(),
            Some(Tok::Cmd(c)) => delimiter_char(&c).unwrap_or("").to_owned(),
            _ => String::new(),
        }
    }

    fn atom(&mut self) -> Option<Node> {
        let tok = self.next()?;
        Some(match tok {
            Tok::Open => {
                let inner = self.list(&|_| false);
                self.expect_close();
                Node::Group(inner, Class::Ord)
            }
            Tok::Close | Tok::Amp => return None,
            Tok::Sup | Tok::Sub | Tok::Prime => return None,
            Tok::Char(c) => char_node(c),
            Tok::Cmd(name) => self.command(&name),
        })
    }

    fn command(&mut self, name: &str) -> Node {
        if let Some(n) = symbol(name) {
            return n;
        }
        match name {
            "frac" | "dfrac" | "tfrac" | "cfrac" | "binom" | "dbinom" | "tbinom" => {
                let num = self.arg_node();
                let den = self.arg_node();
                let style = match name {
                    "dfrac" | "cfrac" | "dbinom" => Some(MathStyle::Display),
                    "tfrac" | "tbinom" => Some(MathStyle::Text),
                    _ => None,
                };
                let binom = name.ends_with("binom");
                Node::Frac {
                    num: Box::new(num),
                    den: Box::new(den),
                    bar: !binom,
                    style,
                    delims: binom.then(|| ("(".to_owned(), ")".to_owned())),
                }
            }
            "sqrt" => {
                let index = self.optional_arg().map(|n| Box::new(Node::Group(n, Class::Ord)));
                let body = self.arg_node();
                Node::Sqrt { body: Box::new(body), index }
            }
            "left" => {
                let left = self.delim_token();
                let body = self.list(&|t| matches!(t, Tok::Cmd(c) if c == "right"));
                let right = if self.peek().is_some_and(|t| matches!(t, Tok::Cmd(c) if c == "right")) {
                    self.next();
                    self.delim_token()
                } else {
                    String::new()
                };
                Node::Delimited { left, right, body }
            }
            "middle" => {
                let d = self.delim_token();
                Node::BigDelim { delim: d, scale: 1.2, class: Class::Rel }
            }
            "big" | "Big" | "bigg" | "Bigg" | "bigl" | "Bigl" | "biggl" | "Biggl" | "bigr" | "Bigr"
            | "biggr" | "Biggr" | "bigm" | "Bigm" | "biggm" | "Biggm" => {
                let scale = match name.trim_end_matches(['l', 'r', 'm']) {
                    "big" => 1.2,
                    "Big" => 1.8,
                    "bigg" => 2.4,
                    _ => 3.0,
                };
                let class = match name.chars().last() {
                    Some('l') => Class::Open,
                    Some('r') => Class::Close,
                    Some('m') => Class::Rel,
                    _ => Class::Ord,
                };
                let delim = self.delim_token();
                Node::BigDelim { delim, scale, class }
            }
            "text" | "textrm" | "textnormal" | "mbox" | "hbox" | "textup" | "textsf" | "texttt" | "textmd" => {
                Node::Glyph { text: self.raw_arg(), class: Class::Ord, glyphs: Glyphs::Text }
            }
            "textbf" => Node::Glyph { text: map_alphabet(&self.raw_arg(), Alphabet::Bold), class: Class::Ord, glyphs: Glyphs::Math },
            "textit" | "emph" => Node::Glyph { text: map_alphabet(&self.raw_arg(), Alphabet::Italic), class: Class::Ord, glyphs: Glyphs::Math },
            "operatorname" | "operatorname*" => {
                let text = self.raw_arg();
                if name.ends_with('*') {
                    Node::Op { text, glyphs: Glyphs::Text, limits: Some(true), integral: false }
                } else {
                    Node::Op { text, glyphs: Glyphs::Text, limits: Some(false), integral: false }
                }
            }
            "mathrm" | "rm" | "mathup" => self.font_arg(Alphabet::Upright),
            "mathbf" | "bf" => self.font_arg(Alphabet::Bold),
            "boldsymbol" | "bm" | "pmb" => self.font_arg(Alphabet::BoldItalic),
            "mathit" | "it" => self.font_arg(Alphabet::Italic),
            "mathbb" | "Bbb" => self.font_arg(Alphabet::DoubleStruck),
            "mathcal" | "cal" | "mathscr" => self.font_arg(Alphabet::Script),
            "mathfrak" | "frak" => self.font_arg(Alphabet::Fraktur),
            "mathsf" | "sf" => self.font_arg(Alphabet::Sans),
            "mathtt" | "tt" => self.font_arg(Alphabet::Mono),
            "hat" | "widehat" => self.accent(AccentKind::Hat, name == "widehat"),
            "check" | "widecheck" => self.accent(AccentKind::Check, name == "widecheck"),
            "tilde" | "widetilde" => self.accent(AccentKind::Tilde, name == "widetilde"),
            "bar" => self.accent(AccentKind::Bar, false),
            "overline" => self.accent(AccentKind::Overline, true),
            "underline" => self.accent(AccentKind::Underline, true),
            "vec" => self.accent(AccentKind::Vec, false),
            "overrightarrow" => self.accent(AccentKind::OverArrow, true),
            "overleftarrow" => self.accent(AccentKind::OverLeftArrow, true),
            "dot" => self.accent(AccentKind::Dot, false),
            "ddot" => self.accent(AccentKind::Ddot, false),
            "breve" => self.accent(AccentKind::Breve, false),
            "acute" => self.accent(AccentKind::Acute, false),
            "grave" => self.accent(AccentKind::Grave, false),
            "overbrace" => self.accent(AccentKind::OverBrace, true),
            "underbrace" => self.accent(AccentKind::UnderBrace, true),
            "overset" | "stackrel" => {
                let over = self.arg_node();
                let body = self.arg_node();
                Node::Group(vec![Node::Stack { body: Box::new(body), over: Some(Box::new(over)), under: None }], Class::Rel)
            }
            "underset" => {
                let under = self.arg_node();
                let body = self.arg_node();
                Node::Stack { body: Box::new(body), over: None, under: Some(Box::new(under)) }
            }
            "color" => {
                let color = parse_color(&self.raw_arg());
                // `\color{red} rest-of-group`.
                let rest = self.list(&|_| false);
                Node::Color(color, rest)
            }
            "textcolor" | "colorbox" => {
                let color = parse_color(&self.raw_arg());
                let body = self.arg_node();
                Node::Color(color, vec![body])
            }
            "boxed" | "fbox" => Node::Boxed(Box::new(self.arg_node())),
            "phantom" | "vphantom" | "hphantom" => Node::Phantom(Box::new(self.arg_node())),
            "displaystyle" => Node::StyleSwitch(MathStyle::Display),
            "textstyle" => Node::StyleSwitch(MathStyle::Text),
            "scriptstyle" => Node::StyleSwitch(MathStyle::Script),
            "scriptscriptstyle" => Node::StyleSwitch(MathStyle::ScriptScript),
            "mathrel" | "mathbin" | "mathop" | "mathord" | "mathopen" | "mathclose" | "mathpunct" | "mathinner" => {
                let class = match name {
                    "mathrel" => Class::Rel,
                    "mathbin" => Class::Bin,
                    "mathop" => Class::Op,
                    "mathopen" => Class::Open,
                    "mathclose" => Class::Close,
                    "mathpunct" => Class::Punct,
                    "mathinner" => Class::Inner,
                    _ => Class::Ord,
                };
                match self.arg_node() {
                    Node::Group(n, _) => Node::Group(n, class),
                    other => Node::Group(vec![other], class),
                }
            }
            "not" => match self.atom() {
                Some(Node::Glyph { text, class, glyphs }) => {
                    let negated = match text.as_str() {
                        "=" => "≠".to_owned(),
                        "∈" => "∉".to_owned(),
                        "⊂" => "⊄".to_owned(),
                        "⊆" => "⊈".to_owned(),
                        "≡" => "≢".to_owned(),
                        "∼" => "≁".to_owned(),
                        "<" => "≮".to_owned(),
                        ">" => "≯".to_owned(),
                        t => format!("{t}\u{0338}"),
                    };
                    Node::Glyph { text: negated, class, glyphs }
                }
                Some(other) => other,
                None => Node::Group(vec![], Class::Ord),
            },
            "hspace" | "hspace*" | "kern" | "mkern" | "hskip" | "mskip" => {
                let arg = self.raw_arg();
                Node::Space(parse_length(&arg))
            }
            "tag" | "tag*" => {
                let t = self.raw_arg();
                Node::Group(vec![Node::Space(2.0), Node::Glyph { text: format!("({t})"), class: Class::Ord, glyphs: Glyphs::Text }], Class::Ord)
            }
            "label" | "ref" | "eqref" => {
                let t = self.raw_arg();
                if name == "label" {
                    Node::Group(vec![], Class::Ord)
                } else {
                    Node::Glyph { text: format!("({t})"), class: Class::Ord, glyphs: Glyphs::Text }
                }
            }
            "nonumber" | "notag" | "hline" | "hdashline" | "displaylimits" | "relax" | "nobreak" | "allowbreak" => {
                Node::Group(vec![], Class::Ord)
            }
            "pmod" => {
                let arg = self.arg_node();
                Node::Group(
                    vec![
                        Node::Space(0.6),
                        glyph("(", Class::Open),
                        Node::Glyph { text: "mod".into(), class: Class::Ord, glyphs: Glyphs::Text },
                        Node::Space(0.33),
                        arg,
                        glyph(")", Class::Close),
                    ],
                    Class::Ord,
                )
            }
            "begin" => self.environment(),
            "end" => {
                self.raw_arg();
                Node::Group(vec![], Class::Ord)
            }
            "\\" | "cr" | "newline" => Node::Group(vec![], Class::Ord),
            other => Node::Error(format!("\\{other}")),
        }
    }

    fn font_arg(&mut self, alphabet: Alphabet) -> Node {
        fn restyle(n: Node, a: Alphabet) -> Node {
            match n {
                Node::Glyph { text, class, glyphs: Glyphs::Math } => {
                    let plain = unmap_italic(&text);
                    if a == Alphabet::Upright {
                        Node::Glyph { text: plain, class, glyphs: Glyphs::Text }
                    } else {
                        Node::Glyph { text: map_alphabet(&plain, a), class, glyphs: Glyphs::Math }
                    }
                }
                Node::Group(ns, c) => Node::Group(ns.into_iter().map(|n| restyle(n, a)).collect(), c),
                Node::Scripts { base, sup, sub } => Node::Scripts { base: Box::new(restyle(*base, a)), sup, sub },
                other => other,
            }
        }
        let arg = self.arg_node();
        restyle(arg, alphabet)
    }

    fn accent(&mut self, kind: AccentKind, wide: bool) -> Node {
        let body = self.arg_node();
        Node::Accent { body: Box::new(body), kind, wide }
    }

    fn environment(&mut self) -> Node {
        let name = self.raw_arg();
        let name = name.trim_end_matches('*');
        let mut aligns = Vec::new();
        if name == "array" || name == "darray" {
            let spec = self.raw_arg();
            aligns = spec
                .chars()
                .filter_map(|c| match c {
                    'l' => Some(ColAlign::Left),
                    'c' => Some(ColAlign::Center),
                    'r' => Some(ColAlign::Right),
                    _ => None,
                })
                .collect();
        }
        let is_end = |t: &Tok| matches!(t, Tok::Cmd(c) if c == "end");
        let mut rows: Vec<Vec<Node>> = Vec::new();
        let mut row: Vec<Node> = Vec::new();
        loop {
            let cell = self.list(&|t| is_end(t) || *t == Tok::Amp || matches!(t, Tok::Cmd(c) if c == "\\" || c == "cr"));
            row.push(Node::Group(cell, Class::Ord));
            match self.next() {
                Some(Tok::Amp) => continue,
                Some(Tok::Cmd(c)) if c == "\\" || c == "cr" => {
                    // Optional spacing argument `\\[2pt]`.
                    if self.peek() == Some(&Tok::Char('[')) {
                        while let Some(t) = self.next() {
                            if t == Tok::Char(']') {
                                break;
                            }
                        }
                    }
                    rows.push(std::mem::take(&mut row));
                }
                Some(Tok::Cmd(c)) if c == "end" => {
                    self.raw_arg();
                    break;
                }
                Some(Tok::Close) | None => break,
                _ => {}
            }
        }
        if !row.iter().all(|n| matches!(n, Node::Group(v, _) if v.is_empty())) || rows.is_empty() {
            rows.push(row);
        }
        let (kind, left, right) = match name {
            "pmatrix" => (MatrixKind::Matrix, "(", ")"),
            "bmatrix" => (MatrixKind::Matrix, "[", "]"),
            "Bmatrix" => (MatrixKind::Matrix, "{", "}"),
            "vmatrix" => (MatrixKind::Matrix, "|", "|"),
            "Vmatrix" => (MatrixKind::Matrix, "‖", "‖"),
            "cases" | "dcases" => (MatrixKind::Cases, "{", ""),
            "rcases" => (MatrixKind::Cases, "", "}"),
            "aligned" | "align" | "alignat" | "alignedat" | "split" | "eqnarray" | "flalign" => (MatrixKind::Aligned, "", ""),
            "gathered" | "gather" | "equation" | "multline" | "displaymath" => (MatrixKind::Gathered, "", ""),
            "array" | "darray" => (MatrixKind::Array, "", ""),
            _ => (MatrixKind::Matrix, "", ""),
        };
        Node::Matrix { rows, kind, aligns, left: left.into(), right: right.into() }
    }
}

fn glyph(s: &str, class: Class) -> Node {
    Node::Glyph { text: s.to_owned(), class, glyphs: Glyphs::Math }
}

fn char_node(c: char) -> Node {
    match c {
        'a'..='z' | 'A'..='Z' => glyph(&map_alphabet(&c.to_string(), Alphabet::Italic), Class::Ord),
        '0'..='9' | '.' => glyph(&c.to_string(), Class::Ord),
        '+' => glyph("+", Class::Bin),
        '-' => glyph("−", Class::Bin),
        '*' => glyph("∗", Class::Bin),
        '=' | '<' | '>' | ':' => glyph(&c.to_string(), Class::Rel),
        ',' | ';' => glyph(&c.to_string(), Class::Punct),
        '(' | '[' => glyph(&c.to_string(), Class::Open),
        ')' | ']' | '!' | '?' => glyph(&c.to_string(), Class::Close),
        '|' => glyph("|", Class::Ord),
        '/' => glyph("/", Class::Ord),
        c if ('\u{3b1}'..='\u{3c9}').contains(&c) => glyph(&map_alphabet(&c.to_string(), Alphabet::Italic), Class::Ord),
        c if c.is_alphabetic() && !c.is_ascii() => Node::Glyph { text: c.to_string(), class: Class::Ord, glyphs: Glyphs::Text },
        c => glyph(&c.to_string(), Class::Ord),
    }
}

fn delimiter_char(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "{" | "lbrace" => "{",
        "}" | "rbrace" => "}",
        "langle" => "⟨",
        "rangle" => "⟩",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "|" | "Vert" | "lVert" | "rVert" => "‖",
        "vert" | "lvert" | "rvert" | "mid" => "|",
        "lbrack" => "[",
        "rbrack" => "]",
        "backslash" => "∖",
        "uparrow" => "↑",
        "downarrow" => "↓",
        _ => return None,
    })
}

fn parse_length(s: &str) -> f32 {
    let s = s.trim();
    let num: String = s.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect();
    let v: f32 = num.parse().unwrap_or(0.0);
    let unit = s[num.len()..].trim();
    match unit {
        "em" => v,
        "ex" => v * 0.43,
        "pt" => v / 10.0,
        "mu" => v / 18.0,
        "px" => v / 16.0,
        "cm" => v * 2.85,
        "mm" => v * 0.285,
        "in" => v * 7.2,
        _ => v / 18.0,
    }
}

pub fn parse_color(s: &str) -> Color32 {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            if let Ok(v) = u32::from_str_radix(hex, 16) {
                return Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
            }
        }
    }
    match s.to_ascii_lowercase().as_str() {
        "red" => Color32::from_rgb(0xe0, 0x4f, 0x4f),
        "green" => Color32::from_rgb(0x3f, 0xa8, 0x5a),
        "blue" => Color32::from_rgb(0x3d, 0x7f, 0xe0),
        "orange" => Color32::from_rgb(0xe8, 0x8a, 0x2e),
        "purple" | "violet" => Color32::from_rgb(0x9b, 0x5d, 0xe5),
        "magenta" => Color32::from_rgb(0xd0, 0x3f, 0xb0),
        "cyan" | "teal" => Color32::from_rgb(0x2a, 0xa8, 0xb8),
        "yellow" => Color32::from_rgb(0xd8, 0xb4, 0x2a),
        "gray" | "grey" => Color32::GRAY,
        "black" => Color32::BLACK,
        "white" => Color32::WHITE,
        "brown" => Color32::from_rgb(0x9a, 0x62, 0x3a),
        _ => Color32::from_rgb(0xe0, 0x4f, 0x4f),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Alphabet {
    Upright,
    Italic,
    Bold,
    BoldItalic,
    DoubleStruck,
    Script,
    Fraktur,
    Sans,
    Mono,
}

/// Map plain ASCII letters/digits (and Greek for italics) to a Unicode math alphabet.
pub fn map_alphabet(s: &str, a: Alphabet) -> String {
    s.chars().map(|c| map_char(c, a)).collect()
}

fn offset(base: u32, c: char, from: char) -> char {
    char::from_u32(base + (c as u32 - from as u32)).unwrap_or(c)
}

fn map_char(c: char, a: Alphabet) -> char {
    let upper = c.is_ascii_uppercase();
    let lower = c.is_ascii_lowercase();
    let digit = c.is_ascii_digit();
    match a {
        Alphabet::Upright => c,
        Alphabet::Italic => match c {
            'h' => 'ℎ',
            _ if upper => offset(0x1D434, c, 'A'),
            _ if lower => offset(0x1D44E, c, 'a'),
            '\u{3b1}'..='\u{3c9}' => offset(0x1D6FC, c, '\u{3b1}'),
            _ => c,
        },
        Alphabet::Bold => match c {
            _ if upper => offset(0x1D400, c, 'A'),
            _ if lower => offset(0x1D41A, c, 'a'),
            _ if digit => offset(0x1D7CE, c, '0'),
            '\u{3b1}'..='\u{3c9}' => offset(0x1D6C2, c, '\u{3b1}'),
            _ => c,
        },
        Alphabet::BoldItalic => match c {
            _ if upper => offset(0x1D468, c, 'A'),
            _ if lower => offset(0x1D482, c, 'a'),
            _ if digit => offset(0x1D7CE, c, '0'),
            '\u{3b1}'..='\u{3c9}' => offset(0x1D736, c, '\u{3b1}'),
            _ => c,
        },
        Alphabet::DoubleStruck => match c {
            'C' => 'ℂ',
            'H' => 'ℍ',
            'N' => 'ℕ',
            'P' => 'ℙ',
            'Q' => 'ℚ',
            'R' => 'ℝ',
            'Z' => 'ℤ',
            _ if upper => offset(0x1D538, c, 'A'),
            _ if lower => offset(0x1D552, c, 'a'),
            _ if digit => offset(0x1D7D8, c, '0'),
            _ => c,
        },
        Alphabet::Script => match c {
            'B' => 'ℬ',
            'E' => 'ℰ',
            'F' => 'ℱ',
            'H' => 'ℋ',
            'I' => 'ℐ',
            'L' => 'ℒ',
            'M' => 'ℳ',
            'R' => 'ℛ',
            'e' => 'ℯ',
            'g' => 'ℊ',
            'o' => 'ℴ',
            _ if upper => offset(0x1D49C, c, 'A'),
            _ if lower => offset(0x1D4B6, c, 'a'),
            _ => c,
        },
        Alphabet::Fraktur => match c {
            'C' => 'ℭ',
            'H' => 'ℌ',
            'I' => 'ℑ',
            'R' => 'ℜ',
            'Z' => 'ℨ',
            _ if upper => offset(0x1D504, c, 'A'),
            _ if lower => offset(0x1D51E, c, 'a'),
            _ => c,
        },
        Alphabet::Sans => match c {
            _ if upper => offset(0x1D5A0, c, 'A'),
            _ if lower => offset(0x1D5BA, c, 'a'),
            _ if digit => offset(0x1D7E2, c, '0'),
            _ => c,
        },
        Alphabet::Mono => match c {
            _ if upper => offset(0x1D670, c, 'A'),
            _ if lower => offset(0x1D68A, c, 'a'),
            _ if digit => offset(0x1D7F6, c, '0'),
            _ => c,
        },
    }
}

/// Undo the default italic mapping so `\mathbf{x}` can re-map from plain `x`.
fn unmap_italic(s: &str) -> String {
    s.chars()
        .map(|c| {
            let u = c as u32;
            match u {
                0x210E => 'h',
                0x1D434..=0x1D44D => char::from_u32('A' as u32 + u - 0x1D434).unwrap_or(c),
                0x1D44E..=0x1D467 => char::from_u32('a' as u32 + u - 0x1D44E).unwrap_or(c),
                0x1D6FC..=0x1D714 => char::from_u32(0x3B1 + u - 0x1D6FC).unwrap_or(c),
                _ => c,
            }
        })
        .collect()
}

fn op(text: &str, limits: bool) -> Node {
    Node::Op { text: text.to_owned(), glyphs: Glyphs::Math, limits: if limits { None } else { Some(false) }, integral: false }
}

fn integral(text: &str) -> Node {
    Node::Op { text: text.to_owned(), glyphs: Glyphs::Math, limits: Some(false), integral: true }
}

fn func(text: &str, limits: bool) -> Node {
    Node::Op { text: text.to_owned(), glyphs: Glyphs::Text, limits: if limits { None } else { Some(false) }, integral: false }
}

fn symbol(name: &str) -> Option<Node> {
    use Class::*;
    let italic_greek = |c: char| glyph(&map_alphabet(&c.to_string(), Alphabet::Italic), Ord);
    Some(match name {
        // Greek
        "alpha" => italic_greek('α'),
        "beta" => italic_greek('β'),
        "gamma" => italic_greek('γ'),
        "delta" => italic_greek('δ'),
        "epsilon" => glyph("\u{1D716}", Ord),
        "varepsilon" => italic_greek('ε'),
        "zeta" => italic_greek('ζ'),
        "eta" => italic_greek('η'),
        "theta" => italic_greek('θ'),
        "vartheta" => glyph("\u{1D717}", Ord),
        "iota" => italic_greek('ι'),
        "kappa" => italic_greek('κ'),
        "varkappa" => glyph("\u{1D718}", Ord),
        "lambda" => italic_greek('λ'),
        "mu" => italic_greek('μ'),
        "nu" => italic_greek('ν'),
        "xi" => italic_greek('ξ'),
        "omicron" => italic_greek('ο'),
        "pi" => italic_greek('π'),
        "varpi" => glyph("\u{1D71B}", Ord),
        "rho" => italic_greek('ρ'),
        "varrho" => glyph("\u{1D71A}", Ord),
        "sigma" => italic_greek('σ'),
        "varsigma" => italic_greek('ς'),
        "tau" => italic_greek('τ'),
        "upsilon" => italic_greek('υ'),
        "phi" => glyph("\u{1D719}", Ord),
        "varphi" => italic_greek('φ'),
        "chi" => italic_greek('χ'),
        "psi" => italic_greek('ψ'),
        "omega" => italic_greek('ω'),
        "Gamma" => glyph("Γ", Ord),
        "Delta" => glyph("Δ", Ord),
        "Theta" => glyph("Θ", Ord),
        "Lambda" => glyph("Λ", Ord),
        "Xi" => glyph("Ξ", Ord),
        "Pi" => glyph("Π", Ord),
        "Sigma" => glyph("Σ", Ord),
        "Upsilon" => glyph("Υ", Ord),
        "Phi" => glyph("Φ", Ord),
        "Psi" => glyph("Ψ", Ord),
        "Omega" => glyph("Ω", Ord),
        // Big operators
        "sum" => op("∑", true),
        "prod" => op("∏", true),
        "coprod" => op("∐", true),
        "bigcup" => op("⋃", true),
        "bigcap" => op("⋂", true),
        "bigvee" => op("⋁", true),
        "bigwedge" => op("⋀", true),
        "bigoplus" => op("⨁", true),
        "bigotimes" => op("⨂", true),
        "bigodot" => op("⨀", true),
        "biguplus" => op("⨄", true),
        "bigsqcup" => op("⨆", true),
        "int" => integral("∫"),
        "iint" => integral("∬"),
        "iiint" => integral("∭"),
        "oint" => integral("∮"),
        "oiint" => integral("∯"),
        // Function names
        "lim" | "max" | "min" | "sup" | "inf" | "det" | "gcd" | "Pr" | "limsup" | "liminf" | "argmax" | "argmin" => {
            let text = match name {
                "limsup" => "lim sup",
                "liminf" => "lim inf",
                "argmax" => "arg max",
                "argmin" => "arg min",
                n => n,
            };
            func(text, true)
        }
        "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "arcsin" | "arccos" | "arctan" | "sinh" | "cosh" | "tanh"
        | "coth" | "log" | "ln" | "lg" | "exp" | "deg" | "dim" | "ker" | "hom" | "arg" | "sgn" | "tr" | "Tr" | "rank"
        | "mod" | "bmod" => func(name, false),
        // Binary operators
        "pm" => glyph("±", Bin),
        "mp" => glyph("∓", Bin),
        "times" => glyph("×", Bin),
        "div" => glyph("÷", Bin),
        "cdot" => glyph("⋅", Bin),
        "ast" => glyph("∗", Bin),
        "star" => glyph("⋆", Bin),
        "circ" => glyph("∘", Bin),
        "bullet" => glyph("∙", Bin),
        "cap" => glyph("∩", Bin),
        "cup" => glyph("∪", Bin),
        "sqcap" => glyph("⊓", Bin),
        "sqcup" => glyph("⊔", Bin),
        "wedge" | "land" => glyph("∧", Bin),
        "vee" | "lor" => glyph("∨", Bin),
        "oplus" => glyph("⊕", Bin),
        "ominus" => glyph("⊖", Bin),
        "otimes" => glyph("⊗", Bin),
        "oslash" => glyph("⊘", Bin),
        "odot" => glyph("⊙", Bin),
        "setminus" => glyph("∖", Bin),
        "uplus" => glyph("⊎", Bin),
        "dagger" => glyph("†", Bin),
        "ddagger" => glyph("‡", Bin),
        "amalg" => glyph("⨿", Bin),
        "wr" => glyph("≀", Bin),
        "diamond" => glyph("⋄", Bin),
        "triangleleft" => glyph("◁", Bin),
        "triangleright" => glyph("▷", Bin),
        // Relations
        "le" | "leq" => glyph("≤", Rel),
        "ge" | "geq" => glyph("≥", Rel),
        "leqslant" => glyph("⩽", Rel),
        "geqslant" => glyph("⩾", Rel),
        "ne" | "neq" => glyph("≠", Rel),
        "approx" => glyph("≈", Rel),
        "equiv" => glyph("≡", Rel),
        "sim" => glyph("∼", Rel),
        "simeq" => glyph("≃", Rel),
        "cong" => glyph("≅", Rel),
        "propto" => glyph("∝", Rel),
        "in" => glyph("∈", Rel),
        "notin" => glyph("∉", Rel),
        "ni" => glyph("∋", Rel),
        "subset" => glyph("⊂", Rel),
        "supset" => glyph("⊃", Rel),
        "subseteq" => glyph("⊆", Rel),
        "supseteq" => glyph("⊇", Rel),
        "subsetneq" => glyph("⊊", Rel),
        "supsetneq" => glyph("⊋", Rel),
        "sqsubseteq" => glyph("⊑", Rel),
        "sqsupseteq" => glyph("⊒", Rel),
        "to" | "rightarrow" => glyph("→", Rel),
        "gets" | "leftarrow" => glyph("←", Rel),
        "Rightarrow" => glyph("⇒", Rel),
        "Leftarrow" => glyph("⇐", Rel),
        "Leftrightarrow" => glyph("⇔", Rel),
        "leftrightarrow" => glyph("↔", Rel),
        "iff" | "Longleftrightarrow" => glyph("⟺", Rel),
        "implies" | "Longrightarrow" => glyph("⟹", Rel),
        "impliedby" | "Longleftarrow" => glyph("⟸", Rel),
        "longrightarrow" => glyph("⟶", Rel),
        "longleftarrow" => glyph("⟵", Rel),
        "longleftrightarrow" => glyph("⟷", Rel),
        "mapsto" => glyph("↦", Rel),
        "longmapsto" => glyph("⟼", Rel),
        "hookrightarrow" => glyph("↪", Rel),
        "hookleftarrow" => glyph("↩", Rel),
        "rightharpoonup" => glyph("⇀", Rel),
        "rightleftharpoons" => glyph("⇌", Rel),
        "uparrow" => glyph("↑", Rel),
        "downarrow" => glyph("↓", Rel),
        "updownarrow" => glyph("↕", Rel),
        "Uparrow" => glyph("⇑", Rel),
        "Downarrow" => glyph("⇓", Rel),
        "nearrow" => glyph("↗", Rel),
        "searrow" => glyph("↘", Rel),
        "ll" => glyph("≪", Rel),
        "gg" => glyph("≫", Rel),
        "prec" => glyph("≺", Rel),
        "succ" => glyph("≻", Rel),
        "preceq" => glyph("⪯", Rel),
        "succeq" => glyph("⪰", Rel),
        "perp" => glyph("⊥", Rel),
        "parallel" => glyph("∥", Rel),
        "mid" => glyph("∣", Rel),
        "nmid" => glyph("∤", Rel),
        "models" => glyph("⊨", Rel),
        "vdash" => glyph("⊢", Rel),
        "dashv" => glyph("⊣", Rel),
        "coloneqq" | "coloneq" => glyph("≔", Rel),
        "triangleq" => glyph("≜", Rel),
        "doteq" => glyph("≐", Rel),
        "asymp" => glyph("≍", Rel),
        "bowtie" => glyph("⋈", Rel),
        "lesssim" => glyph("≲", Rel),
        "gtrsim" => glyph("≳", Rel),
        // Ordinary symbols
        "infty" => glyph("∞", Ord),
        "partial" => glyph("∂", Ord),
        "nabla" => glyph("∇", Ord),
        "forall" => glyph("∀", Ord),
        "exists" => glyph("∃", Ord),
        "nexists" => glyph("∄", Ord),
        "emptyset" => glyph("∅", Ord),
        "varnothing" => glyph("⌀", Ord),
        "neg" | "lnot" => glyph("¬", Ord),
        "hbar" | "hslash" => glyph("ℏ", Ord),
        "ell" => glyph("ℓ", Ord),
        "Re" => glyph("ℜ", Ord),
        "Im" => glyph("ℑ", Ord),
        "aleph" => glyph("ℵ", Ord),
        "beth" => glyph("ℶ", Ord),
        "wp" => glyph("℘", Ord),
        "angle" => glyph("∠", Ord),
        "measuredangle" => glyph("∡", Ord),
        "triangle" | "vartriangle" => glyph("△", Ord),
        "degree" => glyph("°", Ord),
        "prime" => glyph("′", Ord),
        "dots" | "ldots" | "dotsc" | "dotso" => glyph("…", Inner),
        "cdots" | "dotsb" | "dotsm" | "dotsi" => glyph("⋯", Inner),
        "vdots" => glyph("⋮", Ord),
        "ddots" => glyph("⋱", Inner),
        "top" => glyph("⊤", Ord),
        "bot" => glyph("⊥", Ord),
        "checkmark" => glyph("✓", Ord),
        "square" | "Box" => glyph("□", Ord),
        "blacksquare" | "qed" => glyph("■", Ord),
        "clubsuit" => glyph("♣", Ord),
        "diamondsuit" => glyph("♢", Ord),
        "heartsuit" => glyph("♡", Ord),
        "spadesuit" => glyph("♠", Ord),
        "sharp" => glyph("♯", Ord),
        "flat" => glyph("♭", Ord),
        "natural" => glyph("♮", Ord),
        "mho" => glyph("℧", Ord),
        "imath" => glyph("\u{1D6A4}", Ord),
        "jmath" => glyph("\u{1D6A5}", Ord),
        "surd" => glyph("√", Ord),
        "complement" => glyph("∁", Ord),
        "therefore" => glyph("∴", Rel),
        "because" => glyph("∵", Rel),
        "backslash" => glyph("∖", Ord),
        "lbrace" | "{" => glyph("{", Open),
        "rbrace" | "}" => glyph("}", Close),
        "langle" => glyph("⟨", Open),
        "rangle" => glyph("⟩", Close),
        "lceil" => glyph("⌈", Open),
        "rceil" => glyph("⌉", Close),
        "lfloor" => glyph("⌊", Open),
        "rfloor" => glyph("⌋", Close),
        "lvert" => glyph("|", Open),
        "rvert" => glyph("|", Close),
        "vert" => glyph("|", Ord),
        "Vert" | "|" => glyph("‖", Ord),
        "lVert" => glyph("‖", Open),
        "rVert" => glyph("‖", Close),
        "%" => glyph("%", Ord),
        "$" => glyph("$", Ord),
        "#" => glyph("#", Ord),
        "&" => glyph("&", Ord),
        "_" => glyph("_", Ord),
        "colon" => glyph(":", Punct),
        // Spacing
        "," | "thinspace" => Node::Space(3.0 / 18.0),
        ":" | ">" | "medspace" => Node::Space(4.0 / 18.0),
        ";" | "thickspace" => Node::Space(5.0 / 18.0),
        "!" | "negthinspace" => Node::Space(-3.0 / 18.0),
        " " | "space" | "nobreakspace" => Node::Space(0.3),
        "quad" => Node::Space(1.0),
        "qquad" => Node::Space(2.0),
        "enspace" => Node::Space(0.5),
        _ => return None,
    })
}
