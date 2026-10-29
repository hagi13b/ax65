// プロジェクトファイル（17.2節）とターゲット定義（16章）
use crate::ast::*;
use crate::diag::{Diag, Diags, SourceMap, Span};
use crate::lexer::{CharUnit, Tok};
use crate::parser::{units_to_string, Parser};
use std::collections::BTreeMap;
use std::path::{Path as FsPath, PathBuf};

#[derive(Clone, Debug)]
pub struct Project {
    pub name: String,
    pub dir: PathBuf,
    pub file: PathBuf,
    pub main: String,
    pub paths: Vec<String>,
    pub target_dir: String,
    pub targets: Vec<String>,
    pub default: Option<String>,
    pub opt: String,
    pub out_dir: String,
    pub step_limit: u64,
    pub lints: Vec<(String, String)>,
    pub fmt_hex: String,
    pub fmt_array: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RangeV {
    pub lo: u32,
    pub hi: u32,
}

impl RangeV {
    pub fn contains(&self, a: u32) -> bool {
        a >= self.lo && a <= self.hi
    }
    pub fn size(&self) -> u32 {
        if self.hi >= self.lo {
            self.hi - self.lo + 1
        } else {
            0
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Exit {
    Monitor(u32),
    Halt,
    Never,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VecMode {
    Hw,
    Indirect(u32),
    Jump(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TValue {
    Int(i128),
    Bool(bool),
    Str(String),
}

#[derive(Clone, Debug)]
pub struct Target {
    pub name: String,
    pub parent: Option<String>,
    pub file: usize,
    pub cpu: Option<String>,
    pub clock: Option<u64>,
    pub stack: Option<u32>,
    pub run: Option<String>,
    pub load: Option<u32>,
    pub exit: Option<Exit>,
    pub irq: Option<VecMode>,
    pub nmi: Option<VecMode>,
    pub irq_interval: Option<u64>,
    pub regions: BTreeMap<String, Vec<RangeV>>,
    pub reserves: Option<Vec<RangeV>>,
    pub places: Vec<(String, RangeV)>,
    pub output: Option<Vec<String>>,
    pub encodings: BTreeMap<String, BTreeMap<u32, u8>>,
    pub binds: BTreeMap<String, String>,
    pub consts: BTreeMap<String, (String, TValue)>,
}

impl Target {
    fn empty(name: &str, file: usize) -> Target {
        Target {
            name: name.to_string(),
            parent: None,
            file,
            cpu: None,
            clock: None,
            stack: None,
            run: None,
            load: None,
            exit: None,
            irq: None,
            nmi: None,
            irq_interval: None,
            regions: BTreeMap::new(),
            reserves: None,
            places: Vec::new(),
            output: None,
            encodings: BTreeMap::new(),
            binds: BTreeMap::new(),
            consts: BTreeMap::new(),
        }
    }

    /// 親の設定に子の設定を重ねる
    fn merged_onto(&self, parent: &Target) -> Target {
        let mut t = parent.clone();
        t.name = self.name.clone();
        t.parent = self.parent.clone();
        t.file = self.file;
        if self.cpu.is_some() {
            t.cpu = self.cpu.clone();
        }
        if self.clock.is_some() {
            t.clock = self.clock;
        }
        if self.stack.is_some() {
            t.stack = self.stack;
        }
        if self.run.is_some() {
            t.run = self.run.clone();
        }
        if self.load.is_some() {
            t.load = self.load;
        }
        if self.exit.is_some() {
            t.exit = self.exit.clone();
        }
        if self.irq.is_some() {
            t.irq = self.irq.clone();
        }
        if self.nmi.is_some() {
            t.nmi = self.nmi.clone();
        }
        if self.irq_interval.is_some() {
            t.irq_interval = self.irq_interval;
        }
        for (k, v) in &self.regions {
            t.regions.insert(k.clone(), v.clone());
        }
        if self.reserves.is_some() {
            t.reserves = self.reserves.clone();
        }
        if !self.places.is_empty() {
            t.places = self.places.clone();
        }
        if self.output.is_some() {
            t.output = self.output.clone();
        }
        for (k, v) in &self.encodings {
            t.encodings.insert(k.clone(), v.clone());
        }
        for (k, v) in &self.binds {
            t.binds.insert(k.clone(), v.clone());
        }
        for (k, v) in &self.consts {
            t.consts.insert(k.clone(), v.clone());
        }
        t
    }

    pub fn is_nmos(&self) -> bool {
        self.cpu.as_deref() == Some("Nmos6502")
    }
    pub fn is_w65c02s(&self) -> bool {
        self.cpu.as_deref() == Some("W65C02S")
    }
    pub fn run_rom(&self) -> bool {
        self.run.as_deref() == Some("rom")
    }
    pub fn region(&self, kind: &str) -> Vec<RangeV> {
        self.regions.get(kind).cloned().unwrap_or_default()
    }
    pub fn reserved(&self) -> Vec<RangeV> {
        self.reserves.clone().unwrap_or_default()
    }
    pub fn is_reserved(&self, a: u32) -> bool {
        self.reserved().iter().any(|r| r.contains(a))
    }
    pub fn in_region(&self, kind: &str, a: u32) -> bool {
        self.region(kind).iter().any(|r| r.contains(a))
    }
}

pub fn const_int(e: &Expr) -> Option<i128> {
    match &e.kind {
        ExprKind::Int(v, _) => Some(*v),
        ExprKind::Char(CharUnit::B(b), None) => Some(*b as i128),
        ExprKind::Unary(UnOp::Neg, x) => const_int(x).map(|v| -v),
        ExprKind::Binary(op, a, b) => {
            let x = const_int(a)?;
            let y = const_int(b)?;
            match op {
                BinOp::Add => Some(x + y),
                BinOp::Sub => Some(x - y),
                BinOp::Mul => Some(x * y),
                BinOp::Div => {
                    if y == 0 {
                        None
                    } else {
                        Some(x / y)
                    }
                }
                BinOp::Rem => {
                    if y == 0 {
                        None
                    } else {
                        Some(x % y)
                    }
                }
                BinOp::Shl => Some(x << (y as u32)),
                BinOp::Shr => Some(x >> (y as u32)),
                BinOp::And => Some(x & y),
                BinOp::Or => Some(x | y),
                BinOp::Xor => Some(x ^ y),
                _ => None,
            }
        }
        _ => None,
    }
}

fn range_of(e: &Expr) -> Option<RangeV> {
    match &e.kind {
        ExprKind::Range(Some(a), Some(b), incl) => {
            let lo = const_int(a)?;
            let hi = const_int(b)?;
            let hi = if *incl { hi } else { hi - 1 };
            if lo < 0 || hi > 0xFFFF {
                return None;
            }
            Some(RangeV { lo: lo as u32, hi: hi as u32 })
        }
        _ => None,
    }
}

fn ranges_of(e: &Expr) -> Option<Vec<RangeV>> {
    match &e.kind {
        ExprKind::ArrayList(v) => {
            let mut out = Vec::new();
            for x in v {
                out.push(range_of(x)?);
            }
            Some(out)
        }
        _ => range_of(e).map(|r| vec![r]),
    }
}

fn path_name(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Path(p) => Some(p.segs.join("::")),
        _ => None,
    }
}

fn str_of(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Str(_, v, _) => Some(units_to_string(v)),
        _ => None,
    }
}

fn names_of(e: &Expr) -> Option<Vec<String>> {
    match &e.kind {
        ExprKind::ArrayList(v) => {
            let mut out = Vec::new();
            for x in v {
                if let Some(s) = path_name(x) {
                    out.push(s);
                } else if let Some(s) = str_of(x) {
                    out.push(s);
                } else {
                    return None;
                }
            }
            Some(out)
        }
        _ => None,
    }
}

fn call_arg(e: &Expr, fname: &str) -> Option<i128> {
    if let ExprKind::Call(f, args) = &e.kind {
        if path_name(f).as_deref() == Some(fname) && args.len() == 1 {
            return const_int(&args[0].value);
        }
    }
    None
}

fn char_code(e: &Expr) -> Option<u32> {
    match &e.kind {
        ExprKind::Char(CharUnit::B(b), None) => Some(*b as u32),
        ExprKind::Char(CharUnit::U(c), None) => Some(*c as u32),
        _ => None,
    }
}

fn perr(diags: &mut Diags, code: &str, msg: &str, sp: Span) {
    diags.error(Diag::error(code, msg, sp));
}

// ---------------------------------------------------------------------------
// プロジェクトファイル
// ---------------------------------------------------------------------------

pub fn load_project(file: &FsPath, sm: &mut SourceMap, diags: &mut Diags) -> Option<Project> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(_) => {
            diags.error(
                Diag::error("E1109", &format!("プロジェクトファイル {} を読めません", file.display()), Span::none())
                    .help("--project でプロジェクトファイルを指定するか、ax65 new で作ってください"),
            );
            return None;
        }
    };
    let fid = sm.add(&file.display().to_string(), &text);
    let dir = match file.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let mut proj = Project {
        name: String::new(),
        dir,
        file: file.to_path_buf(),
        main: String::new(),
        paths: vec![".".to_string()],
        target_dir: "targets".to_string(),
        targets: Vec::new(),
        default: None,
        opt: "O2".to_string(),
        out_dir: "build".to_string(),
        step_limit: 1_000_000,
        lints: Vec::new(),
        fmt_hex: "dollar".to_string(),
        fmt_array: "rust".to_string(),
    };
    let mut p = Parser::new(&text, fid, 0);
    let ok = parse_project_body(&mut p, &mut proj, diags);
    for e in p.take_errors() {
        diags.error(e);
    }
    if !ok {
        return None;
    }
    if proj.main.is_empty() {
        perr(diags, "E1109", "プロジェクトファイルに main がありません", Span::new(fid, 1, 1, 1));
        return None;
    }
    Some(proj)
}

fn parse_project_body(p: &mut Parser, proj: &mut Project, diags: &mut Diags) -> bool {
    if p.expect_kw("project").is_err() {
        return false;
    }
    match p.parse_any_ident() {
        Ok((n, _)) => proj.name = n,
        Err(_) => return false,
    }
    if p.expect_p("{").is_err() {
        return false;
    }
    loop {
        if p.eat_p("}") {
            break;
        }
        if p.at_eof() {
            let sp = p.peek_span();
            perr(diags, "E1109", "プロジェクトファイルの「}」がありません", sp);
            return false;
        }
        let (key, ksp) = match p.parse_any_ident() {
            Ok(x) => x,
            Err(_) => return false,
        };
        if key == "lints" || key == "fmt" {
            if p.expect_p("{").is_err() {
                return false;
            }
            loop {
                if p.eat_p("}") {
                    break;
                }
                let (k, sp) = match p.parse_any_ident() {
                    Ok(x) => x,
                    Err(_) => return false,
                };
                if p.expect_p("=").is_err() {
                    return false;
                }
                let (v, _) = match p.parse_any_ident() {
                    Ok(x) => x,
                    Err(_) => return false,
                };
                if p.expect_p(";").is_err() {
                    return false;
                }
                if key == "lints" {
                    if !crate::diag::LINT_NAMES.contains(&k.as_str()) {
                        perr(diags, "E1109", &format!("知らない警告の名前 {} です", k), sp.clone());
                    }
                    if crate::diag::lint_level_from(&v).is_none() {
                        perr(diags, "E1109", "警告の扱いは allow・warn・deny のどれかです", sp);
                    }
                    proj.lints.push((k, v));
                } else if k == "hex" {
                    proj.fmt_hex = v;
                } else if k == "array" {
                    proj.fmt_array = v;
                } else {
                    perr(diags, "E1109", &format!("fmt の知らない項目 {} です", k), sp);
                }
            }
            continue;
        }
        if p.expect_p("=").is_err() {
            return false;
        }
        let e = match p.parse_expr() {
            Ok(e) => e,
            Err(_) => return false,
        };
        if p.expect_p(";").is_err() {
            return false;
        }
        let bad = |diags: &mut Diags| {
            perr(diags, "E1109", &format!("プロジェクトファイルの {} の値が正しくありません", key), ksp.clone());
        };
        match key.as_str() {
            "main" => match str_of(&e) {
                Some(s) => proj.main = s,
                None => bad(diags),
            },
            "paths" => match names_of(&e) {
                Some(v) => proj.paths = v,
                None => bad(diags),
            },
            "target_dir" => match str_of(&e) {
                Some(s) => proj.target_dir = s,
                None => bad(diags),
            },
            "targets" => match names_of(&e) {
                Some(v) => proj.targets = v,
                None => bad(diags),
            },
            "default" => match path_name(&e) {
                Some(s) => proj.default = Some(s),
                None => bad(diags),
            },
            "opt" => match path_name(&e) {
                Some(s) => proj.opt = s,
                None => bad(diags),
            },
            "out_dir" => match str_of(&e) {
                Some(s) => proj.out_dir = s,
                None => bad(diags),
            },
            "step_limit" => match const_int(&e) {
                Some(v) if v > 0 => proj.step_limit = v as u64,
                _ => bad(diags),
            },
            _ => perr(diags, "E1109", &format!("プロジェクトファイルの知らない項目 {} です", key), ksp.clone()),
        }
    }
    true
}

// ---------------------------------------------------------------------------
// ターゲット定義
// ---------------------------------------------------------------------------

pub fn target_files(proj: &Project) -> Vec<(String, PathBuf)> {
    let dir = proj.dir.join(&proj.target_dir);
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for ent in rd.flatten() {
            let p = ent.path();
            if p.extension().and_then(|x| x.to_str()) == Some("target") {
                if let Some(stem) = p.file_stem().and_then(|x| x.to_str()) {
                    v.push((stem.to_string(), p.clone()));
                }
            }
        }
    }
    v.sort();
    v
}

fn parse_target_file(path: &FsPath, sm: &mut SourceMap, diags: &mut Diags) -> Option<Target> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => {
            perr(diags, "E1108", &format!("ターゲット定義 {} を読めません", path.display()), Span::none());
            return None;
        }
    };
    let fid = sm.add(&path.display().to_string(), &text);
    let mut p = Parser::new(&text, fid, 0);
    let r = parse_target_body(&mut p, fid, diags);
    for e in p.take_errors() {
        diags.error(e);
    }
    r
}

fn parse_target_body(p: &mut Parser, fid: usize, diags: &mut Diags) -> Option<Target> {
    p.expect_kw("target").ok()?;
    let (name, _) = p.parse_any_ident().ok()?;
    let mut t = Target::empty(&name, fid);
    if p.eat_kw("extends") {
        let (par, _) = p.parse_any_ident().ok()?;
        t.parent = Some(par);
    }
    p.expect_p("{").ok()?;
    loop {
        if p.eat_p("}") {
            break;
        }
        if p.at_eof() {
            let sp = p.peek_span();
            perr(diags, "E1108", "ターゲット定義の「}」がありません", sp);
            return None;
        }
        let (key, ksp) = p.parse_any_ident().ok()?;
        match key.as_str() {
            "region" => {
                let (kind, sp) = p.parse_any_ident().ok()?;
                p.expect_p("=").ok()?;
                let e = p.parse_expr().ok()?;
                p.expect_p(";").ok()?;
                if !["zp", "stack", "ram", "io", "rom"].contains(&kind.as_str()) {
                    perr(diags, "E1108", &format!("区画の種類 {} はありません（zp・stack・ram・io・rom）", kind), sp);
                    continue;
                }
                match ranges_of(&e) {
                    Some(r) => {
                        t.regions.insert(kind, r);
                    }
                    None => perr(diags, "E1108", "区画の範囲が正しくありません", e.span.clone()),
                }
            }
            "reserve" => {
                let e = p.parse_expr().ok()?;
                p.expect_p(";").ok()?;
                match ranges_of(&e) {
                    Some(r) => {
                        let mut v = t.reserves.clone().unwrap_or_default();
                        v.extend(r);
                        t.reserves = Some(v);
                    }
                    None => perr(diags, "E1108", "reserve の範囲が正しくありません", e.span.clone()),
                }
            }
            "place" => {
                let (what, _) = p.parse_any_ident().ok()?;
                p.expect_kw("in").ok()?;
                let e = p.parse_expr().ok()?;
                p.expect_p(";").ok()?;
                match range_of(&e) {
                    Some(r) => t.places.push((what, r)),
                    None => perr(diags, "E1108", "place の範囲が正しくありません", e.span.clone()),
                }
            }
            "bind" => {
                let (iface, _) = p.parse_any_ident().ok()?;
                p.expect_p("=").ok()?;
                let (first, _) = p.parse_any_ident().ok()?;
                let mut segs = vec![first];
                while p.eat_p("::") {
                    let (s, _) = p.parse_any_ident().ok()?;
                    segs.push(s);
                }
                p.expect_p(";").ok()?;
                t.binds.insert(iface, segs.join("::"));
            }
            "const" => {
                let (cname, _) = p.parse_any_ident().ok()?;
                p.expect_p(":").ok()?;
                let ty = p.parse_type().ok()?;
                p.expect_p("=").ok()?;
                let e = p.parse_expr().ok()?;
                p.expect_p(";").ok()?;
                let tname = match &ty.kind {
                    TypeKind::Path(pp) => pp.segs.join("::"),
                    _ => String::from("?"),
                };
                let v = match &e.kind {
                    ExprKind::Bool(b) => Some(TValue::Bool(*b)),
                    ExprKind::Str(_, u, _) => Some(TValue::Str(units_to_string(u))),
                    _ => const_int(&e).map(TValue::Int),
                };
                match v {
                    Some(v) => {
                        t.consts.insert(cname, (tname, v));
                    }
                    None => perr(diags, "E1108", "ターゲット定義の定数の値が正しくありません", e.span.clone()),
                }
            }
            "encoding" => {
                let (ename, _) = p.parse_any_ident().ok()?;
                p.expect_p("{").ok()?;
                let mut map: BTreeMap<u32, u8> = BTreeMap::new();
                loop {
                    if p.eat_p("}") {
                        break;
                    }
                    let lhs = p.parse_expr().ok()?;
                    p.expect_p("=>").ok()?;
                    let rhs = p.parse_expr().ok()?;
                    p.expect_p(";").ok()?;
                    let mut pairs: Vec<(u32, i128)> = Vec::new();
                    match (&lhs.kind, &rhs.kind) {
                        (ExprKind::Range(Some(a), Some(b), ia), ExprKind::Range(Some(c), Some(d), ib)) => {
                            let (a1, b1) = (char_code(a), char_code(b));
                            let (c1, d1) = (const_int(c), const_int(d));
                            match (a1, b1, c1, d1) {
                                (Some(a1), Some(b1), Some(c1), Some(d1)) => {
                                    let b2 = if *ia { b1 as i128 } else { b1 as i128 - 1 };
                                    let d2 = if *ib { d1 } else { d1 - 1 };
                                    if b2 - a1 as i128 != d2 - c1 {
                                        perr(diags, "E1106", "変換表の範囲の両側の個数が違います", lhs.span.clone());
                                    } else {
                                        let mut k = 0i128;
                                        while a1 as i128 + k <= b2 {
                                            pairs.push(((a1 as i128 + k) as u32, c1 + k));
                                            k += 1;
                                        }
                                    }
                                }
                                _ => perr(diags, "E1106", "変換表の項目が正しくありません", lhs.span.clone()),
                            }
                        }
                        _ => match (char_code(&lhs), const_int(&rhs)) {
                            (Some(a), Some(v)) => pairs.push((a, v)),
                            _ => perr(diags, "E1106", "変換表の項目が正しくありません", lhs.span.clone()),
                        },
                    }
                    for (ch, v) in pairs {
                        if !(0..=255).contains(&v) {
                            perr(diags, "E1106", "変換表の値は 0〜255 です", rhs.span.clone());
                            continue;
                        }
                        if map.contains_key(&ch) {
                            perr(diags, "E1106", "変換表で同じ文字を 2 回書いています", lhs.span.clone());
                            continue;
                        }
                        map.insert(ch, v as u8);
                    }
                }
                t.encodings.insert(ename, map);
            }
            _ => {
                p.expect_p("=").ok()?;
                let e = p.parse_expr().ok()?;
                p.expect_p(";").ok()?;
                let bad = |diags: &mut Diags| {
                    perr(diags, "E1108", &format!("ターゲット定義の {} の値が正しくありません", key), ksp.clone());
                };
                match key.as_str() {
                    "cpu" => match &e.kind {
                        ExprKind::Path(pp) if pp.segs.len() == 2 && pp.segs[0] == "Cpu" => {
                            let c = pp.segs[1].clone();
                            if ["Nmos6502", "C65C02", "W65C02S", "R65C02"].contains(&c.as_str()) {
                                t.cpu = Some(c);
                            } else {
                                bad(diags);
                            }
                        }
                        _ => bad(diags),
                    },
                    "clock" => match const_int(&e) {
                        Some(v) if v > 0 => t.clock = Some(v as u64),
                        _ => bad(diags),
                    },
                    "stack" => match const_int(&e) {
                        Some(v) if v > 0 && v <= 256 => t.stack = Some(v as u32),
                        _ => bad(diags),
                    },
                    "run" => match path_name(&e).as_deref() {
                        Some("ram") => t.run = Some("ram".to_string()),
                        Some("rom") => t.run = Some("rom".to_string()),
                        _ => bad(diags),
                    },
                    "load" => match const_int(&e) {
                        Some(v) if (0..=0xFFFF).contains(&v) => t.load = Some(v as u32),
                        _ => bad(diags),
                    },
                    "exit" => {
                        if let Some(a) = call_arg(&e, "monitor") {
                            t.exit = Some(Exit::Monitor(a as u32));
                        } else {
                            match path_name(&e).as_deref() {
                                Some("halt") => t.exit = Some(Exit::Halt),
                                Some("never") => t.exit = Some(Exit::Never),
                                _ => bad(diags),
                            }
                        }
                    }
                    "irq" | "nmi" => {
                        let m = if let Some(a) = call_arg(&e, "indirect") {
                            Some(VecMode::Indirect(a as u32))
                        } else if let Some(a) = call_arg(&e, "jump") {
                            Some(VecMode::Jump(a as u32))
                        } else if path_name(&e).as_deref() == Some("hw") {
                            Some(VecMode::Hw)
                        } else {
                            None
                        };
                        match m {
                            Some(m) => {
                                if key == "irq" {
                                    t.irq = Some(m)
                                } else {
                                    t.nmi = Some(m)
                                }
                            }
                            None => bad(diags),
                        }
                    }
                    "irq_interval" => match const_int(&e) {
                        Some(v) if v > 0 => t.irq_interval = Some(v as u64),
                        _ => bad(diags),
                    },
                    "output" => match names_of(&e) {
                        Some(v) => {
                            for f in &v {
                                if !["bin", "woz", "hex", "rom", "ptp", "sym", "lst", "report"].contains(&f.as_str()) {
                                    perr(diags, "E1108", &format!("出力形式 {} はありません", f), e.span.clone());
                                }
                            }
                            t.output = Some(v)
                        }
                        None => bad(diags),
                    },
                    _ => perr(diags, "E1108", &format!("ターゲット定義の知らない項目 {} です", key), ksp.clone()),
                }
            }
        }
    }
    Some(t)
}

pub struct TargetSet {
    pub raw: BTreeMap<String, Target>,
}

/// target_dir の全ファイルを読む
pub fn load_targets(proj: &Project, sm: &mut SourceMap, diags: &mut Diags) -> TargetSet {
    let mut raw = BTreeMap::new();
    for (stem, path) in target_files(proj) {
        if let Some(t) = parse_target_file(&path, sm, diags) {
            if t.name != stem {
                perr(
                    diags,
                    "E1108",
                    &format!("ターゲット定義 {} のファイル名と名前（{}）が違います", path.display(), t.name),
                    Span::new(t.file, 1, 1, 1),
                );
            }
            raw.insert(t.name.clone(), t);
        }
    }
    TargetSet { raw }
}

impl TargetSet {
    pub fn names(&self) -> Vec<String> {
        self.raw.keys().cloned().collect()
    }

    /// 継承を解決したターゲット
    pub fn resolve(&self, name: &str, diags: &mut Diags) -> Option<Target> {
        let mut chain: Vec<String> = Vec::new();
        let mut cur = name.to_string();
        loop {
            if chain.contains(&cur) {
                perr(diags, "E1108", &format!("ターゲット定義の extends が輪になっています（{}）", chain.join(" → ")), Span::none());
                return None;
            }
            let t = match self.raw.get(&cur) {
                Some(t) => t,
                None => {
                    perr(diags, "E1108", &format!("ターゲット {} が見つかりません", cur), Span::none());
                    return None;
                }
            };
            chain.push(cur.clone());
            match &t.parent {
                Some(p) => cur = p.clone(),
                None => break,
            }
        }
        // 根から順に重ねる
        let mut acc: Option<Target> = None;
        for n in chain.iter().rev() {
            let t = self.raw.get(n).unwrap();
            acc = Some(match acc {
                None => t.clone(),
                Some(base) => t.merged_onto(&base),
            });
        }
        let t = acc?;
        validate_target(&t, diags);
        Some(t)
    }
}

fn validate_target(t: &Target, diags: &mut Diags) {
    let sp = Span::new(t.file, 1, 1, 1);
    let mut missing = Vec::new();
    if t.cpu.is_none() {
        missing.push("cpu");
    }
    if t.clock.is_none() {
        missing.push("clock");
    }
    if t.stack.is_none() {
        missing.push("stack");
    }
    if t.run.is_none() {
        missing.push("run");
    }
    if t.exit.is_none() {
        missing.push("exit");
    }
    if !t.regions.contains_key("zp") {
        missing.push("region zp");
    }
    if !t.regions.contains_key("stack") {
        missing.push("region stack");
    }
    if !t.regions.contains_key("ram") {
        missing.push("region ram");
    }
    if t.run.as_deref() == Some("ram") && t.load.is_none() {
        missing.push("load");
    }
    if t.run.as_deref() == Some("rom") && !t.regions.contains_key("rom") {
        missing.push("region rom");
    }
    if !missing.is_empty() {
        perr(diags, "E1108", &format!("ターゲット {} に必須の項目がありません：{}", t.name, missing.join("、")), sp.clone());
    }
    if let Some(l) = t.load {
        if t.run.as_deref() == Some("ram") && !t.in_region("ram", l) {
            perr(diags, "E0304", &format!("load（${:04X}）が ram の区画の外です", l), sp);
        }
    }
}

pub fn cpu_text(t: &Target) -> String {
    t.cpu.clone().unwrap_or_else(|| "?".to_string())
}

#[allow(dead_code)]
pub fn tok_is_ident(t: &Tok) -> bool {
    matches!(t, Tok::Ident(_))
}
