// プログラム全体：モジュールの読み込み（17.3節）、名前の表、中心となる文脈 Ctx
use crate::ast::*;
use crate::comptime::Value;
use crate::diag::{Diag, Diags, SourceMap, Span};
use crate::lexer::{tokenize_all, Tok};
use crate::parser::parse_source;
use crate::project::{Project, TValue, Target};
use crate::types::{Ty, TypeTable};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OptLevel {
    O0,
    O1,
    O2,
    Os,
}

impl OptLevel {
    pub fn name(&self) -> &'static str {
        match self {
            OptLevel::O0 => "-O0",
            OptLevel::O1 => "-O1",
            OptLevel::O2 => "-O2",
            OptLevel::Os => "-Os",
        }
    }
    pub fn from_str(s: &str) -> Option<OptLevel> {
        match s {
            "O0" | "0" => Some(OptLevel::O0),
            "O1" | "1" => Some(OptLevel::O1),
            "O2" | "2" => Some(OptLevel::O2),
            "Os" | "s" => Some(OptLevel::Os),
            _ => None,
        }
    }
}

/// 条件コンパイルの値（target.* と build.*）
#[derive(Clone, Debug, PartialEq)]
pub enum CfgVal {
    Int(i128),
    Bool(bool),
    Str(String),
    Cpu(String),
}

pub struct Module {
    pub name: String,
    pub file: usize,
    pub dir: PathBuf,
    pub items: Vec<Item>,
    pub impl_of: Option<String>,
    pub impl_span: Span,
    pub iface: Option<InterfaceDecl>,
    pub uses: Vec<(UseDecl, Span, Vec<Attr>)>,
    pub inits: Vec<Block>,
    pub ident_counts: HashMap<String, usize>,
    pub impl_items: HashSet<String>,
    pub impl_pub_spans: Vec<(String, Span)>,
    pub is_std: bool,
}

#[derive(Clone, Debug)]
pub struct ItemEntry {
    pub module: usize,
    pub name: String,
    pub item: Item,
    pub in_impl: bool,
}

#[derive(Clone, Debug)]
pub struct Import {
    pub item: Option<usize>,
    pub module: Option<usize>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GKind {
    Const,
    Static,
    Shared(Option<String>),
    Io(IoDir),
}

#[derive(Clone, Debug)]
pub struct GlobalInfo {
    pub name: String,
    pub module: usize,
    pub kind: GKind,
    pub ty: Ty,
    pub attrs: Vec<Attr>,
    pub span: Span,
    pub label: String,
    pub init_expr: Option<Expr>,
    pub decl_ty: Option<Type>,
    pub value: Option<Value>,
    pub io_addr: Option<u32>,
    pub fn_local: Option<usize>,
    pub is_pub: bool,
    pub asm_ref: bool,
    pub resolved: bool,
}

#[derive(Clone, Debug)]
pub struct LocalInfo {
    pub name: String,
    pub ty: Ty,
    pub is_param: bool,
    pub span: Span,
    pub read: bool,
    pub written: bool,
    pub addr_taken: bool,
    pub asm_ref: bool,
}

#[derive(Clone, Debug)]
pub struct FnInfo {
    pub name: String,
    pub module: usize,
    pub item: Option<usize>,
    pub decl: Option<FnDecl>,
    pub init_body: Option<Block>,
    pub kind: FnKind,
    pub attrs: Vec<Attr>,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    pub interrupt: Option<String>,
    pub locals: Vec<LocalInfo>,
    pub label: String,
    pub span: Span,
    pub sig_done: bool,
    pub checked: bool,
    pub calls_irq_enable: bool,
    pub has_unsafe: bool,
}

#[derive(Clone, Debug)]
pub enum Res {
    Local(usize),
    Global(usize),
    Func,
    Value(Value),
    TypeName(Ty),
}

/// パスの名前解決の結果
#[derive(Clone, Debug)]
pub enum PathRes {
    Global(usize),
    Func(usize),
    Type(Ty),
    Value(Value),
    Module(usize),
    TypeFn(Ty, String),
    Unbound(String),
    Private(String),
    NotFound(String),
}

#[derive(Clone, Debug)]
pub enum CallTarget {
    Fn(usize),
    FnSetVar,
    Builtin(String),
    TypeFn(Ty, String),
    SomeCtor,
    Folded,
}

#[derive(Clone, Debug)]
pub struct AccessEvent {
    pub fid: usize,
    pub gid: usize,
    pub write: bool,
    pub single: bool,
    pub in_atomic: bool,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct CallEdge {
    pub from: usize,
    pub to: usize,
    pub span: Span,
    pub in_atomic: bool,
}

pub struct Ctx {
    pub sm: SourceMap,
    pub diags: Diags,
    pub target: Target,
    pub build: BTreeMap<String, CfgVal>,
    pub opt: OptLevel,
    pub step_limit: u64,
    pub known_ifaces: HashSet<String>,
    pub modules: Vec<Module>,
    pub mod_index: HashMap<String, usize>,
    pub bound_impl: HashMap<String, usize>,
    pub items: Vec<ItemEntry>,
    pub mod_scope: Vec<HashMap<String, usize>>,
    pub imports: Vec<HashMap<String, Import>>,
    pub used_imports: HashSet<(usize, String)>,
    pub types: TypeTable,
    pub item_type: HashMap<usize, Ty>,
    pub type_busy: HashSet<usize>,
    pub globals: Vec<GlobalInfo>,
    pub item_global: HashMap<usize, usize>,
    pub funcs: Vec<FnInfo>,
    pub item_func: HashMap<usize, usize>,
    pub const_busy: HashSet<usize>,
    pub ety: HashMap<NodeId, Ty>,
    pub econst: HashMap<NodeId, Value>,
    pub res: HashMap<NodeId, Res>,
    pub calls: HashMap<NodeId, CallTarget>,
    pub ct_choice: HashMap<NodeId, Option<usize>>,
    pub node_local: HashMap<NodeId, usize>,
    pub events: Vec<AccessEvent>,
    pub call_edges: Vec<CallEdge>,
    pub unsafe_list: Vec<String>,
    pub next_id: usize,
    pub steps: u64,
}

pub fn module_label_prefix(name: &str) -> String {
    name.replace("::", "_")
}

fn flatten_items(
    items: Vec<Item>,
    target: &Target,
    build: &BTreeMap<String, CfgVal>,
    known: &HashSet<String>,
    m: &mut Module,
    out: &mut Vec<Item>,
    diags: &mut Diags,
    in_impl: bool,
) {
    for it in items {
        match it.kind {
            ItemKind::ComptimeIf(c) => {
                match eval_cfg(&c.cond, target, build, known) {
                    Ok(CfgVal::Bool(b)) => {
                        let chosen = if b { c.then_items } else { c.else_items };
                        flatten_items(chosen, target, build, known, m, out, diags, in_impl);
                    }
                    Ok(_) => diags.err("E0401", "comptime if の条件は bool でなければなりません", c.cond.span.clone()),
                    Err((code, msg, sp)) => diags.err(&code, &msg, sp),
                }
            }
            ItemKind::ComptimeMatch(cm) => {
                let v = match eval_cfg(&cm.scrut, target, build, known) {
                    Ok(v) => v,
                    Err((code, msg, sp)) => {
                        diags.err(&code, &msg, sp);
                        continue;
                    }
                };
                let mut taken = false;
                for (pats, arm_items) in cm.arms {
                    if taken {
                        break;
                    }
                    let mut hit = false;
                    for p in &pats {
                        if cfg_pat_matches(p, &v, target, build, known) {
                            hit = true;
                        }
                    }
                    if hit {
                        taken = true;
                        flatten_items(arm_items, target, build, known, m, out, diags, in_impl);
                    }
                }
            }
            ItemKind::Impl(d) => {
                if m.impl_of.is_some() && m.impl_of.as_deref() != Some(d.name.as_str()) {
                    diags.err("E1101", "1 つのモジュールには 1 つの impl だけを書けます", it.span.clone());
                }
                m.impl_of = Some(d.name.clone());
                m.impl_span = it.span.clone();
                for sub in &d.items {
                    if let Some(n) = sub.name() {
                        m.impl_items.insert(n.clone());
                        if sub.is_pub {
                            m.impl_pub_spans.push((n, sub.span.clone()));
                        }
                    }
                }
                flatten_items(d.items, target, build, known, m, out, diags, true);
            }
            ItemKind::Interface(ref d) => {
                m.iface = Some(d.clone());
                out.push(it);
            }
            ItemKind::Init(b) => {
                m.inits.push(b);
            }
            ItemKind::Use(u) => {
                let sp = u.path.span.clone();
                m.uses.push((u, sp, it.attrs.clone()));
            }
            _ => out.push(it),
        }
    }
}

fn cfg_pat_matches(
    p: &Pattern,
    v: &CfgVal,
    target: &Target,
    build: &BTreeMap<String, CfgVal>,
    known: &HashSet<String>,
) -> bool {
    match &p.kind {
        PatKind::Wild | PatKind::Ident(_) => true,
        PatKind::Lit(e) => match eval_cfg(e, target, build, known) {
            Ok(x) => &x == v,
            Err(_) => false,
        },
        PatKind::Range(a, b, incl) => {
            let (a, b) = (eval_cfg(a, target, build, known), eval_cfg(b, target, build, known));
            match (a, b, v) {
                (Ok(CfgVal::Int(a)), Ok(CfgVal::Int(b)), CfgVal::Int(x)) => {
                    if *incl {
                        *x >= a && *x <= b
                    } else {
                        *x >= a && *x < b
                    }
                }
                _ => false,
            }
        }
        _ => false,
    }
}

type CfgErr = (String, String, Span);

fn cfg_err(msg: &str, sp: &Span) -> CfgErr {
    ("E1001".to_string(), msg.to_string(), sp.clone())
}

pub fn tvalue_cfg(v: &TValue) -> CfgVal {
    match v {
        TValue::Int(i) => CfgVal::Int(*i),
        TValue::Bool(b) => CfgVal::Bool(*b),
        TValue::Str(s) => CfgVal::Str(s.clone()),
    }
}

/// target.* と build.* だけを使う式を計算する（13.6節）
pub fn eval_cfg(
    e: &Expr,
    target: &Target,
    build: &BTreeMap<String, CfgVal>,
    known: &HashSet<String>,
) -> Result<CfgVal, CfgErr> {
    match &e.kind {
        ExprKind::Bool(b) => Ok(CfgVal::Bool(*b)),
        ExprKind::Int(v, _) => Ok(CfgVal::Int(*v)),
        ExprKind::Str(_, u, _) => Ok(CfgVal::Str(crate::parser::units_to_string(u))),
        ExprKind::Path(p) => {
            if p.segs.len() == 2 && p.segs[0] == "Cpu" {
                return Ok(CfgVal::Cpu(p.segs[1].clone()));
            }
            Err(cfg_err("条件コンパイルの条件には target.* と build.* だけを使えます", &e.span))
        }
        ExprKind::Field(base, name) => {
            if let ExprKind::Path(p) = &base.kind {
                if p.segs.len() == 1 && p.segs[0] == "target" {
                    return target_field(target, name, &e.span);
                }
                if p.segs.len() == 1 && p.segs[0] == "build" {
                    return Ok(build.get(name).cloned().unwrap_or(CfgVal::Bool(false)));
                }
            }
            Err(cfg_err("条件コンパイルの条件には target.* と build.* だけを使えます", &e.span))
        }
        ExprKind::MethodCall(base, name, args) => {
            if let ExprKind::Path(p) = &base.kind {
                if p.segs.len() == 1 && p.segs[0] == "target" && name == "has" && args.len() == 1 {
                    if let ExprKind::Path(ip) = &args[0].value.kind {
                        let iname = ip.segs.join("::");
                        if !known.contains(&iname) && !target.binds.contains_key(&iname) {
                            return Err((
                                "E1006".to_string(),
                                format!("target.has() に知らないインターフェース {} を書いています", iname),
                                args[0].value.span.clone(),
                            ));
                        }
                        return Ok(CfgVal::Bool(target.binds.contains_key(&iname)));
                    }
                }
            }
            Err(cfg_err("条件コンパイルの条件には target.* と build.* だけを使えます", &e.span))
        }
        ExprKind::Unary(UnOp::Not, x) => match eval_cfg(x, target, build, known)? {
            CfgVal::Bool(b) => Ok(CfgVal::Bool(!b)),
            CfgVal::Int(i) => Ok(CfgVal::Int(!i)),
            _ => Err(cfg_err("! を使えない値です", &e.span)),
        },
        ExprKind::Unary(UnOp::Neg, x) => match eval_cfg(x, target, build, known)? {
            CfgVal::Int(i) => Ok(CfgVal::Int(-i)),
            _ => Err(cfg_err("- を使えない値です", &e.span)),
        },
        ExprKind::Binary(op, a, b) => {
            if *op == BinOp::LAnd || *op == BinOp::LOr {
                let x = eval_cfg(a, target, build, known)?;
                let xb = match x {
                    CfgVal::Bool(v) => v,
                    _ => return Err(cfg_err("&& と || には bool を使います", &e.span)),
                };
                if *op == BinOp::LAnd && !xb {
                    return Ok(CfgVal::Bool(false));
                }
                if *op == BinOp::LOr && xb {
                    return Ok(CfgVal::Bool(true));
                }
                return match eval_cfg(b, target, build, known)? {
                    CfgVal::Bool(v) => Ok(CfgVal::Bool(v)),
                    _ => Err(cfg_err("&& と || には bool を使います", &e.span)),
                };
            }
            let x = eval_cfg(a, target, build, known)?;
            let y = eval_cfg(b, target, build, known)?;
            match op {
                BinOp::Eq => return Ok(CfgVal::Bool(x == y)),
                BinOp::Ne => return Ok(CfgVal::Bool(x != y)),
                _ => {}
            }
            let (i, j) = match (x, y) {
                (CfgVal::Int(i), CfgVal::Int(j)) => (i, j),
                _ => return Err(cfg_err("この演算には整数を使います", &e.span)),
            };
            let r = match op {
                BinOp::Lt => CfgVal::Bool(i < j),
                BinOp::Le => CfgVal::Bool(i <= j),
                BinOp::Gt => CfgVal::Bool(i > j),
                BinOp::Ge => CfgVal::Bool(i >= j),
                BinOp::Add => CfgVal::Int(i + j),
                BinOp::Sub => CfgVal::Int(i - j),
                BinOp::Mul => CfgVal::Int(i * j),
                BinOp::Div => {
                    if j == 0 {
                        return Err(cfg_err("0 で割っています", &e.span));
                    }
                    CfgVal::Int(i / j)
                }
                BinOp::Rem => {
                    if j == 0 {
                        return Err(cfg_err("0 で割っています", &e.span));
                    }
                    CfgVal::Int(i % j)
                }
                BinOp::And => CfgVal::Int(i & j),
                BinOp::Or => CfgVal::Int(i | j),
                BinOp::Xor => CfgVal::Int(i ^ j),
                BinOp::Shl => CfgVal::Int(i << (j as u32)),
                BinOp::Shr => CfgVal::Int(i >> (j as u32)),
                _ => return Err(cfg_err("この演算は条件に使えません", &e.span)),
            };
            Ok(r)
        }
        _ => Err(cfg_err("条件コンパイルの条件には target.* と build.* だけを使えます", &e.span)),
    }
}

pub fn target_field(target: &Target, name: &str, sp: &Span) -> Result<CfgVal, CfgErr> {
    match name {
        "cpu" => Ok(CfgVal::Cpu(target.cpu.clone().unwrap_or_default())),
        "clock" => Ok(CfgVal::Int(target.clock.unwrap_or(0) as i128)),
        "stack" => Ok(CfgVal::Int(target.stack.unwrap_or(0) as i128)),
        "load" => Ok(CfgVal::Int(target.load.unwrap_or(0) as i128)),
        "irq_interval" => Ok(CfgVal::Int(target.irq_interval.unwrap_or(0) as i128)),
        "name" => Ok(CfgVal::Str(target.name.clone())),
        // irq・nmi が none でないか（1.1。13.5節）
        "has_irq" => Ok(CfgVal::Bool(target.has_irq())),
        "has_nmi" => Ok(CfgVal::Bool(target.has_nmi())),
        _ => match target.consts.get(name) {
            Some((_, v)) => Ok(tvalue_cfg(v)),
            None => Err((
                "E1006".to_string(),
                format!("ターゲット定義に定数 {} がありません", name),
                sp.clone(),
            )),
        },
    }
}

/// 標準ライブラリの場所
pub fn find_std_dir() -> PathBuf {
    if let Ok(p) = std::env::var("AX65_STD") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            let cands = [d.join("std"), d.join("..").join("std"), d.join("..").join("..").join("std")];
            for c in cands.iter() {
                if c.join("core.ax65").exists() {
                    return c.clone();
                }
            }
        }
    }
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/std"))
}

pub fn known_interfaces(proj: &Project, std_dir: &PathBuf, target: &Target) -> HashSet<String> {
    let mut s = HashSet::new();
    let mut dirs: Vec<PathBuf> = vec![std_dir.clone()];
    for p in &proj.paths {
        dirs.push(proj.dir.join(p));
    }
    for d in dirs {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for ent in rd.flatten() {
                let p = ent.path();
                if p.extension().and_then(|x| x.to_str()) == Some("ax65") {
                    if let Ok(text) = std::fs::read_to_string(&p) {
                        if text.contains("interface ") {
                            if let Some(stem) = p.file_stem().and_then(|x| x.to_str()) {
                                s.insert(stem.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    for k in target.binds.keys() {
        s.insert(k.clone());
    }
    s
}

fn find_module_file(proj: &Project, std_dir: &PathBuf, segs: &[String]) -> Option<(PathBuf, bool)> {
    let mut rel = PathBuf::new();
    for (i, s) in segs.iter().enumerate() {
        if i + 1 == segs.len() {
            rel.push(format!("{}.ax65", s));
        } else {
            rel.push(s);
        }
    }
    for p in &proj.paths {
        let c = proj.dir.join(p).join(&rel);
        if c.exists() {
            return Some((c, false));
        }
    }
    let c = std_dir.join(&rel);
    if c.exists() {
        return Some((c, true));
    }
    None
}

struct Loader<'a> {
    proj: &'a Project,
    target: &'a Target,
    build: &'a BTreeMap<String, CfgVal>,
    known: &'a HashSet<String>,
    std_dir: &'a PathBuf,
    modules: Vec<Module>,
    index: HashMap<String, usize>,
    next_id: usize,
}

impl<'a> Loader<'a> {
    fn load_file(
        &mut self,
        name: &str,
        path: &PathBuf,
        is_std: bool,
        sm: &mut SourceMap,
        diags: &mut Diags,
    ) -> Option<usize> {
        if let Some(i) = self.index.get(name) {
            return Some(*i);
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => {
                diags.err("E0204", &format!("ファイル {} を読めません", path.display()), Span::none());
                return None;
            }
        };
        let disp = match path.strip_prefix(&self.proj.dir) {
            Ok(r) => r.display().to_string(),
            Err(_) => path.display().to_string(),
        };
        let fid = sm.add(&disp, &text);
        let (items, errs, next) = parse_source(&text, fid, self.next_id);
        self.next_id = next;
        let had_err = !errs.is_empty();
        for e in errs {
            diags.error(e);
        }
        let (toks, _) = tokenize_all(&text, fid);
        let mut counts: HashMap<String, usize> = HashMap::new();
        for t in toks {
            if let Tok::Ident(s) = t.tok {
                *counts.entry(s).or_insert(0) += 1;
            }
        }
        let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        let mut m = Module {
            name: name.to_string(),
            file: fid,
            dir,
            items: Vec::new(),
            impl_of: None,
            impl_span: Span::none(),
            iface: None,
            uses: Vec::new(),
            inits: Vec::new(),
            ident_counts: counts,
            impl_items: HashSet::new(),
            impl_pub_spans: Vec::new(),
            is_std,
        };
        if !had_err {
            let mut out = Vec::new();
            flatten_items(items, self.target, self.build, self.known, &mut m, &mut out, diags, false);
            m.items = out;
        }
        let idx = self.modules.len();
        self.modules.push(m);
        self.index.insert(name.to_string(), idx);
        Some(idx)
    }

    /// use のパスからモジュールを探して読み込む。戻り値は（モジュール、残りのパス）
    fn load_use(&mut self, segs: &[String], sp: &Span, sm: &mut SourceMap, diags: &mut Diags) -> Option<(usize, Vec<String>)> {
        // 長いほうから順に、モジュールのファイルとして探す
        let mut n = segs.len();
        while n >= 1 {
            let name = segs[..n].join("::");
            if let Some(i) = self.index.get(&name) {
                return Some((*i, segs[n..].to_vec()));
            }
            if let Some((path, is_std)) = find_module_file(self.proj, self.std_dir, &segs[..n]) {
                let i = self.load_file(&name, &path, is_std, sm, diags)?;
                return Some((i, segs[n..].to_vec()));
            }
            n -= 1;
        }
        diags.error(
            Diag::error("E0204", &format!("モジュール {} が見つかりません", segs.join("::")), sp.clone())
                .note("プロジェクトの paths のフォルダと、標準ライブラリ（std）の順に探します（17.3節）"),
        );
        None
    }
}

pub struct Loaded {
    pub modules: Vec<Module>,
    pub index: HashMap<String, usize>,
    pub bound_impl: HashMap<String, usize>,
    pub next_id: usize,
}

/// main のファイルから use と bind をたどって、すべてのモジュールを読み込む
pub fn load_program(
    proj: &Project,
    target: &Target,
    build: &BTreeMap<String, CfgVal>,
    known: &HashSet<String>,
    std_dir: &PathBuf,
    sm: &mut SourceMap,
    diags: &mut Diags,
) -> Option<Loaded> {
    let mut ld = Loader {
        proj,
        target,
        build,
        known,
        std_dir,
        modules: Vec::new(),
        index: HashMap::new(),
        next_id: 1,
    };
    let main_path = proj.dir.join(&proj.main);
    let stem = main_path.file_stem().and_then(|x| x.to_str()).unwrap_or("main").to_string();
    ld.load_file(&stem, &main_path, false, sm, diags)?;
    // core は常に使える（20.2節）
    if let Some((p, is_std)) = find_module_file(proj, std_dir, &["core".to_string()]) {
        ld.load_file("core", &p, is_std, sm, diags);
    }
    let mut bound_impl: HashMap<String, usize> = HashMap::new();
    let mut done = 0usize;
    while done < ld.modules.len() {
        let mi = done;
        done += 1;
        let uses: Vec<(UseDecl, Span, Vec<Attr>)> = ld.modules[mi].uses.clone();
        for (u, sp, _) in uses {
            let r = ld.load_use(&u.path.segs, &sp, sm, diags);
            if let Some((m2, _rest)) = r {
                // インターフェースなら bind された実装も読み込む
                let iface_name = ld.modules[m2].iface.as_ref().map(|d| d.name.clone());
                if let Some(iname) = iface_name {
                    if let Some(mp) = target.binds.get(&iname) {
                        if !bound_impl.contains_key(&iname) {
                            let segs: Vec<String> = mp.split("::").map(|s| s.to_string()).collect();
                            if let Some((im, rest)) = ld.load_use(&segs, &sp, sm, diags) {
                                if !rest.is_empty() {
                                    diags.err("E1103", &format!("bind {} = {} のモジュールが見つかりません", iname, mp), Span::new(target.file, 1, 1, 1));
                                } else {
                                    bound_impl.insert(iname.clone(), im);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // bind の誤り（実装のモジュールが impl を持たない、違うインターフェースの impl）
    for (iname, mi) in &bound_impl {
        match &ld.modules[*mi].impl_of {
            Some(x) if x == iname => {}
            Some(x) => diags.err(
                "E1103",
                &format!("bind {} = {} ですが、そのモジュールは impl {} です", iname, ld.modules[*mi].name, x),
                Span::new(target.file, 1, 1, 1),
            ),
            None => diags.err(
                "E1103",
                &format!("bind {} = {} ですが、そのモジュールに impl {} がありません", iname, ld.modules[*mi].name, iname),
                Span::new(target.file, 1, 1, 1),
            ),
        }
    }
    let next_id = ld.next_id;
    Some(Loaded { modules: ld.modules, index: ld.index, bound_impl, next_id })
}

impl Ctx {
    /// 名前を、モジュールの中の宣言と use で取り込んだ名前から探す
    pub fn lookup_name(&mut self, m: usize, name: &str) -> Option<usize> {
        if let Some(i) = self.mod_scope[m].get(name) {
            return Some(*i);
        }
        if let Some(imp) = self.imports[m].get(name) {
            if let Some(it) = imp.item {
                self.used_imports.insert((m, name.to_string()));
                return Some(it);
            }
        }
        None
    }

    /// モジュールの別名（use で取り込んだモジュールと core）
    pub fn lookup_module(&mut self, m: usize, name: &str) -> Option<usize> {
        if let Some(imp) = self.imports[m].get(name) {
            if let Some(mm) = imp.module {
                self.used_imports.insert((m, name.to_string()));
                return Some(mm);
            }
        }
        if name == "core" {
            return self.mod_index.get("core").copied();
        }
        None
    }

    /// インターフェースのモジュールなら、bind された実装のモジュールに置き換える
    pub fn effective_module(&self, m: usize) -> Result<usize, String> {
        if let Some(d) = &self.modules[m].iface {
            match self.bound_impl.get(&d.name) {
                Some(i) => Ok(*i),
                None => Err(d.name.clone()),
            }
        } else {
            Ok(m)
        }
    }

    /// 別のモジュールから見える名前か
    pub fn item_visible_from(&self, item: usize, from: usize) -> bool {
        let e = &self.items[item];
        if e.module == from {
            return true;
        }
        if e.item.is_pub {
            return true;
        }
        if e.in_impl {
            // impl の中で、インターフェースにある名前は自動で公開される
            let m = &self.modules[e.module];
            if let Some(iname) = &m.impl_of {
                for mm in &self.modules {
                    if let Some(d) = &mm.iface {
                        if &d.name == iname {
                            for mem in &d.members {
                                let n = match mem {
                                    IfaceMember::Fn(f) => f.name.clone(),
                                    IfaceMember::Const(n, _) => n.clone(),
                                };
                                if n == e.name {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }
        false
    }

    pub fn new_id(&mut self) -> NodeId {
        let i = self.next_id;
        self.next_id += 1;
        i
    }

    pub fn err(&mut self, code: &str, msg: &str, sp: &Span) {
        self.diags.err(code, msg, sp.clone());
    }

    pub fn tyname(&self, t: &Ty) -> String {
        self.types.name(t)
    }
}

pub fn naming_ok_snake(s: &str) -> bool {
    let t = s.trim_start_matches('_');
    t.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

pub fn naming_ok_upper(s: &str) -> bool {
    let t = s.trim_start_matches('_');
    t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

pub fn naming_ok_camel(s: &str) -> bool {
    let t = s.trim_start_matches('_');
    match t.chars().next() {
        Some(c) if c.is_ascii_uppercase() => !t.contains('_'),
        Some(_) => false,
        None => true,
    }
}
