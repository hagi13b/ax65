// コード生成：値・式・呼び出し・Option・for・インラインアセンブリ
use crate::asm6502::{is_branch, is_cmos_only, AMode, Item, Mode, OpExpr, OpTable};
use crate::asmtext::{parse_lines, AExpr, Operand, Sel, Term};
use crate::ast::*;
use crate::codegen::*;
use crate::comptime::{value_int, Value};
use crate::diag::Span;
use crate::program::*;
use crate::types::*;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub enum RetConv {
    Nothing,
    Carry,
    Regs(u32),
    Area(String, u32),
    OptCarry(Option<String>),
}

fn is_pow2(v: i128) -> bool {
    v > 0 && (v & (v - 1)) == 0
}

fn log2(v: i128) -> u32 {
    let mut k = 0;
    while (1i128 << k) < v {
        k += 1;
    }
    k
}

impl Gen {
    // =====================================================================
    // 値の置き場所
    // =====================================================================

    pub fn global_val(&mut self, g: usize) -> Val {
        self.used_globals.insert(g);
        let gi = self.ctx.globals[g].clone();
        let size = self.sz(&gi.ty);
        match gi.kind {
            GKind::Io(dir) => Val::Mem {
                base: Base::Abs(gi.io_addr.unwrap_or(0) as i64),
                idx: Idx::None,
                stride: 1,
                size,
                io: true,
                rmw: dir == IoDir::Rmw,
            },
            GKind::Const => match gi.ty {
                Ty::Str(_, _) => Val::Addr(Base::Sym(gi.label.clone(), 0)),
                _ => {
                    if let (Some(v), true) = (&gi.value, self.is_scalar_ty(&gi.ty)) {
                        if let Some(x) = value_int(v) {
                            return Val::Imm(x);
                        }
                    }
                    Val::mem(Base::Sym(gi.label.clone(), 0), size)
                }
            },
            _ => Val::mem(Base::Sym(gi.label.clone(), 0), size),
        }
    }

    pub fn pool_string(&mut self, bytes: Vec<u8>) -> String {
        if let Some(l) = self.str_pool.get(&bytes) {
            return l.clone();
        }
        let l = format!("S_{}", self.str_pool.len() + 1);
        self.str_pool.insert(bytes.clone(), l.clone());
        self.data.push(Item::Label(l.clone()));
        self.data.push(Item::Bytes(bytes, "文字列".to_string()));
        l
    }

    /// 式が、コードを出さずに置き場所を言えるものか
    pub fn is_simple(&self, e: &Expr) -> bool {
        if self.const_int(e).is_some() {
            return true;
        }
        match &e.kind {
            ExprKind::Str(_, _, _) => true,
            ExprKind::Path(_) => matches!(self.ctx.res.get(&e.id), Some(Res::Local(_)) | Some(Res::Global(_))),
            ExprKind::Field(b, _) => match self.ty_of(b) {
                Ty::Struct(_) => self.is_simple(b),
                Ty::Ptr(_) => matches!(b.kind, ExprKind::Path(_)) && self.is_simple(b),
                _ => false,
            },
            ExprKind::Index(b, i) => {
                let bt = self.ty_of(b);
                let base_ok = match &bt {
                    Ty::Array(et, _) => !matches!(**et, Ty::Array(_, _)) || self.const_int(i).is_some(),
                    Ty::Ptr(_) => matches!(b.kind, ExprKind::Path(_)),
                    _ => false,
                };
                base_ok && matches!(b.kind, ExprKind::Path(_) | ExprKind::Field(_, _) | ExprKind::Index(_, _)) && self.is_simple(b) && self.is_simple_index(i)
            }
            ExprKind::Unary(UnOp::Addr, x) => self.is_simple(x),
            ExprKind::Cast(x, _) => {
                let from = self.ty_of(x);
                let to = self.ty_of(e);
                let (fs, ts) = (self.sz(&from), self.sz(&to));
                self.is_simple(x) && (ts <= fs || !self.ctx.types.is_signed(&from))
            }
            _ => false,
        }
    }

    fn is_simple_index(&self, i: &Expr) -> bool {
        if self.const_int(i).is_some() {
            return true;
        }
        if let ExprKind::Path(_) = &i.kind {
            match self.ctx.res.get(&i.id) {
                Some(Res::Local(_)) => return self.sz(&self.ty_of(i)) == 1,
                Some(Res::Global(g)) => {
                    let gi = &self.ctx.globals[*g];
                    return self.sz(&gi.ty) == 1 && !matches!(gi.kind, GKind::Io(_));
                }
                _ => {}
            }
        }
        false
    }

    /// 副作用（関数の呼び出し、I/O、代入）を持つか
    pub fn has_effects(&self, e: &Expr) -> bool {
        let mut found = false;
        crate::check::walk_expr(e, &mut |x: &Expr| match &x.kind {
            ExprKind::Call(_, _) => {
                if !matches!(self.ctx.calls.get(&x.id), Some(CallTarget::SomeCtor) | Some(CallTarget::Folded) | Some(CallTarget::TypeFn(_, _))) {
                    found = true;
                }
            }
            ExprKind::MethodCall(_, n, _) => {
                if matches!(n.as_str(), "insert" | "remove" | "toggle" | "push" | "clear") {
                    found = true;
                }
            }
            ExprKind::Assign(_, _, _) | ExprKind::Asm(_) | ExprKind::Atomic(_) | ExprKind::Return(_) | ExprKind::Break(_, _) => found = true,
            ExprKind::Path(_) => {
                if let Some(Res::Global(g)) = self.ctx.res.get(&x.id) {
                    if matches!(self.ctx.globals[*g].kind, GKind::Io(_) | GKind::Shared(_)) {
                        found = true;
                    }
                }
            }
            _ => {}
        });
        found
    }

    pub fn materialize(&mut self, e: &Expr) -> Val {
        let t = self.ty_of(e);
        let n = self.sz(&t);
        if n == 0 {
            self.gen(e, &Want::Effect);
            return Val::Imm(0);
        }
        let v = if self.needs_zp(&t) {
            let b = self.alloc_ztmp(n);
            Val::mem(b, n)
        } else {
            self.tmp_val(n)
        };
        self.gen(e, &Want::Store(v.clone()));
        v
    }

    pub fn materialize_val(&mut self, v: &Val, n: u32) -> Val {
        let t = self.tmp_val(n.max(1));
        self.copy_val(v, &t, n);
        t
    }

    pub fn val_of(&mut self, e: &Expr) -> Val {
        if let Some(c) = self.const_int(e) {
            return Val::Imm(c);
        }
        let t = self.ty_of(e);
        if let Some(Value::Func(f)) = self.ctx.econst.get(&e.id).cloned() {
            if let Ty::FnSet(s) = &t {
                let pos = self.ctx.types.fnsets[*s].fns.iter().position(|x| *x == f).unwrap_or(0);
                return Val::Imm(pos as i128);
            }
        }
        match &e.kind {
            ExprKind::Str(_, _, _) => match self.ctx.econst.get(&e.id).cloned() {
                Some(v) => {
                    let bytes = self.ctx.value_bytes(&v, &t);
                    let l = self.pool_string(bytes);
                    match v {
                        Value::Array(_) => Val::mem(Base::Sym(l, 0), self.sz(&t)),
                        _ => Val::Addr(Base::Sym(l, 0)),
                    }
                }
                None => Val::Imm(0),
            },
            ExprKind::Path(_) => match self.ctx.res.get(&e.id).cloned() {
                Some(Res::Local(l)) => self.local_val(l),
                Some(Res::Global(g)) => self.global_val(g),
                _ => self.materialize(e),
            },
            ExprKind::Field(b, f) => self.field_val(b, f, e),
            ExprKind::Index(b, i) => self.index_val(b, i, e),
            ExprKind::Unary(UnOp::Addr, x) => self.addr_of(x),
            ExprKind::Cast(x, _) if self.is_simple(e) => {
                let from = self.ty_of(x);
                let (fs, ts) = (self.sz(&from), self.sz(&t));
                let inner = self.val_of(x);
                if ts <= fs {
                    match inner {
                        Val::Imm(c) => Val::Imm(c & ((1i128 << (8 * ts)) - 1)),
                        Val::Reg(r) => Val::Reg(r),
                        other => other.part(0, ts),
                    }
                } else {
                    Val::Ext { inner: Box::new(inner), from: fs }
                }
            }
            _ => self.materialize(e),
        }
    }

    pub fn place_val(&mut self, e: &Expr) -> Val {
        self.val_of(e)
    }

    /// ポインタの値をゼロページに置いた場所
    fn ptr_zp(&mut self, b: &Expr) -> Base {
        let v = self.val_of(b);
        if let Val::Mem { base, idx: Idx::None, io: false, .. } = &v {
            if let Base::Sym(s, _) = base {
                if s.starts_with("Z_") {
                    return base.clone();
                }
                // ポインタ型の static はゼロページに置かれる
                if let ExprKind::Path(_) = &b.kind {
                    if let Some(Res::Global(_)) = self.ctx.res.get(&b.id) {
                        return base.clone();
                    }
                }
            }
        }
        let z = self.alloc_ztmp(2);
        let zv = Val::mem(z.clone(), 2);
        self.copy_val(&v, &zv, 2);
        z
    }

    fn field_val(&mut self, b: &Expr, f: &str, e: &Expr) -> Val {
        let bt = self.ty_of(b);
        match &bt {
            Ty::Struct(s) => {
                let (off, ft) = match self.ctx.types.field(*s, f) {
                    Some(x) => x,
                    None => return Val::Imm(0),
                };
                let fsz = self.sz(&ft);
                let bv = self.val_of(b);
                bv.part(off, fsz)
            }
            Ty::Ptr(inner) => {
                if let Ty::Struct(s) = &**inner {
                    let (off, ft) = match self.ctx.types.field(*s, f) {
                        Some(x) => x,
                        None => return Val::Imm(0),
                    };
                    let fsz = self.sz(&ft);
                    let z = self.ptr_zp(b);
                    return Val::Ind { zp: z, idx: Idx::None, off: off as i64, size: fsz };
                }
                Val::Imm(0)
            }
            _ => {
                self.unimpl("このフィールドの読み書き", &e.span);
                Val::Imm(0)
            }
        }
    }

    /// 添字の値をメモリかレジスタで表す
    fn index_of(&mut self, i: &Expr) -> Idx {
        if let ExprKind::Path(_) = &i.kind {
            if let Some(Res::Local(l)) = self.ctx.res.get(&i.id).cloned() {
                match self.f.slots.get(&l) {
                    Some(Slot::RegX) => return Idx::X,
                    Some(Slot::RegY) => return Idx::Y,
                    _ => {}
                }
            }
        }
        if self.is_simple_index(i) {
            if let Val::Mem { base, idx: Idx::None, .. } = self.val_of(i) {
                return Idx::Mem(base);
            }
        }
        let t = self.alloc_tmp(1);
        self.gen_a(i);
        self.emit("sta", AMode::Mem(t.expr()), "添字を一時的にしまう");
        Idx::Mem(t)
    }

    fn array_planar(&self, et: &Ty) -> bool {
        let es = self.sz(et);
        match et {
            Ty::Struct(s) => !self.ctx.types.structs[*s].packed && es > 1,
            Ty::Array(_, _) => false,
            _ => es > 1,
        }
    }

    fn index_val(&mut self, b: &Expr, i: &Expr, e: &Expr) -> Val {
        let bt = self.ty_of(b);
        match &bt {
            Ty::Array(et, len) => {
                let et = (**et).clone();
                let es = self.sz(&et);
                let n = self.ctx.types.len_count(len) as i64;
                let planar = self.array_planar(&et);
                let ci = self.const_int(i);
                // 2 次元：行の先頭アドレスの表（3.8節）
                if let (Ty::Array(_, _), None) = (&et, ci) {
                    let bv = self.val_of(b);
                    return self.row_ptr(&bv, &et, n, i, e);
                }
                let bv = self.val_of(b);
                match bv {
                    Val::Mem { base, idx, stride, io, rmw, .. } => {
                        if let Some(c) = ci {
                            if planar {
                                return Val::Mem { base: base.add(c as i64 * stride), idx, stride: stride * n, size: es, io, rmw };
                            }
                            return Val::Mem { base: base.add(c as i64 * es as i64 * stride), idx, stride, size: es, io, rmw };
                        }
                        if idx != Idx::None || stride != 1 {
                            self.unimpl("添字が 2 つ以上ある変数の添字の読み書き", &e.span);
                            return Val::Imm(0);
                        }
                        if !planar && es > 1 {
                            self.unimpl("#[packed] の構造体の配列の、変数の添字による読み書き", &e.span);
                            return Val::Imm(0);
                        }
                        let ix = self.index_of(i);
                        Val::Mem { base, idx: ix, stride: if planar { n } else { 1 }, size: es, io, rmw }
                    }
                    Val::Ind { zp, idx, off, .. } => {
                        if let Some(c) = ci {
                            return Val::Ind { zp, idx, off: off + c as i64 * es as i64, size: es };
                        }
                        if idx != Idx::None || off != 0 || es != 1 {
                            self.unimpl("この配列の変数の添字", &e.span);
                            return Val::Imm(0);
                        }
                        let ix = self.index_of(i);
                        let ix = self.idx_for_y(ix);
                        Val::Ind { zp, idx: ix, off: 0, size: 1 }
                    }
                    _ => {
                        self.unimpl("この配列の読み書き", &e.span);
                        Val::Imm(0)
                    }
                }
            }
            Ty::Ptr(et) => {
                let es = self.sz(et);
                let z = self.ptr_zp(b);
                match self.const_int(i) {
                    Some(c) => Val::Ind { zp: z, idx: Idx::None, off: c as i64 * es as i64, size: es },
                    None => {
                        if es != 1 {
                            self.unimpl("1 バイトでない要素のポインタの、変数の添字", &e.span);
                        }
                        let ix = self.index_of(i);
                        let ix = self.idx_for_y(ix);
                        Val::Ind { zp: z, idx: ix, off: 0, size: es }
                    }
                }
            }
            _ => Val::Imm(0),
        }
    }

    /// (zp),y で使う添字：X にある値はメモリに移す
    fn idx_for_y(&mut self, ix: Idx) -> Idx {
        match ix {
            Idx::X => {
                let s = self.alloc_tmp(1);
                self.emit("stx", AMode::Mem(s.expr()), "添字を Y で使うためにしまう");
                Idx::Mem(s)
            }
            other => other,
        }
    }

    /// 2 次元配列の行の先頭をゼロページのポインタに置く
    fn row_ptr(&mut self, bv: &Val, row_ty: &Ty, rows: i64, i: &Expr, e: &Expr) -> Val {
        let row_size = self.sz(row_ty) as i64;
        let base = match bv {
            Val::Mem { base, idx: Idx::None, .. } => base.clone(),
            _ => {
                self.unimpl("この 2 次元配列の読み書き", &e.span);
                return Val::Imm(0);
            }
        };
        let key = format!("{:?}/{}/{}", base, row_size, rows);
        let tbl = match self.row_tables.get(&key) {
            Some(t) => t.clone(),
            None => {
                let t = format!("RT_{}", self.row_tables.len() + 1);
                let mut ex = Vec::new();
                for r in 0..rows {
                    ex.push(base.add(r * row_size).expr().lo());
                }
                for r in 0..rows {
                    ex.push(base.add(r * row_size).expr().hi());
                }
                self.data.push(Item::Label(t.clone()));
                self.data.push(Item::Exprs(ex, format!("行の先頭アドレスの表（{} 行 × {} バイト、下位・上位）", rows, row_size)));
                self.row_tables.insert(key, t.clone());
                self.notes.push(format!("2 次元配列の行の表 {}：{} バイト", t, rows * 2));
                t
            }
        };
        let ix = self.index_of(i);
        let z = self.alloc_ztmp(2);
        let r = if self.f.xres { 'y' } else { 'x' };
        let am = |o: OpExpr| if r == 'x' { AMode::MemX(o) } else { AMode::MemY(o) };
        match ix {
            Idx::X => {
                self.emit("lda", AMode::MemX(OpExpr::sym(&tbl)), "行の先頭の下位");
                self.emit("sta", AMode::Mem(z.expr()), "ポインタに書く");
                self.emit("lda", AMode::MemX(OpExpr::sym_off(&tbl, rows)), "行の先頭の上位");
                self.emit("sta", AMode::Mem(z.add(1).expr()), "ポインタに書く");
            }
            Idx::Y => {
                self.emit("lda", AMode::MemY(OpExpr::sym(&tbl)), "行の先頭の下位");
                self.emit("sta", AMode::Mem(z.expr()), "ポインタに書く");
                self.emit("lda", AMode::MemY(OpExpr::sym_off(&tbl, rows)), "行の先頭の上位");
                self.emit("sta", AMode::Mem(z.add(1).expr()), "ポインタに書く");
            }
            Idx::Mem(ib) => {
                self.load_index(r, &ib);
                self.emit("lda", am(OpExpr::sym(&tbl)), "行の先頭の下位");
                self.emit("sta", AMode::Mem(z.expr()), "ポインタに書く");
                self.emit("lda", am(OpExpr::sym_off(&tbl, rows)), "行の先頭の上位");
                self.emit("sta", AMode::Mem(z.add(1).expr()), "ポインタに書く");
            }
            Idx::None => {}
        }
        Val::Ind { zp: z, idx: Idx::None, off: 0, size: row_size as u32 }
    }

    fn addr_of(&mut self, x: &Expr) -> Val {
        let v = self.val_of(x);
        match v {
            Val::Mem { base, idx: Idx::None, .. } => Val::Addr(base),
            Val::Addr(b) => Val::Addr(b),
            _ => {
                self.unimpl("このアドレスの計算（変数の添字の要素のアドレスなど）", &x.span);
                Val::Imm(0)
            }
        }
    }

    // =====================================================================
    // 1 バイトの命令
    // =====================================================================

    fn idx_reg(&self) -> char {
        if self.f.xres {
            'y'
        } else {
            'x'
        }
    }

    pub fn load_index(&mut self, r: char, ib: &Base) {
        let key = ib.expr();
        let vol = self.is_vol(&key);
        if r == 'x' {
            if !vol && self.f.xcache.as_ref() == Some(&key) {
                return;
            }
            self.emit("ldx", AMode::Mem(key.clone()), "添字を X に");
            if !vol {
                self.f.xcache = Some(key);
            }
        } else {
            if !vol && self.f.ycache.as_ref() == Some(&key) {
                return;
            }
            self.emit("ldy", AMode::Mem(key.clone()), "添字を Y に");
            if !vol {
                self.f.ycache = Some(key);
            }
        }
    }

    /// A を使う命令（lda adc sbc and ora eor cmp bit）を、値のバイト k に対して出す
    pub fn emit_byte(&mut self, mn: &str, v: &Val, k: u32, c: &str) {
        match v {
            Val::Imm(x) => {
                let b = ((x >> (8 * k.min(15))) & 0xFF) as i64;
                self.imm(mn, b, c);
            }
            Val::Addr(b) => {
                let e = match k {
                    0 => b.expr().lo(),
                    1 => b.expr().hi(),
                    _ => OpExpr::num(0),
                };
                self.emit(mn, AMode::Imm(e), c);
            }
            Val::Mem { base, idx, stride, size, .. } => {
                if k >= *size {
                    self.imm(mn, 0, c);
                    return;
                }
                let a = base.add(k as i64 * stride).expr();
                match idx {
                    Idx::None => self.emit(mn, AMode::Mem(a), c),
                    Idx::X => self.emit(mn, AMode::MemX(a), c),
                    Idx::Y => self.emit(mn, AMode::MemY(a), c),
                    Idx::Mem(ib) => {
                        let r = self.idx_reg();
                        let ib = ib.clone();
                        self.load_index(r, &ib);
                        if r == 'x' {
                            self.emit(mn, AMode::MemX(a), c)
                        } else {
                            self.emit(mn, AMode::MemY(a), c)
                        }
                    }
                }
            }
            Val::Ind { zp, idx, off, size } => {
                if k >= *size {
                    self.imm(mn, 0, c);
                    return;
                }
                let o = off + k as i64;
                match idx {
                    Idx::None => {
                        if o == 0 && mn != "bit" {
                            self.emit(mn, AMode::Ind(zp.expr()), c);
                        } else {
                            self.imm("ldy", o, "ポインタの位置");
                            self.emit(mn, AMode::IndY(zp.expr()), c);
                        }
                    }
                    Idx::Mem(ib) => {
                        let ib = ib.clone();
                        if o != 0 {
                            self.unimpl("ポインタの、変数の添字と位置の両方", &Span::none());
                        }
                        self.load_index('y', &ib);
                        self.emit(mn, AMode::IndY(zp.expr()), c);
                    }
                    Idx::Y => self.emit(mn, AMode::IndY(zp.expr()), c),
                    Idx::X => self.unimpl("X の添字によるポインタの読み書き", &Span::none()),
                }
            }
            Val::Ext { inner, from } => {
                if k < *from {
                    let inner = (**inner).clone();
                    self.emit_byte(mn, &inner, k, c);
                } else {
                    self.imm(mn, 0, c);
                }
            }
            Val::Reg('a') => {
                // 値はもう A にある
                if mn != "lda" {
                    self.unimpl("A にある値を相手にする演算", &Span::none());
                }
            }
            Val::Reg(r) => {
                if mn == "lda" {
                    self.emit0(if *r == 'x' { "txa" } else { "tya" }, c);
                } else {
                    let s = self.scratch();
                    self.emit(if *r == 'x' { "stx" } else { "sty" }, AMode::Mem(s.expr()), "レジスタの値を一時的にしまう");
                    self.emit(mn, AMode::Mem(s.expr()), c);
                }
            }
        }
    }

    /// A を値のバイト k に書く
    pub fn store_byte(&mut self, v: &Val, k: u32, c: &str) {
        match v {
            Val::Mem { size, .. } if k >= *size => {}
            Val::Mem { .. } | Val::Ind { .. } => self.emit_byte("sta", v, k, c),
            Val::Reg('a') => {}
            Val::Reg(r) => self.emit0(if *r == 'x' { "tax" } else { "tay" }, c),
            _ => self.unimpl("書き込めない場所への書き込み", &Span::none()),
        }
    }

    pub fn store_zero(&mut self, v: &Val, k: u32, c: &str) {
        match v {
            Val::Mem { idx: Idx::None, size, .. } | Val::Mem { idx: Idx::X, size, .. } => {
                if k < *size {
                    self.emit_byte("stz", v, k, c);
                }
            }
            Val::Mem { idx: Idx::Mem(_), size, .. } if !self.f.xres => {
                if k < *size {
                    self.emit_byte("stz", v, k, c);
                }
            }
            _ => {
                self.imm("lda", 0, "0");
                self.store_byte(v, k, c);
            }
        }
    }

    /// 読み・変更・書き込みの命令（inc dec asl lsr rol ror）
    pub fn emit_rmw(&mut self, mn: &str, v: &Val, k: u32, c: &str) {
        match v {
            Val::Reg('a') => self.rmw_on_a(mn, c),
            Val::Reg(r) => match mn {
                "inc" => self.emit0(if *r == 'x' { "inx" } else { "iny" }, c),
                "dec" => self.emit0(if *r == 'x' { "dex" } else { "dey" }, c),
                _ => {
                    self.emit0(if *r == 'x' { "txa" } else { "tya" }, "レジスタの値を A に");
                    self.emit(mn, AMode::Acc, c);
                    self.emit0(if *r == 'x' { "tax" } else { "tay" }, "戻す");
                }
            },
            Val::Mem { io: true, rmw: false, .. } => {
                // io(rmw) でない I/O には RMW 命令を使わない（11.2節）
                self.emit_byte("lda", v, k, "I/O を読む（RMW 命令は使わない）");
                match mn {
                    "inc" => {
                        self.emit0("clc", "足し算の準備");
                        self.imm("adc", 1, "1 を足す");
                    }
                    "dec" => {
                        self.emit0("sec", "引き算の準備");
                        self.imm("sbc", 1, "1 を引く");
                    }
                    _ => self.emit(mn, AMode::Acc, c),
                }
                self.store_byte(v, k, "I/O に書く");
            }
            Val::Mem { idx: Idx::Y, .. } => {
                self.emit_byte("lda", v, k, "読む");
                self.rmw_on_a(mn, c);
                self.store_byte(v, k, "書く");
            }
            Val::Mem { idx: Idx::Mem(_), .. } if self.f.xres => {
                self.emit_byte("lda", v, k, "読む");
                self.rmw_on_a(mn, c);
                self.store_byte(v, k, "書く");
            }
            Val::Mem { .. } => self.emit_byte(mn, v, k, c),
            Val::Ind { .. } => {
                self.emit_byte("lda", v, k, "ポインタの指す先を読む");
                self.rmw_on_a(mn, c);
                self.store_byte(v, k, "書く");
            }
            _ => self.unimpl("この値の書き換え", &Span::none()),
        }
    }

    fn rmw_on_a(&mut self, mn: &str, c: &str) {
        match mn {
            "inc" => self.emit("inc", AMode::Acc, c),
            "dec" => self.emit("dec", AMode::Acc, c),
            _ => self.emit(mn, AMode::Acc, c),
        }
    }

    fn is_zero_byte(v: &Val, k: u32) -> bool {
        match v {
            Val::Imm(x) => (x >> (8 * k.min(15))) & 0xFF == 0,
            Val::Ext { from, .. } => k >= *from,
            Val::Mem { size, .. } => k >= *size,
            _ => false,
        }
    }

    pub fn copy_val(&mut self, src: &Val, dst: &Val, n: u32) {
        let mut last_imm: Option<i64> = None;
        for k in 0..n {
            if Gen::is_zero_byte(src, k) && !matches!(dst, Val::Reg(_)) {
                self.store_zero(dst, k, "0 を書く");
                continue;
            }
            if let Val::Imm(x) = src {
                let b = ((x >> (8 * k.min(15))) & 0xFF) as i64;
                if last_imm != Some(b) {
                    self.imm("lda", b, "値");
                    last_imm = Some(b);
                }
                self.store_byte(dst, k, "書く");
                continue;
            }
            last_imm = None;
            self.emit_byte("lda", src, k, "読む");
            self.store_byte(dst, k, "書く");
        }
    }

    // =====================================================================
    // 値を求められた形で渡す
    // =====================================================================

    pub fn gen_a(&mut self, e: &Expr) {
        self.gen(e, &Want::A);
    }
    pub fn gen_cond(&mut self, e: &Expr, l: &str, j: bool) {
        self.gen(e, &Want::Cond(l.to_string(), j));
    }

    pub fn deliver(&mut self, v: &Val, t: &Ty, want: &Want) {
        match want {
            Want::Effect => {}
            Want::A => self.emit_byte("lda", v, 0, "値を A に"),
            Want::Store(d) => {
                let d = d.clone();
                let n = self.sz(t);
                self.copy_val(v, &d, n);
            }
            Want::Cond(l, j) => {
                if let Val::Imm(x) = v {
                    if (*x != 0) == *j {
                        let l = l.clone();
                        self.jmp(&l, "条件はいつも成り立つ");
                    }
                    return;
                }
                let (l, j) = (l.clone(), *j);
                self.emit_byte("lda", v, 0, "bool を読む");
                self.br(if j { "bne" } else { "beq" }, &l, "条件で分かれる");
            }
            Want::Tail => self.ret_val(v, t),
        }
    }

    /// 関数の戻り値を、値 v から返す（15.3節）
    pub fn ret_val(&mut self, v: &Val, t: &Ty) {
        let rt = self.f.ret.clone().unwrap_or(Ty::Unit);
        let _ = t;
        match &rt {
            Ty::Unit | Ty::Never | Ty::Error => self.emit_ret(),
            Ty::Bool => {
                if let Val::Imm(x) = v {
                    self.leave_prefix();
                    self.emit0(if *x != 0 { "sec" } else { "clc" }, "bool をキャリーで返す");
                    self.emit_rts();
                    return;
                }
                self.emit_byte("lda", v, 0, "bool を読む");
                self.leave_prefix();
                self.imm("cmp", 1, "1 ならキャリー 1、0 ならキャリー 0 になる");
                self.emit_rts();
            }
            Ty::Opt(inner) => {
                let inner = (**inner).clone();
                let isz = self.sz(&inner);
                self.emit_byte("lda", v, isz, "Option の有無");
                self.leave_prefix();
                self.imm("cmp", 1, "有無をキャリーに");
                if isz >= 1 && isz <= 3 {
                    let vv = v.part(0, isz);
                    self.load_regs_keep_carry(&vv, isz);
                } else if isz >= 4 {
                    let area = Val::mem(Base::Sym(ret_area(&self.f.label), 0), isz);
                    self.ensure_area(isz);
                    // キャリーを保つため、A だけでコピーする
                    for k in 0..isz {
                        self.emit_byte("lda", v, k, "結果領域へ");
                        self.store_byte(&area, k, "書く");
                    }
                }
                self.emit_rts();
            }
            _ => {
                let n = self.sz(&rt);
                if !self.by_area(&rt) {
                    self.load_regs_keep_carry(v, n);
                    self.leave_prefix();
                    self.emit_rts();
                } else {
                    let area = Val::mem(Base::Sym(ret_area(&self.f.label), 0), n);
                    self.ensure_area(n);
                    self.copy_val(v, &area, n);
                    self.leave_prefix();
                    self.emit_rts();
                }
            }
        }
    }

    /// 結果領域で返す型か（15.3節：4 バイト以上と構造体）
    pub fn by_area(&self, t: &Ty) -> bool {
        matches!(t, Ty::Struct(_) | Ty::Array(_, _) | Ty::StrBuf(_)) || self.sz(t) >= 4
    }

    pub fn ensure_area(&mut self, n: u32) {
        let l = ret_area(&self.f.label);
        let e = self.area_sizes.entry(l).or_insert(0);
        if *e < n {
            *e = n;
        }
    }

    /// A・X・Y に値を入れる（キャリーは変えない）
    pub fn load_regs_keep_carry(&mut self, v: &Val, n: u32) {
        let v = if self.reg_loadable(v) { v.clone() } else { self.materialize_val(v, n) };
        if n >= 3 {
            self.load_reg('y', &v, 2);
        }
        if n >= 2 {
            self.load_reg('x', &v, 1);
        }
        self.load_reg('a', &v, 0);
    }

    pub fn reg_loadable(&self, v: &Val) -> bool {
        match v {
            Val::Imm(_) | Val::Addr(_) => true,
            Val::Mem { idx: Idx::None, .. } => true,
            Val::Ext { inner, .. } => self.reg_loadable(inner),
            Val::Reg(_) => true,
            _ => false,
        }
    }

    pub fn load_reg(&mut self, r: char, v: &Val, k: u32) {
        if r == 'a' {
            self.emit_byte("lda", v, k, "A に");
            return;
        }
        let mn = if r == 'x' { "ldx" } else { "ldy" };
        match v {
            Val::Imm(x) => {
                let b = ((x >> (8 * k.min(15))) & 0xFF) as i64;
                self.imm(mn, b, "値");
            }
            Val::Addr(b) => {
                let e = match k {
                    0 => b.expr().lo(),
                    1 => b.expr().hi(),
                    _ => OpExpr::num(0),
                };
                self.emit(mn, AMode::Imm(e), "アドレス");
            }
            Val::Mem { base, idx: Idx::None, stride, size, .. } => {
                if k >= *size {
                    self.imm(mn, 0, "0");
                } else {
                    self.emit(mn, AMode::Mem(base.add(k as i64 * stride).expr()), "読む");
                }
            }
            Val::Ext { inner, from } => {
                if k < *from {
                    let inner = (**inner).clone();
                    self.load_reg(r, &inner, k);
                } else {
                    self.imm(mn, 0, "0");
                }
            }
            Val::Reg('a') => self.emit0(if r == 'x' { "tax" } else { "tay" }, "A の値を移す"),
            Val::Reg(x) => {
                if *x != r {
                    self.unimpl("レジスタからレジスタへの受け渡し", &Span::none());
                }
            }
            _ => self.unimpl("この値をレジスタに入れること", &Span::none()),
        }
    }

    /// 2〜3 バイトの値を A・X・Y に
    pub fn gen_regs(&mut self, e: &Expr, n: u32) {
        if n <= 1 {
            self.gen_a(e);
            return;
        }
        if let ExprKind::Call(_, _) = &e.kind {
            if let Some(CallTarget::Fn(_)) = self.ctx.calls.get(&e.id) {
                let conv = self.gen_call(e);
                if let RetConv::Regs(m) = conv {
                    if m == n {
                        return;
                    }
                }
                let v = self.conv_to_val(&conv, n);
                self.load_regs_keep_carry(&v, n);
                return;
            }
        }
        let v = self.val_of(e);
        self.load_regs_keep_carry(&v, n);
    }

    // =====================================================================
    // 式の末端
    // =====================================================================

    pub fn gen_leaf(&mut self, e: &Expr, want: &Want) {
        match want {
            Want::Effect => self.leaf_effect(e),
            Want::A => self.leaf_a(e),
            Want::Store(v) => {
                let v = v.clone();
                self.leaf_store(e, &v)
            }
            Want::Tail => self.leaf_tail(e),
            Want::Cond(l, j) => {
                let (l, j) = (l.clone(), *j);
                self.leaf_cond(e, &l, j)
            }
        }
    }

    fn leaf_effect(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Assign(op, l, r) => self.gen_assign(*op, l, r),
            ExprKind::Call(_, _) => {
                let _ = self.gen_call(e);
            }
            ExprKind::MethodCall(r, name, args) => match name.as_str() {
                "insert" | "remove" | "toggle" => self.gen_flag_op(r, name, args),
                _ => {
                    if self.has_effects(e) {
                        let _ = self.materialize(e);
                    }
                }
            },
            ExprKind::Try(x) => {
                let _ = self.gen_try(x);
            }
            ExprKind::Path(_) => {
                if let Some(Res::Global(g)) = self.ctx.res.get(&e.id).cloned() {
                    if let GKind::Io(_) = self.ctx.globals[g].kind {
                        let v = self.val_of(e);
                        self.emit_byte("lda", &v, 0, "I/O を読む");
                    }
                }
            }
            ExprKind::Unit | ExprKind::Int(_, _) | ExprKind::Bool(_) | ExprKind::Char(_, _) | ExprKind::Str(_, _, _) => {}
            _ => {
                if self.has_effects(e) {
                    let _ = self.materialize(e);
                }
            }
        }
    }

    fn bool_to_a(&mut self, e: &Expr) {
        let l1 = self.new_label("t");
        let l2 = self.new_label("b");
        self.gen_cond(e, &l1, true);
        self.imm("lda", 0, "false");
        self.jmp(&l2, "");
        self.label(&l1);
        self.imm("lda", 1, "true");
        self.label(&l2);
    }

    fn leaf_a(&mut self, e: &Expr) {
        let t = self.ty_of(e);
        if let Some(c) = self.const_int(e) {
            self.imm("lda", (c & 0xFF) as i64, "定数");
            return;
        }
        if t == Ty::Bool {
            match &e.kind {
                ExprKind::Path(_) | ExprKind::Field(_, _) | ExprKind::Index(_, _) => {
                    let v = self.val_of(e);
                    self.emit_byte("lda", &v, 0, "bool を読む");
                }
                ExprKind::Call(_, _) => {
                    let conv = self.gen_call(e);
                    self.conv_to_a(&conv);
                }
                _ => self.bool_to_a(e),
            }
            return;
        }
        match &e.kind {
            ExprKind::Path(_) | ExprKind::Field(_, _) | ExprKind::Index(_, _) | ExprKind::Str(_, _, _) => {
                let v = self.val_of(e);
                self.emit_byte("lda", &v, 0, "読む");
            }
            ExprKind::Unary(UnOp::Not, x) => {
                self.gen_a(x);
                self.imm("eor", 0xFF, "ビットを反転する");
            }
            ExprKind::Unary(UnOp::Neg, x) => {
                self.gen_a(x);
                self.imm("eor", 0xFF, "符号を反転する（ビット反転）");
                self.emit0("clc", "");
                self.imm("adc", 1, "1 を足す");
            }
            ExprKind::Unary(UnOp::Addr, _) => {
                let v = self.val_of(e);
                self.emit_byte("lda", &v, 0, "アドレスの下位");
            }
            ExprKind::Binary(op, a, b) => self.binop_a(*op, a, b, &t, e),
            ExprKind::Cast(x, _) => {
                let from = self.ty_of(x);
                if from == Ty::Bool || self.sz(&from) == 1 {
                    self.gen_a(x);
                } else {
                    let v = self.val_of(x);
                    self.emit_byte("lda", &v, 0, "下位のバイト");
                }
            }
            ExprKind::Call(_, _) => {
                if let Some(CallTarget::TypeFn(tt, name)) = self.ctx.calls.get(&e.id).cloned() {
                    self.type_fn_a(e, &tt, &name);
                    return;
                }
                let conv = self.gen_call(e);
                self.conv_to_a(&conv);
            }
            ExprKind::MethodCall(r, name, args) => self.method_a(e, r, name, args),
            ExprKind::Try(x) => {
                let v = self.gen_try(x);
                self.emit_byte("lda", &v, 0, "Some の値");
            }
            _ => {
                let v = self.materialize(e);
                self.emit_byte("lda", &v, 0, "値");
            }
        }
    }

    fn leaf_store(&mut self, e: &Expr, dest: &Val) {
        let t = self.ty_of(e);
        let n = self.sz(&t);
        if n == 0 {
            self.leaf_effect(e);
            return;
        }
        if let Ty::Opt(_) = &t {
            self.store_option(e, dest);
            return;
        }
        if n == 1 && !matches!(dest, Val::Mem { .. } | Val::Ind { .. } | Val::Reg(_)) {
            self.unimpl("この場所への書き込み", &e.span);
            return;
        }
        if n == 1 {
            if let Some(0) = self.const_int(e) {
                self.store_zero(dest, 0, "0 を書く");
                return;
            }
            self.gen_a(e);
            self.store_byte(dest, 0, "書く");
            return;
        }
        // 定数（配列・構造体を含む）
        if let Some(v) = self.ctx.econst.get(&e.id).cloned() {
            let is_addr_ty = matches!(t, Ty::Str(_, _) | Ty::Ptr(_));
            if !is_addr_ty && !matches!(v, Value::Func(_)) {
                let bytes = self.ctx.value_bytes(&v, &t);
                let mut last: Option<u8> = None;
                for (k, b) in bytes.iter().enumerate() {
                    if *b == 0 {
                        self.store_zero(dest, k as u32, "0 を書く");
                    } else {
                        if last != Some(*b) {
                            self.imm("lda", *b as i64, "定数");
                            last = Some(*b);
                        }
                        self.store_byte(dest, k as u32, "書く");
                    }
                }
                return;
            }
        }
        match &e.kind {
            ExprKind::Path(_) | ExprKind::Field(_, _) | ExprKind::Index(_, _) | ExprKind::Str(_, _, _) | ExprKind::Unary(UnOp::Addr, _) => {
                let v = self.val_of(e);
                self.copy_val(&v, dest, n);
            }
            ExprKind::Unary(UnOp::Not, x) => {
                let v = self.val_of(x);
                for k in 0..n {
                    self.emit_byte("lda", &v, k, "読む");
                    self.imm("eor", 0xFF, "ビットを反転する");
                    self.store_byte(dest, k, "書く");
                }
            }
            ExprKind::Unary(UnOp::Neg, x) => {
                let v = self.val_of(x);
                self.emit0("sec", "0 から引く");
                for k in 0..n {
                    self.imm("lda", 0, "0");
                    self.emit_byte("sbc", &v, k, "引く");
                    self.store_byte(dest, k, "書く");
                }
            }
            ExprKind::Binary(op, a, b) => self.binop_store(*op, a, b, &t, dest, e),
            ExprKind::Cast(x, _) => self.cast_store(x, &t, dest),
            ExprKind::Call(_, _) => {
                if let Some(CallTarget::TypeFn(tt, name)) = self.ctx.calls.get(&e.id).cloned() {
                    if name == "unchecked" || name == "wrap" {
                        if let ExprKind::Call(_, args) = &e.kind {
                            if let Some(a) = args.first() {
                                let v = self.val_of(&a.value);
                                self.copy_val(&v, dest, n);
                                return;
                            }
                        }
                    }
                    let _ = tt;
                    self.unimpl(&format!("2 バイトの値の {}", name), &e.span);
                    return;
                }
                let conv = self.gen_call(e);
                self.conv_store(&conv, dest, n);
            }
            ExprKind::StructLit(_, fields, base) => {
                let sid = match &t {
                    Ty::Struct(s) => *s,
                    _ => return,
                };
                let def = self.ctx.types.structs[sid].clone();
                let mut done = vec![false; def.fields.len()];
                for (fname, fe, _) in fields {
                    if let Some(i) = def.fields.iter().position(|f| &f.0 == fname) {
                        let (off, ft) = self.ctx.types.field(sid, fname).unwrap_or((0, Ty::Error));
                        let fsz = self.sz(&ft);
                        let fd = dest.part(off, fsz);
                        self.gen(fe, &Want::Store(fd));
                        done[i] = true;
                    }
                }
                if let Some(b) = base {
                    if let Some(Value::Struct(_, bf)) = self.ctx.econst.get(&b.id).cloned() {
                        for (i, d) in done.iter().enumerate() {
                            if !*d {
                                let (fname, ft) = def.fields[i].clone();
                                let (off, _) = self.ctx.types.field(sid, &fname).unwrap_or((0, Ty::Error));
                                let fsz = self.sz(&ft);
                                let bytes = self.ctx.value_bytes(&bf[i], &ft);
                                let fd = dest.part(off, fsz);
                                let v = Val::Imm(bytes.iter().enumerate().fold(0i128, |acc, (k, b)| acc | ((*b as i128) << (8 * k))));
                                self.copy_val(&v, &fd, fsz);
                            }
                        }
                    }
                }
            }
            ExprKind::ArrayList(items) => {
                let et = match &t {
                    Ty::Array(et, _) => (**et).clone(),
                    _ => return,
                };
                let es = self.sz(&et);
                let cnt = items.len() as i64;
                let planar = self.array_planar(&et);
                for (i, x) in items.iter().enumerate() {
                    let d = match dest {
                        Val::Mem { base, idx, stride, io, rmw, .. } => {
                            if planar {
                                Val::Mem { base: base.add(i as i64 * stride), idx: idx.clone(), stride: stride * cnt, size: es, io: *io, rmw: *rmw }
                            } else {
                                Val::Mem { base: base.add(i as i64 * es as i64 * stride), idx: idx.clone(), stride: *stride, size: es, io: *io, rmw: *rmw }
                            }
                        }
                        _ => {
                            self.unimpl("この場所への配列の書き込み", &e.span);
                            return;
                        }
                    };
                    self.gen(x, &Want::Store(d));
                }
            }
            ExprKind::MethodCall(r, name, args) => self.method_store(e, r, name, args, dest, &t),
            ExprKind::Try(x) => {
                let v = self.gen_try(x);
                self.copy_val(&v, dest, n);
            }
            _ => self.unimpl(&format!("{} の値の計算", self.ctx.tyname(&t)), &e.span),
        }
    }

    fn leaf_cond(&mut self, e: &Expr, l: &str, j: bool) {
        if let Some(Value::Bool(b)) = self.ctx.econst.get(&e.id).cloned() {
            if b == j {
                self.jmp(l, "条件はいつも成り立つ");
            }
            return;
        }
        match &e.kind {
            ExprKind::Unary(UnOp::Not, x) => self.gen_cond(x, l, !j),
            ExprKind::Binary(BinOp::LAnd, a, b) => {
                if j {
                    let skip = self.new_label("and");
                    self.gen_cond(a, &skip, false);
                    self.gen_cond(b, l, true);
                    self.label(&skip);
                } else {
                    self.gen_cond(a, l, false);
                    self.gen_cond(b, l, false);
                }
            }
            ExprKind::Binary(BinOp::LOr, a, b) => {
                if j {
                    self.gen_cond(a, l, true);
                    self.gen_cond(b, l, true);
                } else {
                    let skip = self.new_label("or");
                    self.gen_cond(a, &skip, true);
                    self.gen_cond(b, l, false);
                    self.label(&skip);
                }
            }
            ExprKind::Binary(op, a, b) if op.is_cmp() => self.gen_cmp(*op, a, b, l, j),
            ExprKind::Call(_, _) => {
                let conv = self.gen_call(e);
                match conv {
                    RetConv::Carry => self.br(if j { "bcs" } else { "bcc" }, l, "bool はキャリーで返る"),
                    other => {
                        self.conv_to_a(&other);
                        self.br(if j { "bne" } else { "beq" }, l, "");
                    }
                }
            }
            ExprKind::MethodCall(r, name, args) if name == "contains" => self.gen_contains(r, args, l, j),
            ExprKind::Path(_) | ExprKind::Field(_, _) | ExprKind::Index(_, _) => {
                let v = self.val_of(e);
                self.emit_byte("lda", &v, 0, "bool を読む");
                self.br(if j { "bne" } else { "beq" }, l, "条件で分かれる");
            }
            _ => {
                self.gen_a(e);
                self.br(if j { "bne" } else { "beq" }, l, "条件で分かれる");
            }
        }
    }

    fn leaf_tail(&mut self, e: &Expr) {
        let rt = self.f.ret.clone().unwrap_or(Ty::Unit);
        let t = self.ty_of(e);
        if t == Ty::Never {
            self.gen(e, &Want::Effect);
            return;
        }
        match &rt {
            Ty::Unit | Ty::Never | Ty::Error => {
                self.leaf_effect(e);
                self.emit_ret();
            }
            Ty::Bool => {
                if let ExprKind::Call(_, _) = &e.kind {
                    if self.f.atomic == 0 {
                        if let Some(CallTarget::Fn(f)) = self.ctx.calls.get(&e.id).cloned() {
                            if self.ctx.funcs[f].ret == Ty::Bool {
                                let _ = self.gen_call(e);
                                self.emit_rts();
                                return;
                            }
                        }
                    }
                }
                let lt = self.new_label("rtrue");
                self.gen_cond(e, &lt, true);
                self.leave_prefix();
                self.emit0("clc", "false（キャリー 0）");
                self.emit_rts();
                self.label(&lt);
                self.leave_prefix();
                self.emit0("sec", "true（キャリー 1）");
                self.emit_rts();
            }
            Ty::Opt(inner) => {
                let inner = (**inner).clone();
                let isz = self.sz(&inner);
                if let Some(Value::Opt(None)) = self.ctx.econst.get(&e.id) {
                    self.leave_prefix();
                    self.emit0("clc", "None（キャリー 0）");
                    self.emit_rts();
                    return;
                }
                if let ExprKind::Call(_, args) = &e.kind {
                    match self.ctx.calls.get(&e.id).cloned() {
                        Some(CallTarget::SomeCtor) => {
                            if let Some(a) = args.first() {
                                let x = a.value.clone();
                                self.regs_for(&x, &inner);
                            }
                            self.leave_prefix();
                            self.emit0("sec", "Some（キャリー 1）");
                            self.emit_rts();
                            return;
                        }
                        Some(CallTarget::Fn(f)) if self.f.atomic == 0 && self.ctx.funcs[f].ret == rt => {
                            let _ = self.gen_call(e);
                            self.emit_rts();
                            return;
                        }
                        _ => {}
                    }
                }
                let l_none = self.new_label("rnone");
                let v = self.gen_opt_test(e, &l_none);
                if isz >= 1 && isz <= 3 {
                    self.load_regs_keep_carry(&v, isz);
                } else if isz >= 4 {
                    self.ensure_area(isz);
                    let area = Val::mem(Base::Sym(ret_area(&self.f.label), 0), isz);
                    self.copy_val(&v, &area, isz);
                }
                self.leave_prefix();
                self.emit0("sec", "Some（キャリー 1）");
                self.emit_rts();
                self.label(&l_none);
                self.leave_prefix();
                self.emit0("clc", "None（キャリー 0）");
                self.emit_rts();
            }
            _ => {
                let n = self.sz(&rt);
                if !self.by_area(&rt) {
                    if let ExprKind::Call(_, _) = &e.kind {
                        if let Some(CallTarget::Fn(f)) = self.ctx.calls.get(&e.id).cloned() {
                            if self.ctx.funcs[f].ret == rt && self.f.atomic == 0 {
                                let _ = self.gen_call(e);
                                self.emit_rts();
                                return;
                            }
                        }
                    }
                    self.gen_regs(e, n);
                    self.leave_prefix();
                    self.emit_rts();
                } else {
                    self.ensure_area(n);
                    let area = Val::mem(Base::Sym(ret_area(&self.f.label), 0), n);
                    self.gen(e, &Want::Store(area));
                    self.leave_prefix();
                    self.emit_rts();
                }
            }
        }
    }

    /// Option の中身を返す形でレジスタに
    fn regs_for(&mut self, x: &Expr, inner: &Ty) {
        let isz = self.sz(inner);
        if isz == 0 {
            self.gen(x, &Want::Effect);
        } else if isz <= 3 {
            self.gen_regs(x, isz);
        } else {
            self.ensure_area(isz);
            let area = Val::mem(Base::Sym(ret_area(&self.f.label), 0), isz);
            self.gen(x, &Want::Store(area));
        }
    }

    // =====================================================================
    // 演算
    // =====================================================================

    /// 二項演算の両辺の値（評価の順序を保つ）
    fn two_vals(&mut self, a: &Expr, b: &Expr) -> (Val, Val) {
        if self.is_simple(a) && self.is_simple(b) {
            let va = self.val_of(a);
            let vb = self.val_of(b);
            return (va, vb);
        }
        if self.is_simple(a) && (self.const_int(a).is_some() || !self.has_effects(b)) {
            let vb = self.materialize(b);
            let va = self.val_of(a);
            return (va, vb);
        }
        let va = if self.is_simple(a) { self.val_of(a) } else { self.materialize(a) };
        let va = if self.has_effects(b) && !matches!(va, Val::Imm(_)) && self.is_simple(a) { self.materialize_val(&va, self.val_size(&va)) } else { va };
        let vb = if self.is_simple(b) { self.val_of(b) } else { self.materialize(b) };
        (va, vb)
    }

    fn val_size(&self, v: &Val) -> u32 {
        match v {
            Val::Mem { size, .. } | Val::Ind { size, .. } => *size,
            _ => 1,
        }
    }

    fn binop_a(&mut self, op: BinOp, a: &Expr, b: &Expr, t: &Ty, e: &Expr) {
        match op {
            BinOp::Add | BinOp::Sub | BinOp::And | BinOp::Or | BinOp::Xor => {
                let bcd = matches!(t, Ty::Bcd(_));
                let mn = match op {
                    BinOp::Add => "adc",
                    BinOp::Sub => "sbc",
                    BinOp::And => "and",
                    BinOp::Or => "ora",
                    _ => "eor",
                };
                let vb = if self.is_simple(b) {
                    None
                } else if self.is_simple(a) && (self.const_int(a).is_some() || !self.has_effects(b)) {
                    Some(self.materialize(b))
                } else {
                    let ta = self.materialize(a);
                    let tb = self.materialize(b);
                    if bcd {
                        self.emit0("sed", "10進モードにする");
                    }
                    self.emit_byte("lda", &ta, 0, "左の値");
                    self.carry_prep(op);
                    self.emit_byte(mn, &tb, 0, &format!("{} する", op.text()));
                    if bcd {
                        self.emit0("cld", "10進モードを解除する");
                    }
                    return;
                };
                if bcd {
                    self.emit0("sed", "10進モードにする");
                }
                self.gen_a(a);
                self.carry_prep(op);
                let vb = match vb {
                    Some(v) => v,
                    None => self.val_of(b),
                };
                self.emit_byte(mn, &vb, 0, &format!("{} する", op.text()));
                if bcd {
                    self.emit0("cld", "10進モードを解除する");
                }
            }
            BinOp::Shl | BinOp::Shr => {
                let k = self.const_int(b).unwrap_or(0);
                let signed = self.ctx.types.is_signed(t);
                self.gen_a(a);
                if k >= 8 {
                    if op == BinOp::Shr && signed {
                        let l = self.new_label("sgn");
                        self.imm("and", 0x80, "符号");
                        self.br("beq", &l, "正なら 0");
                        self.imm("lda", 0xFF, "負なら $FF");
                        self.label(&l);
                    } else {
                        self.imm("lda", 0, "すべてはみ出す");
                    }
                    return;
                }
                for _ in 0..k {
                    if op == BinOp::Shl {
                        self.emit0("asl", "左に 1 ビットずらす");
                    } else if signed {
                        self.imm("cmp", 0x80, "符号をキャリーに");
                        self.emit0("ror", "符号を保って右に 1 ビットずらす");
                    } else {
                        self.emit0("lsr", "右に 1 ビットずらす");
                    }
                }
            }
            BinOp::Mul => {
                let (x, c) = match (self.const_int(a), self.const_int(b)) {
                    (_, Some(c)) => (a, c),
                    (Some(c), _) => (b, c),
                    _ => {
                        self.unimpl("変数同士の掛け算", &e.span);
                        return;
                    }
                };
                let c = c & 0xFF;
                if c == 0 {
                    self.imm("lda", 0, "0 を掛ける");
                    return;
                }
                if is_pow2(c) {
                    self.gen_a(x);
                    for _ in 0..log2(c) {
                        self.emit0("asl", "2 倍する");
                    }
                    return;
                }
                let xv0 = if self.is_simple(x) { self.val_of(x) } else { self.materialize(x) };
                // 何度も読む値は、I/O・ポインタ・レジスタなら一時領域に移す（先に判定してから動かす）
                let need_copy = matches!(&xv0, Val::Mem { io: true, .. } | Val::Ind { .. } | Val::Reg(_));
                let xv = if need_copy { self.materialize_val(&xv0, 1) } else { xv0 };
                // ホーナー法：上のビットから
                let bits: Vec<bool> = (0..8).rev().map(|i| (c >> i) & 1 == 1).collect();
                let first = bits.iter().position(|b| *b).unwrap_or(7);
                self.emit_byte("lda", &xv, 0, &format!("× {}：シフトと足し算に展開する", c));
                for bit in bits.iter().skip(first + 1) {
                    self.emit0("asl", "2 倍する");
                    if *bit {
                        self.emit0("clc", "");
                        self.emit_byte("adc", &xv, 0, "元の値を足す");
                    }
                }
                self.notes.push(format!("定数の掛け算 × {}（{}行）：シフトと足し算に展開", c, e.span.line));
            }
            BinOp::Div | BinOp::Rem => {
                let c = self.const_int(b).unwrap_or(1);
                if self.ctx.types.is_signed(t) {
                    self.unimpl("符号付きの / と %", &e.span);
                    return;
                }
                self.gen_a(a);
                if op == BinOp::Div {
                    for _ in 0..log2(c) {
                        self.emit0("lsr", "2 で割る");
                    }
                } else {
                    self.imm("and", (c - 1) as i64, "余り（下位のビット）");
                }
            }
            _ => self.bool_to_a(e),
        }
    }

    fn carry_prep(&mut self, op: BinOp) {
        match op {
            BinOp::Add => self.emit0("clc", "足し算の準備"),
            BinOp::Sub => self.emit0("sec", "引き算の準備"),
            _ => {}
        }
    }

    fn binop_store(&mut self, op: BinOp, a: &Expr, b: &Expr, t: &Ty, dest: &Val, e: &Expr) {
        let n = self.sz(t);
        match op {
            BinOp::Add | BinOp::Sub | BinOp::And | BinOp::Or | BinOp::Xor => {
                let (va, vb) = self.two_vals(a, b);
                self.arith_store(op, &va, &vb, t, dest, n);
            }
            BinOp::Shl | BinOp::Shr => {
                let k = self.const_int(b).unwrap_or(0) as u32;
                let va = self.val_of(a);
                self.shift_store(op, &va, k, t, dest, n);
            }
            BinOp::Mul => {
                let (x, c) = match (self.const_int(a), self.const_int(b)) {
                    (_, Some(c)) => (a, c),
                    (Some(c), _) => (b, c),
                    _ => {
                        self.unimpl("変数同士の掛け算", &e.span);
                        return;
                    }
                };
                let xv = self.val_of(x);
                self.mul_const_store(&xv, c, t, dest, n);
                self.notes.push(format!("定数の掛け算 × {}（{}行、{} バイト）：シフトと足し算に展開", c, e.span.line, n));
            }
            BinOp::Div => {
                if self.ctx.types.is_signed(t) {
                    self.unimpl("符号付きの / と %", &e.span);
                    return;
                }
                let c = self.const_int(b).unwrap_or(1);
                let va = self.val_of(a);
                self.shift_store(BinOp::Shr, &va, log2(c), t, dest, n);
            }
            BinOp::Rem => {
                if self.ctx.types.is_signed(t) {
                    self.unimpl("符号付きの / と %", &e.span);
                    return;
                }
                let c = self.const_int(b).unwrap_or(1);
                let va = self.val_of(a);
                let mask = Val::Imm(c - 1);
                self.arith_store(BinOp::And, &va, &mask, t, dest, n);
            }
            _ => self.unimpl("この演算", &e.span),
        }
    }

    pub fn arith_store(&mut self, op: BinOp, va: &Val, vb: &Val, t: &Ty, dest: &Val, n: u32) {
        let bcd = matches!(t, Ty::Bcd(_));
        let mn = match op {
            BinOp::Add => "adc",
            BinOp::Sub => "sbc",
            BinOp::And => "and",
            BinOp::Or => "ora",
            _ => "eor",
        };
        if bcd {
            self.emit0("sed", "10進モードにする（BCD の計算）");
        }
        for k in 0..n {
            self.emit_byte("lda", va, k, "左の値");
            if k == 0 {
                self.carry_prep(op);
            }
            self.emit_byte(mn, vb, k, &format!("{} する", op.text()));
            self.store_byte(dest, k, "書く");
        }
        if bcd {
            self.emit0("cld", "10進モードを解除する");
        }
    }

    fn plain_mem(v: &Val) -> bool {
        matches!(v, Val::Mem { idx: Idx::None, io: false, .. } | Val::Mem { idx: Idx::X, io: false, .. })
    }

    fn shift_store(&mut self, op: BinOp, va: &Val, k: u32, t: &Ty, dest: &Val, n: u32) {
        let signed = self.ctx.types.is_signed(t);
        let work = if Gen::plain_mem(dest) { dest.clone() } else { self.tmp_val(n) };
        self.copy_val(va, &work, n);
        let k = k.min(8 * n);
        for _ in 0..k {
            if op == BinOp::Shl {
                for i in 0..n {
                    self.emit_rmw(if i == 0 { "asl" } else { "rol" }, &work, i, "左に 1 ビットずらす");
                }
            } else {
                if signed {
                    self.emit_byte("lda", &work, n - 1, "最上位");
                    self.imm("cmp", 0x80, "符号をキャリーに");
                    for i in (0..n).rev() {
                        self.emit_rmw("ror", &work, i, "右に 1 ビットずらす");
                    }
                } else {
                    for i in (0..n).rev() {
                        self.emit_rmw(if i == n - 1 { "lsr" } else { "ror" }, &work, i, "右に 1 ビットずらす");
                    }
                }
            }
        }
        if work != *dest {
            self.copy_val(&work, dest, n);
        }
    }

    fn mul_const_store(&mut self, xv: &Val, c: i128, t: &Ty, dest: &Val, n: u32) {
        let mask = if n >= 16 { -1i128 } else { (1i128 << (8 * n)) - 1 };
        let c = c & mask;
        let tmpx = self.tmp_val(n);
        self.copy_val(xv, &tmpx, n);
        let acc = if Gen::plain_mem(dest) { dest.clone() } else { self.tmp_val(n) };
        for k in 0..n {
            self.store_zero(&acc, k, "結果を 0 から");
        }
        let bits = 8 * n;
        let mut last_bit = 0;
        for i in 0..bits {
            if (c >> i) & 1 == 1 {
                last_bit = i;
            }
        }
        for i in 0..=last_bit {
            if (c >> i) & 1 == 1 {
                let a2 = acc.clone();
                self.arith_store(BinOp::Add, &a2, &tmpx, t, &acc, n);
            }
            if i < last_bit {
                for j in 0..n {
                    self.emit_rmw(if j == 0 { "asl" } else { "rol" }, &tmpx, j, "2 倍する");
                }
            }
        }
        if acc != *dest {
            self.copy_val(&acc, dest, n);
        }
    }

    fn cast_store(&mut self, x: &Expr, to: &Ty, dest: &Val) {
        let from = self.ty_of(x);
        let fs = self.sz(&from).max(1);
        let ts = self.sz(to);
        if ts <= fs {
            let v = self.val_of(x);
            self.copy_val(&v, dest, ts);
            return;
        }
        let v = self.val_of(x);
        if fs == 1 && !matches!(v, Val::Mem { .. } | Val::Imm(_)) {
            self.gen_a(x);
            self.store_byte(dest, 0, "下位");
        } else {
            self.copy_val(&v, dest, fs);
        }
        if self.ctx.types.is_signed(&from) {
            // 符号を広げる
            self.emit_byte("lda", dest, fs - 1, "最上位のバイト");
            self.emit0("asl", "符号をキャリーに");
            self.imm("lda", 0xFF, "");
            self.imm("adc", 0, "負なら $00、正なら $FF");
            self.imm("eor", 0xFF, "反転して符号のバイトにする");
            for k in fs..ts {
                self.store_byte(dest, k, "符号を広げる");
            }
        } else {
            for k in fs..ts {
                self.store_zero(dest, k, "上位は 0");
            }
        }
    }

    // ---- 比較 ----
    fn gen_cmp(&mut self, op: BinOp, a: &Expr, b: &Expr, l: &str, j: bool) {
        let mut t = self.ty_of(a);
        if self.const_int(a).is_some() && self.const_int(b).is_none() {
            t = self.ty_of(b);
        }
        let n = self.sz(&t).max(1);
        let signed = self.ctx.types.is_signed(&t);
        // 定数との比較を整える：a > c → a >= c+1 など
        let cb = self.const_int(b);
        let (op, cb) = match (op, cb) {
            (BinOp::Gt, Some(c)) if !signed && n == 1 && c < 255 => (BinOp::Ge, Some(c + 1)),
            (BinOp::Le, Some(c)) if !signed && n == 1 && c < 255 => (BinOp::Lt, Some(c + 1)),
            (BinOp::Gt, Some(c)) if !signed && n == 1 && c >= 255 => {
                // いつも成り立たない
                if !j {
                    self.jmp(l, "");
                }
                return;
            }
            (BinOp::Le, Some(c)) if !signed && n == 1 && c >= 255 => {
                if j {
                    self.jmp(l, "");
                }
                return;
            }
            (o, c) => (o, c),
        };
        let (va, vb) = if let Some(c) = cb {
            // 1 バイトの式と定数：式の値を A に作って、そのまま比べる（一時領域に置かない）
            let va = if n == 1 && !self.is_simple(a) && matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Ge) {
                self.gen_a(a);
                Val::Reg('a')
            } else {
                self.val_of(a)
            };
            (va, Val::Imm(c))
        } else {
            self.two_vals(a, b)
        };
        let (op, va, vb) = match op {
            BinOp::Gt => (BinOp::Lt, vb, va),
            BinOp::Le => (BinOp::Ge, vb, va),
            o => (o, va, vb),
        };
        match op {
            BinOp::Eq | BinOp::Ne => {
                let to_l_on_eq = (op == BinOp::Eq) == j;
                if to_l_on_eq {
                    // すべて等しければ l へ
                    let skip = self.new_label("ne");
                    for k in 0..n {
                        let zero = matches!(&vb, Val::Imm(x) if (x >> (8 * k)) & 0xFF == 0);
                        if !(n == 1 && !zero && self.a_holds(&va)) {
                            self.emit_byte("lda", &va, k, "左の値");
                        }
                        // 0 と比べるとき：N と Z が A の値を表していれば cmp は要らない
                        if !(zero && self.f.nz_a) {
                            self.emit_byte("cmp", &vb, k, "比べる");
                        }
                        if k + 1 < n {
                            self.br("bne", &skip, "違う");
                        } else {
                            self.br("beq", l, "等しい");
                        }
                    }
                    self.label(&skip);
                } else {
                    for k in 0..n {
                        let zero = matches!(&vb, Val::Imm(x) if (x >> (8 * k)) & 0xFF == 0);
                        if !(n == 1 && !zero && self.a_holds(&va)) {
                            self.emit_byte("lda", &va, k, "左の値");
                        }
                        // 0 と比べるとき：N と Z が A の値を表していれば cmp は要らない
                        if !(zero && self.f.nz_a) {
                            self.emit_byte("cmp", &vb, k, "比べる");
                        }
                        self.br("bne", l, "違う");
                    }
                }
            }
            BinOp::Lt | BinOp::Ge => {
                let less_true = op == BinOp::Lt;
                if !signed {
                    if !(n == 1 && self.a_holds(&va)) {
                        self.emit_byte("lda", &va, 0, "左の値");
                    }
                    self.emit_byte("cmp", &vb, 0, "比べる");
                    for k in 1..n {
                        self.emit_byte("lda", &va, k, "左の値（上位）");
                        self.emit_byte("sbc", &vb, k, "借りを伝えて比べる");
                    }
                    // キャリー 0 なら a < b
                    let mn = if less_true == j { "bcc" } else { "bcs" };
                    self.br(mn, l, "大小で分かれる");
                } else {
                    self.emit_byte("lda", &va, 0, "左の値");
                    if n == 1 {
                        self.emit0("sec", "符号付きの比較");
                        self.emit_byte("sbc", &vb, 0, "引く");
                    } else {
                        self.emit_byte("cmp", &vb, 0, "比べる");
                        for k in 1..n {
                            self.emit_byte("lda", &va, k, "左の値（上位）");
                            self.emit_byte("sbc", &vb, k, "借りを伝えて比べる");
                        }
                    }
                    let lv = self.new_label("v");
                    self.br("bvc", &lv, "あふれていなければ N がそのまま");
                    self.imm("eor", 0x80, "あふれたら N を反転");
                    self.label(&lv);
                    let mn = if less_true == j { "bmi" } else { "bpl" };
                    self.br(mn, l, "大小で分かれる");
                }
            }
            _ => {}
        }
    }

    fn gen_contains(&mut self, r: &Expr, args: &[Arg], l: &str, j: bool) {
        let rv = self.val_of(r);
        let size = self.sz(&self.ty_of(r));
        let m = args.first().and_then(|a| self.const_int(&a.value));
        match m {
            Some(mask) => {
                if size == 1 {
                    let simple_mem = matches!(rv, Val::Mem { idx: Idx::None, .. });
                    if mask == 0x80 && simple_mem {
                        self.emit_byte("bit", &rv, 0, "ビット 7 を N で調べる");
                        self.br(if j { "bmi" } else { "bpl" }, l, "立っているかで分かれる");
                        return;
                    }
                    if mask == 0x40 && simple_mem {
                        self.emit_byte("bit", &rv, 0, "ビット 6 を V で調べる");
                        self.br(if j { "bvs" } else { "bvc" }, l, "立っているかで分かれる");
                        return;
                    }
                    self.emit_byte("lda", &rv, 0, "読む");
                    self.imm("and", (mask & 0xFF) as i64, "ビットを取り出す");
                    if is_pow2(mask) {
                        self.br(if j { "bne" } else { "beq" }, l, "立っているかで分かれる");
                    } else {
                        self.imm("cmp", (mask & 0xFF) as i64, "すべて立っているか");
                        self.br(if j { "beq" } else { "bne" }, l, "立っているかで分かれる");
                    }
                } else {
                    let fail = self.new_label("nc");
                    for k in 0..size {
                        let mk = ((mask >> (8 * k)) & 0xFF) as i64;
                        if mk == 0 {
                            continue;
                        }
                        self.emit_byte("lda", &rv, k, "読む");
                        self.imm("and", mk, "ビットを取り出す");
                        self.imm("cmp", mk, "すべて立っているか");
                        if j {
                            self.br("bne", &fail, "立っていない");
                        } else {
                            self.br("bne", l, "立っていない");
                        }
                    }
                    if j {
                        self.jmp(l, "すべて立っている");
                    }
                    self.label(&fail);
                }
            }
            None => {
                let a = &args[0].value;
                let mv = if self.is_simple(a) { self.val_of(a) } else { self.materialize(a) };
                for k in 0..size {
                    self.emit_byte("lda", &rv, k, "読む");
                    self.emit_byte("and", &mv, k, "ビットを取り出す");
                    self.emit_byte("cmp", &mv, k, "すべて立っているか");
                    self.br("bne", if j { "@@" } else { l }, "立っていない");
                }
                if j {
                    self.unimpl("変数のビットフラグとの contains を条件に使うこと", &a.span);
                }
            }
        }
    }

    fn gen_flag_op(&mut self, r: &Expr, name: &str, args: &[Arg]) {
        let dv = self.place_val(r);
        let size = self.sz(&self.ty_of(r));
        let a = match args.first() {
            Some(a) => a.value.clone(),
            None => return,
        };
        let m = self.const_int(&a);
        if let (Some(mask), Val::Mem { idx: Idx::None, io, rmw, .. }) = (m, &dv) {
            let use_tsb = size == 1 && (*rmw || !*io) && (name == "insert" || name == "remove");
            if use_tsb && *rmw {
                self.imm("lda", (mask & 0xFF) as i64, "ビット");
                self.emit_byte(if name == "insert" { "tsb" } else { "trb" }, &dv, 0, if name == "insert" { "ビットを立てる（tsb）" } else { "ビットを消す（trb）" });
                return;
            }
        }
        let mv = match m {
            Some(mask) => Val::Imm(mask),
            None => {
                if self.is_simple(&a) {
                    self.val_of(&a)
                } else {
                    self.materialize(&a)
                }
            }
        };
        for k in 0..size {
            match name {
                "insert" => {
                    self.emit_byte("lda", &dv, k, "読む");
                    self.emit_byte("ora", &mv, k, "ビットを立てる");
                }
                "remove" => {
                    if let Val::Imm(x) = &mv {
                        let inv = !((x >> (8 * k)) & 0xFF) & 0xFF;
                        self.emit_byte("lda", &dv, k, "読む");
                        self.imm("and", inv as i64, "ビットを消す");
                    } else {
                        self.emit_byte("lda", &mv, k, "消すビット");
                        self.imm("eor", 0xFF, "反転");
                        let s = self.scratch();
                        self.emit("sta", AMode::Mem(s.expr()), "一時的にしまう");
                        self.emit_byte("lda", &dv, k, "読む");
                        self.emit("and", AMode::Mem(s.expr()), "ビットを消す");
                    }
                }
                _ => {
                    self.emit_byte("lda", &dv, k, "読む");
                    self.emit_byte("eor", &mv, k, "ビットを反転する");
                }
            }
            self.store_byte(&dv, k, "書く");
        }
    }

    // ---- 代入 ----
    fn gen_assign(&mut self, op: Option<BinOp>, lhs: &Expr, rhs: &Expr) {
        let t = self.ty_of(lhs);
        let n = self.sz(&t);
        match op {
            None => {
                // 右辺に左辺の場所の計算が要らないなら、先に場所を求める
                let dest = self.place_val(lhs);
                self.gen(rhs, &Want::Store(dest));
            }
            Some(o) => {
                let c = self.const_int(rhs);
                let bcd = matches!(t, Ty::Bcd(_));
                // 右辺を先に（複雑なら一時領域へ）
                let rv = if self.is_simple(rhs) { None } else { Some(self.materialize(rhs)) };
                let dest = self.place_val(lhs);
                let rmw_ok = match &dest {
                    Val::Mem { io, rmw, .. } => !*io || *rmw,
                    Val::Reg(_) => true,
                    Val::Ind { .. } => true,
                    _ => false,
                };
                if !bcd && n == 1 && (o == BinOp::Add || o == BinOp::Sub) && c == Some(1) && rmw_ok {
                    let mn = if o == BinOp::Add { "inc" } else { "dec" };
                    self.emit_rmw(mn, &dest, 0, if o == BinOp::Add { "1 増やす" } else { "1 減らす" });
                    return;
                }
                if !bcd && n >= 2 && o == BinOp::Add && c == Some(1) && rmw_ok && matches!(dest, Val::Mem { idx: Idx::None, .. }) {
                    let l = self.new_label("inc");
                    for k in 0..n {
                        self.emit_rmw("inc", &dest, k, if k == 0 { "1 増やす（下位）" } else { "繰り上げる" });
                        if k + 1 < n {
                            self.br("bne", &l, "0 に戻らなければ終わり");
                        }
                    }
                    self.label(&l);
                    return;
                }
                let vb = match rv {
                    Some(v) => v,
                    None => self.val_of(rhs),
                };
                match o {
                    BinOp::Add | BinOp::Sub | BinOp::And | BinOp::Or | BinOp::Xor => {
                        let d2 = dest.clone();
                        self.arith_store(o, &d2, &vb, &t, &dest, n);
                    }
                    BinOp::Shl | BinOp::Shr => {
                        let k = c.unwrap_or(0) as u32;
                        let d2 = dest.clone();
                        self.shift_store(o, &d2, k, &t, &dest, n);
                    }
                    BinOp::Mul => {
                        let d2 = dest.clone();
                        self.mul_const_store(&d2, c.unwrap_or(0), &t, &dest, n);
                    }
                    BinOp::Div => {
                        let d2 = dest.clone();
                        self.shift_store(BinOp::Shr, &d2, log2(c.unwrap_or(1)), &t, &dest, n);
                    }
                    BinOp::Rem => {
                        let d2 = dest.clone();
                        let m = Val::Imm(c.unwrap_or(1) - 1);
                        self.arith_store(BinOp::And, &d2, &m, &t, &dest, n);
                    }
                    _ => self.unimpl("この代入の演算", &lhs.span),
                }
            }
        }
    }

    // =====================================================================
    // 呼び出し（15.2節・15.3節）
    // =====================================================================

    pub fn gen_call(&mut self, e: &Expr) -> RetConv {
        let args: Vec<Arg> = match &e.kind {
            ExprKind::Call(_, a) => a.clone(),
            _ => return RetConv::Nothing,
        };
        match self.ctx.calls.get(&e.id).cloned() {
            Some(CallTarget::Fn(f)) => self.call_fn(f, &args, &e.span),
            Some(CallTarget::FnSetVar) => self.call_fnset(e, &args),
            Some(CallTarget::Builtin(n)) => {
                match n.as_str() {
                    "irq_enable" => self.emit0("cli", "irq_enable()：割り込みを許可する"),
                    "irq_disable" => self.emit0("sei", "irq_disable()：割り込みを止める"),
                    "wait_interrupt" => self.emit0("wai", "wait_interrupt()：割り込みが来るまで止まる"),
                    "delay" => {
                        let c = args.first().and_then(|a| self.ctx.econst.get(&a.value.id).cloned()).and_then(|v| value_int(&v)).unwrap_or(0);
                        self.emit_delay(c, &e.span);
                    }
                    _ => {}
                }
                RetConv::Nothing
            }
            Some(CallTarget::Folded) => RetConv::Nothing,
            Some(CallTarget::SomeCtor) | Some(CallTarget::TypeFn(_, _)) => {
                let _ = self.materialize(e);
                RetConv::Nothing
            }
            None => RetConv::Nothing,
        }
    }

    fn ret_conv_of(&mut self, f: usize) -> RetConv {
        let rt = self.ctx.funcs[f].ret.clone();
        let label = self.ctx.funcs[f].label.clone();
        match &rt {
            Ty::Unit | Ty::Never | Ty::Error => RetConv::Nothing,
            Ty::Bool => RetConv::Carry,
            Ty::Opt(inner) => {
                if self.sz(inner) >= 4 {
                    RetConv::OptCarry(Some(ret_area(&label)))
                } else {
                    RetConv::OptCarry(None)
                }
            }
            t => {
                let n = self.sz(t);
                if !self.by_area(t) {
                    RetConv::Regs(n)
                } else {
                    RetConv::Area(ret_area(&label), n)
                }
            }
        }
    }

    fn call_fn(&mut self, f: usize, args: &[Arg], sp: &Span) -> RetConv {
        let fi = self.ctx.funcs[f].clone();
        let layout = self.param_layout(f);
        if fi.kind != FnKind::Extern {
            if !self.generated.contains(&f) {
                self.queue.push(f);
            }
        }
        self.f.calls.push(f);
        // A だけで渡す 1 バイトの引数は、最後に計算する
        let a_only: Option<usize> = layout.iter().position(|p| p.regs == vec!['a']);
        let has_mem_args = layout.iter().any(|p| p.regs.is_empty());
        let mut vals: Vec<Option<Val>> = vec![None; args.len()];
        for (i, a) in args.iter().enumerate() {
            if Some(i) == a_only && !self.is_simple(&a.value) && !(has_mem_args && self.has_effects(&a.value)) {
                continue;
            }
            let pl = match layout.get(i) {
                Some(p) => p.clone(),
                None => continue,
            };
            let v = if self.is_simple(&a.value) { self.val_of(&a.value) } else { self.materialize(&a.value) };
            let v = if !pl.regs.is_empty() && !self.reg_loadable(&v) { self.materialize_val(&v, pl.size) } else { v };
            vals[i] = Some(v);
        }
        // 4 バイトめ以降：呼び出し先の引数領域に直接書く
        for (i, pl) in layout.iter().enumerate() {
            if pl.regs.is_empty() {
                if let Some(v) = vals[i].clone() {
                    let dst = Val::mem(self.slot_base(&fi.label, &pl.slot), pl.size);
                    self.copy_val(&v, &dst, pl.size);
                }
            }
        }
        // A の引数
        let mut a_loaded = false;
        if let Some(i) = a_only {
            if vals[i].is_none() {
                if let Some(a) = args.get(i) {
                    let x = a.value.clone();
                    self.gen_a(&x);
                    a_loaded = true;
                }
            }
        }
        // X・Y、最後に A
        for (i, pl) in layout.iter().enumerate() {
            if let Some(v) = vals[i].clone() {
                for (k, r) in pl.regs.iter().enumerate() {
                    if *r != 'a' {
                        self.load_reg(*r, &v, k as u32);
                    }
                }
            }
        }
        if !a_loaded {
            for (i, pl) in layout.iter().enumerate() {
                if let Some(v) = vals[i].clone() {
                    for (k, r) in pl.regs.iter().enumerate() {
                        if *r == 'a' {
                            self.load_reg('a', &v, k as u32);
                        }
                    }
                }
            }
        }
        // 呼ぶ関数が X・Y を壊さないなら、X・Y に読んである自分のローカル変数はそのまま使える
        let saved_x = self.f.xcache.clone();
        let saved_y = self.f.ycache.clone();
        let lends = args.iter().any(|a| matches!(a.value.kind, ExprKind::Unary(UnOp::Addr, _)));
        if fi.kind == FnKind::Extern {
            let addr = match fi.decl.as_ref().and_then(|d| d.extern_addr.clone()) {
                Some(a) => match self.ctx.eval_in_module(&a, fi.module, Some(&Ty::CInt)) {
                    Ok(v) => value_int(&v).unwrap_or(0) as i64,
                    Err(_) => 0,
                },
                None => 0,
            };
            let st = fi
                .decl
                .as_ref()
                .and_then(|d| d.stack.clone())
                .and_then(|s| self.ctx.eval_in_module(&s, fi.module, Some(&Ty::CInt)).ok())
                .and_then(|v| value_int(&v))
                .unwrap_or(2) as u32;
            if st > self.f.ext_stack {
                self.f.ext_stack = st;
            }
            self.emit("jsr", AMode::Mem(OpExpr::num(addr)), &format!("extern fn {}", fi.name));
        } else {
            self.emit("jsr", AMode::Mem(OpExpr::sym(&fi.label)), &format!("{} を呼ぶ", fi.name));
        }
        let _ = sp;
        let cm = self.clob_of(f);
        self.f.callee_clob |= cm;
        if !lends {
            let own = |e: &Option<OpExpr>, me: &str| -> bool {
                match e.as_ref().and_then(|x| x.sym.as_deref()) {
                    Some(s) => s == frame_sym(me) || s == zframe_sym(me),
                    None => false,
                }
            };
            let me = self.f.label.clone();
            if cm & 2 == 0 && own(&saved_x, &me) {
                self.f.xcache = saved_x;
            }
            if cm & 4 == 0 && own(&saved_y, &me) {
                self.f.ycache = saved_y;
            }
        }
        if fi.kind == FnKind::Extern {
            // extern fn の戻り値（7.6節）：bool in C、Option<u8> in A（有無はキャリー）、u8 in A、u16 in AX
            return match &fi.ret {
                Ty::Unit => RetConv::Nothing,
                Ty::Bool => RetConv::Carry,
                Ty::Opt(_) => RetConv::OptCarry(None),
                t => RetConv::Regs(self.sz(t).min(3)),
            };
        }
        self.ret_conv_of(f)
    }

    fn call_fnset(&mut self, e: &Expr, args: &[Arg]) -> RetConv {
        let callee = match &e.kind {
            ExprKind::Call(c, _) => (**c).clone(),
            _ => return RetConv::Nothing,
        };
        let st = self.ty_of(&callee);
        let sid = match st {
            Ty::FnSet(s) => s,
            _ => return RetConv::Nothing,
        };
        let def = self.ctx.types.fnsets[sid].clone();
        self.fnset_used.insert(sid);
        for f in &def.fns {
            if !self.generated.contains(f) {
                self.queue.push(*f);
            }
            self.f.calls.push(*f);
        }
        let first = match def.fns.first() {
            Some(f) => *f,
            None => return RetConv::Nothing,
        };
        let layout = self.param_layout(first);
        if layout.iter().any(|p| p.regs.iter().any(|r| *r != 'a')) {
            self.unimpl("X・Y で引数を渡す関数の関数集合", &e.span);
        }
        // 引数（A だけ）を先に計算してしまう
        let mut aval = None;
        for (i, a) in args.iter().enumerate() {
            let pl = match layout.get(i) {
                Some(p) => p.clone(),
                None => continue,
            };
            let v = self.materialize(&a.value);
            if pl.regs.is_empty() {
                for f in &def.fns {
                    let fl = self.ctx.funcs[*f].label.clone();
                    let fpl = self.param_layout(*f);
                    if let Some(p2) = fpl.get(i) {
                        let dst = Val::mem(self.slot_base(&fl, &p2.slot), p2.size);
                        self.copy_val(&v, &dst, p2.size);
                    }
                }
            } else {
                aval = Some(v);
            }
        }
        let cv = self.val_of(&callee);
        self.emit_byte("lda", &cv, 0, "関数集合の値（番号）");
        self.emit0("asl", "表の位置（2 倍）");
        self.emit0("tax", "X に置く");
        if let Some(v) = aval {
            self.emit_byte("lda", &v, 0, "引数を A に");
        }
        let disp = format!("FS_{}_call", def.name);
        self.emit("jsr", AMode::Mem(OpExpr::sym(&disp)), &format!("関数集合 {} の X 番目を呼ぶ", def.name));
        self.f.callee_clob |= 7;
        self.ret_conv_of(first)
    }

    pub fn conv_to_a(&mut self, c: &RetConv) {
        match c {
            RetConv::Carry => {
                self.imm("lda", 0, "");
                self.emit0("rol", "キャリーを A に（0 か 1）");
            }
            RetConv::Area(l, _) => {
                let l = l.clone();
                self.emit("lda", AMode::Mem(OpExpr::sym(&l)), "結果領域を読む");
            }
            _ => {}
        }
    }

    fn conv_to_val(&mut self, c: &RetConv, n: u32) -> Val {
        match c {
            RetConv::Area(l, m) => Val::mem(Base::Sym(l.clone(), 0), *m),
            _ => {
                let t = self.tmp_val(n.max(1));
                self.conv_store(c, &t, n);
                t
            }
        }
    }

    fn conv_store(&mut self, c: &RetConv, dest: &Val, n: u32) {
        match c {
            RetConv::Nothing => {}
            RetConv::Carry => {
                self.imm("lda", 0, "");
                self.emit0("rol", "キャリーを A に");
                self.store_byte(dest, 0, "書く");
            }
            RetConv::Regs(m) => {
                self.store_byte(dest, 0, "戻り値（A）");
                let plain = matches!(dest, Val::Mem { idx: Idx::None, .. });
                if *m >= 2 && n >= 2 {
                    if plain {
                        if let Val::Mem { base, stride, .. } = dest {
                            self.emit("stx", AMode::Mem(base.add(*stride).expr()), "戻り値（X）");
                        }
                    } else {
                        self.emit0("txa", "X の戻り値");
                        self.store_byte(dest, 1, "書く");
                    }
                }
                if *m >= 3 && n >= 3 {
                    if plain {
                        if let Val::Mem { base, stride, .. } = dest {
                            self.emit("sty", AMode::Mem(base.add(2 * stride).expr()), "戻り値（Y）");
                        }
                    } else {
                        self.emit0("tya", "Y の戻り値");
                        self.store_byte(dest, 2, "書く");
                    }
                }
            }
            RetConv::Area(l, m) => {
                let src = Val::mem(Base::Sym(l.clone(), 0), *m);
                self.copy_val(&src, dest, n.min(*m));
            }
            RetConv::OptCarry(_) => self.unimpl("Option の戻り値の、この使い方", &Span::none()),
        }
    }

    fn type_fn_a(&mut self, e: &Expr, t: &Ty, name: &str) {
        let args = match &e.kind {
            ExprKind::Call(_, a) => a.clone(),
            _ => return,
        };
        let a = match args.first() {
            Some(a) => a.value.clone(),
            None => return,
        };
        match (t, name) {
            (Ty::Range(r), "wrap") => {
                let d = self.ctx.types.ranges[*r].clone();
                self.gen_a(&a);
                self.imm("and", (d.count() - 1) as i64, &format!("{}::wrap：下位のビットだけを取る", d.name));
            }
            (_, "unchecked") => {
                self.gen_a(&a);
            }
            _ => {
                let v = self.materialize(e);
                self.emit_byte("lda", &v, 0, "値");
            }
        }
    }

    // =====================================================================
    // メソッド
    // =====================================================================

    fn method_a(&mut self, e: &Expr, r: &Expr, name: &str, args: &[Arg]) {
        let rt = self.ty_of(r);
        match name {
            "saturating_add" | "saturating_sub" => {
                let add = name == "saturating_add";
                if self.ctx.types.is_signed(&rt) {
                    self.unimpl("符号付きの saturating_add・saturating_sub", &e.span);
                    return;
                }
                let b = &args[0].value;
                let vb = if self.is_simple(b) { None } else { Some(self.materialize(b)) };
                self.gen_a(r);
                let vb = match vb {
                    Some(v) => v,
                    None => self.val_of(b),
                };
                let ok = self.new_label("sat");
                if add {
                    self.emit0("clc", "足し算の準備");
                    self.emit_byte("adc", &vb, 0, "足す（255 を超えるとキャリーが 1）");
                    self.br("bcc", &ok, "あふれていなければそのまま");
                    self.imm("lda", 0xFF, "あふれたら 255 で止める");
                } else {
                    self.emit0("sec", "引き算の準備");
                    self.emit_byte("sbc", &vb, 0, "引く（0 未満になるとキャリーが 0）");
                    self.br("bcs", &ok, "借りがなければそのまま");
                    self.imm("lda", 0, "0 未満になったら 0 で止める");
                }
                self.label(&ok);
            }
            "rol" | "ror" => {
                self.gen_a(r);
                self.emit0(name, "キャリーを通して回す");
            }
            "len" => match &rt {
                Ty::Str(_, _) => {
                    let v = self.val_of(r);
                    match v {
                        Val::Addr(b) => self.emit("lda", AMode::Mem(b.expr()), "文字列の長さ"),
                        other => {
                            let z = match other {
                                Val::Mem { base, .. } if matches!(&base, Base::Sym(s, _) if s.starts_with("Z_")) => base,
                                o => {
                                    let z = self.alloc_ztmp(2);
                                    let zv = Val::mem(z.clone(), 2);
                                    self.copy_val(&o, &zv, 2);
                                    z
                                }
                            };
                            self.emit("lda", AMode::Ind(z.expr()), "文字列の長さ（先頭のバイト）");
                        }
                    }
                }
                _ => {
                    let v = self.materialize(e);
                    self.emit_byte("lda", &v, 0, "長さ");
                }
            },
            "shl" | "shr" => {
                let v = self.materialize(e);
                self.emit_byte("lda", &v, 0, "結果");
            }
            _ => {
                let v = self.materialize(e);
                self.emit_byte("lda", &v, 0, "結果");
            }
        }
    }

    fn method_store(&mut self, e: &Expr, r: &Expr, name: &str, args: &[Arg], dest: &Val, t: &Ty) {
        let n = self.sz(t);
        let rt = self.ty_of(r);
        match name {
            "digits" => {
                let nb = self.sz(&rt);
                let src = self.val_of(r);
                let src = if self.reg_loadable(&src) { src } else { self.materialize_val(&src, nb) };
                for b in (0..nb).rev() {
                    let pos = 2 * (nb - 1 - b);
                    self.emit_byte("lda", &src, b, "上の 2 桁");
                    for _ in 0..4 {
                        self.emit0("lsr", "上位 4 ビットを下へ");
                    }
                    self.store_byte(dest, pos, "上の桁");
                    self.emit_byte("lda", &src, b, "もう一度読む");
                    self.imm("and", 0x0F, "下位 4 ビット");
                    self.store_byte(dest, pos + 1, "下の桁");
                }
                self.notes.push(format!("digits()（{}行）：{} 桁を結果領域に作る", e.span.line, nb * 2));
            }
            "saturating_add" | "saturating_sub" => {
                let add = name == "saturating_add";
                let va = self.val_of(r);
                let b = &args[0].value;
                let vb = if self.is_simple(b) { self.val_of(b) } else { self.materialize(b) };
                let work = if Gen::plain_mem(dest) { dest.clone() } else { self.tmp_val(n) };
                self.arith_store(if add { BinOp::Add } else { BinOp::Sub }, &va, &vb, t, &work, n);
                let ok = self.new_label("sat");
                self.br(if add { "bcc" } else { "bcs" }, &ok, "あふれていなければそのまま");
                let fill = Val::Imm(if add { -1 } else { 0 });
                self.copy_val(&fill, &work, n);
                self.label(&ok);
                if work != *dest {
                    self.copy_val(&work, dest, n);
                }
            }
            "shl" | "shr" => {
                let va = self.val_of(r);
                let work = if Gen::plain_mem(dest) { dest.clone() } else { self.tmp_val(n) };
                self.copy_val(&va, &work, n);
                let cnt = &args[0].value;
                let cv = if self.is_simple(cnt) { self.val_of(cnt) } else { self.materialize(cnt) };
                let top = self.new_label("sh");
                let done = self.new_label("shd");
                if self.f.yres {
                    self.unimpl("Y を使うループの中の shl・shr", &e.span);
                }
                self.load_reg('y', &cv, 0);
                self.br("beq", &done, "0 回なら何もしない");
                self.label(&top);
                if name == "shl" {
                    for i in 0..n {
                        self.emit_rmw(if i == 0 { "asl" } else { "rol" }, &work, i, "左に 1 ビット");
                    }
                } else {
                    for i in (0..n).rev() {
                        self.emit_rmw(if i == n - 1 { "lsr" } else { "ror" }, &work, i, "右に 1 ビット");
                    }
                }
                self.emit0("dey", "回数を 1 減らす");
                self.br("bne", &top, "繰り返す");
                self.label(&done);
                if work != *dest {
                    self.copy_val(&work, dest, n);
                }
            }
            "add" | "add_pages" => {
                let pv = self.val_of(r);
                let av = &args[0].value;
                let a = if self.is_simple(av) { self.val_of(av) } else { self.materialize(av) };
                let asz = self.sz(&self.ty_of(av));
                let work = if Gen::plain_mem(dest) { dest.clone() } else { self.tmp_val(2) };
                if name == "add_pages" {
                    self.copy_val(&pv, &work, 1);
                    self.emit_byte("lda", &pv, 1, "上位");
                    self.emit0("clc", "");
                    self.emit_byte("adc", &a, 0, "ページの数を足す");
                    self.store_byte(&work, 1, "書く");
                } else {
                    let a2 = if asz == 1 { Val::Ext { inner: Box::new(a), from: 1 } } else { a };
                    self.arith_store(BinOp::Add, &pv, &a2, &Ty::Int(IntK::U16), &work, 2);
                }
                if work != *dest {
                    self.copy_val(&work, dest, 2);
                }
                let fname = self.ctx.funcs.get(self.f.fid).map(|f| f.name.clone()).unwrap_or_default();
                self.ctx.unsafe_list.push(format!("ポインタの計算 {}（{}、{}行）", name, fname, e.span.line));
            }
            _ => self.unimpl(&format!("メソッド {}", name), &e.span),
        }
    }

    // =====================================================================
    // Option（3.11節）
    // =====================================================================

    /// Option の値を調べ、None なら none へ飛ぶ。Some の値の置き場所を返す
    pub fn gen_opt_test(&mut self, e: &Expr, none: &str) -> Val {
        let t = self.ty_of(e);
        let inner = match &t {
            Ty::Opt(i) => (**i).clone(),
            _ => Ty::Error,
        };
        let isz = self.sz(&inner);
        if let Some(v) = self.ctx.econst.get(&e.id).cloned() {
            match v {
                Value::Opt(None) => {
                    self.jmp(none, "None");
                    return Val::Imm(0);
                }
                Value::Opt(Some(x)) => {
                    let bytes = self.ctx.value_bytes(&x, &inner);
                    let c = bytes.iter().enumerate().fold(0i128, |acc, (k, b)| acc | ((*b as i128) << (8 * k)));
                    return Val::Imm(c);
                }
                _ => {}
            }
        }
        match &e.kind {
            ExprKind::Call(_, args) => match self.ctx.calls.get(&e.id).cloned() {
                Some(CallTarget::SomeCtor) => {
                    let x = args[0].value.clone();
                    return if self.is_simple(&x) { self.val_of(&x) } else { self.materialize(&x) };
                }
                Some(CallTarget::Fn(_)) | Some(CallTarget::FnSetVar) => {
                    let conv = self.gen_call(e);
                    self.br("bcc", none, "キャリー 0（None）なら飛ぶ");
                    return match conv {
                        RetConv::OptCarry(Some(area)) => Val::mem(Base::Sym(area, 0), isz),
                        _ => {
                            if isz == 0 {
                                Val::Imm(0)
                            } else {
                                let tv = self.tmp_val(isz);
                                self.conv_store(&RetConv::Regs(isz.min(3)), &tv, isz);
                                tv
                            }
                        }
                    };
                }
                Some(CallTarget::TypeFn(tt, name)) if name == "check" => {
                    let a = args[0].value.clone();
                    return self.gen_check(&tt, &a, none, &e.span);
                }
                _ => {}
            },
            ExprKind::MethodCall(r, name, args) => match name.as_str() {
                "get" => {
                    let rt = self.ty_of(r);
                    if let Ty::Array(et, len) = &rt {
                        let n = self.ctx.types.len_count(len);
                        let es = self.sz(et);
                        self.gen_a(&args[0].value);
                        if n < 256 {
                            self.imm("cmp", n as i64, "長さと比べる");
                            self.br("bcs", none, "範囲の外なら None");
                        }
                        let reg = self.idx_reg();
                        self.emit0(if reg == 'x' { "tax" } else { "tay" }, "添字");
                        let av = self.val_of(r);
                        let tv = self.tmp_val(es);
                        let planar = self.array_planar(et);
                        if let Val::Mem { base, idx: Idx::None, .. } = av {
                            for k in 0..es {
                                let off = if planar { k as i64 * n as i64 } else { k as i64 };
                                let am = if reg == 'x' { AMode::MemX(base.add(off).expr()) } else { AMode::MemY(base.add(off).expr()) };
                                self.emit("lda", am, "要素を読む");
                                self.store_byte(&tv, k, "Some の値");
                            }
                        } else {
                            self.unimpl("この配列の get", &e.span);
                        }
                        return tv;
                    }
                }
                "checked_add" | "checked_sub" => {
                    let rt = self.ty_of(r);
                    if self.ctx.types.is_signed(&rt) {
                        self.unimpl("符号付きの checked_add・checked_sub", &e.span);
                        return Val::Imm(0);
                    }
                    let (va, vb) = self.two_vals(r, &args[0].value);
                    let tv = self.tmp_val(isz);
                    let add = name == "checked_add";
                    self.arith_store(if add { BinOp::Add } else { BinOp::Sub }, &va, &vb, &rt, &tv, isz);
                    self.br(if add { "bcs" } else { "bcc" }, none, "あふれたら None");
                    return tv;
                }
                _ => {}
            },
            ExprKind::Path(_) | ExprKind::Field(_, _) | ExprKind::Index(_, _) => {
                let v = self.val_of(e);
                self.emit_byte("lda", &v, isz, "Option の有無");
                self.br("beq", none, "なし（None）なら飛ぶ");
                return v.part(0, isz);
            }
            _ => {}
        }
        // 一般の式：一時領域に Option として作ってから調べる
        let tv = self.tmp_val(isz + 1);
        self.gen(e, &Want::Store(tv.clone()));
        self.emit_byte("lda", &tv, isz, "Option の有無");
        self.br("beq", none, "なし（None）なら飛ぶ");
        tv.part(0, isz)
    }

    fn gen_check(&mut self, t: &Ty, a: &Expr, none: &str, sp: &Span) -> Val {
        match t {
            Ty::Range(r) => {
                let d = self.ctx.types.ranges[*r].clone();
                if d.base_size() == 1 {
                    self.gen_a(a);
                    if d.lo > 0 {
                        self.imm("cmp", d.lo as i64, &format!("{}::check：下限と比べる", d.name));
                        self.br("bcc", none, "下限より小さければ None");
                    }
                    if d.hi < 256 {
                        self.imm("cmp", d.hi as i64, &format!("{}::check：上限と比べる", d.name));
                        self.br("bcs", none, "上限以上なら None");
                    }
                    let tv = self.tmp_val(1);
                    self.store_byte(&tv, 0, "範囲に収まった値");
                    return tv;
                }
                let v = self.val_of(a);
                if d.lo == 1 && d.hi == 65536 {
                    self.emit_byte("lda", &v, 0, &format!("{}::check：0 でないか", d.name));
                    self.emit_byte("ora", &v, 1, "上位と合わせる");
                    self.br("beq", none, "0 なら None");
                    return v;
                }
                if d.lo == 0 {
                    let tv = self.tmp_val(2);
                    self.copy_val(&v, &tv, 2);
                    let hv = Val::Imm(d.hi);
                    let ok = self.new_label("chk");
                    self.emit_byte("lda", &tv, 0, "下位");
                    self.emit_byte("cmp", &hv, 0, "上限と比べる");
                    self.emit_byte("lda", &tv, 1, "上位");
                    self.emit_byte("sbc", &hv, 1, "上限と比べる");
                    self.br("bcc", &ok, "上限より小さければ Some");
                    self.jmp(none, "None");
                    self.label(&ok);
                    return tv;
                }
                self.unimpl("下限が 0 でも 1 でもない 16 ビットの範囲型の check", sp);
                v
            }
            Ty::Enum(x) => {
                let d = self.ctx.types.enums[*x].clone();
                self.gen_a(a);
                if d.contiguous() {
                    self.imm("cmp", d.variants.len() as i64, &format!("{}::check：値の数と比べる", d.name));
                    self.br("bcs", none, "範囲の外なら None");
                } else {
                    let ok = self.new_label("chk");
                    for (_, v) in &d.variants {
                        self.imm("cmp", *v as i64, "値と比べる");
                        self.br("beq", &ok, "あれば Some");
                    }
                    self.jmp(none, "どれでもなければ None");
                    self.label(&ok);
                }
                let tv = self.tmp_val(1);
                self.store_byte(&tv, 0, "値");
                tv
            }
            _ => {
                self.unimpl("この型の check", sp);
                Val::Imm(0)
            }
        }
    }

    fn store_option(&mut self, e: &Expr, dest: &Val) {
        let t = self.ty_of(e);
        let inner = match &t {
            Ty::Opt(i) => (**i).clone(),
            _ => Ty::Error,
        };
        let isz = self.sz(&inner);
        if let Some(v) = self.ctx.econst.get(&e.id).cloned() {
            let bytes = self.ctx.value_bytes(&v, &t);
            let c = bytes.iter().enumerate().fold(0i128, |acc, (k, b)| acc | ((*b as i128) << (8 * k)));
            self.copy_val(&Val::Imm(c), dest, isz + 1);
            return;
        }
        if let ExprKind::Call(_, args) = &e.kind {
            if let Some(CallTarget::SomeCtor) = self.ctx.calls.get(&e.id) {
                let x = args[0].value.clone();
                let d = dest.part(0, isz);
                self.gen(&x, &Want::Store(d));
                self.imm("lda", 1, "Some");
                self.store_byte(dest, isz, "有無を書く");
                return;
            }
        }
        if let ExprKind::Path(_) | ExprKind::Field(_, _) | ExprKind::Index(_, _) = &e.kind {
            let v = self.val_of(e);
            self.copy_val(&v, dest, isz + 1);
            return;
        }
        let l_none = self.new_label("none");
        let l_end = self.new_label("opt");
        let v = self.gen_opt_test(e, &l_none);
        let d = dest.part(0, isz);
        self.copy_val(&v, &d, isz);
        self.imm("lda", 1, "Some");
        self.store_byte(dest, isz, "有無を書く");
        self.jmp(&l_end, "");
        self.label(&l_none);
        self.store_zero(dest, isz, "None");
        self.label(&l_end);
    }

    /// x? （値が None なら、この関数も None を返す）
    fn gen_try(&mut self, x: &Expr) -> Val {
        // 1 バイト以下の Option を返す関数の呼び出し：None ならキャリー 0 のまま、そのまま戻る（値は A のまま）
        if let (ExprKind::Call(_, _), Some(CallTarget::Fn(f))) = (&x.kind, self.ctx.calls.get(&x.id).cloned()) {
            let isz = match &self.ctx.funcs[f].ret {
                Ty::Opt(i) => self.sz(i),
                _ => 99,
            };
            if isz <= 1 && self.f.atomic == 0 && !self.f.interrupt {
                let _ = self.gen_call(x);
                let ok = self.new_label("ok");
                self.br("bcs", &ok, "Some なら続ける");
                self.emit_rts();
                if let Some(Item::Ins(i)) = self.f.items.last_mut() {
                    i.comment = "? 演算子：None（キャリー 0）のまま戻る".to_string();
                }
                self.label(&ok);
                return if isz == 0 { Val::Imm(0) } else { Val::Reg('a') };
            }
        }
        let l_none = self.new_label("try");
        let l_ok = self.new_label("ok");
        let v = self.gen_opt_test(x, &l_none);
        let v = match v {
            Val::Imm(_) | Val::Mem { .. } => v,
            other => {
                let n = self.val_size(&other);
                self.materialize_val(&other, n)
            }
        };
        self.jmp(&l_ok, "Some なら続ける");
        self.label(&l_none);
        self.leave_prefix();
        self.emit0("clc", "? 演算子：None を返す（キャリー 0）");
        self.emit_rts();
        self.label(&l_ok);
        v
    }

    // =====================================================================
    // for（6.4節）
    // =====================================================================

    pub fn body_allows_reg(&self, body: &Block, reg: char) -> bool {
        let mut ok = true;
        crate::check::walk_block(body, &mut |x: &Expr| match &x.kind {
            ExprKind::Call(_, _) => match self.ctx.calls.get(&x.id) {
                Some(CallTarget::Builtin(n)) => {
                    if n == "delay" {
                        ok = false;
                    }
                }
                Some(CallTarget::SomeCtor) | Some(CallTarget::Folded) | Some(CallTarget::TypeFn(_, _)) => {}
                // 呼ぶ関数がそのレジスタを壊さず、引数・戻り値にも使わなければよい
                Some(CallTarget::Fn(f)) => {
                    if !self.call_keeps_reg(*f, reg) {
                        ok = false;
                    }
                }
                _ => ok = false,
            },
            ExprKind::MethodCall(_, n, _) => {
                if matches!(n.as_str(), "shl" | "shr" | "push" | "clear") || (reg == 'y' && n == "get") {
                    ok = false;
                }
            }
            ExprKind::Asm(_) => ok = false,
            ExprKind::Match(_, _, attrs) => {
                if find_attr(attrs, "jump_table").is_some() {
                    ok = false;
                }
            }
            ExprKind::Try(_) => ok = false,
            ExprKind::Index(_, _) | ExprKind::Field(_, _) => {
                if reg == 'y' {
                    ok = false;
                }
            }
            _ => {}
        });
        ok
    }

    /// 関数 f を呼んでも、レジスタ reg（x か y）の値が保たれるか
    pub fn call_keeps_reg(&self, f: usize, reg: char) -> bool {
        let bit = if reg == 'x' { 2 } else { 4 };
        if self.clob_of(f) & bit != 0 {
            return false;
        }
        if self.param_layout(f).iter().any(|p| p.regs.contains(&reg)) {
            return false;
        }
        let rt = self.ctx.funcs[f].ret.clone();
        let rs = match &rt {
            Ty::Opt(i) => self.sz(i),
            t => self.sz(t),
        };
        if !self.by_area(&rt) && ((reg == 'x' && rs >= 2) || (reg == 'y' && rs >= 3)) {
            return false;
        }
        true
    }

    pub fn gen_for(&mut self, e: &Expr, label: &Option<String>, iter: &Expr, body: &Block, attrs: &[Attr]) {
        let lv = self.ctx.node_local.get(&e.id).cloned();
        let et = match lv {
            Some(l) => self.ctx.funcs[self.f.fid].locals[l].ty.clone(),
            None => self.elem_type_of_iter(iter),
        };
        let esz = self.sz(&et).max(1);
        let forced = find_attr(attrs, "reg").and_then(attr_ident_arg);
        // 繰り返しの種類
        let mut rev = false;
        let mut it = iter.clone();
        if let ExprKind::MethodCall(b, n, _) = &iter.kind {
            if n == "rev" {
                rev = true;
                it = (**b).clone();
            }
        }
        // 範囲（a..b、a..=b、型、Eid::range）
        let mut range: Option<(Val, Val, bool, bool)> = None; // (始め, 終わり, 終わりを含むか, 両端が定数か)
        match &it.kind {
            ExprKind::Range(Some(a), Some(b), incl) => {
                let ca = self.const_int(a);
                let cb = self.const_int(b);
                let va = match ca {
                    Some(c) => Val::Imm(c),
                    None => {
                        let n = self.sz(&self.ty_of(a));
                        let v = self.val_of(a);
                        self.materialize_val(&v, n)
                    }
                };
                let vb = match cb {
                    Some(c) => Val::Imm(c),
                    None => {
                        let n = self.sz(&self.ty_of(b));
                        let v = self.val_of(b);
                        self.materialize_val(&v, n)
                    }
                };
                range = Some((va, vb, *incl, ca.is_some() && cb.is_some()));
            }
            ExprKind::Path(_) => {
                if let Some(Res::TypeName(tt)) = self.ctx.res.get(&it.id).cloned() {
                    match tt {
                        Ty::Range(r) => {
                            let d = self.ctx.types.ranges[r].clone();
                            range = Some((Val::Imm(d.lo), Val::Imm(d.hi), false, true));
                        }
                        Ty::Enum(x) => {
                            let d = self.ctx.types.enums[x].clone();
                            if d.contiguous() {
                                range = Some((Val::Imm(0), Val::Imm(d.variants.len() as i128), false, true));
                            } else {
                                // 値の表を回す
                                let tbl = format!("EV_{}", d.name);
                                if !self.row_tables.contains_key(&tbl) {
                                    self.data.push(Item::Label(tbl.clone()));
                                    self.data.push(Item::Bytes(d.variants.iter().map(|v| v.1 as u8).collect(), format!("列挙型 {} の値", d.name)));
                                    self.row_tables.insert(tbl.clone(), tbl.clone());
                                }
                                let arr = Val::mem(Base::Sym(tbl, 0), d.variants.len() as u32);
                                self.for_array(label, lv, &arr, &Ty::Enum(x), d.variants.len() as i64, body, forced, &e.span);
                                return;
                            }
                        }
                        _ => {}
                    }
                }
            }
            ExprKind::Call(_, args) => {
                if let Some(CallTarget::TypeFn(_, n)) = self.ctx.calls.get(&it.id).cloned() {
                    if n == "range" && args.len() == 2 {
                        let a = args[0].value.clone();
                        let b = args[1].value.clone();
                        let va = match self.const_int(&a) {
                            Some(c) => Val::Imm(c),
                            None => {
                                let v = self.val_of(&a);
                                self.materialize_val(&v, esz)
                            }
                        };
                        let vb = match self.const_int(&b) {
                            Some(c) => Val::Imm(c),
                            None => {
                                let v = self.val_of(&b);
                                self.materialize_val(&v, esz)
                            }
                        };
                        let both = matches!(va, Val::Imm(_)) && matches!(vb, Val::Imm(_));
                        range = Some((va, vb, true, both));
                    }
                }
            }
            _ => {}
        }
        if let Some((va, vb, incl, both_const)) = range {
            self.for_range(label, lv, &et, esz, va, vb, incl, both_const, rev, body, forced, &e.span);
            return;
        }
        if rev {
            self.unimpl("範囲でないものの rev()", &iter.span);
            return;
        }
        // 配列・文字列
        let itt = self.ty_of(&it);
        match &itt {
            Ty::Array(iet, len) => {
                let n = self.ctx.types.len_count(len) as i64;
                let arr = self.val_of(&it);
                let iet = (**iet).clone();
                self.for_array(label, lv, &arr, &iet, n, body, forced, &e.span);
            }
            Ty::Str(StrK::P, _) => self.for_str(label, lv, &it, body),
            _ => self.unimpl(&format!("{} を for で回すこと", self.ctx.tyname(&itt)), &iter.span),
        }
    }

    fn elem_type_of_iter(&self, iter: &Expr) -> Ty {
        match self.ctx.ety.get(&iter.id) {
            Some(Ty::RangeIter(t)) => (**t).clone(),
            Some(Ty::Array(t, _)) => (**t).clone(),
            Some(Ty::TypeName(t)) => (**t).clone(),
            _ => Ty::Int(IntK::U8),
        }
    }

    /// ループ変数の置き場所を決める
    fn loop_var_slot(&mut self, lv: Option<usize>, esz: u32, body: &Block, forced: &Option<String>, sp: &Span, need_index: bool) -> Option<char> {
        let want_reg: Option<char> = match forced.as_deref() {
            Some("x") => Some('x'),
            Some("y") => Some('y'),
            Some(_) => {
                self.ctx.diags.err("E0703", "#[reg(…)] には x か y を書きます", sp.clone());
                None
            }
            None => None,
        };
        let can = |g: &Gen, r: char| -> bool { esz == 1 && g.body_allows_reg(body, r) && !g.f.xres && !g.f.yres };
        let reg = match want_reg {
            Some(r) => {
                if !can(self, r) {
                    self.ctx.diags.error(
                        crate::diag::Diag::error("E0703", &format!("ループ変数を {} に置けません", r.to_ascii_uppercase()), sp.clone())
                            .note("ループの中で関数を呼ぶ・ループの外の値を指すなど、そのレジスタが壊れる処理があります"),
                    );
                    None
                } else {
                    Some(r)
                }
            }
            None => {
                if can(self, 'x') && !need_index {
                    Some('x')
                } else if can(self, 'x') && need_index {
                    Some('x')
                } else {
                    None
                }
            }
        };
        if let Some(l) = lv {
            match reg {
                Some('x') => {
                    self.f.slots.insert(l, Slot::RegX);
                }
                Some(_) => {
                    self.f.slots.insert(l, Slot::RegY);
                }
                None => {
                    let t = self.ctx.funcs[self.f.fid].locals[l].ty.clone();
                    let s = self.new_slot(&t);
                    self.f.slots.insert(l, s);
                }
            }
        }
        match reg {
            Some('x') => self.f.xres = true,
            Some(_) => self.f.yres = true,
            None => {}
        }
        if reg.is_some() {
            let name = lv.map(|l| self.ctx.funcs[self.f.fid].locals[l].name.clone()).unwrap_or_default();
            self.notes.push(format!("ループ変数 {}（{}行）を {} に置いた", name, sp.line, reg.unwrap_or('x').to_ascii_uppercase()));
        }
        reg
    }

    fn end_loop_var(&mut self, reg: Option<char>) {
        match reg {
            Some('x') => self.f.xres = false,
            Some(_) => self.f.yres = false,
            None => {}
        }
        self.f.xcache = None;
        self.f.ycache = None;
    }

    #[allow(clippy::too_many_arguments)]
    fn for_range(
        &mut self,
        label: &Option<String>,
        lv: Option<usize>,
        et: &Ty,
        esz: u32,
        va: Val,
        vb: Val,
        incl: bool,
        both_const: bool,
        rev: bool,
        body: &Block,
        forced: Option<String>,
        sp: &Span,
    ) {
        if self.ctx.types.is_signed(et) {
            self.unimpl("符号付きの整数の範囲の for", sp);
            return;
        }
        let (ca, cb) = (match &va {
            Val::Imm(x) => Some(*x),
            _ => None,
        }, match &vb {
            Val::Imm(x) => Some(*x),
            _ => None,
        });
        if both_const {
            let (a, b) = (ca.unwrap_or(0), cb.unwrap_or(0));
            let empty = if incl { a > b } else { a >= b };
            if empty {
                return;
            }
        }
        let top = self.new_label("for");
        let cont = self.new_label("next");
        let brk = self.new_label("brk");
        let reg = self.loop_var_slot(lv, esz, body, &forced, sp, false);
        let var: Val = match (lv, reg) {
            (_, Some('x')) => Val::Reg('x'),
            (_, Some(_)) => Val::Reg('y'),
            (Some(l), None) => self.local_val(l),
            (None, None) => self.tmp_val(esz),
        };
        if esz != 1 && esz != 2 {
            self.unimpl("3 バイト以上の範囲の for", sp);
            return;
        }
        if esz == 2 && rev {
            self.unimpl("16 ビットの範囲の rev()", sp);
            return;
        }
        let ld = |g: &mut Gen, v: &Val, var: &Val| match var {
            Val::Reg(r) => g.load_reg(*r, v, 0),
            _ => {
                let n = g.val_size(var);
                g.copy_val(v, var, n);
            }
        };
        // 比べる：var と v（1 バイト）
        let cmp1 = |g: &mut Gen, var: &Val, v: &Val| match var {
            Val::Reg(r) => {
                let mn = if *r == 'x' { "cpx" } else { "cpy" };
                match v {
                    Val::Imm(x) => g.imm(mn, (*x & 0xFF) as i64, "終わりと比べる"),
                    Val::Mem { base, idx: Idx::None, .. } => g.emit(mn, AMode::Mem(base.expr()), "終わりと比べる"),
                    _ => g.unimpl("この終わりの値との比較", &Span::none()),
                }
            }
            _ => {
                g.emit_byte("lda", var, 0, "ループ変数");
                g.emit_byte("cmp", v, 0, "終わりと比べる");
            }
        };
        self.f.loops.push(GLoop { label: label.clone(), brk: brk.clone(), cont: cont.clone(), result: None, atomic: self.f.atomic });
        if !rev {
            ld(self, &va, &var);
            if esz == 1 {
                if !both_const {
                    // 空かどうか
                    if incl {
                        // b < a なら空
                        match &var {
                            Val::Reg(_) => {
                                let tv = self.materialize_val(&vb, 1);
                                cmp1(self, &var, &tv);
                                // var（=a）と b：a > b なら空 → b < a
                                let skip = self.new_label("ne");
                                self.br("beq", &skip, "");
                                self.br("bcs", &brk, "始めが終わりより大きければ回らない");
                                self.label(&skip);
                            }
                            _ => {
                                self.emit_byte("lda", &vb, 0, "終わり");
                                self.emit_byte("cmp", &var, 0, "始めと比べる");
                                self.br("bcc", &brk, "終わりが始めより小さければ回らない");
                            }
                        }
                    } else {
                        cmp1(self, &var, &vb);
                        self.br("bcs", &brk, "始めが終わり以上なら回らない");
                    }
                }
                self.label(&top);
                self.gen_block(body, &Want::Effect);
                self.label(&cont);
                if incl {
                    cmp1(self, &var, &vb);
                    self.br("beq", &brk, "終わりに達した");
                    self.emit_rmw("inc", &var, 0, "次の値へ");
                    self.jmp(&top, "繰り返す");
                } else {
                    self.emit_rmw("inc", &var, 0, "次の値へ");
                    cmp1(self, &var, &vb);
                    self.br("bne", &top, "終わりに達していなければ繰り返す");
                }
            } else {
                // 16 ビット
                if !both_const {
                    let (t1, t2) = (var.clone(), vb.clone());
                    self.emit_byte("lda", &t1, 0, "始め");
                    self.emit_byte("cmp", &t2, 0, "終わりと比べる");
                    self.emit_byte("lda", &t1, 1, "上位");
                    self.emit_byte("sbc", &t2, 1, "");
                    if incl {
                        let ok = self.new_label("ok");
                        self.br("bcc", &ok, "始め < 終わり");
                        self.emit_byte("lda", &t1, 0, "");
                        self.emit_byte("cmp", &t2, 0, "");
                        self.br("bne", &brk, "");
                        self.emit_byte("lda", &t1, 1, "");
                        self.emit_byte("cmp", &t2, 1, "");
                        self.br("bne", &brk, "");
                        self.label(&ok);
                    } else {
                        self.br("bcs", &brk, "始めが終わり以上なら回らない");
                    }
                }
                self.label(&top);
                self.gen_block(body, &Want::Effect);
                self.label(&cont);
                let eq16 = |g: &mut Gen, l: &str, jump_if_eq: bool, var: &Val, vb: &Val| {
                    let skip = g.new_label("ne");
                    g.emit_byte("lda", var, 0, "ループ変数（下位）");
                    g.emit_byte("cmp", vb, 0, "終わりと比べる");
                    g.br("bne", if jump_if_eq { skip.as_str() } else { l }, "");
                    g.emit_byte("lda", var, 1, "上位");
                    g.emit_byte("cmp", vb, 1, "");
                    g.br(if jump_if_eq { "beq" } else { "bne" }, l, "");
                    g.label(&skip);
                };
                if incl {
                    eq16(self, brk.as_str(), true, &var, &vb);
                    self.inc16(&var);
                    self.jmp(&top, "繰り返す");
                } else {
                    self.inc16(&var);
                    eq16(self, top.as_str(), false, &var, &vb);
                }
            }
        } else {
            // 逆順：b-1（含むなら b）から a まで
            if let (Some(0), Some(b), Val::Reg(r), false) = (ca, cb, &var, incl) {
                if b <= 129 {
                    let r = *r;
                    let mn_ld = if r == 'x' { "ldx" } else { "ldy" };
                    let mn_dec = if r == 'x' { "dex" } else { "dey" };
                    self.imm(mn_ld, (b - 1) as i64, "逆順：最後の値から");
                    self.label(&top);
                    self.gen_block(body, &Want::Effect);
                    self.label(&cont);
                    self.emit0(mn_dec, "1 減らす");
                    self.br("bpl", &top, "負にならない間くり返す（比較が要らない）");
                    self.label(&brk);
                    self.f.loops.pop();
                    self.end_loop_var(reg);
                    if let Some(l) = lv {
                        self.f.slots.remove(&l);
                    }
                    return;
                }
            }
            ld(self, &vb, &var);
            if !both_const {
                // 空かどうか
                match &var {
                    Val::Reg(_) => {
                        let ta = self.materialize_val(&va, 1);
                        cmp1(self, &var, &ta);
                        if incl {
                            self.br("bcc", &brk, "終わりが始めより小さければ回らない");
                        } else {
                            self.br("beq", &brk, "空");
                            self.br("bcc", &brk, "空");
                        }
                    }
                    _ => {
                        cmp1(self, &var, &va);
                        if incl {
                            self.br("bcc", &brk, "回らない");
                        } else {
                            self.br("beq", &brk, "空");
                            self.br("bcc", &brk, "空");
                        }
                    }
                }
            }
            if incl {
                self.label(&top);
                self.gen_block(body, &Want::Effect);
                self.label(&cont);
                cmp1(self, &var, &va);
                self.br("beq", &brk, "始めに達した");
                self.emit_rmw("dec", &var, 0, "1 減らす");
                self.jmp(&top, "繰り返す");
            } else {
                self.label(&top);
                self.emit_rmw("dec", &var, 0, "逆順：1 減らす");
                self.gen_block(body, &Want::Effect);
                self.label(&cont);
                cmp1(self, &var, &va);
                self.br("bne", &top, "始めに達していなければ繰り返す");
            }
        }
        self.label(&brk);
        self.f.loops.pop();
        self.end_loop_var(reg);
        if let Some(l) = lv {
            self.f.slots.remove(&l);
        }
    }

    fn inc16(&mut self, v: &Val) {
        let l = self.new_label("inc");
        self.emit_rmw("inc", v, 0, "次の値へ（下位）");
        self.br("bne", &l, "繰り上がりなし");
        self.emit_rmw("inc", v, 1, "上位に繰り上げる");
        self.label(&l);
    }

    #[allow(clippy::too_many_arguments)]
    fn for_array(&mut self, label: &Option<String>, lv: Option<usize>, arr: &Val, et: &Ty, n: i64, body: &Block, forced: Option<String>, sp: &Span) {
        let es = self.sz(et);
        let top = self.new_label("for");
        let cont = self.new_label("next");
        let brk = self.new_label("brk");
        let planar = self.array_planar(et);
        let base = match arr {
            Val::Mem { base, idx: Idx::None, .. } => base.clone(),
            _ => {
                self.unimpl("この配列を for で回すこと", sp);
                return;
            }
        };
        // 添字は X（使えれば）か一時領域。ループ変数自身は一時領域に置く
        let idx_reg = if forced.is_none() && self.body_allows_reg(body, 'x') && !self.f.xres && !self.f.yres { Some('x') } else { None };
        if forced.is_some() {
            self.unimpl("配列の for での #[reg(…)]", sp);
        }
        if let Some(l) = lv {
            let t = self.ctx.funcs[self.f.fid].locals[l].ty.clone();
            let s = self.new_slot(&t);
            self.f.slots.insert(l, s);
        }
        let var = match lv {
            Some(l) => self.local_val(l),
            None => self.tmp_val(es.max(1)),
        };
        let counter = if idx_reg.is_none() { Some(self.tmp_val(1)) } else { None };
        if idx_reg.is_some() {
            self.f.xres = true;
            self.imm("ldx", 0, "for：0 番目から");
        } else if let Some(c) = &counter {
            let c = c.clone();
            self.store_zero(&c, 0, "for：0 番目から");
        }
        self.f.loops.push(GLoop { label: label.clone(), brk: brk.clone(), cont: cont.clone(), result: None, atomic: self.f.atomic });
        self.label(&top);
        for k in 0..es {
            let off = if planar { k as i64 * n } else { k as i64 };
            let a = base.add(off).expr();
            match (&idx_reg, &counter) {
                (Some(_), _) => self.emit("lda", AMode::MemX(a), "要素を読む"),
                (None, Some(c)) => {
                    if let Val::Mem { base: cb, .. } = c {
                        let cb = cb.clone();
                        let r = self.idx_reg();
                        self.load_index(r, &cb);
                        if r == 'x' {
                            self.emit("lda", AMode::MemX(a), "要素を読む")
                        } else {
                            self.emit("lda", AMode::MemY(a), "要素を読む")
                        }
                    }
                }
                _ => {}
            }
            self.store_byte(&var, k, "ループ変数に入れる");
        }
        self.gen_block(body, &Want::Effect);
        self.label(&cont);
        match (&idx_reg, &counter) {
            (Some(_), _) => {
                self.emit0("inx", "次の要素へ");
                if n < 256 {
                    self.imm("cpx", n, "要素の数と比べる");
                }
                self.br("bne", &top, "残っていれば繰り返す");
            }
            (None, Some(c)) => {
                let c = c.clone();
                self.emit_rmw("inc", &c, 0, "次の要素へ");
                if n < 256 {
                    self.emit_byte("lda", &c, 0, "番号");
                    self.imm("cmp", n, "要素の数と比べる");
                }
                self.br("bne", &top, "残っていれば繰り返す");
            }
            _ => {}
        }
        self.label(&brk);
        self.f.loops.pop();
        if idx_reg.is_some() {
            self.f.xres = false;
        }
        self.f.xcache = None;
        if let Some(l) = lv {
            self.f.slots.remove(&l);
        }
    }

    fn for_str(&mut self, label: &Option<String>, lv: Option<usize>, s: &Expr, body: &Block) {
        let top = self.new_label("for");
        let cont = self.new_label("next");
        let brk = self.new_label("brk");
        let sv = self.val_of(s);
        // 定数の文字列ならそのアドレス、変数ならゼロページのポインタ
        let (data, zp) = match &sv {
            Val::Addr(b) => (Some(b.clone()), None),
            Val::Mem { base, idx: Idx::None, .. } if matches!(base, Base::Sym(n, _) if n.starts_with("Z_")) => (None, Some(base.clone())),
            other => {
                let z = self.alloc_ztmp(2);
                let zv = Val::mem(z.clone(), 2);
                let o = other.clone();
                self.copy_val(&o, &zv, 2);
                (None, Some(z))
            }
        };
        if let Some(l) = lv {
            let t = self.ctx.funcs[self.f.fid].locals[l].ty.clone();
            let sl = self.new_slot(&t);
            self.f.slots.insert(l, sl);
        }
        let var = match lv {
            Some(l) => self.local_val(l),
            None => self.tmp_val(1),
        };
        let kcount = self.tmp_val(1);
        let icount = self.tmp_val(1);
        match (&data, &zp) {
            (Some(d), _) => self.emit("lda", AMode::Mem(d.expr()), "文字列の長さ"),
            (_, Some(z)) => self.emit("lda", AMode::Ind(z.expr()), "文字列の長さ（先頭のバイト）"),
            _ => {}
        }
        self.store_byte(&kcount, 0, "残りの文字数");
        self.br("beq", &brk, "長さ 0 なら回らない");
        self.imm("lda", 1, "1 文字目から");
        self.store_byte(&icount, 0, "位置");
        self.f.loops.push(GLoop { label: label.clone(), brk: brk.clone(), cont: cont.clone(), result: None, atomic: self.f.atomic });
        self.label(&top);
        if let Val::Mem { base: ib, .. } = &icount {
            let ib = ib.clone();
            match (&data, &zp) {
                (Some(d), _) => {
                    let r = self.idx_reg();
                    self.load_index(r, &ib);
                    let am = if r == 'x' { AMode::MemX(d.expr()) } else { AMode::MemY(d.expr()) };
                    self.emit("lda", am, "for c in s：1 文字読む");
                }
                (_, Some(z)) => {
                    self.load_index('y', &ib);
                    self.emit("lda", AMode::IndY(z.expr()), "for c in s：1 文字読む");
                }
                _ => {}
            }
        }
        self.store_byte(&var, 0, "文字をループ変数に");
        self.gen_block(body, &Want::Effect);
        self.label(&cont);
        self.emit_rmw("inc", &icount, 0, "次の位置へ");
        self.emit_rmw("dec", &kcount, 0, "残りを 1 減らす");
        self.br("bne", &top, "残っていれば繰り返す");
        self.label(&brk);
        self.f.loops.pop();
        if let Some(l) = lv {
            self.f.slots.remove(&l);
        }
    }

    // =====================================================================
    // delay（12.1節）
    // =====================================================================

    pub fn emit_delay(&mut self, n: i128, sp: &Span) {
        if n <= 0 {
            return;
        }
        if self.f.xres || self.f.yres {
            self.unimpl("レジスタに置いたループ変数がある中での delay()", sp);
        }
        let n = n as u64;
        let total: u64;
        if n <= 6 {
            let k = (n + 1) / 2;
            for _ in 0..k {
                self.emit0("nop", "delay：2 サイクル");
            }
            total = 2 * k;
        } else if n <= 1276 {
            let k = ((n - 1) + 4) / 5;
            let l = self.new_label("dly");
            self.imm("ldx", (k & 0xFF) as i64, &format!("delay({})：待ちの回数", n));
            self.label(&l);
            self.emit0("dex", "1 減らす");
            self.br("bne", &l, "0 になるまで待つ");
            total = 5 * k + 1;
        } else {
            let mut m = ((n - 1) + 1285) / 1286;
            let mut t = 0u64;
            while m > 0 {
                let mm = m.min(255);
                let l1 = self.new_label("dly");
                let l2 = self.new_label("dly");
                self.imm("ldy", mm as i64, &format!("delay({})：外側の回数", n));
                self.label(&l1);
                self.imm("ldx", 0, "内側は 256 回");
                self.label(&l2);
                self.emit0("dex", "");
                self.br("bne", &l2, "");
                self.emit0("dey", "");
                self.br("bne", &l1, "");
                t += 2 + mm * 1286 - 1;
                m -= mm;
            }
            total = t;
        }
        self.notes.push(format!("delay({})（{}行）：{} サイクル待つ", n, sp.line, total));
    }

    // =====================================================================
    // インラインアセンブリ（14章）
    // =====================================================================

    pub fn translate_asm(&mut self, text: &AsmText, prefix: &str, is_fn: bool, is_unsafe: bool, sp: &Span) {
        let lines = match parse_lines(&text.text) {
            Ok(l) => l,
            Err((ln, msg)) => {
                let s = Span::new(text.file, text.line0 + ln as u32, 1, 1);
                self.ctx.diags.err("E1210", &format!("アセンブリの書き方が正しくありません：{}", msg), s);
                return;
            }
        };
        let table = OpTable::new();
        let mut local: HashMap<String, String> = HashMap::new();
        let mut smc_labels: Vec<String> = Vec::new();
        for l in &lines {
            if let Some(n) = &l.label {
                local.insert(n.clone(), format!("{}{}", prefix, n));
                if l.smc {
                    smc_labels.push(n.clone());
                }
            }
        }
        let rom = self.ctx.target.run_rom();
        let asm = crate::asm6502::Assembler::new();
        let mut depth: i32 = 0;
        let mut clobbers: Vec<char> = Vec::new();
        let mut cycles = 0u32;
        for l in &lines {
            let line_no = text.line0 + l.line as u32;
            let lsp = Span::new(text.file, line_no, 1, 1);
            if let Some(n) = &l.label {
                let u = local.get(n).cloned().unwrap_or_default();
                self.label(&u);
            }
            if l.smc {
                if rom {
                    self.ctx.diags.err("E1208", "run = rom のターゲットでは smc（自己書き換え）は使えません", lsp.clone());
                }
                let fname = self.ctx.funcs.get(self.f.fid).map(|f| f.name.clone()).unwrap_or_default();
                self.ctx.unsafe_list.push(format!("smc（{}、{}行）", fname, line_no));
            }
            let mn = match &l.mn {
                Some(m) => m.clone(),
                None => continue,
            };
            if !table.is_mnemonic(&mn) {
                self.ctx.diags.err("E1204", &format!("{} は W65C02S の命令ではありません（未定義命令は使えません）", mn), lsp.clone());
                continue;
            }
            // 検査できない命令
            let forbidden = matches!(mn.as_str(), "txs" | "sei" | "cli" | "plp" | "rti" | "brk" | "stp");
            if forbidden && !is_unsafe {
                self.ctx.diags.error(
                    crate::diag::Diag::error("E1205", &format!("{} は通常の asm では書けません（unsafe asm が必要です）", mn), lsp.clone())
                        .help("割り込みを止めるだけなら atomic { } の中に asm を書いてください"),
                );
            }
            if mn == "rts" && !is_fn {
                self.ctx.diags.err("E1202", "asm ブロックの中で rts は使えません（ブロックの外への分岐）", lsp.clone());
            }
            match mn.as_str() {
                "pha" | "phx" | "phy" | "php" => depth += 1,
                "pla" | "plx" | "ply" | "plp" => depth -= 1,
                "rts" => {
                    if depth != 0 {
                        self.ctx.diags.err("E1201", "rts の時点でスタックの深さが入口と違います", lsp.clone());
                    }
                }
                _ => {}
            }
            match mn.as_str() {
                "lda" | "pla" | "txa" | "tya" | "adc" | "sbc" | "and" | "ora" | "eor" => clobbers.push('A'),
                "ldx" | "plx" | "tax" | "inx" | "dex" | "tsx" => clobbers.push('X'),
                "ldy" | "ply" | "tay" | "iny" | "dey" => clobbers.push('Y'),
                "asl" | "lsr" | "rol" | "ror" | "inc" | "dec" => {
                    if l.operand == Operand::Acc {
                        clobbers.push('A');
                    }
                }
                "jsr" => {
                    clobbers.push('A');
                    clobbers.push('X');
                    clobbers.push('Y');
                }
                _ => {}
            }
            // オペランド
            let am = match &l.operand {
                Operand::Imp => AMode::Imp,
                Operand::Acc => AMode::Acc,
                Operand::Imm(x) => match self.asm_expr(x, &local, &lsp, true) {
                    Some(e) => AMode::Imm(e),
                    None => continue,
                },
                Operand::Abs(x) => {
                    if is_branch(&mn) {
                        let tgt = match x.terms.as_slice() {
                            [(1, Term::Ident(n))] => local.get(n).cloned(),
                            _ => None,
                        };
                        match tgt {
                            Some(t) => AMode::Rel(t),
                            None => {
                                self.ctx.diags.err("E1202", "分岐の飛び先は、ブロックの中のラベルだけです", lsp.clone());
                                continue;
                            }
                        }
                    } else {
                        let e = match self.asm_expr(x, &local, &lsp, false) {
                            Some(e) => e,
                            None => continue,
                        };
                        if mn == "jmp" {
                            let is_local = matches!(&e.sym, Some(s) if local.values().any(|v| v == s));
                            if !is_local {
                                self.ctx.diags.error(
                                    crate::diag::Diag::error("E1202", "ブロックの外への jmp は書けません", lsp.clone()).help("jsr {関数} を使ってください"),
                                );
                            }
                        }
                        if mn == "jsr" {
                            if e.sym.is_none() {
                                self.ctx.diags.error(
                                    crate::diag::Diag::error("E1203", "数値のアドレスへの jsr は書けません", lsp.clone()).help("extern fn で宣言して jsr {名前} と書いてください"),
                                );
                            }
                        }
                        let writes = matches!(mn.as_str(), "sta" | "stx" | "sty" | "stz" | "inc" | "dec" | "asl" | "lsr" | "rol" | "ror" | "tsb" | "trb");
                        if writes && e.sym.is_none() && !is_unsafe {
                            self.ctx.diags.err("E1205", "数値のアドレスへの書き込みは通常の asm では書けません（unsafe asm が必要です）", lsp.clone());
                        }
                        if writes && e.sym.is_some() {
                            let s = e.sym.clone().unwrap_or_default();
                            let is_code_label = local.iter().any(|(k, v)| v == &s && !smc_labels.contains(k));
                            if is_code_label {
                                self.ctx.diags.err("E1207", "smc を付けていない命令を書き換えています", lsp.clone());
                            }
                        }
                        if writes && e.sym.is_none() && is_unsafe {
                            let fname = self.ctx.funcs.get(self.f.fid).map(|f| f.name.clone()).unwrap_or_default();
                            self.ctx.unsafe_list.push(format!("unsafe asm の数値のアドレス ${:04X}（{}、{}行）", e.off, fname, line_no));
                        }
                        AMode::Mem(e)
                    }
                }
                Operand::AbsX(x) => match self.asm_expr(x, &local, &lsp, false) {
                    Some(e) => AMode::MemX(e),
                    None => continue,
                },
                Operand::AbsY(x) => match self.asm_expr(x, &local, &lsp, false) {
                    Some(e) => AMode::MemY(e),
                    None => continue,
                },
                Operand::IndY(x) => match self.asm_expr(x, &local, &lsp, false) {
                    Some(e) => AMode::IndY(e),
                    None => continue,
                },
                Operand::IndX(x) => match self.asm_expr(x, &local, &lsp, false) {
                    Some(e) => {
                        if mn == "jmp" {
                            if !is_unsafe {
                                self.ctx.diags.err("E1205", "間接ジャンプは通常の asm では書けません", lsp.clone());
                            }
                            AMode::IndAbsX(e)
                        } else {
                            AMode::IndX(e)
                        }
                    }
                    None => continue,
                },
                Operand::Ind(x) => match self.asm_expr(x, &local, &lsp, false) {
                    Some(e) => {
                        if mn == "jmp" && !is_unsafe {
                            self.ctx.diags.err("E1205", "間接ジャンプは通常の asm では書けません", lsp.clone());
                        }
                        AMode::Ind(e)
                    }
                    None => continue,
                },
            };
            // CPU にない命令
            let mode = match &am {
                AMode::Imp => Mode::Imp,
                AMode::Acc => Mode::Acc,
                AMode::Imm(_) => Mode::Imm,
                AMode::Ind(_) => {
                    if mn == "jmp" {
                        Mode::Ind
                    } else {
                        Mode::Izp
                    }
                }
                AMode::IndAbsX(_) => Mode::Iax,
                AMode::Rel(_) => Mode::Rel,
                _ => Mode::Abs,
            };
            if self.opts.nmos && is_cmos_only(&mn, mode) {
                self.ctx.diags.err("E1204", &format!("{} は NMOS 6502 にない命令です", mn), lsp.clone());
            }
            let c = if l.comment.is_empty() { String::new() } else { l.comment.clone() };
            // jsr {関数}：呼び出しグラフに入れる
            if mn == "jsr" {
                if let AMode::Mem(e) = &am {
                    if let Some(s) = &e.sym {
                        let target = self.ctx.funcs.iter().position(|f| &f.label == s);
                        if let Some(f) = target {
                            self.f.calls.push(f);
                            if !self.generated.contains(&f) {
                                self.queue.push(f);
                            }
                        }
                    }
                }
            }
            let ins = crate::asm6502::Ins::new(&mn, am.clone(), &c);
            let (cy, _) = asm.ins_cycles(&ins, &HashMap::new());
            cycles += cy;
            self.emit(&mn, am, &c);
        }
        if !is_fn && depth != 0 {
            self.ctx.diags.err("E1201", &format!("asm ブロックの入口と出口でスタックの深さが違います（{:+} バイト）", depth), sp.clone());
        }
        clobbers.sort();
        clobbers.dedup();
        let cl: Vec<String> = clobbers.iter().map(|c| c.to_string()).collect();
        let fname = self.ctx.funcs.get(self.f.fid).map(|f| f.name.clone()).unwrap_or_else(|| self.f.label.clone());
        self.asm_notes.push(format!(
            "{}（{}行）：壊すレジスタ {}、フラグ。スタック {} バイト。約 {} サイクル（分岐しない場合）",
            if is_fn { format!("asm fn {}", fname) } else { format!("asm ブロック（{}）", fname) },
            sp.line,
            if cl.is_empty() { "なし".to_string() } else { cl.join("・") },
            depth.max(0),
            cycles
        ));
    }

    /// アセンブリの式を、記号＋数に直す
    fn asm_expr(&mut self, x: &AExpr, local: &HashMap<String, String>, sp: &Span, imm: bool) -> Option<OpExpr> {
        let mut sym: Option<String> = None;
        let mut off: i64 = 0;
        for (sign, t) in &x.terms {
            match t {
                Term::Num(v) => off += sign * v,
                Term::Ident(n) => match local.get(n) {
                    Some(u) => {
                        if sym.is_some() {
                            self.ctx.diags.err("E1210", "アセンブリの式に記号は 1 つだけ書けます", sp.clone());
                            return None;
                        }
                        sym = Some(u.clone());
                    }
                    None => {
                        self.ctx.diags.error(
                            crate::diag::Diag::error("E1206", &format!("{} が見つかりません", n), sp.clone()).help("言語の名前は {名前} で参照します（14.2節）"),
                        );
                        return None;
                    }
                },
                Term::Ref(r) => {
                    let (e, _) = self.asm_ref(r, sp, imm)?;
                    if let Some(s) = e.sym {
                        if sym.is_some() {
                            self.ctx.diags.err("E1210", "アセンブリの式に記号は 1 つだけ書けます", sp.clone());
                            return None;
                        }
                        sym = Some(s);
                    }
                    off += sign * e.off;
                }
            }
        }
        let mut e = OpExpr { sym, off, part: crate::asm6502::Part::Full };
        match x.sel {
            Sel::Lo => e = e.lo(),
            Sel::Hi => e = e.hi(),
            Sel::Full => {}
        }
        Some(e)
    }

    /// {名前}、{名前.lo}、{名前.hi}
    fn asm_ref(&mut self, r: &str, sp: &Span, imm: bool) -> Option<(OpExpr, u32)> {
        let (name, byte) = if let Some(n) = r.strip_suffix(".lo") {
            (n.to_string(), Some(0))
        } else if let Some(n) = r.strip_suffix(".hi") {
            (n.to_string(), Some(1))
        } else {
            (r.to_string(), None)
        };
        // ローカル変数・引数
        if !name.contains("::") {
            if let Some(v) = self.find_named_local(&name) {
                return match v {
                    Val::Mem { base, size, .. } => {
                        if size > 1 && byte.is_none() {
                            self.ctx.diags.error(
                                crate::diag::Diag::error("E1206", &format!("{} は {} バイトなので、{{{}.lo}}・{{{}.hi}} のようにバイトを指定してください", name, size, name, name), sp.clone()),
                            );
                        }
                        Some((base.add(byte.unwrap_or(0)).expr(), size))
                    }
                    _ => {
                        self.ctx.diags.err(E_UNIMPL_ASM, &format!("未実装：レジスタに置いた変数 {} のアセンブリからの参照", name), sp.clone());
                        None
                    }
                };
            }
        }
        let segs: Vec<String> = name.split("::").map(|s| s.to_string()).collect();
        let m = self.f.module;
        match self.ctx.resolve_value_path(m, &segs, sp) {
            PathRes::Global(g) => {
                let gi = self.ctx.globals[g].clone();
                let size = self.sz(&gi.ty);
                let v = self.global_val(g);
                match v {
                    Val::Imm(x) => Some((OpExpr::num(((x >> (8 * byte.unwrap_or(0))) & if imm { 0xFF } else { 0xFFFF }) as i64), size)),
                    Val::Mem { base, .. } | Val::Addr(base) => {
                        if size > 1 && byte.is_none() && !matches!(gi.ty, Ty::Array(_, _) | Ty::Str(_, _)) {
                            self.ctx.diags.error(
                                crate::diag::Diag::error("E1206", &format!("{} は {} バイトなので、{{{}.lo}}・{{{}.hi}} のようにバイトを指定してください", name, size, name, name), sp.clone()),
                            );
                        }
                        Some((base.add(byte.unwrap_or(0)).expr(), size))
                    }
                    _ => None,
                }
            }
            PathRes::Func(f) => {
                let l = self.ctx.funcs[f].label.clone();
                if self.ctx.funcs[f].kind == FnKind::Extern {
                    let addr = self.ctx.funcs[f]
                        .decl
                        .as_ref()
                        .and_then(|d| d.extern_addr.clone())
                        .and_then(|a| self.ctx.eval_in_module(&a, self.ctx.funcs[f].module, Some(&Ty::CInt)).ok())
                        .and_then(|v| value_int(&v))
                        .unwrap_or(0);
                    return Some((OpExpr::num(addr as i64), 2));
                }
                Some((OpExpr::sym(&l), 2))
            }
            PathRes::Value(v) => Some((OpExpr::num(value_int(&v).unwrap_or(0) as i64), 1)),
            _ => {
                self.ctx.diags.err("E0201", &format!("アセンブリの中の {{{}}} が見つかりません", r), sp.clone());
                None
            }
        }
    }
}

pub const E_UNIMPL_ASM: &str = "E0000";
