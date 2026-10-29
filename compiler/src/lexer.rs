// 字句解析（仕様書 2章）
use crate::diag::{Diag, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum IntStyle {
    Dec,
    HexDollar,
    HexC,
    Bin,
}

/// 文字列・文字の 1 単位。ASCII とエスケープはバイト、ASCII 以外の文字はそのまま持つ
#[derive(Clone, Debug, PartialEq)]
pub enum CharUnit {
    B(u8),
    U(char),
}

#[derive(Clone, Debug, PartialEq)]
pub enum StrPrefix {
    Raw,
    P,
    Z,
    Hi,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(i128, IntStyle),
    Real(f64),
    Char(CharUnit),
    Str(StrPrefix, Vec<CharUnit>),
    Label(String),
    Punct(&'static str),
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    pub start: usize,
    pub end: usize,
}

const PUNCTS: [&str; 48] = [
    "<<=", ">>=", "..=", "::", "->", "=>", "..", "==", "!=", "<=", ">=", "&&", "||", "<<", ">>", "+=",
    "-=", "*=", "/=", "%=", "&=", "|=", "^=", "+", "-", "*", "/", "%", "&", "|", "^", "!", "<", ">",
    "=", ".", ",", ";", ":", "(", ")", "[", "]", "{", "}", "@", "#", "?",
];

pub struct Lexer {
    pub chars: Vec<char>,
    pub pos: usize,
    pub line: u32,
    pub col: u32,
    pub file: usize,
    pub errors: Vec<Diag>,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}
fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

impl Lexer {
    pub fn new(src: &str, file: usize) -> Lexer {
        Lexer { chars: src.chars().collect(), pos: 0, line: 1, col: 1, file, errors: Vec::new() }
    }

    fn peek_c(&self, n: usize) -> char {
        if self.pos + n < self.chars.len() {
            self.chars[self.pos + n]
        } else {
            '\0'
        }
    }

    fn adv(&mut self) -> char {
        let c = self.peek_c(0);
        if self.pos < self.chars.len() {
            self.pos += 1;
            if c == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
        }
        c
    }

    fn span_from(&self, line: u32, col: u32, start: usize) -> Span {
        let len = if self.pos > start { (self.pos - start) as u32 } else { 1 };
        Span::new(self.file, line, col, len)
    }

    fn skip_ws(&mut self) {
        loop {
            let c = self.peek_c(0);
            if c == '\0' && self.pos >= self.chars.len() {
                return;
            }
            if c == ' ' || c == '\t' || c == '\r' || c == '\n' || c == '\u{feff}' {
                self.adv();
                continue;
            }
            if c == '/' && self.peek_c(1) == '/' {
                while self.pos < self.chars.len() && self.peek_c(0) != '\n' {
                    self.adv();
                }
                continue;
            }
            if c == '/' && self.peek_c(1) == '*' {
                let (l, cl, st) = (self.line, self.col, self.pos);
                self.adv();
                self.adv();
                let mut depth = 1;
                while depth > 0 {
                    if self.pos >= self.chars.len() {
                        let sp = self.span_from(l, cl, st);
                        self.errors.push(Diag::error("E0101", "コメントが閉じられていません", sp));
                        return;
                    }
                    if self.peek_c(0) == '*' && self.peek_c(1) == '/' {
                        self.adv();
                        self.adv();
                        depth -= 1;
                    } else if self.peek_c(0) == '/' && self.peek_c(1) == '*' {
                        self.adv();
                        self.adv();
                        depth += 1;
                    } else {
                        self.adv();
                    }
                }
                continue;
            }
            return;
        }
    }

    /// 次のトークンを返す
    pub fn next_token(&mut self) -> Token {
        loop {
            self.skip_ws();
            let (line, col, start) = (self.line, self.col, self.pos);
            if self.pos >= self.chars.len() {
                return Token { tok: Tok::Eof, span: Span::new(self.file, line, col, 1), start, end: start };
            }
            let c = self.peek_c(0);
            let tok = if is_ident_start(c) {
                self.lex_ident_or_prefixed()
            } else if c.is_ascii_digit() {
                self.lex_number()
            } else if c == '$' && self.peek_c(1).is_ascii_hexdigit() {
                self.adv();
                let mut s = String::new();
                while self.peek_c(0).is_ascii_hexdigit() || self.peek_c(0) == '_' {
                    let d = self.adv();
                    if d != '_' {
                        s.push(d);
                    }
                }
                Some(Tok::Int(i128::from_str_radix(&s, 16).unwrap_or(0), IntStyle::HexDollar))
            } else if c == '\'' {
                self.lex_quote()
            } else if c == '"' {
                self.adv();
                let v = self.lex_string_body();
                Some(Tok::Str(StrPrefix::Raw, v))
            } else if !c.is_ascii() {
                self.adv();
                let sp = self.span_from(line, col, start);
                self.errors.push(
                    Diag::error("E0106", "ASCII 以外の文字は、コメントと変換表を付けた文字・文字列の中だけに書けます", sp)
                        .note("2.1節"),
                );
                None
            } else {
                let mut found: Option<&'static str> = None;
                for p in PUNCTS.iter() {
                    let pc: Vec<char> = p.chars().collect();
                    let mut ok = true;
                    for (i, ch) in pc.iter().enumerate() {
                        if self.peek_c(i) != *ch {
                            ok = false;
                            break;
                        }
                    }
                    if ok {
                        found = Some(*p);
                        break;
                    }
                }
                match found {
                    Some(p) => {
                        for _ in 0..p.chars().count() {
                            self.adv();
                        }
                        Some(Tok::Punct(p))
                    }
                    None => {
                        self.adv();
                        let sp = self.span_from(line, col, start);
                        self.errors.push(Diag::error("E0101", &format!("使えない文字 '{}' があります", c), sp));
                        None
                    }
                }
            };
            if let Some(t) = tok {
                let span = self.span_from(line, col, start);
                return Token { tok: t, span, start, end: self.pos };
            }
        }
    }

    fn lex_ident_or_prefixed(&mut self) -> Option<Tok> {
        // 接頭辞付きの文字列 p"…" z"…" hi"…"
        let c0 = self.peek_c(0);
        if (c0 == 'p' || c0 == 'z') && self.peek_c(1) == '"' {
            self.adv();
            self.adv();
            let v = self.lex_string_body();
            let k = if c0 == 'p' { StrPrefix::P } else { StrPrefix::Z };
            return Some(Tok::Str(k, v));
        }
        if c0 == 'h' && self.peek_c(1) == 'i' && self.peek_c(2) == '"' {
            self.adv();
            self.adv();
            self.adv();
            let v = self.lex_string_body();
            return Some(Tok::Str(StrPrefix::Hi, v));
        }
        let mut s = String::new();
        while is_ident_char(self.peek_c(0)) {
            s.push(self.adv());
        }
        Some(Tok::Ident(s))
    }

    fn lex_number(&mut self) -> Option<Tok> {
        let (line, col, start) = (self.line, self.col, self.pos);
        if self.peek_c(0) == '0' && (self.peek_c(1) == 'x' || self.peek_c(1) == 'X') {
            self.adv();
            self.adv();
            let mut s = String::new();
            while self.peek_c(0).is_ascii_hexdigit() || self.peek_c(0) == '_' {
                let d = self.adv();
                if d != '_' {
                    s.push(d);
                }
            }
            if s.is_empty() {
                let sp = self.span_from(line, col, start);
                self.errors.push(Diag::error("E0101", "16進の数字がありません", sp));
                return None;
            }
            return Some(Tok::Int(i128::from_str_radix(&s, 16).unwrap_or(0), IntStyle::HexC));
        }
        if self.peek_c(0) == '0' && (self.peek_c(1) == 'b' || self.peek_c(1) == 'B') {
            self.adv();
            self.adv();
            let mut s = String::new();
            while self.peek_c(0) == '0' || self.peek_c(0) == '1' || self.peek_c(0) == '_' {
                let d = self.adv();
                if d != '_' {
                    s.push(d);
                }
            }
            if s.is_empty() {
                let sp = self.span_from(line, col, start);
                self.errors.push(Diag::error("E0101", "2進の数字がありません", sp));
                return None;
            }
            return Some(Tok::Int(i128::from_str_radix(&s, 2).unwrap_or(0), IntStyle::Bin));
        }
        let mut s = String::new();
        while self.peek_c(0).is_ascii_digit() || self.peek_c(0) == '_' {
            let d = self.adv();
            if d != '_' {
                s.push(d);
            }
        }
        // 小数（2.0 など）。0..8 の範囲と区別する
        if self.peek_c(0) == '.' && self.peek_c(1).is_ascii_digit() {
            s.push(self.adv());
            while self.peek_c(0).is_ascii_digit() || self.peek_c(0) == '_' {
                let d = self.adv();
                if d != '_' {
                    s.push(d);
                }
            }
            if self.peek_c(0) == 'e' || self.peek_c(0) == 'E' {
                s.push(self.adv());
                if self.peek_c(0) == '-' || self.peek_c(0) == '+' {
                    s.push(self.adv());
                }
                while self.peek_c(0).is_ascii_digit() {
                    s.push(self.adv());
                }
            }
            let v: f64 = s.parse().unwrap_or(0.0);
            return Some(Tok::Real(v));
        }
        match s.parse::<i128>() {
            Ok(v) => Some(Tok::Int(v, IntStyle::Dec)),
            Err(_) => {
                let sp = self.span_from(line, col, start);
                self.errors.push(Diag::error("E0101", "数が大きすぎます", sp));
                None
            }
        }
    }

    fn lex_escape(&mut self) -> Option<CharUnit> {
        // '\\' は読み終えている
        let (line, col, start) = (self.line, self.col, self.pos);
        let c = self.adv();
        let b = match c {
            'n' => 0x0A,
            'r' => 0x0D,
            't' => 0x09,
            '0' => 0x00,
            '\\' => b'\\',
            '\'' => b'\'',
            '"' => b'"',
            'x' => {
                let h1 = self.adv();
                let h2 = self.adv();
                let s: String = [h1, h2].iter().collect();
                match u8::from_str_radix(&s, 16) {
                    Ok(v) => v,
                    Err(_) => {
                        let sp = self.span_from(line, col, start);
                        self.errors.push(Diag::error("E0101", "\\x の後ろには 16進 2 桁が必要です", sp));
                        0
                    }
                }
            }
            _ => {
                let sp = self.span_from(line, col, start);
                self.errors.push(Diag::error("E0101", &format!("知らないエスケープ \\{} です", c), sp));
                0
            }
        };
        Some(CharUnit::B(b))
    }

    fn lex_string_body(&mut self) -> Vec<CharUnit> {
        // 開きの " は読み終えている
        let (line, col, start) = (self.line, self.col, self.pos);
        let mut v = Vec::new();
        loop {
            if self.pos >= self.chars.len() || self.peek_c(0) == '\n' {
                let sp = self.span_from(line, col, start);
                self.errors.push(Diag::error("E0101", "文字列が閉じられていません", sp));
                return v;
            }
            let c = self.adv();
            if c == '"' {
                return v;
            }
            if c == '\\' {
                if let Some(u) = self.lex_escape() {
                    v.push(u);
                }
                continue;
            }
            if c.is_ascii() {
                v.push(CharUnit::B(c as u8));
            } else {
                v.push(CharUnit::U(c));
            }
        }
    }

    fn lex_quote(&mut self) -> Option<Tok> {
        let (line, col, start) = (self.line, self.col, self.pos);
        // 'A' 'x' '\n' または 'label
        let c1 = self.peek_c(1);
        let c2 = self.peek_c(2);
        if c1 == '\\' {
            self.adv();
            self.adv();
            let u = self.lex_escape();
            if self.peek_c(0) == '\'' {
                self.adv();
            } else {
                let sp = self.span_from(line, col, start);
                self.errors.push(Diag::error("E0101", "文字が閉じられていません", sp));
            }
            return u.map(Tok::Char);
        }
        if c2 == '\'' && c1 != '\0' {
            self.adv();
            let c = self.adv();
            self.adv();
            let u = if c.is_ascii() { CharUnit::B(c as u8) } else { CharUnit::U(c) };
            return Some(Tok::Char(u));
        }
        if is_ident_start(c1) {
            self.adv();
            let mut s = String::new();
            while is_ident_char(self.peek_c(0)) {
                s.push(self.adv());
            }
            return Some(Tok::Label(s));
        }
        self.adv();
        let sp = self.span_from(line, col, start);
        self.errors.push(Diag::error("E0101", "文字リテラルの書き方が正しくありません", sp));
        None
    }

    /// `{` の直後から対応する `}` までの生のテキストを取り出す（インラインアセンブリ用）。
    /// 返り値は（テキスト、開始行）。終わると `}` の後ろに進む
    pub fn raw_block(&mut self, after_brace: usize, brace_line: u32, brace_col: u32) -> (String, u32) {
        self.pos = after_brace;
        self.line = brace_line;
        self.col = brace_col + 1;
        let line0 = self.line;
        let mut depth = 1;
        let mut text = String::new();
        loop {
            if self.pos >= self.chars.len() {
                let sp = Span::new(self.file, brace_line, brace_col, 1);
                self.errors.push(Diag::error("E0101", "asm のブロックが閉じられていません", sp));
                return (text, line0);
            }
            let c = self.peek_c(0);
            if c == ';' {
                while self.pos < self.chars.len() && self.peek_c(0) != '\n' {
                    text.push(self.adv());
                }
                continue;
            }
            if c == '{' {
                depth += 1;
            }
            if c == '}' {
                depth -= 1;
                if depth == 0 {
                    self.adv();
                    return (text, line0);
                }
            }
            text.push(self.adv());
        }
    }
}

/// ファイル全体を字句解析する（fmt と識別子の集計に使う）
pub fn tokenize_all(src: &str, file: usize) -> (Vec<Token>, Vec<Diag>) {
    let mut lx = Lexer::new(src, file);
    let mut v = Vec::new();
    loop {
        let t = lx.next_token();
        let eof = t.tok == Tok::Eof;
        v.push(t);
        if eof {
            break;
        }
    }
    (v, lx.errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        let (v, e) = tokenize_all(s, 0);
        assert!(e.is_empty(), "lex errors: {:?}", e);
        v.into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn numbers() {
        let t = toks("255 $FF 0xFF 0b1010_0101 1_000");
        assert_eq!(t[0], Tok::Int(255, IntStyle::Dec));
        assert_eq!(t[1], Tok::Int(255, IntStyle::HexDollar));
        assert_eq!(t[2], Tok::Int(255, IntStyle::HexC));
        assert_eq!(t[3], Tok::Int(0xA5, IntStyle::Bin));
        assert_eq!(t[4], Tok::Int(1000, IntStyle::Dec));
    }

    #[test]
    fn ranges_and_reals() {
        let t = toks("0..8 2.5 0..=255");
        assert_eq!(t[0], Tok::Int(0, IntStyle::Dec));
        assert_eq!(t[1], Tok::Punct(".."));
        assert_eq!(t[2], Tok::Int(8, IntStyle::Dec));
        assert_eq!(t[3], Tok::Real(2.5));
        assert_eq!(t[5], Tok::Punct("..="));
    }

    #[test]
    fn chars_labels_strings() {
        let t = toks("'A' '\\r' 'game: p\"HI\" z\"A\" hi\"OK\" \"x\"");
        assert_eq!(t[0], Tok::Char(CharUnit::B(b'A')));
        assert_eq!(t[1], Tok::Char(CharUnit::B(0x0D)));
        assert_eq!(t[2], Tok::Label("game".to_string()));
        assert_eq!(t[3], Tok::Punct(":"));
        assert_eq!(t[4], Tok::Str(StrPrefix::P, vec![CharUnit::B(b'H'), CharUnit::B(b'I')]));
        assert_eq!(t[5], Tok::Str(StrPrefix::Z, vec![CharUnit::B(b'A')]));
        assert_eq!(t[6], Tok::Str(StrPrefix::Hi, vec![CharUnit::B(b'O'), CharUnit::B(b'K')]));
        assert_eq!(t[7], Tok::Str(StrPrefix::Raw, vec![CharUnit::B(b'x')]));
    }

    #[test]
    fn comments_and_puncts() {
        let t = toks("a /* c */ <<= // x\n :: =>");
        assert_eq!(t[0], Tok::Ident("a".to_string()));
        assert_eq!(t[1], Tok::Punct("<<="));
        assert_eq!(t[2], Tok::Punct("::"));
        assert_eq!(t[3], Tok::Punct("=>"));
    }
}
