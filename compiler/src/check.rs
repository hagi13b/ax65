// 関数の本体の型検査（3章〜12章の規則）
use crate::ast::*;
use crate::comptime::{value_int, Value};
use crate::diag::{Diag, LintLevel, Span};
use crate::program::*;
use crate::sema::lint_overrides;
use crate::types::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub enum ScopeEnt {
    Local(usize),
    Global(usize),
}

pub struct LoopCx {
    pub label: Option<String>,
    pub is_loop: bool,
    pub has_break: bool,
    pub break_ty: Option<Ty>,
}

pub struct FnCx {
    pub fid: usize,
    pub module: usize,
    pub scopes: Vec<HashMap<String, ScopeEnt>>,
    pub loops: Vec<LoopCx>,
    pub atomic: u32,
    pub timed: u32,
    pub assigned: Option<HashSet<usize>>,
    pub ret: Ty,
    pub lints: HashMap<String, LintLevel>,
    pub is_const_fn: bool,
    pub interrupt: bool,
    pub loop_vars: HashSet<usize>,
}

fn is_pow2(v: i128) -> bool {
    v > 0 && (v & (v - 1)) == 0
}

impl Ctx {
    fn add_local(&mut self, fx: &mut FnCx, name: &str, ty: Ty, is_param: bool, sp: &Span, node: NodeId) -> usize {
        let idx = self.funcs[fx.fid].locals.len();
        self.funcs[fx.fid].locals.push(LocalInfo {
            name: name.to_string(),
            ty,
            is_param,
            span: sp.clone(),
            read: false,
            written: false,
            addr_taken: false,
            asm_ref: false,
        });
        if let Some(s) = fx.scopes.last_mut() {
            s.insert(name.to_string(), ScopeEnt::Local(idx));
        }
        self.node_local.insert(node, idx);
        idx
    }

    fn lookup_scope(&self, fx: &FnCx, name: &str) -> Option<ScopeEnt> {
        for s in fx.scopes.iter().rev() {
            if let Some(e) = s.get(name) {
                return Some(e.clone());
            }
        }
        None
    }

    fn set_assigned(&mut self, fx: &mut FnCx, l: usize) {
        if let Some(s) = &mut fx.assigned {
            s.insert(l);
        }
    }

    fn warn_fx(&mut self, fx: &FnCx, d: Diag) {
        let l = fx.lints.clone();
        self.diags.warn_with(d, &l);
    }

    pub fn check_fn(&mut self, fid: usize) {
        if self.funcs[fid].checked {
            return;
        }
        self.funcs[fid].checked = true;
        self.ensure_sig(fid);
        let f = self.funcs[fid].clone();
        let mut lints = lint_overrides(&f.attrs);
        if let Some(it) = f.item {
            for (k, v) in lint_overrides(&self.items[it].item.attrs) {
                lints.insert(k, v);
            }
        }
        let mut fx = FnCx {
            fid,
            module: f.module,
            scopes: vec![HashMap::new()],
            loops: Vec::new(),
            atomic: 0,
            timed: 0,
            assigned: Some(HashSet::new()),
            ret: f.ret.clone(),
            lints,
            is_const_fn: f.kind == FnKind::Const,
            interrupt: f.interrupt.is_some(),
            loop_vars: HashSet::new(),
        };
        if f.kind == FnKind::Extern {
            self.unsafe_list.push(format!("extern fn {}（{}）", f.name, self.modules[f.module].name));
        }
        if f.kind == FnKind::Comptime {
            return;
        }
        // 引数
        if let Some(d) = &f.decl {
            for (i, p) in d.params.iter().enumerate() {
                let t = f.params.get(i).map(|x| x.1.clone()).unwrap_or(Ty::Error);
                let id = self.new_id();
                let l = self.add_local(&mut fx, &p.name, t, true, &p.span, id);
                self.funcs[fid].locals[l].written = true;
                self.set_assigned(&mut fx, l);
                if (p.name.starts_with('_') || p.name == "_") && l < self.funcs[fid].locals.len() {
                    self.funcs[fid].locals[l].read = true;
                }
                if !self.modules[f.module].is_std && !naming_ok_snake(&p.name) {
                    let d = Diag::warning("naming", &format!("{} は名前の慣習に合いません（引数は snake_case）", p.name), p.span.clone());
                    self.warn_fx(&fx, d);
                }
            }
        }
        if f.kind == FnKind::Extern || f.kind == FnKind::Asm {
            if let Some(d) = &f.decl {
                if let Some(a) = &d.asm_body {
                    self.check_asm_refs(&mut fx, &a.text, &d.span, false);
                }
                if let Some(s) = &d.stack {
                    if let Err(e) = self.eval_in_module(s, f.module, Some(&Ty::CInt)) {
                        self.diags.err(&e.code, &e.msg, e.span);
                    }
                }
                if let Some(a) = &d.extern_addr {
                    if let Err(e) = self.eval_in_module(a, f.module, Some(&Ty::CInt)) {
                        self.diags.err(&e.code, &e.msg, e.span);
                    }
                }
            }
            // asm fn の引数は読まれたとみなす
            for l in self.funcs[fid].locals.iter_mut() {
                l.read = true;
            }
            return;
        }
        let body = match (&f.decl, &f.init_body) {
            (Some(d), _) => d.body.clone(),
            (None, Some(b)) => Some(b.clone()),
            _ => None,
        };
        let body = match body {
            Some(b) => b,
            None => return,
        };
        let ret = f.ret.clone();
        let t = self.check_block(&mut fx, &body, Some(&ret));
        if ret != Ty::Unit && t != Ty::Never {
            match &body.tail {
                Some(tail) => {
                    self.expect_ty(tail, &t, &ret);
                    self.check_escape(&fx, tail);
                }
                None => {
                    if t != Ty::Never {
                        let sp = body.span.clone();
                        self.diags.error(
                            Diag::error("E0401", &format!("関数 {} は {} を返しますが、最後に値がありません", f.name, self.tyname(&ret)), sp),
                        );
                    }
                }
            }
        }
        // 使っていない変数
        let locals = self.funcs[fid].locals.clone();
        if !self.modules[f.module].is_std {
            for l in locals {
                if !l.read && !l.asm_ref && !l.name.starts_with('_') {
                    let what = if l.is_param { "引数" } else { "変数" };
                    let d = Diag::warning("unused_variables", &format!("{} {} を使っていません", what, l.name), l.span.clone())
                        .help(&format!("使わないなら _{} と書いてください", l.name));
                    self.warn_fx(&fx, d);
                }
            }
        }
    }

    pub fn check_block(&mut self, fx: &mut FnCx, b: &Block, exp: Option<&Ty>) -> Ty {
        fx.scopes.push(HashMap::new());
        let mut diverges = false;
        for s in &b.stmts {
            if self.check_stmt(fx, s) {
                diverges = true;
            }
        }
        let t = match &b.tail {
            Some(e) => {
                let t = self.check_expr(fx, e, exp);
                if diverges {
                    Ty::Never
                } else {
                    t
                }
            }
            None => {
                if diverges {
                    Ty::Never
                } else {
                    Ty::Unit
                }
            }
        };
        fx.scopes.pop();
        self.ety.insert(b.id, t.clone());
        t
    }

    /// 文を検査する。戻り値は「ここで制御が戻らない」か
    fn check_stmt(&mut self, fx: &mut FnCx, s: &Stmt) -> bool {
        match &s.kind {
            StmtKind::Let(name, ty, init) => {
                let m = fx.module;
                let decl = ty.as_ref().map(|t| self.resolve_type(m, t));
                let mut vt = decl.clone().unwrap_or(Ty::Error);
                let mut diverge = false;
                if let Some(e) = init {
                    let t = self.check_expr(fx, e, decl.as_ref());
                    if t == Ty::Never {
                        diverge = true;
                    }
                    match &decl {
                        Some(d) => {
                            self.expect_ty(e, &t, d);
                        }
                        None => {
                            vt = self.default_type(e, &t, true);
                        }
                    }
                } else if decl.is_none() {
                    self.err("E0401", "let には型か初期値が必要です", &s.span);
                }
                if let Some(n) = name {
                    if is_keyword(n) {
                        self.err("E0101", &format!("{} はキーワードなので、名前に使えません", n), &s.span);
                    }
                    if !self.modules[m].is_std && !naming_ok_snake(n) {
                        let d = Diag::warning("naming", &format!("{} は名前の慣習に合いません（ローカル変数は snake_case）", n), s.span.clone());
                        self.warn_fx(fx, d);
                    }
                    match vt {
                        Ty::FnItem(_) | Ty::TypeName(_) | Ty::RangeIter(_) | Ty::Cpu => {
                            self.err("E0401", "この値は変数に入れられません", &s.span);
                        }
                        _ => {}
                    }
                    if let Ty::Ptr(_) = vt {}
                    let l = self.add_local(fx, n, vt, false, &s.span, s.id);
                    if init.is_some() {
                        self.funcs[fx.fid].locals[l].written = true;
                        self.set_assigned(fx, l);
                    }
                } else if let Some(e) = init {
                    // let _ = X; は値を捨てる（11.3節）
                    let _ = e;
                }
                diverge
            }
            StmtKind::Expr(e, semi) => {
                let t = self.check_expr(fx, e, None);
                if *semi {
                    let is_call = matches!(e.kind, ExprKind::Call(..) | ExprKind::MethodCall(..));
                    if is_call {
                        if let ExprKind::MethodCall(_, name, _) = &e.kind {
                            if name == "insert" || name == "remove" || name == "toggle" || name == "clear" {
                                return t == Ty::Never;
                            }
                        }
                        let warn = matches!(t, Ty::Bool | Ty::Opt(_));
                        if warn {
                            let d = Diag::warning("unused_results", &format!("{} の値を使っていません", self.tyname(&t)), e.span.clone());
                            self.warn_fx(fx, d);
                        }
                    }
                } else if t != Ty::Unit && t != Ty::Never && t != Ty::Error && !e.is_block_like() {
                    self.err("E0105", "文の区切りの「;」がありません", &e.span);
                }
                t == Ty::Never
            }
            StmtKind::Item(it) => {
                match &it.kind {
                    ItemKind::Static(_) | ItemKind::Const(_) => {
                        let name = it.name().unwrap_or_default();
                        if let Some(gid) = self.register_item_info(usize::MAX, fx.module, it, Some(fx.fid)) {
                            let label = format!("{}_{}", self.funcs[fx.fid].label, name);
                            self.globals[gid].label = label;
                            self.ensure_global(gid);
                            if let Some(sc) = fx.scopes.last_mut() {
                                sc.insert(name, ScopeEnt::Global(gid));
                            }
                        }
                    }
                    _ => self.err(crate::diag::E_UNIMPL, "未実装：関数の中には static と const だけを宣言できます", &it.span),
                }
                false
            }
            StmtKind::ComptimeAssert(c, msg) => {
                match self.eval_in_module(c, fx.module, None) {
                    Ok(Value::Bool(true)) => {}
                    Ok(Value::Bool(false)) => self.err("E1005", &format!("comptime assert が成り立ちません：{}", msg), &c.span),
                    Ok(_) => self.err("E0403", "comptime assert の条件は bool でなければなりません", &c.span),
                    Err(e) => self.diags.err(&e.code, &e.msg, e.span),
                }
                false
            }
        }
    }

    /// 型を書かない let の型（整数リテラルだけなら u8）
    fn default_type(&mut self, e: &Expr, t: &Ty, for_let: bool) -> Ty {
        match t {
            Ty::IntLit => {
                if let Some(Value::Int(v, None)) = self.econst.get(&e.id).cloned() {
                    if !(0..=255).contains(&v) {
                        let code = if for_let { "E0511" } else { "E0510" };
                        self.diags.error(
                            Diag::error(code, &format!("型を書かない整数 {} は u8 になりますが、u8 に収まりません", v), e.span.clone())
                                .help("let x: u16 = … のように型を書いてください"),
                        );
                        return Ty::Int(IntK::U8);
                    }
                }
                self.retag(e, &Ty::Int(IntK::U8));
                Ty::Int(IntK::U8)
            }
            Ty::Opt(inner) if **inner == Ty::IntLit => {
                self.retag_some(e, &Ty::Int(IntK::U8));
                Ty::Opt(Box::new(Ty::Int(IntK::U8)))
            }
            Ty::Never => Ty::Error,
            t => t.clone(),
        }
    }

    fn retag_some(&mut self, e: &Expr, t: &Ty) {
        if let ExprKind::Call(_, args) = &e.kind {
            if let Some(a) = args.first() {
                self.retag(&a.value, t);
            }
        }
        self.ety.insert(e.id, Ty::Opt(Box::new(t.clone())));
    }

    /// 整数リテラルの型を決める
    pub fn retag(&mut self, e: &Expr, t: &Ty) {
        self.ety.insert(e.id, t.clone());
        let hex = matches!(e.kind, ExprKind::Int(_, true));
        if let Some(Value::Int(v, None)) = self.econst.get(&e.id).cloned() {
            if let Ty::Bcd(_) = t {
                if hex {
                    self.err("E0509", "BCD の型に 16進のリテラルは書けません（10進で書いてください）", &e.span);
                    return;
                }
            }
            match self.convert_value(Value::Int(v, None), t, &e.span) {
                Ok(nv) => {
                    self.econst.insert(e.id, nv);
                }
                Err(er) => self.diags.err(&er.code, &er.msg, er.span),
            }
        }
        // 括弧の中の式などにも伝える
        match &e.kind {
            ExprKind::Unary(UnOp::Neg, x) | ExprKind::Unary(UnOp::Not, x) => {
                if self.ety.get(&x.id) == Some(&Ty::IntLit) {
                    self.retag(x, t);
                }
            }
            ExprKind::Binary(op, a, b) if !op.is_cmp() && *op != BinOp::LAnd && *op != BinOp::LOr => {
                if self.ety.get(&a.id) == Some(&Ty::IntLit) {
                    self.retag(a, t);
                }
                if *op != BinOp::Shl && *op != BinOp::Shr && self.ety.get(&b.id) == Some(&Ty::IntLit) {
                    self.retag(b, t);
                }
            }
            _ => {}
        }
    }

    /// 型を合わせる。合わせられなければ false
    pub fn coerce(&mut self, e: &Expr, got: &Ty, want: &Ty) -> bool {
        if got == want {
            return true;
        }
        match (got, want) {
            (Ty::Never, _) | (_, Ty::Error) | (Ty::Error, _) => true,
            (Ty::IntLit, Ty::Int(_)) | (Ty::IntLit, Ty::Bcd(_)) | (Ty::IntLit, Ty::Range(_)) | (Ty::IntLit, Ty::CInt) => {
                self.retag(e, want);
                true
            }
            (Ty::IntLit, Ty::Flags(_)) => {
                self.retag(e, want);
                true
            }
            (Ty::Int(_), Ty::CInt) => true,
            (Ty::Range(r), Ty::Int(k)) => {
                let bs = self.types.ranges[*r].base_size();
                !k.signed() && k.size() == bs
            }
            (Ty::FnItem(f), Ty::FnSet(s)) => {
                let set = self.types.fnsets[*s].clone();
                if set.fns.contains(f) {
                    self.econst.insert(e.id, Value::Func(*f));
                    self.ety.insert(e.id, want.clone());
                    true
                } else {
                    self.err("E0602", &format!("{} は関数集合 {} の一覧にありません", self.funcs[*f].name, set.name), &e.span);
                    true
                }
            }
            (Ty::Opt(a), Ty::Opt(b)) => {
                if **a == Ty::Never {
                    self.ety.insert(e.id, want.clone());
                    return true;
                }
                if **a == Ty::IntLit {
                    let bb = (**b).clone();
                    if let ExprKind::Call(_, args) = &e.kind {
                        if let Some(arg) = args.first() {
                            let ok = self.coerce(&arg.value, &Ty::IntLit, &bb);
                            self.ety.insert(e.id, want.clone());
                            return ok;
                        }
                    }
                }
                false
            }
            (Ty::Ptr(a), Ty::Ptr(b)) => **a == Ty::Never || **b == Ty::Never,
            _ => false,
        }
    }

    pub fn expect_ty(&mut self, e: &Expr, got: &Ty, want: &Ty) -> bool {
        if self.coerce(e, got, want) {
            return true;
        }
        self.mismatch(&e.span, want, got);
        false
    }

    fn mismatch(&mut self, sp: &Span, want: &Ty, got: &Ty) {
        let wn = self.tyname(want);
        let gn = self.tyname(got);
        let signs = match (want, got) {
            (Ty::Int(a), Ty::Int(b)) => a.signed() != b.signed(),
            _ => false,
        };
        let mut d = if signs {
            Diag::error("E0402", &format!("符号付きと符号なしを混ぜています（{} が必要ですが、{} です）", wn, gn), sp.clone())
                .help("as で変換してください")
        } else {
            Diag::error("E0401", &format!("型が違います（{} が必要ですが、{} です）", wn, gn), sp.clone())
        };
        match (want, got) {
            (Ty::Int(a), Ty::Int(b)) if a.size() != b.size() => {
                d = d.note("暗黙の変換はしません（3.1節）").help(&format!("as {} で変換してください", a.name()));
            }
            (Ty::Str(_, _), Ty::Array(_, _)) => {
                d = d.help("長さ付きの文字列は p\"…\" と書きます（3.9節）");
            }
            (Ty::Char(_), Ty::Char(_)) => {
                d = d.note("変換表ごとに別の型です（3.3節）");
            }
            (Ty::Range(_), Ty::Int(_)) => {
                d = d.help("::check(n) で範囲を確かめてください（3.4節）");
            }
            _ => {}
        }
        self.diags.error(d);
    }

    fn check_cond(&mut self, fx: &mut FnCx, e: &Expr) {
        let t = self.check_expr(fx, e, Some(&Ty::Bool));
        if t != Ty::Bool && t != Ty::Never && t != Ty::Error {
            self.diags.error(
                Diag::error("E0403", &format!("条件には bool だけを書けます（{} です）", self.tyname(&t)), e.span.clone())
                    .help("x != 0 のように比較を書いてください"),
            );
        }
    }

    fn note_access(&mut self, fx: &FnCx, gid: usize, write: bool, single: bool, sp: &Span) {
        match self.globals[gid].kind {
            GKind::Static | GKind::Shared(_) => {}
            _ => return,
        }
        self.events.push(AccessEvent {
            fid: fx.fid,
            gid,
            write,
            single,
            in_atomic: fx.atomic > 0 || fx.timed > 0,
            span: sp.clone(),
        });
    }

    fn global_ok_in_const_fn(&mut self, fx: &FnCx, gid: usize, sp: &Span) {
        if fx.is_const_fn && self.globals[gid].kind != GKind::Const {
            self.err("E0603", &format!("const fn の中で {} を使っています（static・shared・I/O には触れられません）", self.globals[gid].name), sp);
        }
    }

    /// パス（名前）の検査。as_place は代入の左辺
    fn check_path(&mut self, fx: &mut FnCx, e: &Expr, p: &Path, exp: Option<&Ty>, as_place: bool) -> Ty {
        if p.segs.len() == 1 {
            let n = &p.segs[0];
            if let Some(ent) = self.lookup_scope(fx, n) {
                match ent {
                    ScopeEnt::Local(l) => {
                        self.res.insert(e.id, Res::Local(l));
                        if !as_place {
                            self.funcs[fx.fid].locals[l].read = true;
                            let assigned = match &fx.assigned {
                                Some(s) => s.contains(&l),
                                None => true,
                            };
                            if !assigned {
                                self.diags.error(
                                    Diag::error("E0501", &format!("{} に値を入れる前に読んでいます", n), e.span.clone()),
                                );
                            }
                        }
                        return self.funcs[fx.fid].locals[l].ty.clone();
                    }
                    ScopeEnt::Global(g) => {
                        return self.global_path(fx, e, g, as_place);
                    }
                }
            }
            if n == "None" {
                self.econst.insert(e.id, Value::Opt(None));
                return match exp {
                    Some(Ty::Opt(t)) => Ty::Opt(t.clone()),
                    _ => Ty::Opt(Box::new(Ty::Never)),
                };
            }
        }
        if p.segs.len() == 2 && p.segs[0] == "Cpu" {
            self.econst.insert(e.id, Value::Cpu(p.segs[1].clone()));
            return Ty::Cpu;
        }
        let m = fx.module;
        match self.resolve_value_path(m, &p.segs, &e.span) {
            PathRes::Global(g) => self.global_path(fx, e, g, as_place),
            PathRes::Func(f) => {
                self.res.insert(e.id, Res::Func);
                self.econst.insert(e.id, Value::Func(f));
                Ty::FnItem(f)
            }
            PathRes::Type(t) => {
                self.res.insert(e.id, Res::TypeName(t.clone()));
                Ty::TypeName(Box::new(t))
            }
            PathRes::Value(v) => {
                let t = match &v {
                    Value::Enum(id, _) => Ty::Enum(*id),
                    Value::Flags(id, _) => Ty::Flags(*id),
                    _ => Ty::Error,
                };
                self.res.insert(e.id, Res::Value(v.clone()));
                self.econst.insert(e.id, v);
                t
            }
            PathRes::Module(mm) => {
                let mname = self.modules[mm].name.clone();
                self.err("E0201", &format!("{} はモジュール（{}）なので、値として使えません", p.text(), mname), &e.span);
                Ty::Error
            }
            PathRes::TypeFn(_, _) => {
                self.err("E0201", &format!("{} は関数なので、( ) を付けて呼んでください", p.text()), &e.span);
                Ty::Error
            }
            PathRes::Unbound(i) => {
                self.diags.error(
                    Diag::error("E1104", &format!("インターフェース {} はこのターゲットで bind されていません", i), e.span.clone())
                        .help(&format!("comptime if target.has({}) {{ … }} の中で使ってください", i)),
                );
                Ty::Error
            }
            PathRes::Private(msg) => {
                self.err("E0203", &msg, &e.span);
                Ty::Error
            }
            PathRes::NotFound(msg) => {
                self.err("E0201", &msg, &e.span);
                Ty::Error
            }
        }
    }

    fn global_path(&mut self, fx: &mut FnCx, e: &Expr, g: usize, as_place: bool) -> Ty {
        self.ensure_global(g);
        self.res.insert(e.id, Res::Global(g));
        let gi = self.globals[g].clone();
        self.global_ok_in_const_fn(fx, g, &e.span);
        match &gi.kind {
            GKind::Const => {
                if let Some(v) = &gi.value {
                    self.econst.insert(e.id, v.clone());
                }
            }
            GKind::Io(dir) => {
                if !as_place && *dir == IoDir::Out {
                    self.err("E0901", &format!("{} は io(out)（書き込み専用）なので読めません", gi.name), &e.span);
                }
                if gi.module != fx.module {
                    self.err("E0203", &format!("I/O 変数 {} は、宣言したモジュールだけが使えます", gi.name), &e.span);
                }
            }
            _ => {
                if !as_place {
                    let single = self.types.size_of(&gi.ty) == 1;
                    if !matches!(gi.ty, Ty::Array(_, _)) {
                        self.note_access(fx, g, false, single, &e.span);
                    }
                }
            }
        }
        gi.ty.clone()
    }

    /// 式が指す global（配列の要素・フィールドを含む）
    fn base_global(&self, e: &Expr) -> Option<usize> {
        match &e.kind {
            ExprKind::Path(_) => match self.res.get(&e.id) {
                Some(Res::Global(g)) => Some(*g),
                _ => None,
            },
            ExprKind::Index(b, _) | ExprKind::Field(b, _) => self.base_global(b),
            _ => None,
        }
    }

    fn base_local(&self, e: &Expr) -> Option<usize> {
        match &e.kind {
            ExprKind::Path(_) => match self.res.get(&e.id) {
                Some(Res::Local(l)) => Some(*l),
                _ => None,
            },
            ExprKind::Index(b, _) | ExprKind::Field(b, _) => self.base_local(b),
            _ => None,
        }
    }

    fn mentions_global(&self, e: &Expr, g: usize) -> bool {
        let mut found = false;
        walk_expr(e, &mut |x: &Expr| {
            if let ExprKind::Path(_) = &x.kind {
                if let Some(Res::Global(gg)) = self.res.get(&x.id) {
                    if *gg == g {
                        found = true;
                    }
                }
            }
        });
        found
    }

    /// 代入の左辺
    fn check_place(&mut self, fx: &mut FnCx, e: &Expr) -> Ty {
        let t = match &e.kind {
            ExprKind::Path(p) => {
                let t = self.check_path(fx, e, p, None, true);
                match self.res.get(&e.id).cloned() {
                    Some(Res::Local(l)) => {
                        if fx.loop_vars.contains(&l) {
                            self.err("E0512", "ループ変数には代入できません", &e.span);
                        }
                        self.funcs[fx.fid].locals[l].written = true;
                        self.set_assigned(fx, l);
                    }
                    Some(Res::Global(g)) => {
                        let gi = self.globals[g].clone();
                        match gi.kind {
                            GKind::Const => self.err("E0502", &format!("const {} には代入できません", gi.name), &e.span),
                            GKind::Io(IoDir::In) => self.err("E0901", &format!("{} は io(in)（読み込み専用）なので書き込めません", gi.name), &e.span),
                            _ => {}
                        }
                    }
                    Some(_) => self.err("E0512", "代入できない式です", &e.span),
                    None => {}
                }
                t
            }
            ExprKind::Index(b, i) => self.check_index(fx, e, b, i, true),
            ExprKind::Field(b, f) => self.check_field(fx, e, b, f, true),
            _ => {
                self.err("E0512", "代入できない式です", &e.span);
                Ty::Error
            }
        };
        self.ety.insert(e.id, t.clone());
        t
    }

    fn check_index(&mut self, fx: &mut FnCx, e: &Expr, base: &Expr, idx: &Expr, as_place: bool) -> Ty {
        let bt = if as_place {
            match &base.kind {
                ExprKind::Path(_) | ExprKind::Index(_, _) | ExprKind::Field(_, _) => self.check_place_base(fx, base),
                _ => self.check_expr(fx, base, None),
            }
        } else {
            self.check_expr(fx, base, None)
        };
        let r = match &bt {
            Ty::Array(et, len) => {
                let want_idx = match len {
                    Len::R(r) => Some(Ty::Range(*r)),
                    Len::E(x) => Some(Ty::Enum(*x)),
                    Len::N(_) => None,
                };
                let it = self.check_expr(fx, idx, want_idx.as_ref());
                let n = self.types.len_count(len);
                let cidx = self.econst.get(&idx.id).cloned().and_then(|v| value_int(&v));
                if let Some(ci) = cidx {
                    if ci < 0 || ci >= n as i128 {
                        self.diags.error(Diag::error("E0412", &format!("添字 {} が配列の範囲（0〜{}）の外です", ci, n as i128 - 1), idx.span.clone()));
                    } else if it == Ty::IntLit {
                        self.retag(idx, &Ty::Int(IntK::U8));
                    }
                } else {
                    let ok = match len {
                        Len::N(256) => matches!(it, Ty::Int(IntK::U8)) || matches!(it, Ty::Range(_)) || matches!(it, Ty::Enum(_)),
                        Len::N(_) => false,
                        Len::R(r) => it == Ty::Range(*r),
                        Len::E(x) => it == Ty::Enum(*x),
                    };
                    if !ok && it != Ty::Error {
                        let bname = match &base.kind {
                            ExprKind::Path(p) => p.text(),
                            _ => "配列".to_string(),
                        };
                        let mut d = Diag::error("E0412", &format!("{} の添字に {} は使えません", bname, self.tyname(&it)), idx.span.clone());
                        d = d.label(&format!("{} 型で、範囲に収まる保証がありません", self.tyname(&it)));
                        match len {
                            Len::R(r) => {
                                let rd = self.types.ranges[*r].clone();
                                d = d.note(&format!("{} の添字の型は {}（{}..{}）です", bname, rd.name, rd.lo, rd.hi))
                                    .help(&format!("{}::check(n) で範囲を確かめてください", rd.name));
                            }
                            Len::N(n) => {
                                d = d.note(&format!("{} は長さ {} の配列です", bname, n)).help("変数で読むときは .get(n) を使うか、範囲型を宣言してください");
                            }
                            Len::E(x) => {
                                let en = self.types.enums[*x].name.clone();
                                d = d.note(&format!("{} の添字の型は {} です", bname, en));
                            }
                        }
                        self.diags.error(d);
                    }
                }
                // 共有変数の要素
                if let Some(g) = self.base_global(base) {
                    let es = self.types.size_of(et);
                    if !as_place {
                        self.note_access(fx, g, false, es == 1, &e.span);
                    }
                }
                // 定数の畳み込み
                if let (Some(Value::Array(items)), Some(ci)) = (self.econst.get(&base.id).cloned(), cidx) {
                    if ci >= 0 && (ci as usize) < items.len() {
                        self.econst.insert(e.id, items[ci as usize].clone());
                    }
                }
                (**et).clone()
            }
            Ty::Ptr(t) => {
                let it = self.check_expr(fx, idx, Some(&Ty::Int(IntK::U8)));
                if it != Ty::Int(IntK::U8) && it != Ty::Error {
                    if !self.coerce(idx, &it, &Ty::Int(IntK::U8)) {
                        self.err("E0401", "ポインタの添字は u8 です", &idx.span);
                    }
                }
                let fname = self.funcs[fx.fid].name.clone();
                let what = if as_place { "書き込み" } else { "読み込み" };
                self.unsafe_list.push(format!("ポインタを通した{}（{}、{}行）", what, fname, e.span.line));
                (**t).clone()
            }
            Ty::Error => {
                self.check_expr(fx, idx, None);
                Ty::Error
            }
            other => {
                let n = self.tyname(other);
                self.check_expr(fx, idx, None);
                self.err("E0401", &format!("{} には添字を使えません", n), &e.span);
                Ty::Error
            }
        };
        self.ety.insert(e.id, r.clone());
        r
    }

    /// 代入の左辺の、添字・フィールドの土台
    fn check_place_base(&mut self, fx: &mut FnCx, e: &Expr) -> Ty {
        let t = match &e.kind {
            ExprKind::Path(p) => {
                let t = self.check_path(fx, e, p, None, true);
                match self.res.get(&e.id).cloned() {
                    Some(Res::Local(l)) => {
                        if let Ty::Ptr(_) = t {
                            // ポインタの指す先への書き込み：ポインタ自身は読む
                            self.funcs[fx.fid].locals[l].read = true;
                        } else {
                            self.funcs[fx.fid].locals[l].written = true;
                            self.set_assigned(fx, l);
                        }
                    }
                    Some(Res::Global(g)) => {
                        let gi = self.globals[g].clone();
                        match gi.kind {
                            GKind::Const => self.err("E0502", &format!("const {} には代入できません", gi.name), &e.span),
                            GKind::Io(IoDir::In) => self.err("E0901", &format!("{} は io(in)（読み込み専用）なので書き込めません", gi.name), &e.span),
                            _ => {}
                        }
                    }
                    _ => {}
                }
                t
            }
            ExprKind::Index(b, i) => self.check_index(fx, e, b, i, true),
            ExprKind::Field(b, f) => self.check_field(fx, e, b, f, true),
            _ => self.check_expr(fx, e, None),
        };
        self.ety.insert(e.id, t.clone());
        t
    }

    fn check_field(&mut self, fx: &mut FnCx, e: &Expr, base: &Expr, name: &str, as_place: bool) -> Ty {
        // target.* と build.*
        if let ExprKind::Path(p) = &base.kind {
            if p.segs.len() == 1 && (p.segs[0] == "target" || p.segs[0] == "build") && self.lookup_scope(fx, &p.segs[0]).is_none() {
                return match self.eval_in_module(e, fx.module, None) {
                    Ok(v) => {
                        let t = match &v {
                            Value::Int(_, None) => Ty::IntLit,
                            Value::Bool(_) => Ty::Bool,
                            Value::Cpu(_) => Ty::Cpu,
                            Value::Str(k, _, enc) => Ty::Str(k.clone(), enc.clone()),
                            _ => Ty::Error,
                        };
                        self.econst.insert(e.id, v);
                        self.ety.insert(e.id, t.clone());
                        t
                    }
                    Err(er) => {
                        self.diags.err(&er.code, &er.msg, er.span);
                        Ty::Error
                    }
                };
            }
        }
        let bt = if as_place { self.check_place_base(fx, base) } else { self.check_expr(fx, base, None) };
        let r = match &bt {
            Ty::Struct(s) => match self.types.field(*s, name) {
                Some((_, ft)) => {
                    if let Some(Value::Struct(_, fields)) = self.econst.get(&base.id).cloned() {
                        let def = self.types.structs[*s].clone();
                        if let Some(i) = def.fields.iter().position(|f| f.0 == name) {
                            self.econst.insert(e.id, fields[i].clone());
                        }
                    }
                    if let Some(g) = self.base_global(base) {
                        if !as_place && !matches!(self.globals[g].ty, Ty::Array(_, _)) {
                            let single = self.types.size_of(&ft) == 1;
                            self.note_access(fx, g, false, single, &e.span);
                        }
                    }
                    ft
                }
                None => {
                    let sn = self.types.structs[*s].name.clone();
                    self.err("E0416", &format!("構造体 {} にフィールド {} はありません", sn, name), &e.span);
                    Ty::Error
                }
            },
            Ty::Ptr(inner) => match &**inner {
                Ty::Struct(s) => {
                    if !self.types.structs[*s].packed {
                        self.err("E0401", "ポインタでフィールドを読めるのは #[packed] の構造体だけです", &e.span);
                    }
                    let fname = self.funcs[fx.fid].name.clone();
                    self.unsafe_list.push(format!("ポインタを通した読み書き（{}、{}行）", fname, e.span.line));
                    match self.types.field(*s, name) {
                        Some((_, ft)) => ft,
                        None => {
                            self.err("E0416", &format!("フィールド {} はありません", name), &e.span);
                            Ty::Error
                        }
                    }
                }
                _ => {
                    self.err("E0401", "このポインタはフィールドを持ちません", &e.span);
                    Ty::Error
                }
            },
            Ty::Error => Ty::Error,
            other => {
                let n = self.tyname(other);
                self.err("E0416", &format!("{} にフィールド {} はありません", n, name), &e.span);
                Ty::Error
            }
        };
        self.ety.insert(e.id, r.clone());
        r
    }

    /// 式の検査。exp は期待する型（整数リテラルの型を決めるのに使う）
    pub fn check_expr(&mut self, fx: &mut FnCx, e: &Expr, exp: Option<&Ty>) -> Ty {
        let t = self.check_expr_inner(fx, e, exp);
        if !self.ety.contains_key(&e.id) || self.ety.get(&e.id) != Some(&t) {
            // retag などで更新済みのときはそちらを使う
            if !matches!(self.ety.get(&e.id), Some(_)) || t != Ty::IntLit {
                self.ety.insert(e.id, t.clone());
            }
        }
        self.ety.get(&e.id).cloned().unwrap_or(t)
    }

    fn check_expr_inner(&mut self, fx: &mut FnCx, e: &Expr, exp: Option<&Ty>) -> Ty {
        match &e.kind {
            ExprKind::Int(v, hex) => {
                self.econst.insert(e.id, Value::Int(*v, None));
                self.ety.insert(e.id, Ty::IntLit);
                match exp {
                    Some(t @ Ty::Int(_)) | Some(t @ Ty::Bcd(_)) | Some(t @ Ty::Range(_)) => {
                        let t = t.clone();
                        if let Ty::Bcd(_) = t {
                            if *hex {
                                self.err("E0509", "BCD の型に 16進のリテラルは書けません（10進で書いてください）", &e.span);
                                return t;
                            }
                        }
                        self.retag(e, &t);
                        t
                    }
                    Some(Ty::CInt) => Ty::CInt,
                    _ => Ty::IntLit,
                }
            }
            ExprKind::Real(r) => {
                self.econst.insert(e.id, Value::Real(*r));
                Ty::CReal
            }
            ExprKind::Bool(b) => {
                self.econst.insert(e.id, Value::Bool(*b));
                Ty::Bool
            }
            ExprKind::Char(u, enc) => match self.encode_char(u, enc, &e.span) {
                Ok(b) => {
                    self.econst.insert(e.id, Value::Char(b, enc.clone()));
                    Ty::Char(enc.clone())
                }
                Err(er) => {
                    self.diags.err(&er.code, &er.msg, er.span);
                    Ty::Char(enc.clone())
                }
            },
            ExprKind::Str(p, units, enc) => match self.string_value(p, units, enc, &e.span) {
                Ok(v) => {
                    let t = match &v {
                        Value::Array(items) => Ty::Array(Box::new(Ty::Char(enc.clone())), Len::N(items.len().max(1) as u32)),
                        Value::Str(k, _, en) => Ty::Str(k.clone(), en.clone()),
                        _ => Ty::Error,
                    };
                    if let Value::Array(items) = &v {
                        if items.is_empty() || items.len() > 256 {
                            self.err("E0510", "\"…\" の文字列は 1〜256 文字です", &e.span);
                        }
                    }
                    self.econst.insert(e.id, v);
                    t
                }
                Err(er) => {
                    self.diags.err(&er.code, &er.msg, er.span);
                    Ty::Error
                }
            },
            ExprKind::Unit => Ty::Unit,
            ExprKind::Path(p) => self.check_path(fx, e, p, exp, false),
            ExprKind::Unary(op, x) => self.check_unary(fx, e, *op, x, exp),
            ExprKind::Binary(op, a, b) => self.check_binary(fx, e, *op, a, b, exp),
            ExprKind::Assign(op, lhs, rhs) => {
                self.check_assign(fx, e, *op, lhs, rhs);
                Ty::Unit
            }
            ExprKind::Cast(x, t) => self.check_cast(fx, e, x, t),
            ExprKind::Try(x) => {
                let t = self.check_expr(fx, x, None);
                match &fx.ret {
                    Ty::Opt(_) => {}
                    _ => {
                        self.diags.error(
                            Diag::error("E0407", "? は戻り値が Option の関数の中だけで使えます", e.span.clone()),
                        );
                    }
                }
                match t {
                    Ty::Opt(inner) => *inner,
                    Ty::Error => Ty::Error,
                    other => {
                        self.err("E0406", &format!("? は Option の値にだけ使えます（{} です）", self.tyname(&other)), &e.span);
                        Ty::Error
                    }
                }
            }
            ExprKind::Field(b, f) => self.check_field(fx, e, b, f, false),
            ExprKind::MethodCall(r, name, args) => self.check_method(fx, e, r, name, args, exp),
            ExprKind::Call(f, args) => self.check_call(fx, e, f, args, exp),
            ExprKind::Index(b, i) => self.check_index(fx, e, b, i, false),
            ExprKind::Range(a, b, _) => {
                let at = match a {
                    Some(x) => self.check_expr(fx, x, None),
                    None => Ty::IntLit,
                };
                let bt = match b {
                    Some(x) => {
                        let want = if at == Ty::IntLit { None } else { Some(at.clone()) };
                        self.check_expr(fx, x, want.as_ref())
                    }
                    None => at.clone(),
                };
                let mut et = at.clone();
                if at == Ty::IntLit && bt != Ty::IntLit {
                    if let Some(x) = a {
                        self.coerce(x, &Ty::IntLit, &bt);
                    }
                    et = bt.clone();
                } else if bt != at && bt != Ty::IntLit {
                    if let Some(x) = b {
                        self.expect_ty(x, &bt, &at);
                    }
                }
                if et == Ty::IntLit {
                    if let Some(x) = a {
                        self.default_type(x, &Ty::IntLit, false);
                    }
                    if let Some(x) = b {
                        self.default_type(x, &Ty::IntLit, false);
                    }
                    et = Ty::Int(IntK::U8);
                }
                Ty::RangeIter(Box::new(et))
            }
            ExprKind::StructLit(path, fields, base) => self.check_struct_lit(fx, e, path, fields, base),
            ExprKind::ArrayList(items) => {
                let (et, n) = match exp {
                    Some(Ty::Array(t, l)) => (Some((**t).clone()), Some(self.types.len_count(l))),
                    _ => (None, None),
                };
                let mut elem = et.clone();
                let mut all_const = true;
                let mut vals = Vec::new();
                for x in items {
                    let t = self.check_expr(fx, x, elem.as_ref());
                    if let Some(w) = elem.clone() {
                        self.expect_ty(x, &t, &w);
                    } else {
                        let d = self.default_type(x, &t, false);
                        elem = Some(d);
                    }
                    match self.econst.get(&x.id) {
                        Some(v) => vals.push(v.clone()),
                        None => all_const = false,
                    }
                }
                if let Some(n) = n {
                    if n as usize != items.len() {
                        self.err("E0508", &format!("配列の値の個数（{}）が配列の長さ（{}）と違います", items.len(), n), &e.span);
                    }
                }
                if all_const {
                    self.econst.insert(e.id, Value::Array(vals));
                }
                match exp {
                    Some(t @ Ty::Array(_, _)) => t.clone(),
                    _ => Ty::Array(Box::new(elem.unwrap_or(Ty::Error)), Len::N(items.len().max(1) as u32)),
                }
            }
            ExprKind::ArrayRepeat(_, _) | ExprKind::ArrayFor(_, _, _) => {
                match self.eval_in_module(e, fx.module, exp) {
                    Ok(v) => {
                        let t = match exp {
                            Some(t) => t.clone(),
                            None => Ty::Error,
                        };
                        if t == Ty::Error {
                            self.err("E0401", "配列の値の型が決まりません（型を書いてください）", &e.span);
                        } else {
                            match self.convert_value(v, &t, &e.span) {
                                Ok(v2) => {
                                    self.econst.insert(e.id, v2);
                                }
                                Err(er) => self.diags.err(&er.code, &er.msg, er.span),
                            }
                        }
                        t
                    }
                    Err(er) => {
                        self.diags.err(&er.code, &er.msg, er.span);
                        Ty::Error
                    }
                }
            }
            ExprKind::Block(b) => self.check_block(fx, b, exp),
            ExprKind::If(c, then_b, els) => self.check_if(fx, e, c, then_b, els, exp),
            ExprKind::IfLet(pat, scrut, then_b, els) => self.check_if_let(fx, pat, scrut, then_b, els, exp),
            ExprKind::Match(scrut, arms, _) => self.check_match(fx, e, scrut, arms, exp),
            ExprKind::Loop(label, body) => {
                let before = fx.assigned.clone();
                fx.loops.push(LoopCx { label: label.clone(), is_loop: true, has_break: false, break_ty: None });
                self.check_block(fx, body, Some(&Ty::Unit));
                let lc = fx.loops.pop().unwrap();
                if !lc.has_break {
                    fx.assigned = None;
                    return Ty::Never;
                }
                fx.assigned = before;
                lc.break_ty.unwrap_or(Ty::Unit)
            }
            ExprKind::While(label, c, body, attrs) => {
                let bounded = find_attr(attrs, "max").is_some();
                if let Some(a) = find_attr(attrs, "max") {
                    if attr_int_arg(a).map(|v| v <= 0).unwrap_or(true) {
                        self.err("E0102", "#[max(n)] には正の数を書きます", &a.span);
                    }
                }
                if !bounded && (fx.interrupt || fx.timed > 0) {
                    self.diags.error(
                        Diag::error("E0702", "割り込み処理・サイクル数のブロックの中で、上限のないループを使っています", e.span.clone())
                            .help("#[max(n)] を付けるか、for を使ってください"),
                    );
                }
                self.check_cond(fx, c);
                let before = fx.assigned.clone();
                fx.loops.push(LoopCx { label: label.clone(), is_loop: false, has_break: false, break_ty: None });
                self.check_block(fx, body, Some(&Ty::Unit));
                fx.loops.pop();
                fx.assigned = merge_after_loop(before, fx.assigned.clone());
                Ty::Unit
            }
            ExprKind::For(label, name, iter, body, _attrs) => {
                let et = self.check_for_iter(fx, iter);
                let before = fx.assigned.clone();
                fx.scopes.push(HashMap::new());
                if name != "_" {
                    let l = self.add_local(fx, name, et, false, &iter.span, e.id);
                    self.funcs[fx.fid].locals[l].written = true;
                    self.set_assigned(fx, l);
                    fx.loop_vars.insert(l);
                    if name.starts_with('_') {
                        self.funcs[fx.fid].locals[l].read = true;
                    }
                }
                fx.loops.push(LoopCx { label: label.clone(), is_loop: false, has_break: false, break_ty: None });
                self.check_block(fx, body, Some(&Ty::Unit));
                fx.loops.pop();
                fx.scopes.pop();
                fx.assigned = merge_after_loop(before, fx.assigned.clone());
                Ty::Unit
            }
            ExprKind::Atomic(b) => {
                fx.atomic += 1;
                let t = self.check_block(fx, b, exp);
                fx.atomic -= 1;
                t
            }
            ExprKind::Timed(kind, n, b) => {
                match self.eval_in_module(n, fx.module, Some(&Ty::CInt)) {
                    Ok(v) => {
                        let x = value_int(&v).unwrap_or(0);
                        if x <= 0 {
                            self.err("E0510", "サイクル数は正の数です", &n.span);
                        }
                        self.econst.insert(n.id, Value::Int(x, None));
                    }
                    Err(er) => self.diags.err(&er.code, &er.msg, er.span),
                }
                if *kind != TimedKind::MaxCycles {
                    fx.timed += 1;
                }
                let t = self.check_block(fx, b, Some(&Ty::Unit));
                if *kind != TimedKind::MaxCycles {
                    fx.timed -= 1;
                }
                let _ = t;
                Ty::Unit
            }
            ExprKind::Asm(a) => {
                for bnd in &a.binds {
                    match bnd {
                        AsmBind::In(reg, x) => {
                            if !["A", "X", "Y", "a", "x", "y"].contains(&reg.as_str()) {
                                self.err("E1210", "asm の受け渡しのレジスタは A・X・Y です", &x.span);
                            }
                            let t = self.check_expr(fx, x, None);
                            let t = self.default_type(x, &t, false);
                            if self.types.size_of(&t) != 1 {
                                self.err("E1210", "レジスタで受け渡せるのは 1 バイトの値だけです", &x.span);
                            }
                        }
                        AsmBind::Out(reg, name, sp) => {
                            if !["A", "X", "Y", "a", "x", "y"].contains(&reg.as_str()) {
                                self.err("E1210", "asm の受け渡しのレジスタは A・X・Y です", sp);
                            }
                            match self.lookup_scope(fx, name) {
                                Some(ScopeEnt::Local(l)) => {
                                    self.funcs[fx.fid].locals[l].written = true;
                                    self.set_assigned(fx, l);
                                }
                                Some(ScopeEnt::Global(_)) => {}
                                None => self.err("E0201", &format!("変数 {} が見つかりません", name), sp),
                            }
                        }
                    }
                }
                if a.is_unsafe {
                    self.funcs[fx.fid].has_unsafe = true;
                    let props: Vec<String> = a.props.iter().map(|(k, v)| format!("{}({})", k, v)).collect();
                    let fname = self.funcs[fx.fid].name.clone();
                    self.unsafe_list.push(format!("unsafe asm（{}、{}行）：{}", fname, e.span.line, props.join(", ")));
                }
                let text = a.text.text.clone();
                self.check_asm_refs(fx, &text, &e.span, a.is_unsafe);
                Ty::Unit
            }
            ExprKind::Break(label, v) => {
                let idx = self.find_loop(fx, label, &e.span, true);
                if let Some(x) = v {
                    let bt = match idx.and_then(|i| fx.loops[i].break_ty.clone()) {
                        Some(t) => Some(t),
                        None => None,
                    };
                    let t = self.check_expr(fx, x, bt.as_ref());
                    if let Some(i) = idx {
                        if !fx.loops[i].is_loop {
                            self.err("E0704", "値を返す break は loop の中だけで使えます", &e.span);
                        }
                        if let Some(p) = fx.loops[i].break_ty.clone() {
                            self.expect_ty(x, &t, &p);
                        } else {
                            let d = self.default_type(x, &t, false);
                            fx.loops[i].break_ty = Some(d);
                        }
                    }
                }
                fx.assigned = None;
                Ty::Never
            }
            ExprKind::Continue(label) => {
                self.find_loop(fx, label, &e.span, false);
                fx.assigned = None;
                Ty::Never
            }
            ExprKind::Return(v) => {
                let rt = fx.ret.clone();
                match v {
                    Some(x) => {
                        let t = self.check_expr(fx, x, Some(&rt));
                        self.expect_ty(x, &t, &rt);
                        self.check_escape(fx, x);
                    }
                    None => {
                        if rt != Ty::Unit {
                            self.err("E0401", &format!("return に値がありません（{} を返す関数です）", self.tyname(&rt)), &e.span);
                        }
                    }
                }
                fx.assigned = None;
                Ty::Never
            }
            ExprKind::ComptimeIf(c, then_b, els) => {
                let cv = self.eval_in_module(c, fx.module, None);
                match cv {
                    Ok(Value::Bool(true)) => {
                        self.ct_choice.insert(e.id, Some(0));
                        self.check_block(fx, then_b, exp)
                    }
                    Ok(Value::Bool(false)) => {
                        self.ct_choice.insert(e.id, Some(1));
                        match els {
                            Some(x) => self.check_expr(fx, x, exp),
                            None => Ty::Unit,
                        }
                    }
                    Ok(_) => {
                        self.err("E0403", "comptime if の条件は bool でなければなりません", &c.span);
                        Ty::Error
                    }
                    Err(er) => {
                        self.diags.err(&er.code, &er.msg, er.span);
                        Ty::Error
                    }
                }
            }
            ExprKind::ComptimeMatch(scrut, arms) => {
                let sv = match self.eval_in_module(scrut, fx.module, None) {
                    Ok(v) => v,
                    Err(er) => {
                        self.diags.err(&er.code, &er.msg, er.span);
                        return Ty::Error;
                    }
                };
                let mut chosen = None;
                for (i, arm) in arms.iter().enumerate() {
                    if chosen.is_some() {
                        break;
                    }
                    for p in &arm.pats {
                        if self.ct_pat_matches(fx.module, p, &sv) {
                            chosen = Some(i);
                            break;
                        }
                    }
                }
                self.ct_choice.insert(e.id, chosen);
                match chosen {
                    Some(i) => self.check_expr(fx, &arms[i].body, exp),
                    None => Ty::Unit,
                }
            }
        }
    }

    fn ct_pat_matches(&mut self, m: usize, p: &Pattern, v: &Value) -> bool {
        match &p.kind {
            PatKind::Wild | PatKind::Ident(_) => true,
            PatKind::Lit(e) => match self.eval_in_module(e, m, None) {
                Ok(x) => match (&x, v) {
                    (Value::Cpu(a), Value::Cpu(b)) => a == b,
                    _ => value_int(&x).is_some() && value_int(&x) == value_int(v),
                },
                Err(_) => false,
            },
            PatKind::Range(a, b, incl) => {
                let x = self.eval_in_module(a, m, None).ok().and_then(|v| value_int(&v));
                let y = self.eval_in_module(b, m, None).ok().and_then(|v| value_int(&v));
                match (x, y, value_int(v)) {
                    (Some(x), Some(y), Some(z)) => {
                        if *incl {
                            z >= x && z <= y
                        } else {
                            z >= x && z < y
                        }
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn find_loop(&mut self, fx: &mut FnCx, label: &Option<String>, sp: &Span, is_break: bool) -> Option<usize> {
        if fx.loops.is_empty() {
            self.err("E0704", if is_break { "ループの外で break を使っています" } else { "ループの外で continue を使っています" }, sp);
            return None;
        }
        match label {
            None => {
                let i = fx.loops.len() - 1;
                if is_break {
                    fx.loops[i].has_break = true;
                }
                Some(i)
            }
            Some(l) => {
                for i in (0..fx.loops.len()).rev() {
                    if fx.loops[i].label.as_ref() == Some(l) {
                        if is_break {
                            fx.loops[i].has_break = true;
                        }
                        return Some(i);
                    }
                }
                self.err("E0704", &format!("ラベル '{} のループがありません", l), sp);
                None
            }
        }
    }

    /// ローカル変数のアドレスを外に持ち出していないか（4.9節）
    fn check_escape(&mut self, fx: &FnCx, e: &Expr) {
        if let ExprKind::Unary(UnOp::Addr, x) = &e.kind {
            if self.base_local(x).is_some() {
                let _ = fx;
                self.err("E0506", "ローカル変数のアドレスを関数の外に持ち出しています", &e.span);
            }
        }
    }

    fn check_unary(&mut self, fx: &mut FnCx, e: &Expr, op: UnOp, x: &Expr, exp: Option<&Ty>) -> Ty {
        match op {
            UnOp::Addr => {
                let t = match &x.kind {
                    ExprKind::Path(_) | ExprKind::Index(_, _) | ExprKind::Field(_, _) => {
                        let t = self.check_expr(fx, x, None);
                        t
                    }
                    _ => {
                        self.err("E0401", "& はアドレスを持つもの（変数・配列・定数の表）にだけ使えます", &e.span);
                        return Ty::Error;
                    }
                };
                if let Some(l) = self.base_local(x) {
                    self.funcs[fx.fid].locals[l].addr_taken = true;
                }
                if let Some(g) = self.base_global(x) {
                    let gi = self.globals[g].clone();
                    if gi.kind == GKind::Const {
                        let scalar = !matches!(gi.ty, Ty::Array(_, _) | Ty::Struct(_) | Ty::Str(_, _));
                        if scalar {
                            self.err("E0504", &format!("数値の定数 {} はアドレスを持ちません", gi.name), &e.span);
                        }
                    }
                }
                // 定数の畳み込みの値はアドレスには使わない
                match t {
                    Ty::Array(et, _) => match *et {
                        Ty::Array(inner, _) => Ty::Ptr(inner),
                        other => Ty::Ptr(Box::new(other)),
                    },
                    Ty::Str(_, _) => Ty::Ptr(Box::new(Ty::Int(IntK::U8))),
                    Ty::Error => Ty::Error,
                    other => Ty::Ptr(Box::new(other)),
                }
            }
            UnOp::Neg => {
                let t = self.check_expr(fx, x, exp);
                let r = match &t {
                    Ty::Int(k) if k.signed() => t.clone(),
                    Ty::IntLit | Ty::CInt | Ty::CReal | Ty::Error => t.clone(),
                    _ => {
                        self.err("E0401", &format!("{} に - は使えません（符号付きの整数だけ）", self.tyname(&t)), &e.span);
                        Ty::Error
                    }
                };
                if let Some(v) = self.econst.get(&x.id).cloned() {
                    match self.ct_unop(UnOp::Neg, v, &e.span) {
                        Ok(nv) => {
                            self.econst.insert(e.id, nv);
                        }
                        Err(er) => self.diags.err(&er.code, &er.msg, er.span),
                    }
                }
                r
            }
            UnOp::Not => {
                let t = self.check_expr(fx, x, exp);
                let r = match &t {
                    Ty::Bool | Ty::Int(_) | Ty::Flags(_) | Ty::IntLit | Ty::Error => t.clone(),
                    _ => {
                        self.err("E0401", &format!("{} に ! は使えません", self.tyname(&t)), &e.span);
                        Ty::Error
                    }
                };
                if let Some(v) = self.econst.get(&x.id).cloned() {
                    if let Ok(nv) = self.ct_unop(UnOp::Not, v, &e.span) {
                        self.econst.insert(e.id, nv);
                    }
                }
                r
            }
        }
    }

    fn arith_kind(&self, t: &Ty) -> Option<Ty> {
        match t {
            Ty::Int(_) | Ty::IntLit | Ty::CInt | Ty::Bcd(_) => Some(t.clone()),
            Ty::Range(r) => Some(Ty::Int(IntK::unsigned_of_size(self.types.ranges[*r].base_size()))),
            _ => None,
        }
    }

    fn check_binary(&mut self, fx: &mut FnCx, e: &Expr, op: BinOp, a: &Expr, b: &Expr, exp: Option<&Ty>) -> Ty {
        if op == BinOp::LAnd || op == BinOp::LOr {
            self.check_cond(fx, a);
            let before = fx.assigned.clone();
            self.check_cond(fx, b);
            fx.assigned = before;
            if let (Some(Value::Bool(x)), Some(Value::Bool(y))) = (self.econst.get(&a.id).cloned(), self.econst.get(&b.id).cloned()) {
                let r = if op == BinOp::LAnd { x && y } else { x || y };
                self.econst.insert(e.id, Value::Bool(r));
            }
            return Ty::Bool;
        }
        let is_cmp = op.is_cmp();
        let is_shift = op == BinOp::Shl || op == BinOp::Shr;
        let sub_exp: Option<Ty> = if is_cmp {
            None
        } else {
            match exp {
                Some(t) if self.arith_kind(t).is_some() || matches!(t, Ty::Flags(_)) => Some(t.clone()),
                _ => None,
            }
        };
        let mut lt = self.check_expr(fx, a, sub_exp.as_ref());
        let mut rt = if is_shift {
            self.check_expr(fx, b, None)
        } else {
            let want = if lt == Ty::IntLit || lt == Ty::Error { sub_exp.clone() } else { Some(lt.clone()) };
            // u8 ± i8 のときは右の型をそのまま
            self.check_expr(fx, b, want.as_ref())
        };
        if !is_shift {
            if lt == Ty::IntLit && rt != Ty::IntLit && rt != Ty::Error {
                if let Some(k) = self.arith_kind(&rt).or_else(|| if matches!(rt, Ty::Char(_) | Ty::Enum(_) | Ty::Flags(_)) { Some(rt.clone()) } else { None }) {
                    let target = if is_cmp { rt.clone() } else { k };
                    if self.coerce(a, &Ty::IntLit, &target) {
                        lt = target;
                    } else {
                        self.mismatch(&a.span, &rt, &lt);
                    }
                }
            } else if rt == Ty::IntLit && lt != Ty::IntLit && lt != Ty::Error {
                let target = if is_cmp { lt.clone() } else { self.arith_kind(&lt).unwrap_or(lt.clone()) };
                if matches!(target, Ty::Int(_) | Ty::Bcd(_) | Ty::Range(_) | Ty::CInt | Ty::Char(_) | Ty::Enum(_)) {
                    if matches!(target, Ty::Char(_) | Ty::Enum(_)) {
                        // 文字と整数リテラルは比べられない
                        self.mismatch(&b.span, &lt, &rt);
                    } else if self.coerce(b, &Ty::IntLit, &target) {
                        rt = target;
                    }
                }
            }
        }
        let fold = |ctx: &mut Ctx, e: &Expr| {
            if let (Some(x), Some(y)) = (ctx.econst.get(&a.id).cloned(), ctx.econst.get(&b.id).cloned()) {
                match ctx.ct_binop(op, x, y, &e.span) {
                    Ok(v) => {
                        ctx.econst.insert(e.id, v);
                    }
                    Err(er) => ctx.diags.err(&er.code, &er.msg, er.span),
                }
            }
        };
        if lt == Ty::Error || rt == Ty::Error {
            return if is_cmp { Ty::Bool } else { Ty::Error };
        }
        if is_cmp {
            let ok = match (&lt, &rt) {
                (x, y) if x == y => {
                    if matches!(x, Ty::Opt(_) | Ty::Struct(_) | Ty::Array(_, _) | Ty::Str(_, _)) {
                        self.err("E0401", &format!("{} は比べられません", self.tyname(x)), &e.span);
                    }
                    true
                }
                (Ty::Range(_), Ty::Int(IntK::U8)) | (Ty::Int(IntK::U8), Ty::Range(_)) => true,
                (Ty::IntLit, Ty::IntLit) => true,
                (Ty::Int(x), Ty::Int(y)) if x.signed() != y.signed() => {
                    self.diags.error(
                        Diag::error("E0402", &format!("符号付きと符号なしを比べています（{} と {}）", x.name(), y.name()), e.span.clone())
                            .help("as で変換してください"),
                    );
                    true
                }
                (Ty::CInt, Ty::Int(_)) | (Ty::Int(_), Ty::CInt) => true,
                _ => false,
            };
            if !ok {
                self.mismatch(&b.span, &lt, &rt);
            }
            if (op != BinOp::Eq && op != BinOp::Ne) && matches!(lt, Ty::Bool | Ty::Ptr(_) | Ty::FnSet(_) | Ty::Flags(_)) {
                self.err("E0401", &format!("{} は大小を比べられません（== と != だけ）", self.tyname(&lt)), &e.span);
            }
            if lt == Ty::IntLit && rt == Ty::IntLit {
                self.default_type(a, &lt, false);
                self.default_type(b, &rt, false);
            }
            fold(&mut *self, e);
            return Ty::Bool;
        }
        // 算術・ビット演算
        let result: Ty = match op {
            BinOp::Shl | BinOp::Shr => {
                if !self.econst.contains_key(&b.id) {
                    self.diags.error(
                        Diag::error("E0409", "<< と >> の右側には定数だけを書けます", b.span.clone()).help("回数が変数のシフトは a.shl(n)・a.shr(n) と書きます（5.3節）"),
                    );
                } else if rt == Ty::IntLit {
                    self.default_type(b, &rt, false);
                }
                match &lt {
                    Ty::Int(_) | Ty::IntLit | Ty::CInt => lt.clone(),
                    Ty::Range(_) => self.arith_kind(&lt).unwrap_or(Ty::Error),
                    _ => {
                        self.err("E0401", &format!("{} はシフトできません", self.tyname(&lt)), &e.span);
                        Ty::Error
                    }
                }
            }
            BinOp::And | BinOp::Or | BinOp::Xor => match (&lt, &rt) {
                (Ty::Flags(x), Ty::Flags(y)) if x == y => lt.clone(),
                (Ty::Bool, Ty::Bool) => Ty::Bool,
                _ => {
                    let la = self.arith_kind(&lt);
                    let ra = self.arith_kind(&rt);
                    match (la, ra) {
                        (Some(x), Some(y)) if x == y && !matches!(x, Ty::Bcd(_)) => x,
                        _ => {
                            self.mismatch(&b.span, &lt, &rt);
                            Ty::Error
                        }
                    }
                }
            },
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                let la = self.arith_kind(&lt);
                let ra = self.arith_kind(&rt);
                let t = match (la, ra) {
                    (Some(x), Some(y)) if x == y => x,
                    (Some(Ty::Int(IntK::U8)), Some(Ty::Int(IntK::I8))) if op == BinOp::Add || op == BinOp::Sub => Ty::Int(IntK::U8),
                    (Some(Ty::Bcd(n)), Some(Ty::IntLit)) => Ty::Bcd(n),
                    _ => {
                        if let (Ty::Int(x), Ty::Int(y)) = (&lt, &rt) {
                            if x.signed() != y.signed() {
                                self.diags.error(Diag::error("E0402", &format!("符号付きと符号なしを混ぜて計算しています（{} と {}）", x.name(), y.name()), e.span.clone()).help("as で変換してください"));
                            } else {
                                self.diags.error(
                                    Diag::error("E0401", &format!("{} と {} を混ぜて計算できません", x.name(), y.name()), e.span.clone())
                                        .note("暗黙の変換はしません（3.1節）")
                                        .help("as で型をそろえてください"),
                                );
                            }
                        } else {
                            self.mismatch(&b.span, &lt, &rt);
                        }
                        Ty::Error
                    }
                };
                if let Ty::Bcd(_) = t {
                    if op != BinOp::Add && op != BinOp::Sub {
                        self.err("E0401", "BCD に使える演算は + と - と比較だけです（5.5節）", &e.span);
                    }
                }
                let lc = self.econst.contains_key(&a.id);
                let rc = self.econst.contains_key(&b.id);
                match op {
                    BinOp::Mul => {
                        if !lc && !rc {
                            self.diags.error(
                                Diag::error("E0408", "変数同士の掛け算は演算子では書けません", e.span.clone()).help("core::mul8(a, b) などを使ってください（5.6節）"),
                            );
                        }
                    }
                    BinOp::Div | BinOp::Rem => {
                        if !(lc && rc) {
                            let ok = rc && self.econst.get(&b.id).and_then(value_int).map(is_pow2).unwrap_or(false);
                            if !ok {
                                self.diags.error(
                                    Diag::error("E0408", "/ と % は 2 のべき乗の定数だけに使えます", b.span.clone()).help("core::divmod8(a, b) を使ってください（5.6節）"),
                                );
                            }
                        }
                    }
                    _ => {}
                }
                t
            }
            _ => Ty::Error,
        };
        if result == Ty::IntLit {
            // 両方がリテラル
        }
        fold(&mut *self, e);
        result
    }

    fn check_assign(&mut self, fx: &mut FnCx, e: &Expr, op: Option<BinOp>, lhs: &Expr, rhs: &Expr) {
        let lt = self.check_place(fx, lhs);
        match op {
            None => {
                let rt = self.check_expr(fx, rhs, Some(&lt));
                self.expect_ty(rhs, &rt, &lt);
                // ローカル変数のアドレスを static に保存していないか
                if self.base_global(lhs).is_some() {
                    self.check_escape(fx, rhs);
                }
            }
            Some(o) => {
                // 読んでいる
                if let Some(l) = self.base_local(lhs) {
                    self.funcs[fx.fid].locals[l].read = true;
                }
                if let Some(g) = self.base_global(lhs) {
                    if let GKind::Io(IoDir::Out) = self.globals[g].kind {
                        self.err("E0901", &format!("{} は io(out) なので読めません（{}= は読み書きの両方をします）", self.globals[g].name, o.text()), &lhs.span);
                    }
                }
                let is_shift = o == BinOp::Shl || o == BinOp::Shr;
                let want = if is_shift { None } else { self.arith_kind(&lt).or(Some(lt.clone())) };
                let rt = self.check_expr(fx, rhs, want.as_ref());
                if is_shift {
                    if !self.econst.contains_key(&rhs.id) {
                        self.err("E0409", "<<= と >>= の右側には定数だけを書けます", &rhs.span);
                    } else if rt == Ty::IntLit {
                        self.default_type(rhs, &rt, false);
                    }
                } else {
                    let ok = match (&lt, &rt) {
                        (Ty::Int(IntK::U8), Ty::Int(IntK::I8)) => o == BinOp::Add || o == BinOp::Sub,
                        (Ty::Flags(x), Ty::Flags(y)) => x == y && matches!(o, BinOp::And | BinOp::Or | BinOp::Xor),
                        (Ty::Bcd(_), _) => {
                            if o != BinOp::Add && o != BinOp::Sub {
                                self.err("E0401", "BCD に使える演算は + と - だけです（5.5節）", &e.span);
                            }
                            self.coerce(rhs, &rt, &lt)
                        }
                        _ => {
                            let w = want.clone().unwrap_or(Ty::Error);
                            self.coerce(rhs, &rt, &w)
                        }
                    };
                    if !ok {
                        self.mismatch(&rhs.span, &lt, &rt);
                    }
                    match o {
                        BinOp::Mul => {
                            if !self.econst.contains_key(&rhs.id) {
                                self.err("E0408", "*= の右側には定数だけを書けます（変数同士は core::mul8）", &rhs.span);
                            }
                        }
                        BinOp::Div | BinOp::Rem => {
                            let ok = self.econst.get(&rhs.id).and_then(value_int).map(is_pow2).unwrap_or(false);
                            if !ok {
                                self.err("E0408", "/= と %= は 2 のべき乗の定数だけに使えます", &rhs.span);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        // shared の書き込みの分類（10.4節）
        if let Some(g) = self.base_global(lhs) {
            if let GKind::Shared(_) | GKind::Static = self.globals[g].kind {
                let lsize = self.types.size_of(&lt);
                let single = lsize == 1
                    && match op {
                        None => !self.mentions_global(rhs, g),
                        Some(BinOp::Add) | Some(BinOp::Sub) => self.econst.get(&rhs.id).and_then(value_int) == Some(1),
                        _ => false,
                    };
                self.note_access(fx, g, true, single, &lhs.span);
            }
        }
    }

    fn check_cast(&mut self, fx: &mut FnCx, e: &Expr, x: &Expr, t: &Type) -> Ty {
        let to = self.resolve_type(fx.module, t);
        let from = self.check_expr(fx, x, None);
        let ok = match (&from, &to) {
            (_, Ty::Error) | (Ty::Error, _) => true,
            (a, b) if a == b => true,
            (Ty::IntLit, Ty::Int(_)) | (Ty::IntLit, Ty::Char(_)) | (Ty::IntLit, Ty::Flags(_)) | (Ty::IntLit, Ty::CInt) | (Ty::IntLit, Ty::CReal) => true,
            (Ty::CInt, _) | (Ty::CReal, _) => true,
            (Ty::Int(_), Ty::Int(_)) => true,
            (Ty::Int(_), Ty::Char(_)) | (Ty::Char(_), Ty::Int(_)) | (Ty::Char(_), Ty::Char(_)) => true,
            (Ty::Range(_), Ty::Int(_)) | (Ty::Enum(_), Ty::Int(_)) => true,
            (Ty::Flags(_), Ty::Int(_)) | (Ty::Int(_), Ty::Flags(_)) => true,
            (Ty::Bool, Ty::Int(_)) => true,
            (Ty::Ptr(_), Ty::Int(IntK::U16)) | (Ty::Int(IntK::U16), Ty::Ptr(_)) | (Ty::Ptr(_), Ty::Ptr(_)) => true,
            (Ty::FnSet(_), Ty::Int(IntK::U8)) => true,
            (Ty::Int(_), Ty::Range(_)) | (Ty::Int(_), Ty::Enum(_)) | (Ty::IntLit, Ty::Range(_)) | (Ty::IntLit, Ty::Enum(_)) => {
                self.diags.error(
                    Diag::error("E0410", "整数から範囲型・列挙型へは as で変換できません", e.span.clone()).help("::check(n) か ::wrap(n) を使ってください"),
                );
                return to.clone();
            }
            (Ty::Bcd(_), Ty::Int(_)) | (Ty::Int(_), Ty::Bcd(_)) | (Ty::IntLit, Ty::Bcd(_)) => {
                self.diags.error(
                    Diag::error("E0411", "BCD と整数の間は as で変換できません", e.span.clone()).help("bcd::to_u16(x)・bcd::from_u16(n) などを使ってください（20.3節）"),
                );
                return to.clone();
            }
            _ => false,
        };
        if !ok {
            self.err("E0419", &format!("{} を {} に変換できません", self.tyname(&from), self.tyname(&to)), &e.span);
            return Ty::Error;
        }
        if from == Ty::IntLit {
            // リテラルは int として扱い、範囲を検査する
        }
        if let Some(v) = self.econst.get(&x.id).cloned() {
            match self.cast_value(v, &to, &e.span) {
                Ok(nv) => {
                    self.econst.insert(e.id, nv);
                }
                Err(er) => self.diags.err(&er.code, &er.msg, er.span),
            }
        }
        to
    }

    fn check_struct_lit(&mut self, fx: &mut FnCx, e: &Expr, path: &Path, fields: &[(String, Expr, Span)], base: &Option<Box<Expr>>) -> Ty {
        let t = match self.resolve_type_segs(fx.module, &path.segs, &path.span) {
            Some(t) => t,
            None => return Ty::Error,
        };
        let sid = match t {
            Ty::Struct(s) => s,
            _ => {
                self.err("E0401", &format!("{} は構造体ではありません", path.text()), &path.span);
                return Ty::Error;
            }
        };
        let def = self.types.structs[sid].clone();
        let mut seen = HashSet::new();
        let mut all_const = true;
        for (fname, fe, fsp) in fields {
            match def.fields.iter().find(|f| &f.0 == fname) {
                Some((_, ft)) => {
                    let ft = ft.clone();
                    let got = self.check_expr(fx, fe, Some(&ft));
                    self.expect_ty(fe, &got, &ft);
                    if !self.econst.contains_key(&fe.id) {
                        all_const = false;
                    }
                }
                None => {
                    self.err("E0416", &format!("構造体 {} にフィールド {} はありません", def.name, fname), fsp);
                    all_const = false;
                }
            }
            if !seen.insert(fname.clone()) {
                self.err("E0206", &format!("フィールド {} を 2 回書いています", fname), fsp);
            }
        }
        if let Some(b) = base {
            let bt = self.check_expr(fx, b, Some(&Ty::Struct(sid)));
            if bt != Ty::Struct(sid) && bt != Ty::Error {
                self.mismatch(&b.span, &Ty::Struct(sid), &bt);
            }
            if !self.econst.contains_key(&b.id) {
                self.err("E1001", "..の後ろには定数を書きます（4.5節）", &b.span);
                all_const = false;
            }
        } else {
            for (fname, _) in &def.fields {
                if !seen.contains(fname) {
                    self.err("E0507", &format!("構造体 {} の値にフィールド {} がありません", def.name, fname), &e.span);
                }
            }
        }
        if all_const {
            if let Ok(v) = self.eval_in_module(e, fx.module, Some(&Ty::Struct(sid))) {
                self.econst.insert(e.id, v);
            }
        }
        Ty::Struct(sid)
    }

    fn check_if(&mut self, fx: &mut FnCx, e: &Expr, c: &Expr, then_b: &Block, els: &Option<Box<Expr>>, exp: Option<&Ty>) -> Ty {
        self.check_cond(fx, c);
        let before = fx.assigned.clone();
        let tt = self.check_block(fx, then_b, exp);
        let after_then = fx.assigned.clone();
        fx.assigned = before.clone();
        match els {
            Some(x) => {
                let want = match exp {
                    Some(t) => Some(t.clone()),
                    None => {
                        if tt != Ty::Never && tt != Ty::IntLit {
                            Some(tt.clone())
                        } else {
                            None
                        }
                    }
                };
                let et = self.check_expr(fx, x, want.as_ref());
                let after_else = fx.assigned.clone();
                fx.assigned = intersect(after_then, after_else);
                self.unify_branches(e, then_b.tail.as_deref(), &tt, Some(x.as_ref()), &et)
            }
            None => {
                fx.assigned = intersect(after_then, before);
                if tt != Ty::Unit && tt != Ty::Never && tt != Ty::Error {
                    if let Some(t) = exp {
                        if *t != Ty::Unit {
                            self.diags.error(Diag::error("E0405", "値を返す if には else が必要です", e.span.clone()));
                        }
                    }
                }
                Ty::Unit
            }
        }
    }

    fn unify_branches(&mut self, e: &Expr, a: Option<&Expr>, at: &Ty, b: Option<&Expr>, bt: &Ty) -> Ty {
        if at == bt {
            return at.clone();
        }
        if *at == Ty::Never {
            return bt.clone();
        }
        if *bt == Ty::Never {
            return at.clone();
        }
        if *at == Ty::IntLit {
            if let Some(x) = a {
                if self.coerce(x, at, bt) {
                    return bt.clone();
                }
            }
        }
        if *bt == Ty::IntLit {
            if let Some(x) = b {
                if self.coerce(x, bt, at) {
                    return at.clone();
                }
            }
        }
        if let (Ty::Opt(x), Ty::Opt(y)) = (at, bt) {
            if **x == Ty::Never {
                return bt.clone();
            }
            if **y == Ty::Never {
                return at.clone();
            }
        }
        if *at == Ty::Error || *bt == Ty::Error {
            return Ty::Error;
        }
        self.diags.error(Diag::error(
            "E0401",
            &format!("分かれた先の値の型が違います（{} と {}）", self.tyname(at), self.tyname(bt)),
            e.span.clone(),
        ));
        Ty::Error
    }

    fn check_if_let(&mut self, fx: &mut FnCx, pat: &Pattern, scrut: &Expr, then_b: &Block, els: &Option<Box<Expr>>, exp: Option<&Ty>) -> Ty {
        let st = self.check_expr(fx, scrut, None);
        let before = fx.assigned.clone();
        fx.scopes.push(HashMap::new());
        self.check_pattern(fx, pat, &st);
        let tt = self.check_block(fx, then_b, exp);
        fx.scopes.pop();
        let after_then = fx.assigned.clone();
        fx.assigned = before.clone();
        match els {
            Some(x) => {
                let want = match exp {
                    Some(t) => Some(t.clone()),
                    None => {
                        if tt != Ty::Never && tt != Ty::IntLit {
                            Some(tt.clone())
                        } else {
                            None
                        }
                    }
                };
                let et = self.check_expr(fx, x, want.as_ref());
                let after_else = fx.assigned.clone();
                fx.assigned = intersect(after_then, after_else);
                self.unify_branches(scrut, then_b.tail.as_deref(), &tt, Some(x.as_ref()), &et)
            }
            None => {
                fx.assigned = intersect(after_then, before);
                Ty::Unit
            }
        }
    }

    /// パターンの検査と束縛（値は econst / res に記録する）
    fn check_pattern(&mut self, fx: &mut FnCx, p: &Pattern, st: &Ty) {
        match &p.kind {
            PatKind::Wild => {}
            PatKind::Ident(n) => {
                // 定数・列挙型の値か、束縛か
                let m = fx.module;
                let is_local = self.lookup_scope(fx, n).is_some();
                let r = if is_local { PathRes::NotFound(String::new()) } else { self.resolve_value_path(m, &[n.clone()], &p.span) };
                match r {
                    PathRes::Global(g) if self.globals[g].kind == GKind::Const => {
                        self.ensure_global(g);
                        if let Some(v) = self.globals[g].value.clone() {
                            let gt = self.globals[g].ty.clone();
                            if &gt != st && !matches!((st, &gt), (Ty::Range(_), Ty::Int(IntK::U8))) {
                                self.mismatch(&p.span, st, &gt);
                            }
                            self.res.insert(p.id, Res::Value(v));
                        }
                    }
                    PathRes::Value(v) => {
                        self.res.insert(p.id, Res::Value(v));
                    }
                    _ => {
                        let id = p.id;
                        let l = self.add_local(fx, n, st.clone(), false, &p.span, id);
                        self.funcs[fx.fid].locals[l].written = true;
                        self.set_assigned(fx, l);
                        if n.starts_with('_') {
                            self.funcs[fx.fid].locals[l].read = true;
                        }
                    }
                }
            }
            PatKind::Lit(e) => {
                let want = match st {
                    Ty::Range(r) => Ty::Int(IntK::unsigned_of_size(self.types.ranges[*r].base_size())),
                    t => t.clone(),
                };
                let t = self.check_expr(fx, e, Some(&want));
                if !self.econst.contains_key(&e.id) {
                    self.err("E1001", "パターンには定数を書きます", &e.span);
                } else if t != *st && !(matches!(st, Ty::Range(_)) && t == want) {
                    self.expect_ty(e, &t, st);
                }
            }
            PatKind::Range(a, b, _) => {
                let want = match st {
                    Ty::Range(r) => Ty::Int(IntK::unsigned_of_size(self.types.ranges[*r].base_size())),
                    t => t.clone(),
                };
                for x in [a, b] {
                    let t = self.check_expr(fx, x, Some(&want));
                    if !self.econst.contains_key(&x.id) {
                        self.err("E1001", "パターンには定数を書きます", &x.span);
                    } else if t != want && t != *st {
                        self.expect_ty(x, &t, &want);
                    }
                }
            }
            PatKind::Some(inner) => match st {
                Ty::Opt(t) => {
                    let tt = (**t).clone();
                    self.check_pattern(fx, inner, &tt);
                }
                Ty::Error => {}
                _ => self.err("E0401", &format!("Some のパターンは Option の値にだけ使えます（{} です）", self.tyname(st)), &p.span),
            },
            PatKind::None => match st {
                Ty::Opt(_) | Ty::Error => {}
                _ => self.err("E0401", &format!("None のパターンは Option の値にだけ使えます（{} です）", self.tyname(st)), &p.span),
            },
        }
    }

    /// パターンが表す値（網羅性の検査用）。None は「すべて」
    fn pattern_values(&self, p: &Pattern) -> Option<Vec<i128>> {
        match &p.kind {
            PatKind::Wild => None,
            PatKind::Ident(_) => match self.res.get(&p.id) {
                Some(Res::Value(v)) => value_int(v).map(|x| vec![x]),
                _ => None,
            },
            PatKind::Lit(e) => Some(self.econst.get(&e.id).and_then(value_int).map(|x| vec![x]).unwrap_or_default()),
            PatKind::Range(a, b, incl) => {
                let x = self.econst.get(&a.id).and_then(value_int);
                let y = self.econst.get(&b.id).and_then(value_int);
                match (x, y) {
                    (Some(x), Some(y)) => {
                        let hi = if *incl { y } else { y - 1 };
                        if hi - x > 70000 {
                            return Some(Vec::new());
                        }
                        Some((x..=hi).collect())
                    }
                    _ => Some(Vec::new()),
                }
            }
            _ => Some(Vec::new()),
        }
    }

    fn check_match(&mut self, fx: &mut FnCx, e: &Expr, scrut: &Expr, arms: &[Arm], exp: Option<&Ty>) -> Ty {
        let st0 = self.check_expr(fx, scrut, None);
        let st = self.default_type(scrut, &st0, false);
        let before = fx.assigned.clone();
        let mut result: Option<Ty> = exp.cloned();
        let mut res_from: Option<Ty> = None;
        let mut merged: Option<Option<HashSet<usize>>> = None;
        // 網羅性
        let mut covered: HashSet<i128> = HashSet::new();
        let mut all = false;
        let mut opt_none = false;
        let mut opt_some_all = false;
        let mut opt_some_vals: HashSet<i128> = HashSet::new();
        for arm in arms {
            fx.assigned = before.clone();
            fx.scopes.push(HashMap::new());
            for p in &arm.pats {
                self.check_pattern(fx, p, &st);
                match &p.kind {
                    PatKind::None => opt_none = true,
                    PatKind::Some(inner) => match self.pattern_values(inner) {
                        None => opt_some_all = true,
                        Some(v) => {
                            for x in v {
                                opt_some_vals.insert(x);
                            }
                        }
                    },
                    _ => match self.pattern_values(p) {
                        None => all = true,
                        Some(v) => {
                            for x in v {
                                covered.insert(x);
                            }
                        }
                    },
                }
            }
            if arm.pats.len() > 1 {
                for p in &arm.pats {
                    if let PatKind::Ident(n) = &p.kind {
                        if !matches!(self.res.get(&p.id), Some(Res::Value(_))) {
                            self.err("E0401", &format!("| でまとめたパターンでは名前 {} を束縛できません", n), &p.span);
                        }
                    }
                }
            }
            let want = result.clone();
            let t = self.check_expr(fx, &arm.body, want.as_ref());
            fx.scopes.pop();
            match result.clone() {
                None => {
                    if t != Ty::Never {
                        let d = self.default_type(&arm.body, &t, false);
                        result = Some(d.clone());
                        res_from = Some(d);
                    }
                }
                Some(r) => {
                    if t != Ty::Never && t != r {
                        if !self.coerce(&arm.body, &t, &r) {
                            if let (Ty::Opt(a), Ty::Opt(b)) = (&r, &t) {
                                if **a == Ty::Never {
                                    result = Some(t.clone());
                                    continue;
                                }
                                if **b == Ty::Never {
                                    continue;
                                }
                            }
                            if r != Ty::Unit || exp.is_some() {
                                self.diags.error(Diag::error(
                                    "E0401",
                                    &format!("match の腕の値の型が違います（{} と {}）", self.tyname(&r), self.tyname(&t)),
                                    arm.body.span.clone(),
                                ));
                            }
                        }
                    }
                }
            }
            let a = fx.assigned.clone();
            merged = Some(match merged {
                None => a,
                Some(m) => intersect(m, a),
            });
        }
        let _ = res_from;
        fx.assigned = merged.unwrap_or(before);
        // 網羅性の判定（6.6節）
        if !all {
            match &st {
                Ty::Opt(inner) => {
                    let some_ok = opt_some_all
                        || match self.types.value_set(inner) {
                            Some(vs) => vs.iter().all(|v| opt_some_vals.contains(v)),
                            None => false,
                        };
                    if !opt_none || !some_ok {
                        let mut miss = Vec::new();
                        if !opt_none {
                            miss.push("None".to_string());
                        }
                        if !some_ok {
                            miss.push("Some(_)".to_string());
                        }
                        self.diags.error(
                            Diag::error("E0701", &format!("match の場合が抜けています（{}）", miss.join("、")), e.span.clone()),
                        );
                    }
                }
                t => match self.types.value_set(t) {
                    Some(vs) => {
                        let missing: Vec<i128> = vs.iter().filter(|v| !covered.contains(*v)).cloned().collect();
                        if !missing.is_empty() {
                            let names: Vec<String> = missing.iter().take(4).map(|v| self.value_name(t, *v)).collect();
                            let more = if missing.len() > 4 { "…" } else { "" };
                            let mut d = Diag::error("E0701", &format!("match の場合が抜けています（{}{}）", names.join("、"), more), e.span.clone());
                            if missing.len() > 4 {
                                d = d.help("_ => で残りをまとめてください");
                            }
                            self.diags.error(d);
                        }
                    }
                    None => {
                        if *t != Ty::Error {
                            self.diags.error(
                                Diag::error("E0701", &format!("{} の match には _ => が必要です", self.tyname(t)), e.span.clone()),
                            );
                        }
                    }
                },
            }
        }
        match result {
            Some(t) => t,
            None => Ty::Never,
        }
    }

    fn value_name(&self, t: &Ty, v: i128) -> String {
        match t {
            Ty::Enum(e) => {
                let d = &self.types.enums[*e];
                match d.variants.iter().find(|x| x.1 == v) {
                    Some(x) => format!("{}::{}", d.name, x.0),
                    None => format!("{}", v),
                }
            }
            Ty::Bool => (if v != 0 { "true" } else { "false" }).to_string(),
            Ty::Char(_) => {
                if (0x20..0x7F).contains(&v) {
                    format!("'{}'", v as u8 as char)
                } else {
                    format!("'\\x{:02X}'", v)
                }
            }
            _ => format!("{}", v),
        }
    }

    /// for で回すものの要素の型
    fn check_for_iter(&mut self, fx: &mut FnCx, iter: &Expr) -> Ty {
        if let ExprKind::MethodCall(base, name, args) = &iter.kind {
            if name == "rev" && args.is_empty() {
                let t = self.check_for_iter(fx, base);
                self.ety.insert(iter.id, Ty::RangeIter(Box::new(t.clone())));
                if !matches!(base.kind, ExprKind::Range(_, _, _)) {
                    let bt = self.ety.get(&base.id).cloned();
                    if !matches!(bt, Some(Ty::RangeIter(_))) {
                        self.err(crate::diag::E_UNIMPL, "未実装：rev() は範囲（a..b）にだけ使えます", &iter.span);
                    }
                }
                return t;
            }
        }
        let t = self.check_expr(fx, iter, None);
        match t {
            Ty::RangeIter(et) => *et,
            Ty::TypeName(inner) => match *inner {
                Ty::Range(r) => Ty::Range(r),
                Ty::Enum(x) => Ty::Enum(x),
                other => {
                    self.err("E0401", &format!("型 {} は for で回せません", self.tyname(&other)), &iter.span);
                    Ty::Error
                }
            },
            Ty::Array(et, _) => *et,
            Ty::Str(_, enc) => Ty::Char(enc),
            Ty::Error => Ty::Error,
            other => {
                self.err("E0401", &format!("{} は for で回せません", self.tyname(&other)), &iter.span);
                Ty::Error
            }
        }
    }

    // ---- 呼び出し ----

    fn check_args(&mut self, fx: &mut FnCx, args: &[Arg], params: &[(String, Ty)], fname: &str, sp: &Span) {
        if args.len() != params.len() {
            self.err("E0418", &format!("{} の引数の数は {} ですが、{} 個渡しています", fname, params.len(), args.len()), sp);
        }
        for (i, a) in args.iter().enumerate() {
            if a.name.is_some() {
                self.err("E0102", "名前付きの引数は include_bin の offset だけに使えます", &a.value.span);
            }
            match params.get(i) {
                Some((_, pt)) => {
                    let pt = pt.clone();
                    let t = self.check_expr(fx, &a.value, Some(&pt));
                    self.expect_ty(&a.value, &t, &pt);
                }
                None => {
                    self.check_expr(fx, &a.value, None);
                }
            }
        }
    }

    fn add_edge(&mut self, fx: &FnCx, to: usize, sp: &Span) {
        self.call_edges.push(CallEdge { from: fx.fid, to, span: sp.clone(), in_atomic: fx.atomic > 0 || fx.timed > 0 });
    }

    fn check_call(&mut self, fx: &mut FnCx, e: &Expr, callee: &Expr, args: &[Arg], exp: Option<&Ty>) -> Ty {
        // 組み込み関数（ローカル変数・宣言で隠されていなければ）
        if let ExprKind::Path(p) = &callee.kind {
            if p.segs.len() == 1 && self.lookup_scope(fx, &p.segs[0]).is_none() {
                let n = p.segs[0].clone();
                let shadowed = self.mod_scope[fx.module].contains_key(&n) || self.imports[fx.module].contains_key(&n);
                if !shadowed {
                    if let Some(t) = self.check_builtin_call(fx, e, &n, args, exp) {
                        self.calls.entry(e.id).or_insert(CallTarget::Builtin(n));
                        return t;
                    }
                }
            }
        }
        // 呼ぶ先の解決
        let target: Option<PathRes> = match &callee.kind {
            ExprKind::Path(p) => {
                if p.segs.len() == 1 {
                    match self.lookup_scope(fx, &p.segs[0]) {
                        Some(ScopeEnt::Local(l)) => {
                            self.res.insert(callee.id, Res::Local(l));
                            self.funcs[fx.fid].locals[l].read = true;
                            None
                        }
                        Some(ScopeEnt::Global(g)) => Some(PathRes::Global(g)),
                        None => Some(self.resolve_value_path(fx.module, &p.segs, &callee.span)),
                    }
                } else {
                    Some(self.resolve_value_path(fx.module, &p.segs, &callee.span))
                }
            }
            _ => None,
        };
        match target {
            Some(PathRes::Func(f)) => {
                self.res.insert(callee.id, Res::Func);
                self.ensure_sig(f);
                let fi = self.funcs[f].clone();
                self.check_args(fx, args, &fi.params, &fi.name, &e.span);
                self.calls.insert(e.id, CallTarget::Fn(f));
                if fi.interrupt.is_some() {
                    self.err("E0808", "割り込み処理の関数は呼べません", &e.span);
                }
                match fi.kind {
                    FnKind::Comptime => {
                        // すべての引数が定数なら、ここで計算する
                        let mut vals = Vec::new();
                        let mut ok = true;
                        for a in args {
                            match self.econst.get(&a.value.id) {
                                Some(v) => vals.push(v.clone()),
                                None => ok = false,
                            }
                        }
                        if !ok {
                            self.diags.error(
                                Diag::error("E0604", &format!("comptime fn {} は実行時には呼べません", fi.name), e.span.clone())
                                    .note("引数がすべて定数なら、コンパイル時に計算されます"),
                            );
                            return fi.ret.clone();
                        }
                        let mut conv = Vec::new();
                        for (i, v) in vals.into_iter().enumerate() {
                            let pt = fi.params.get(i).map(|x| x.1.clone()).unwrap_or(Ty::CInt);
                            let use_cast = matches!((&v, &pt), (Value::Int(_, Some(_)), Ty::CInt));
                            let r = if use_cast {
                                self.cast_value(v, &pt, &e.span)
                            } else {
                                self.convert_value(v, &pt, &e.span)
                            };
                            match r {
                                Ok(x) => conv.push(x),
                                Err(er) => {
                                    self.diags.err(&er.code, &er.msg, er.span);
                                    return fi.ret.clone();
                                }
                            }
                        }
                        match self.call_ct(f, conv, &e.span) {
                            Ok(v) => {
                                self.check_inexact_time(&fi, &v, args, &e.span);
                                self.econst.insert(e.id, v);
                                self.calls.insert(e.id, CallTarget::Folded);
                            }
                            Err(er) => self.diags.err(&er.code, &er.msg, er.span),
                        }
                        let rt = fi.ret.clone();
                        if rt == Ty::CInt {
                            return Ty::IntLit;
                        }
                        return rt;
                    }
                    FnKind::Const => {}
                    _ => {
                        if fx.is_const_fn {
                            self.err("E0603", &format!("const fn の中で const fn でない {} を呼んでいます", fi.name), &e.span);
                        }
                    }
                }
                self.add_edge(fx, f, &e.span);
                fi.ret.clone()
            }
            Some(PathRes::TypeFn(t, name)) => self.check_type_fn(fx, e, &t, &name, args, exp),
            Some(PathRes::Global(g)) => {
                self.ensure_global(g);
                self.res.insert(callee.id, Res::Global(g));
                let gt = self.globals[g].ty.clone();
                self.call_fnset(fx, e, &gt, args)
            }
            Some(PathRes::Unbound(i)) => {
                self.diags.error(
                    Diag::error("E1104", &format!("インターフェース {} はこのターゲットで bind されていません", i), callee.span.clone())
                        .help(&format!("comptime if target.has({}) {{ … }} の中で使ってください", i)),
                );
                for a in args {
                    self.check_expr(fx, &a.value, None);
                }
                Ty::Error
            }
            Some(PathRes::Private(msg)) => {
                self.err("E0203", &msg, &callee.span);
                Ty::Error
            }
            Some(PathRes::NotFound(msg)) => {
                self.err("E0201", &msg, &callee.span);
                for a in args {
                    self.check_expr(fx, &a.value, None);
                }
                Ty::Error
            }
            Some(_) => {
                self.err("E0401", "関数ではないものを呼んでいます", &callee.span);
                Ty::Error
            }
            None => {
                // ローカル変数（関数集合の値）
                if let Some(Res::Local(l)) = self.res.get(&callee.id).cloned() {
                    let lt = self.funcs[fx.fid].locals[l].ty.clone();
                    return self.call_fnset(fx, e, &lt, args);
                }
                self.err("E0401", "関数ではないものを呼んでいます", &callee.span);
                Ty::Error
            }
        }
    }

    fn check_inexact_time(&mut self, fi: &FnInfo, v: &Value, args: &[Arg], sp: &Span) {
        if fi.name == "us" && self.modules[fi.module].name == "core" {
            if let (Some(a), Some(c)) = (args.first(), value_int(v)) {
                if let Some(t) = self.econst.get(&a.value.id).and_then(value_int) {
                    let clock = self.target.clock.unwrap_or(1_000_000) as i128;
                    if (t * clock) % 1_000_000 != 0 {
                        let d = Diag::warning("inexact_time", &format!("{} マイクロ秒はちょうどのサイクル数にならないので、{} サイクルに切り上げました", t, c), sp.clone());
                        self.diags.warn(d);
                    }
                }
            }
        }
    }

    fn call_fnset(&mut self, fx: &mut FnCx, e: &Expr, t: &Ty, args: &[Arg]) -> Ty {
        match t {
            Ty::FnSet(s) => {
                let set = self.types.fnsets[*s].clone();
                let (params, ret) = match set.fns.first() {
                    Some(f) => {
                        self.ensure_sig(*f);
                        (self.funcs[*f].params.clone(), self.funcs[*f].ret.clone())
                    }
                    None => (Vec::new(), Ty::Unit),
                };
                self.check_args(fx, args, &params, &set.name, &e.span);
                for f in &set.fns {
                    self.add_edge(fx, *f, &e.span);
                }
                self.calls.insert(e.id, CallTarget::FnSetVar);
                ret
            }
            Ty::Error => Ty::Error,
            other => {
                self.err("E0401", &format!("{} は関数ではないので呼べません", self.tyname(other)), &e.span);
                Ty::Error
            }
        }
    }

    fn check_type_fn(&mut self, fx: &mut FnCx, e: &Expr, t: &Ty, name: &str, args: &[Arg], exp: Option<&Ty>) -> Ty {
        self.calls.insert(e.id, CallTarget::TypeFn(t.clone(), name.to_string()));
        let tn = self.tyname(t);
        let base_int = match t {
            Ty::Range(r) => Ty::Int(IntK::unsigned_of_size(self.types.ranges[*r].base_size())),
            _ => Ty::Int(IntK::U8),
        };
        match name {
            "new" => {
                for a in args {
                    self.check_expr(fx, &a.value, None);
                }
                return match exp {
                    Some(Ty::StrBuf(n)) => Ty::StrBuf(*n),
                    _ => {
                        self.err("E0401", "strbuf::new() の長さが決まりません（型を書いてください）", &e.span);
                        Ty::Error
                    }
                };
            }
            "range" => {
                if args.len() != 2 {
                    self.err("E0418", &format!("{}::range の引数は 2 つです", tn), &e.span);
                }
                for a in args {
                    let at = self.check_expr(fx, &a.value, Some(t));
                    self.expect_ty(&a.value, &at, t);
                }
                return Ty::RangeIter(Box::new(t.clone()));
            }
            _ => {}
        }
        if args.len() != 1 {
            self.err("E0418", &format!("{}::{} の引数は 1 つです", tn, name), &e.span);
            for a in args {
                self.check_expr(fx, &a.value, None);
            }
            return Ty::Error;
        }
        let a = &args[0].value;
        let at = self.check_expr(fx, a, Some(&base_int));
        if !self.coerce(a, &at, &base_int) {
            self.mismatch(&a.span, &base_int, &at);
        }
        let r = match name {
            "check" => Ty::Opt(Box::new(t.clone())),
            "wrap" => {
                if let Ty::Range(r) = t {
                    let d = self.types.ranges[*r].clone();
                    if !is_pow2(d.count()) || d.lo != 0 {
                        self.diags.error(
                            Diag::error("E0413", &format!("{}::wrap は、0 から始まり個数が 2 のべき乗の範囲型だけに使えます", d.name), e.span.clone())
                                .help("::check(n) を使ってください"),
                        );
                    }
                } else {
                    self.err("E0413", "wrap は範囲型だけに使えます", &e.span);
                }
                t.clone()
            }
            "unchecked" => {
                let fname = self.funcs[fx.fid].name.clone();
                self.unsafe_list.push(format!("{}::unchecked()（{}、{}行）", tn, fname, e.span.line));
                t.clone()
            }
            _ => {
                self.err("E0201", &format!("{} に関数 {} はありません", tn, name), &e.span);
                Ty::Error
            }
        };
        // 定数の畳み込み
        if let Some(v) = self.econst.get(&a.id).cloned() {
            if let Some(x) = value_int(&v) {
                let folded = match (t, name) {
                    (Ty::Range(rid), "check") => {
                        let d = self.types.ranges[*rid].clone();
                        Some(Value::Opt(if x >= d.lo && x < d.hi { Some(Box::new(Value::Range(*rid, x))) } else { None }))
                    }
                    (Ty::Range(rid), "wrap") => {
                        let d = self.types.ranges[*rid].clone();
                        Some(Value::Range(*rid, d.lo + (x - d.lo).rem_euclid(d.count().max(1))))
                    }
                    (Ty::Range(rid), "unchecked") => Some(Value::Range(*rid, x)),
                    (Ty::Enum(eid), "check") => {
                        let d = self.types.enums[*eid].clone();
                        Some(Value::Opt(if d.variants.iter().any(|v| v.1 == x) { Some(Box::new(Value::Enum(*eid, x))) } else { None }))
                    }
                    _ => None,
                };
                if let Some(fv) = folded {
                    self.econst.insert(e.id, fv);
                }
            }
        }
        r
    }

    fn check_builtin_call(&mut self, fx: &mut FnCx, e: &Expr, n: &str, args: &[Arg], exp: Option<&Ty>) -> Option<Ty> {
        let t = match n {
            "Some" => {
                if args.len() != 1 {
                    self.err("E0418", "Some には値が 1 つ必要です", &e.span);
                    return Some(Ty::Error);
                }
                let inner_exp = match exp {
                    Some(Ty::Opt(t)) => Some((**t).clone()),
                    _ => None,
                };
                let a = &args[0].value;
                let t = self.check_expr(fx, a, inner_exp.as_ref());
                let t = match &inner_exp {
                    Some(w) => {
                        self.expect_ty(a, &t, w);
                        w.clone()
                    }
                    None => t,
                };
                self.calls.insert(e.id, CallTarget::SomeCtor);
                if let Some(v) = self.econst.get(&a.id).cloned() {
                    self.econst.insert(e.id, Value::Opt(Some(Box::new(v))));
                }
                Ty::Opt(Box::new(t))
            }
            "irq_enable" | "irq_disable" | "wait_interrupt" => {
                if !args.is_empty() {
                    self.err("E0418", &format!("{}() に引数はありません", n), &e.span);
                }
                if fx.is_const_fn {
                    self.err("E0603", &format!("const fn の中で {}() を呼んでいます", n), &e.span);
                }
                if n == "irq_enable" {
                    self.funcs[fx.fid].calls_irq_enable = true;
                    if fx.atomic > 0 || fx.timed > 0 {
                        self.err("E0802", "atomic・cycles()・io_span() の中で irq_enable() を呼んでいます", &e.span);
                    }
                    if fx.interrupt {
                        self.err("E0802", "割り込み処理の中で irq_enable() を呼んでいます", &e.span);
                    }
                }
                if n == "wait_interrupt" && !self.target.is_w65c02s() {
                    self.diags.error(
                        Diag::error("E1105", &format!("wait_interrupt() は W65C02S のターゲットだけで使えます（このターゲットは {}）", crate::project::cpu_text(&self.target)), e.span.clone())
                            .help("comptime if target.cpu == Cpu::W65C02S && target.has_irq { … } で囲んでください"),
                    );
                }
                // irq = none のターゲットでは、割り込みを許可・待つことはできない（1.1。16.4節）
                if (n == "irq_enable" || n == "wait_interrupt") && !self.target.has_irq() {
                    let help = if n == "wait_interrupt" {
                        "comptime if target.cpu == Cpu::W65C02S && target.has_irq { … } で囲んでください"
                    } else {
                        "割り込みのあるターゲットだけで使うなら comptime if target.has_irq { … } で囲んでください"
                    };
                    self.diags.error(
                        Diag::error("E1110", &format!("ターゲット {} は irq = none なので、{}() は使えません", self.target.name, n), e.span.clone())
                            .note("irq を省略すると none になります（16.4節）")
                            .help(help),
                    );
                }
                Ty::Unit
            }
            "delay" => {
                if args.len() != 1 {
                    self.err("E0418", "delay には引数が 1 つ必要です", &e.span);
                    return Some(Ty::Unit);
                }
                let a = &args[0].value;
                self.check_expr(fx, a, None);
                match self.econst.get(&a.id).and_then(value_int) {
                    Some(v) if v >= 0 => {}
                    _ => self.err("E1001", "delay の引数はコンパイル時に計算できる 0 以上の数です", &a.span),
                }
                Ty::Unit
            }
            "size_of" => {
                if args.len() != 1 {
                    self.err("E0418", "size_of には型が 1 つ必要です", &e.span);
                    return Some(Ty::IntLit);
                }
                match self.eval_in_module(e, fx.module, None) {
                    Ok(v) => {
                        self.econst.insert(e.id, v);
                    }
                    Err(er) => self.diags.err(&er.code, &er.msg, er.span),
                }
                Ty::IntLit
            }
            "include_bin" => {
                self.err("E1001", "include_bin は const の値の中だけで使えます", &e.span);
                Ty::Error
            }
            "sin" | "cos" | "sqrt" | "round" | "floor" | "abs" => {
                match self.eval_in_module(e, fx.module, None) {
                    Ok(v) => {
                        self.econst.insert(e.id, v);
                        Ty::CReal
                    }
                    Err(_) => {
                        self.err("E0604", &format!("{} はコンパイル時の数学関数で、実行時には呼べません", n), &e.span);
                        Ty::Error
                    }
                }
            }
            _ => return None,
        };
        Some(t)
    }

    fn check_method(&mut self, fx: &mut FnCx, e: &Expr, recv: &Expr, name: &str, args: &[Arg], _exp: Option<&Ty>) -> Ty {
        // 書き換えるメソッドは、受け手を代入の左辺として調べる
        if name == "insert" || name == "remove" || name == "toggle" {
            let rt = self.check_place_base(fx, recv);
            if let Some(l) = self.base_local(recv) {
                self.funcs[fx.fid].locals[l].read = true;
            }
            match &rt {
                Ty::Flags(_) => {
                    if args.len() != 1 {
                        self.err("E0418", &format!("{} の引数は 1 つです", name), &e.span);
                    } else {
                        let at = self.check_expr(fx, &args[0].value, Some(&rt));
                        self.expect_ty(&args[0].value, &at, &rt);
                    }
                    if let Some(g) = self.base_global(recv) {
                        if let GKind::Io(d) = &self.globals[g].kind {
                            if *d == IoDir::Out {
                                self.err("E0901", &format!("{} は io(out) なので {} はできません（読んで書く操作です）", self.globals[g].name, name), &e.span);
                            }
                        }
                        self.note_access(fx, g, true, false, &e.span);
                    }
                }
                Ty::Error => {}
                other => {
                    self.err("E0417", &format!("{} にメソッド {} はありません", self.tyname(other), name), &e.span);
                }
            }
            return Ty::Unit;
        }
        let rt = self.check_expr(fx, recv, None);
        let rt = if rt == Ty::IntLit { self.default_type(recv, &rt, false) } else { rt };
        let nargs = args.len();
        let mut arg_types = Vec::new();
        let r = match (&rt, name) {
            (Ty::Int(_), "checked_add") | (Ty::Int(_), "checked_sub") => {
                self.method_args(fx, args, &[rt.clone()], &mut arg_types, name, &e.span);
                Ty::Opt(Box::new(rt.clone()))
            }
            (Ty::Int(_), "saturating_add") | (Ty::Int(_), "saturating_sub") => {
                self.method_args(fx, args, &[rt.clone()], &mut arg_types, name, &e.span);
                rt.clone()
            }
            (Ty::Int(_), "shl") | (Ty::Int(_), "shr") => {
                self.method_args(fx, args, &[Ty::Int(IntK::U8)], &mut arg_types, name, &e.span);
                rt.clone()
            }
            (Ty::Int(_), "rol") | (Ty::Int(_), "ror") => {
                self.method_args(fx, args, &[], &mut arg_types, name, &e.span);
                rt.clone()
            }
            (Ty::Bcd(n), "digits") => {
                self.method_args(fx, args, &[], &mut arg_types, name, &e.span);
                Ty::Array(Box::new(Ty::Int(IntK::U8)), Len::N(n * 2))
            }
            (Ty::Flags(_), "contains") => {
                self.method_args(fx, args, &[rt.clone()], &mut arg_types, name, &e.span);
                if let (Some(Value::Flags(_, x)), Some(a)) = (self.econst.get(&recv.id).cloned(), args.first()) {
                    if let Some(m) = self.econst.get(&a.value.id).and_then(value_int) {
                        self.econst.insert(e.id, Value::Bool(x & m == m));
                    }
                }
                Ty::Bool
            }
            (Ty::Array(_, l), "len") => {
                self.method_args(fx, args, &[], &mut arg_types, name, &e.span);
                let n = self.types.len_count(l);
                self.econst.insert(e.id, Value::Int(n as i128, None));
                Ty::IntLit
            }
            (Ty::Array(et, _), "get") => {
                self.method_args(fx, args, &[Ty::Int(IntK::U8)], &mut arg_types, name, &e.span);
                Ty::Opt(et.clone())
            }
            (Ty::Str(_, _), "len") => {
                self.method_args(fx, args, &[], &mut arg_types, name, &e.span);
                if let Some(Value::Str(_, b, _)) = self.econst.get(&recv.id).cloned() {
                    self.econst.insert(e.id, Value::Int(b.len() as i128, Some(IntK::U8)));
                }
                Ty::Int(IntK::U8)
            }
            (Ty::StrBuf(_), "clear") => {
                self.method_args(fx, args, &[], &mut arg_types, name, &e.span);
                Ty::Unit
            }
            (Ty::StrBuf(_), "push") => {
                self.method_args(fx, args, &[Ty::Char(None)], &mut arg_types, name, &e.span);
                Ty::Bool
            }
            (Ty::StrBuf(_), "len") => Ty::Int(IntK::U8),
            (Ty::StrBuf(_), "as_str") => Ty::Str(StrK::P, None),
            (Ty::Ptr(_), "add") => {
                if nargs == 1 {
                    let at = self.check_expr(fx, &args[0].value, None);
                    let at = self.default_type(&args[0].value, &at, false);
                    if at != Ty::Int(IntK::U8) && at != Ty::Int(IntK::U16) && at != Ty::Error {
                        self.err("E0401", "add の引数は u8 か u16 です", &args[0].value.span);
                    }
                } else {
                    self.err("E0418", "add の引数は 1 つです", &e.span);
                }
                rt.clone()
            }
            (Ty::Ptr(_), "add_pages") => {
                self.method_args(fx, args, &[Ty::Int(IntK::U8)], &mut arg_types, name, &e.span);
                rt.clone()
            }
            (Ty::RangeIter(_), "rev") => rt.clone(),
            (Ty::Error, _) => {
                for a in args {
                    self.check_expr(fx, &a.value, None);
                }
                Ty::Error
            }
            _ => {
                self.err("E0417", &format!("{} にメソッド {} はありません", self.tyname(&rt), name), &e.span);
                for a in args {
                    self.check_expr(fx, &a.value, None);
                }
                Ty::Error
            }
        };
        if let Some(g) = self.base_global(recv) {
            if let GKind::Io(IoDir::Out) = self.globals[g].kind {
                self.err("E0901", &format!("{} は io(out)（書き込み専用）なので読めません", self.globals[g].name), &recv.span);
            }
        }
        r
    }

    fn method_args(&mut self, fx: &mut FnCx, args: &[Arg], want: &[Ty], out: &mut Vec<Ty>, name: &str, sp: &Span) {
        if args.len() != want.len() {
            self.err("E0418", &format!("{} の引数は {} つです", name, want.len()), sp);
        }
        for (i, a) in args.iter().enumerate() {
            match want.get(i) {
                Some(w) => {
                    let t = self.check_expr(fx, &a.value, Some(w));
                    self.expect_ty(&a.value, &t, w);
                    out.push(w.clone());
                }
                None => {
                    let t = self.check_expr(fx, &a.value, None);
                    out.push(t);
                }
            }
        }
    }

    /// インラインアセンブリの {名前} の参照を調べる
    pub fn check_asm_refs(&mut self, fx: &mut FnCx, text: &str, sp: &Span, _is_unsafe: bool) {
        let names = asm_ref_names(text);
        for (full, _line) in names {
            let base = match full.find('.') {
                Some(i) => full[..i].to_string(),
                None => full.clone(),
            };
            let segs: Vec<String> = base.split("::").map(|s| s.to_string()).collect();
            if segs.len() == 1 {
                if let Some(ent) = self.lookup_scope(fx, &segs[0]) {
                    match ent {
                        ScopeEnt::Local(l) => {
                            self.funcs[fx.fid].locals[l].read = true;
                            self.funcs[fx.fid].locals[l].asm_ref = true;
                        }
                        ScopeEnt::Global(g) => {
                            self.globals[g].asm_ref = true;
                        }
                    }
                    continue;
                }
            }
            match self.resolve_value_path(fx.module, &segs, sp) {
                PathRes::Global(g) => {
                    self.ensure_global(g);
                    self.globals[g].asm_ref = true;
                    self.note_access(fx, g, true, false, sp);
                }
                PathRes::Func(f) => {
                    self.add_edge(fx, f, sp);
                }
                PathRes::Value(_) => {}
                PathRes::Private(msg) => self.err("E0203", &msg, sp),
                _ => self.err("E0201", &format!("アセンブリの中の {{{}}} が見つかりません", full), sp),
            }
        }
    }
}

/// アセンブリのテキストの中の {名前} を集める
pub fn asm_ref_names(text: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    for (ln, line) in text.lines().enumerate() {
        let mut code = match line.find(';') {
            Some(i) => &line[..i],
            None => line,
        };
        if let Some(i) = code.find("//") {
            code = &code[..i];
        }
        let mut rest = code;
        while let Some(i) = rest.find('{') {
            let after = &rest[i + 1..];
            match after.find('}') {
                Some(j) => {
                    out.push((after[..j].trim().to_string(), ln));
                    rest = &after[j + 1..];
                }
                None => break,
            }
        }
    }
    out
}

fn intersect(a: Option<HashSet<usize>>, b: Option<HashSet<usize>>) -> Option<HashSet<usize>> {
    match (a, b) {
        (None, x) => x,
        (x, None) => x,
        (Some(x), Some(y)) => Some(x.intersection(&y).cloned().collect()),
    }
}

/// ループの後の「値が入っている変数」：本体で入れたものも数える（寛容な扱い）
fn merge_after_loop(before: Option<HashSet<usize>>, after_body: Option<HashSet<usize>>) -> Option<HashSet<usize>> {
    match (before, after_body) {
        (None, _) => None,
        (Some(b), None) => Some(b),
        (Some(b), Some(a)) => {
            let mut s = b;
            for x in a {
                s.insert(x);
            }
            Some(s)
        }
    }
}

/// 式の木をたどる（子を先に見る必要はない）
pub fn walk_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    match &e.kind {
        ExprKind::Unary(_, x) | ExprKind::Cast(x, _) | ExprKind::Try(x) | ExprKind::Field(x, _) => walk_expr(x, f),
        ExprKind::Binary(_, a, b) | ExprKind::Assign(_, a, b) | ExprKind::Index(a, b) => {
            walk_expr(a, f);
            walk_expr(b, f);
        }
        ExprKind::MethodCall(r, _, args) => {
            walk_expr(r, f);
            for a in args {
                walk_expr(&a.value, f);
            }
        }
        ExprKind::Call(c, args) => {
            walk_expr(c, f);
            for a in args {
                walk_expr(&a.value, f);
            }
        }
        ExprKind::Range(a, b, _) => {
            if let Some(x) = a {
                walk_expr(x, f);
            }
            if let Some(x) = b {
                walk_expr(x, f);
            }
        }
        ExprKind::StructLit(_, fields, base) => {
            for (_, x, _) in fields {
                walk_expr(x, f);
            }
            if let Some(b) = base {
                walk_expr(b, f);
            }
        }
        ExprKind::ArrayList(v) => {
            for x in v {
                walk_expr(x, f);
            }
        }
        ExprKind::ArrayRepeat(a, b) => {
            walk_expr(a, f);
            walk_expr(b, f);
        }
        ExprKind::Block(b) | ExprKind::Loop(_, b) | ExprKind::Atomic(b) => walk_block(b, f),
        ExprKind::If(c, b, e2) | ExprKind::ComptimeIf(c, b, e2) => {
            walk_expr(c, f);
            walk_block(b, f);
            if let Some(x) = e2 {
                walk_expr(x, f);
            }
        }
        ExprKind::IfLet(_, s, b, e2) => {
            walk_expr(s, f);
            walk_block(b, f);
            if let Some(x) = e2 {
                walk_expr(x, f);
            }
        }
        ExprKind::Match(s, arms, _) | ExprKind::ComptimeMatch(s, arms) => {
            walk_expr(s, f);
            for a in arms {
                walk_expr(&a.body, f);
            }
        }
        ExprKind::While(_, c, b, _) => {
            walk_expr(c, f);
            walk_block(b, f);
        }
        ExprKind::For(_, _, it, b, _) => {
            walk_expr(it, f);
            walk_block(b, f);
        }
        ExprKind::Timed(_, n, b) => {
            walk_expr(n, f);
            walk_block(b, f);
        }
        ExprKind::Break(_, Some(x)) | ExprKind::Return(Some(x)) => walk_expr(x, f),
        _ => {}
    }
}

pub fn walk_block(b: &Block, f: &mut dyn FnMut(&Expr)) {
    for s in &b.stmts {
        match &s.kind {
            StmtKind::Let(_, _, Some(x)) => walk_expr(x, f),
            StmtKind::Expr(x, _) => walk_expr(x, f),
            _ => {}
        }
    }
    if let Some(t) = &b.tail {
        walk_expr(t, f);
    }
}
