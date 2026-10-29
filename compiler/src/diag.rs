// 診断（エラー・警告）とソースの管理（仕様書 18章）
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    pub file: usize,
    pub line: u32,
    pub col: u32,
    pub len: u32,
}

impl Span {
    pub fn new(file: usize, line: u32, col: u32, len: u32) -> Span {
        Span { file, line, col, len }
    }
    pub fn none() -> Span {
        Span { file: usize::MAX, line: 0, col: 0, len: 0 }
    }
    pub fn is_none(&self) -> bool {
        self.file == usize::MAX
    }
}

pub struct SourceFile {
    pub path: String,
    pub lines: Vec<String>,
}

#[derive(Default)]
pub struct SourceMap {
    pub files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> SourceMap {
        SourceMap { files: Vec::new() }
    }
    pub fn add(&mut self, path: &str, text: &str) -> usize {
        let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
        self.files.push(SourceFile { path: path.to_string(), lines });
        self.files.len() - 1
    }
    pub fn path(&self, file: usize) -> String {
        if file < self.files.len() {
            self.files[file].path.clone()
        } else {
            String::from("<不明>")
        }
    }
    pub fn line_text(&self, file: usize, line: u32) -> String {
        if file < self.files.len() && line >= 1 {
            let f = &self.files[file];
            let i = (line - 1) as usize;
            if i < f.lines.len() {
                return f.lines[i].clone();
            }
        }
        String::new()
    }
    pub fn file_name(&self, file: usize) -> String {
        let p = self.path(file);
        let p2 = p.replace('\\', "/");
        match p2.rfind('/') {
            Some(i) => p2[i + 1..].to_string(),
            None => p2,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Level {
    Error,
    Warning,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LintLevel {
    Allow,
    Warn,
    Deny,
}

pub fn lint_level_from(s: &str) -> Option<LintLevel> {
    match s {
        "allow" => Some(LintLevel::Allow),
        "warn" => Some(LintLevel::Warn),
        "deny" => Some(LintLevel::Deny),
        _ => None,
    }
}

pub const LINT_NAMES: [&str; 8] = [
    "unused_imports",
    "unused_variables",
    "unused_results",
    "naming",
    "inexact_time",
    "nmi_in_atomic",
    "irq_without_handler",
    "reversible_loop",
];

#[derive(Clone, Debug)]
pub struct Diag {
    pub level: Level,
    pub code: String,
    pub msg: String,
    pub span: Span,
    pub label: String,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
}

impl Diag {
    pub fn error(code: &str, msg: &str, span: Span) -> Diag {
        Diag {
            level: Level::Error,
            code: code.to_string(),
            msg: msg.to_string(),
            span,
            label: String::new(),
            notes: Vec::new(),
            helps: Vec::new(),
        }
    }
    pub fn warning(lint: &str, msg: &str, span: Span) -> Diag {
        Diag {
            level: Level::Warning,
            code: lint.to_string(),
            msg: msg.to_string(),
            span,
            label: String::new(),
            notes: Vec::new(),
            helps: Vec::new(),
        }
    }
    pub fn label(mut self, s: &str) -> Diag {
        self.label = s.to_string();
        self
    }
    pub fn note(mut self, s: &str) -> Diag {
        self.notes.push(s.to_string());
        self
    }
    pub fn help(mut self, s: &str) -> Diag {
        self.helps.push(s.to_string());
        self
    }
}

pub struct Diags {
    pub list: Vec<Diag>,
    pub levels: HashMap<String, LintLevel>,
    pub deny_warnings: bool,
}

impl Diags {
    pub fn new() -> Diags {
        Diags { list: Vec::new(), levels: HashMap::new(), deny_warnings: false }
    }
    pub fn error(&mut self, d: Diag) {
        // 同じ場所・同じ番号の重複は出さない
        for x in &self.list {
            if x.code == d.code && x.span == d.span && x.msg == d.msg {
                return;
            }
        }
        self.list.push(d);
    }
    pub fn err(&mut self, code: &str, msg: &str, span: Span) {
        self.error(Diag::error(code, msg, span));
    }
    /// 警告を出す。overrides はソースの属性による扱い（最優先）
    pub fn warn_with(&mut self, mut d: Diag, overrides: &HashMap<String, LintLevel>) {
        let lvl = if let Some(l) = overrides.get(&d.code) {
            *l
        } else if let Some(l) = self.levels.get(&d.code) {
            *l
        } else {
            LintLevel::Warn
        };
        match lvl {
            LintLevel::Allow => return,
            LintLevel::Deny => {
                d.level = Level::Error;
            }
            LintLevel::Warn => {
                if self.deny_warnings {
                    d.level = Level::Error;
                }
            }
        }
        for x in &self.list {
            if x.code == d.code && x.span == d.span && x.msg == d.msg {
                return;
            }
        }
        self.list.push(d);
    }
    pub fn warn(&mut self, d: Diag) {
        let empty: HashMap<String, LintLevel> = HashMap::new();
        self.warn_with(d, &empty);
    }
    pub fn has_errors(&self) -> bool {
        self.list.iter().any(|d| d.level == Level::Error)
    }
    pub fn error_count(&self) -> usize {
        self.list.iter().filter(|d| d.level == Level::Error).count()
    }
    pub fn warning_count(&self) -> usize {
        self.list.iter().filter(|d| d.level == Level::Warning).count()
    }
    pub fn render(&self, sm: &SourceMap) -> String {
        let mut out = String::new();
        for d in &self.list {
            out.push_str(&render_one(d, sm));
            out.push('\n');
        }
        out
    }

    pub fn to_json(&self, sm: &SourceMap) -> String {
        let mut s = String::from("[");
        let mut first = true;
        for d in &self.list {
            if !first {
                s.push(',');
            }
            first = false;
            s.push_str("{");
            s.push_str(&format!(
                "\"level\":\"{}\",",
                if d.level == Level::Error { "error" } else { "warning" }
            ));
            s.push_str(&format!("\"code\":\"{}\",", json_escape(&d.code)));
            s.push_str(&format!("\"message\":\"{}\",", json_escape(&d.msg)));
            if !d.span.is_none() {
                s.push_str(&format!(
                    "\"file\":\"{}\",\"line\":{},\"column\":{},",
                    json_escape(&sm.path(d.span.file)),
                    d.span.line,
                    d.span.col
                ));
            }
            s.push_str(&format!("\"label\":\"{}\",", json_escape(&d.label)));
            s.push_str("\"notes\":[");
            let notes: Vec<String> = d.notes.iter().map(|n| format!("\"{}\"", json_escape(n))).collect();
            s.push_str(&notes.join(","));
            s.push_str("],\"helps\":[");
            let helps: Vec<String> = d.helps.iter().map(|n| format!("\"{}\"", json_escape(n))).collect();
            s.push_str(&helps.join(","));
            s.push_str("]}");
        }
        s.push(']');
        s
    }
}

pub fn render_one(d: &Diag, sm: &SourceMap) -> String {
    let mut out = String::new();
    let head = if d.level == Level::Error { "エラー" } else { "警告" };
    out.push_str(&format!("{}[{}]：{}\n", head, d.code, d.msg));
    if !d.span.is_none() {
        let ln = d.span.line;
        let w = format!("{}", ln).len();
        let pad: String = " ".repeat(w);
        out.push_str(&format!("{}--> {}:{}:{}\n", " ".repeat(w + 1), sm.path(d.span.file), ln, d.span.col));
        let text = sm.line_text(d.span.file, ln);
        if !text.is_empty() || ln > 0 {
            out.push_str(&format!("{} |\n", pad));
            out.push_str(&format!("{} | {}\n", ln, text));
            // キャレットの位置（タブはそのまま写す）
            let mut caret = String::new();
            let mut i = 1u32;
            for ch in text.chars() {
                if i >= d.span.col {
                    break;
                }
                if ch == '\t' {
                    caret.push('\t');
                } else if (ch as u32) >= 0x1100 {
                    caret.push_str("  ");
                } else {
                    caret.push(' ');
                }
                i += 1;
            }
            let n = if d.span.len == 0 { 1 } else { d.span.len };
            caret.push_str(&"^".repeat(n as usize));
            if !d.label.is_empty() {
                caret.push(' ');
                caret.push_str(&d.label);
            }
            out.push_str(&format!("{} | {}\n", pad, caret));
            if !d.notes.is_empty() || !d.helps.is_empty() {
                out.push_str(&format!("{} |\n", pad));
            }
        }
    } else if !d.label.is_empty() {
        out.push_str(&format!("  {}\n", d.label));
    }
    for n in &d.notes {
        out.push_str(&format!("補足：{}\n", n));
    }
    for h in &d.helps {
        out.push_str(&format!("候補：{}\n", h));
    }
    out
}

pub fn json_escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

/// 未実装の機能に使う番号
pub const E_UNIMPL: &str = "E0000";
