// コード生成（W65C02S、-O0 相当の素直なコード）：関数・文・制御構文
use crate::asm6502::{AMode, Ins, Item, OpExpr, Part};
use crate::ast::*;
use crate::comptime::{value_int, Value};
use crate::diag::{Span, E_UNIMPL};
use crate::program::*;
use crate::project::{Exit, VecMode};
use crate::types::*;
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone, Debug, PartialEq)]
pub enum Slot {
    Mem(u32),
    Zp(u32),
    RegX,
    RegY,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Base {
    Sym(String, i64),
    Abs(i64),
}

impl Base {
    pub fn expr(&self) -> OpExpr {
        match self {
            Base::Sym(s, o) => OpExpr::sym_off(s, *o),
            Base::Abs(a) => OpExpr::num(*a),
        }
    }
    pub fn add(&self, d: i64) -> Base {
        match self {
            Base::Sym(s, o) => Base::Sym(s.clone(), o + d),
            Base::Abs(a) => Base::Abs(a + d),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Idx {
    None,
    X,
    Y,
    Mem(Base),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Imm(i128),
    Addr(Base),
    Mem { base: Base, idx: Idx, stride: i64, size: u32, io: bool, rmw: bool },
    Ind { zp: Base, idx: Idx, off: i64, size: u32 },
    Ext { inner: Box<Val>, from: u32 },
    Reg(char),
}

impl Val {
    pub fn mem(base: Base, size: u32) -> Val {
        Val::Mem { base, idx: Idx::None, stride: 1, size, io: false, rmw: false }
    }
    /// バイト from から size バイトの部分
    pub fn part(&self, from: u32, size: u32) -> Val {
        match self {
            Val::Imm(v) => Val::Imm((v >> (8 * from)) & ((1i128 << (8 * size.min(15))) - 1)),
            Val::Mem { base, idx, stride, io, rmw, .. } => Val::Mem {
                base: base.add(from as i64 * stride),
                idx: idx.clone(),
                stride: *stride,
                size,
                io: *io,
                rmw: *rmw,
            },
            Val::Ind { zp, idx, off, .. } => Val::Ind { zp: zp.clone(), idx: idx.clone(), off: off + from as i64, size },
            other => other.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Want {
    Effect,
    A,
    Store(Val),
    Tail,
    Cond(String, bool),
}

#[derive(Clone, Debug)]
pub struct GLoop {
    pub label: Option<String>,
    pub brk: String,
    pub cont: String,
    pub result: Option<Val>,
    pub atomic: u32,
}

#[derive(Clone, Debug)]
pub struct FnCode {
    pub fid: usize,
    pub label: String,
    pub items: Vec<Item>,
    pub frame: u32,
    pub zframe: u32,
    pub calls: Vec<usize>,
    pub force_inline: bool,
    pub interrupt: Option<String>,
    pub is_start: bool,
    pub ext_stack: u32,
    pub emitted: bool,
}

#[derive(Default)]
pub struct FnState {
    pub fid: usize,
    pub label: String,
    pub module: usize,
    pub items: Vec<Item>,
    pub frame_top: u32,
    pub frame_max: u32,
    pub ztop: u32,
    pub zmax: u32,
    pub slots: HashMap<usize, Slot>,
    pub loops: Vec<GLoop>,
    pub xres: bool,
    pub yres: bool,
    pub atomic: u32,
    pub interrupt: bool,
    pub epi: String,
    pub ret: Option<Ty>,
    pub calls: Vec<usize>,
    pub last_line: (usize, u32),
    /// X・Y が持っている値（その番地の中身）
    pub xcache: Option<OpExpr>,
    pub ycache: Option<OpExpr>,
    /// A が持っている値：番地の中身（acache）か定数（aimm）。nz_a：N と Z が A の値を表している
    pub acache: Option<OpExpr>,
    pub aimm: Option<i64>,
    pub nz_a: bool,
    /// アセンブリ・cycles() の中（命令をそのまま出す）
    pub keep: u32,
    /// この関数が呼ぶ関数が壊すレジスタ（ビット 1=A、2=X、4=Y）
    pub callee_clob: u8,
    pub scratch: Option<u32>,
    pub ext_stack: u32,
}

pub struct GenOpts {
    pub nmos: bool,
}

pub struct Gen {
    pub ctx: Ctx,
    pub opts: GenOpts,
    pub funcs: Vec<FnCode>,
    pub generated: HashSet<usize>,
    pub queue: Vec<usize>,
    pub data: Vec<Item>,
    pub used_globals: BTreeSet<usize>,
    pub str_pool: HashMap<Vec<u8>, String>,
    pub row_tables: HashMap<String, String>,
    pub fnset_used: BTreeSet<usize>,
    pub notes: Vec<String>,
    pub asm_notes: Vec<String>,
    pub area_sizes: HashMap<String, u32>,
    pub nlabel: usize,
    pub f: FnState,
    /// 読み直しを省けない記号（shared 変数・アセンブリから参照される変数）
    pub volatile_syms: HashSet<String>,
    /// 関数ごとの、壊すレジスタ（ビット 1=A、2=X、4=Y）。呼ばれる側から先に作るので、呼ぶ側で使える
    pub clob: HashMap<usize, u8>,
}

pub fn ret_area(label: &str) -> String {
    format!("R_{}", label)
}

pub fn frame_sym(label: &str) -> String {
    format!("F_{}", label)
}

pub fn zframe_sym(label: &str) -> String {
    format!("Z_{}", label)
}

/// 引数の渡し方（15.2節）
#[derive(Clone, Debug)]
pub struct ParamLoc {
    pub regs: Vec<char>,
    pub slot: Slot,
    pub size: u32,
}

pub fn generate(ctx: Ctx, opts: GenOpts) -> Gen {
    let mut g = Gen {
        ctx,
        opts,
        funcs: Vec::new(),
        generated: HashSet::new(),
        queue: Vec::new(),
        data: Vec::new(),
        used_globals: BTreeSet::new(),
        str_pool: HashMap::new(),
        row_tables: HashMap::new(),
        fnset_used: BTreeSet::new(),
        notes: Vec::new(),
        asm_notes: Vec::new(),
        area_sizes: HashMap::new(),
        nlabel: 0,
        f: FnState::default(),
        volatile_syms: HashSet::new(),
        clob: HashMap::new(),
    };
    for gi in &g.ctx.globals {
        if matches!(gi.kind, GKind::Shared(_)) || gi.asm_ref {
            g.volatile_syms.insert(gi.label.clone());
        }
    }
    if g.opts.nmos {
        let sp = Span::new(g.ctx.target.file, 1, 1, 1);
        g.ctx.diags.err(E_UNIMPL, "未実装：NMOS 6502 向けのコード生成はまだありません（65C02 の命令を使わないコードを作れません）", sp);
        return g;
    }
    // 根：main、割り込み処理、init
    let mut roots = Vec::new();
    for (i, f) in g.ctx.funcs.iter().enumerate() {
        if (f.name == "main" && f.module == 0 && f.init_body.is_none()) || f.interrupt.is_some() || f.init_body.is_some() {
            roots.push(i);
        }
    }
    // 呼ばれる側から先に作る（呼ぶ側で、呼ばれる側が壊すレジスタを使えるように）
    let order = g.callee_first_order(&roots);
    for fid in order {
        if g.generated.contains(&fid) {
            continue;
        }
        g.generated.insert(fid);
        g.gen_fn(fid);
    }
    for r in roots {
        g.queue.push(r);
    }
    while let Some(fid) = g.queue.pop() {
        if g.generated.contains(&fid) {
            continue;
        }
        g.generated.insert(fid);
        g.gen_fn(fid);
    }
    g.gen_start();
    g.gen_fnset_tables();
    g
}

impl Gen {
    // ---- 出力の補助 ----
    pub fn emit(&mut self, mn: &str, am: AMode, c: &str) {
        if self.f.keep == 0 {
            // A にすでにある値の読み直しは出さない（N と Z も A の値のままのときだけ）
            if mn == "lda" && self.f.nz_a && self.a_has(&am) {
                return;
            }
            // A と同じ値を X・Y に読むときは、転送命令にする
            if mn == "ldx" || mn == "ldy" {
                if let AMode::Mem(e) = &am {
                    if !self.is_vol(e) && self.f.acache.as_ref() == Some(e) {
                        let t = if mn == "ldx" { "tax" } else { "tay" };
                        self.track_regs(t, &AMode::Imp);
                        self.track_a(t, &AMode::Imp);
                        self.f.items.push(Item::Ins(Ins::new(t, AMode::Imp, &format!("{}（A にある値を移す）", c))));
                        return;
                    }
                }
            }
        }
        self.track_regs(mn, &am);
        self.track_a(mn, &am);
        self.f.items.push(Item::Ins(Ins::new(mn, am, c)));
    }
    /// 読み直しを省けない番地（I/O などの記号のない番地、shared 変数）
    pub fn is_vol(&self, e: &OpExpr) -> bool {
        match &e.sym {
            None => true,
            Some(s) => e.part != Part::Full || self.volatile_syms.contains(s),
        }
    }
    /// A がこの値（番地の中身・定数）を持っているか
    fn a_has(&self, am: &AMode) -> bool {
        match am {
            AMode::Mem(e) => !self.is_vol(e) && self.f.acache.as_ref() == Some(e),
            AMode::Imm(e) => e.sym.is_none() && e.part == Part::Full && self.f.aimm == Some(e.off & 0xFF),
            _ => false,
        }
    }
    /// A が値 v のバイト 0 を持っているか（v は 1 バイトの、添字のない値）
    pub fn a_holds(&self, v: &Val) -> bool {
        match v {
            Val::Imm(x) => self.f.aimm == Some((*x & 0xFF) as i64),
            Val::Mem { base, idx: Idx::None, io: false, .. } => {
                let e = base.expr();
                !self.is_vol(&e) && self.f.acache.as_ref() == Some(&e)
            }
            Val::Reg('a') => true,
            _ => false,
        }
    }
    pub fn forget_a(&mut self) {
        self.f.acache = None;
        self.f.aimm = None;
        self.f.nz_a = false;
    }
    /// アセンブリ・cycles() の始まりと終わり（中の命令はどの処理も変えない）
    pub fn keep_begin(&mut self) {
        self.f.items.push(Item::Mark(crate::peep::KEEP_BEGIN.to_string()));
        self.f.keep += 1;
        self.forget_a();
        self.f.xcache = None;
        self.f.ycache = None;
    }
    pub fn keep_end(&mut self) {
        self.f.items.push(Item::Mark(crate::peep::KEEP_END.to_string()));
        if self.f.keep > 0 {
            self.f.keep -= 1;
        }
        self.forget_a();
        self.f.xcache = None;
        self.f.ycache = None;
    }
    fn track_a(&mut self, mn: &str, am: &AMode) {
        let same_sym = |x: &Option<OpExpr>, e: &OpExpr| -> bool {
            match x {
                Some(c) => e.sym.is_none() || c.sym == e.sym,
                None => false,
            }
        };
        match mn {
            "lda" => {
                match am {
                    AMode::Mem(e) if !self.is_vol(e) => {
                        self.f.acache = Some(e.clone());
                        self.f.aimm = None;
                    }
                    AMode::Imm(e) if e.sym.is_none() && e.part == Part::Full => {
                        self.f.aimm = Some(e.off & 0xFF);
                        self.f.acache = None;
                    }
                    _ => {
                        self.f.acache = None;
                        self.f.aimm = None;
                    }
                }
                self.f.nz_a = true;
            }
            "sta" => match am {
                AMode::Mem(e) if !self.is_vol(e) => self.f.acache = Some(e.clone()),
                AMode::Mem(_) => {}
                AMode::MemX(e) | AMode::MemY(e) => {
                    if same_sym(&self.f.acache, e) {
                        self.f.acache = None;
                    }
                }
                _ => self.f.acache = None,
            },
            "stx" | "sty" | "stz" => match am {
                AMode::Mem(e) => {
                    if self.f.acache.as_ref() == Some(e) {
                        self.f.acache = None;
                    }
                }
                AMode::MemX(e) | AMode::MemY(e) => {
                    if same_sym(&self.f.acache, e) {
                        self.f.acache = None;
                    }
                }
                _ => self.f.acache = None,
            },
            "inc" | "dec" | "asl" | "lsr" | "rol" | "ror" | "trb" | "tsb" => match am {
                AMode::Imp | AMode::Acc => {
                    self.f.acache = None;
                    self.f.aimm = None;
                    self.f.nz_a = true;
                }
                AMode::Mem(e) => {
                    if self.f.acache.as_ref() == Some(e) {
                        self.f.acache = None;
                    }
                    self.f.nz_a = false;
                }
                AMode::MemX(e) | AMode::MemY(e) => {
                    if same_sym(&self.f.acache, e) {
                        self.f.acache = None;
                    }
                    self.f.nz_a = false;
                }
                _ => {
                    self.f.acache = None;
                    self.f.nz_a = false;
                }
            },
            "and" | "ora" | "eor" | "adc" | "sbc" | "pla" | "txa" | "tya" => {
                self.f.acache = None;
                self.f.aimm = None;
                self.f.nz_a = true;
            }
            "tax" | "tay" => {}
            "cmp" | "cpx" | "cpy" | "bit" | "inx" | "dex" | "iny" | "dey" | "ldx" | "ldy" | "plx" | "ply" | "tsx" | "plp" => self.f.nz_a = false,
            "clc" | "sec" | "sed" | "cld" | "sei" | "cli" | "clv" | "nop" | "pha" | "phx" | "phy" | "php" | "txs" => {}
            _ => self.forget_a(),
        }
    }
    pub fn emit0(&mut self, mn: &str, c: &str) {
        let am = if matches!(mn, "asl" | "lsr" | "rol" | "ror") { AMode::Acc } else { AMode::Imp };
        self.emit(mn, am, c);
    }
    pub fn imm(&mut self, mn: &str, v: i64, c: &str) {
        self.emit(mn, AMode::Imm(OpExpr::num(v & 0xFF)), c);
    }
    pub fn label(&mut self, l: &str) {
        self.f.xcache = None;
        self.f.ycache = None;
        self.forget_a();
        self.f.items.push(Item::Label(l.to_string()));
    }
    pub fn new_label(&mut self, hint: &str) -> String {
        self.nlabel += 1;
        format!("{}@{}{}", self.f.label, hint, self.nlabel)
    }
    pub fn br(&mut self, mn: &str, l: &str, c: &str) {
        self.f.items.push(Item::Ins(Ins::new(mn, AMode::Rel(l.to_string()), c)));
    }
    pub fn jmp(&mut self, l: &str, c: &str) {
        self.f.items.push(Item::Ins(Ins::new("bra", AMode::Rel(l.to_string()), c)));
    }
    fn track_regs(&mut self, mn: &str, am: &AMode) {
        match mn {
            "ldx" | "tax" | "tsx" | "inx" | "dex" | "plx" => self.f.xcache = None,
            "ldy" | "tay" | "iny" | "dey" | "ply" => self.f.ycache = None,
            "jsr" | "jmp" | "rts" | "rti" => {
                self.f.xcache = None;
                self.f.ycache = None;
            }
            // 書き込み：その番地（添字付きなら同じ記号、ポインタ経由ならすべて）の値を持つ X・Y を忘れる
            "sta" | "stx" | "sty" | "stz" | "tsb" | "trb" | "inc" | "dec" | "asl" | "lsr" | "rol" | "ror" => {
                let hit = |c: &Option<OpExpr>| -> bool {
                    match (c, am) {
                        (None, _) => false,
                        (_, AMode::Imp) | (_, AMode::Acc) => false,
                        (Some(x), AMode::Mem(e)) => x == e,
                        (Some(x), AMode::MemX(e)) | (Some(x), AMode::MemY(e)) => e.sym.is_none() || x.sym == e.sym,
                        _ => true,
                    }
                };
                if hit(&self.f.xcache) {
                    self.f.xcache = None;
                }
                if hit(&self.f.ycache) {
                    self.f.ycache = None;
                }
            }
            _ => {}
        }
    }
    pub fn unimpl(&mut self, what: &str, sp: &Span) {
        self.ctx.diags.err(E_UNIMPL, &format!("未実装：{}", what), sp.clone());
    }
    pub fn src_line(&mut self, sp: &Span) {
        if sp.is_none() {
            return;
        }
        let key = (sp.file, sp.line);
        if self.f.last_line == key {
            return;
        }
        self.f.last_line = key;
        let text = self.ctx.sm.line_text(sp.file, sp.line);
        let name = self.ctx.sm.file_name(sp.file);
        self.f.items.push(Item::Src(format!("; {}:{}   {}", name, sp.line, text.trim())));
    }

    // ---- 型と定数 ----
    pub fn ty_of(&self, e: &Expr) -> Ty {
        match self.ctx.ety.get(&e.id) {
            Some(Ty::IntLit) => Ty::Int(IntK::U8),
            Some(t) => t.clone(),
            None => Ty::Error,
        }
    }
    pub fn sz(&self, t: &Ty) -> u32 {
        self.ctx.types.size_of(t)
    }
    pub fn is_scalar_ty(&self, t: &Ty) -> bool {
        !matches!(t, Ty::Array(_, _) | Ty::Struct(_) | Ty::Str(_, _) | Ty::Opt(_) | Ty::StrBuf(_) | Ty::Unit | Ty::Never | Ty::Error)
    }
    /// 定数の整数値（スカラーの型だけ）
    pub fn const_int(&self, e: &Expr) -> Option<i128> {
        let t = self.ty_of(e);
        if !self.is_scalar_ty(&t) {
            return None;
        }
        let v = self.ctx.econst.get(&e.id)?;
        match v {
            Value::Real(_) | Value::Cpu(_) | Value::Unit => None,
            _ => value_int(v),
        }
    }

    // ---- フレーム（ローカル変数と一時領域）----
    pub fn alloc_tmp(&mut self, size: u32) -> Base {
        let off = self.f.frame_top;
        self.f.frame_top += size.max(1);
        if self.f.frame_top > self.f.frame_max {
            self.f.frame_max = self.f.frame_top;
        }
        Base::Sym(frame_sym(&self.f.label), off as i64)
    }
    pub fn alloc_ztmp(&mut self, size: u32) -> Base {
        let off = self.f.ztop;
        self.f.ztop += size.max(1);
        if self.f.ztop > self.f.zmax {
            self.f.zmax = self.f.ztop;
        }
        Base::Sym(zframe_sym(&self.f.label), off as i64)
    }
    pub fn tmp_val(&mut self, size: u32) -> Val {
        let b = self.alloc_tmp(size);
        Val::mem(b, size)
    }
    pub fn mark(&self) -> (u32, u32) {
        (self.f.frame_top, self.f.ztop)
    }
    pub fn release(&mut self, m: (u32, u32)) {
        self.f.frame_top = m.0;
        self.f.ztop = m.1;
    }
    /// レジスタの値を一時的に置く 1 バイト（フレームの最後に置く。end_fn で位置が決まる）
    pub fn scratch(&mut self) -> Base {
        self.f.scratch = Some(0);
        Base::Sym(format!("S_{}", self.f.label), 0)
    }

    pub fn needs_zp(&self, t: &Ty) -> bool {
        matches!(t, Ty::Ptr(_) | Ty::Str(_, _))
    }

    pub fn local_val(&mut self, l: usize) -> Val {
        let lt = self.ctx.funcs[self.f.fid].locals[l].ty.clone();
        let size = self.sz(&lt);
        match self.f.slots.get(&l).cloned() {
            Some(Slot::Mem(o)) => Val::mem(Base::Sym(frame_sym(&self.f.label), o as i64), size),
            Some(Slot::Zp(o)) => Val::mem(Base::Sym(zframe_sym(&self.f.label), o as i64), size),
            Some(Slot::RegX) => Val::Reg('x'),
            Some(Slot::RegY) => Val::Reg('y'),
            None => {
                // まだ置き場所がない（パターンの束縛など）：ここで確保する
                let s = self.new_slot(&lt);
                self.f.slots.insert(l, s);
                self.local_val(l)
            }
        }
    }

    pub fn new_slot(&mut self, t: &Ty) -> Slot {
        let size = self.sz(t).max(1);
        if self.needs_zp(t) {
            let b = self.alloc_ztmp(size);
            if let Base::Sym(_, o) = b {
                return Slot::Zp(o as u32);
            }
        }
        let b = self.alloc_tmp(size);
        if let Base::Sym(_, o) = b {
            return Slot::Mem(o as u32);
        }
        Slot::Mem(0)
    }

    /// 関数の引数の渡し方
    pub fn param_layout(&self, fid: usize) -> Vec<ParamLoc> {
        let params = self.ctx.funcs[fid].params.clone();
        let mut out = Vec::new();
        let regs = ['a', 'x', 'y'];
        let mut used = 0usize;
        let mut reg_ok = true;
        let mut moff = 0u32;
        let mut zoff = 0u32;
        let is_extern = self.ctx.funcs[fid].kind == FnKind::Extern;
        let decl_regs: Vec<Option<String>> = match &self.ctx.funcs[fid].decl {
            Some(d) => d.params.iter().map(|p| p.reg.clone()).collect(),
            None => Vec::new(),
        };
        for (i, (_, t)) in params.iter().enumerate() {
            let size = self.sz(t).max(1);
            let mut pr = Vec::new();
            if is_extern {
                if let Some(Some(r)) = decl_regs.get(i) {
                    for c in r.to_lowercase().chars() {
                        pr.push(c);
                    }
                }
            } else if reg_ok && used + size as usize <= 3 {
                for k in 0..size as usize {
                    pr.push(regs[used + k]);
                }
                used += size as usize;
            } else {
                reg_ok = false;
            }
            let slot = if self.needs_zp(t) {
                let s = Slot::Zp(zoff);
                zoff += size;
                s
            } else {
                let s = Slot::Mem(moff);
                moff += size;
                s
            };
            out.push(ParamLoc { regs: pr, slot, size });
        }
        out
    }

    pub fn slot_base(&self, label: &str, s: &Slot) -> Base {
        match s {
            Slot::Mem(o) => Base::Sym(frame_sym(label), *o as i64),
            Slot::Zp(o) => Base::Sym(zframe_sym(label), *o as i64),
            _ => Base::Abs(0),
        }
    }

    // ---- 関数 ----
    fn begin_fn(&mut self, fid: usize) {
        let fi = self.ctx.funcs[fid].clone();
        self.f = FnState::default();
        self.f.fid = fid;
        self.f.label = fi.label.clone();
        self.f.module = fi.module;
        self.f.interrupt = fi.interrupt.is_some();
        self.f.ret = Some(fi.ret.clone());
        self.f.epi = format!("{}@exit", fi.label);
        self.f.last_line = (usize::MAX, 0);
    }

    fn end_fn(&mut self, force_inline: bool) {
        let mut st = std::mem::take(&mut self.f);
        if st.scratch.is_some() {
            let off = st.frame_max as i64;
            st.frame_max += 1;
            let sname = format!("S_{}", st.label);
            let fname = frame_sym(&st.label);
            for it in st.items.iter_mut() {
                crate::opt::rename_sym_in_item(it, &sname, &fname, off);
            }
        }
        let mask = self.clobber_mask(&st.items, st.callee_clob);
        self.clob.insert(st.fid, mask);
        let fi = &self.ctx.funcs[st.fid];
        let mut calls = st.calls.clone();
        calls.sort();
        calls.dedup();
        self.funcs.push(FnCode {
            fid: st.fid,
            label: st.label.clone(),
            items: st.items,
            frame: st.frame_max,
            zframe: st.zmax,
            calls,
            force_inline,
            interrupt: fi.interrupt.clone(),
            is_start: false,
            ext_stack: st.ext_stack,
            emitted: true,
        });
    }

    /// 根から呼び出しをたどり、呼ばれる側が先に来る順に並べる
    fn callee_first_order(&self, roots: &[usize]) -> Vec<usize> {
        let n = self.ctx.funcs.len();
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for e in &self.ctx.call_edges {
            if e.from < n && e.to < n && !adj[e.from].contains(&e.to) {
                adj[e.from].push(e.to);
            }
        }
        let mut state = vec![0u8; n];
        let mut out = Vec::new();
        for &r in roots {
            if r >= n || state[r] != 0 {
                continue;
            }
            // 再帰はないが、念のため繰り返しで深さ優先にたどる
            let mut stack: Vec<(usize, usize)> = vec![(r, 0)];
            state[r] = 1;
            loop {
                let (v, k) = match stack.last() {
                    Some(&(v, k)) => (v, k),
                    None => break,
                };
                if k < adj[v].len() {
                    let w = adj[v][k];
                    if let Some(top) = stack.last_mut() {
                        top.1 += 1;
                    }
                    if state[w] == 0 {
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                } else {
                    state[v] = 2;
                    out.push(v);
                    stack.pop();
                }
            }
        }
        out
    }

    /// 関数のコードが壊すレジスタ（ビット 1=A、2=X、4=Y）
    fn clobber_mask(&self, items: &[Item], callee: u8) -> u8 {
        let mut m = callee;
        let mut keep = 0u32;
        for it in items {
            match it {
                Item::Mark(k) if k == crate::peep::KEEP_BEGIN => keep += 1,
                Item::Mark(k) if k == crate::peep::KEEP_END => keep = keep.saturating_sub(1),
                Item::Ins(ins) => {
                    let acc = matches!(ins.am, AMode::Acc | AMode::Imp);
                    match ins.mn.as_str() {
                        "lda" | "pla" | "txa" | "tya" | "adc" | "sbc" | "and" | "ora" | "eor" => m |= 1,
                        "asl" | "lsr" | "rol" | "ror" | "inc" | "dec" if acc => m |= 1,
                        "ldx" | "plx" | "tax" | "inx" | "dex" | "tsx" => m |= 2,
                        "ldy" | "ply" | "tay" | "iny" | "dey" => m |= 4,
                        "jsr" | "jmp" => match &ins.am {
                            AMode::Mem(e) => match &e.sym {
                                Some(s) => {
                                    match self.ctx.funcs.iter().position(|f| &f.label == s) {
                                        Some(fid) => m |= self.clob.get(&fid).copied().unwrap_or(7),
                                        // 関数でない記号：jsr なら分からない。jmp は関数の中のラベル
                                        None => {
                                            if ins.mn == "jsr" {
                                                m |= 7;
                                            }
                                        }
                                    }
                                }
                                // 番地への jsr（extern fn）：呼ぶところで clobbers を足している。アセンブリの中ならすべて
                                None => {
                                    if keep > 0 {
                                        m |= 7;
                                    }
                                }
                            },
                            _ => m |= 7,
                        },
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        m
    }

    /// X に置いたままにする引数（A で届く 1 バイトの引数で、配列の添字に使い、関数の中で X を壊さないもの）
    fn param_for_x(&self, fi: &FnInfo, layout: &[ParamLoc]) -> Option<usize> {
        if !matches!(fi.kind, FnKind::Normal | FnKind::Const) || fi.interrupt.is_some() || fi.init_body.is_some() {
            return None;
        }
        let body = fi.decl.as_ref()?.body.as_ref()?;
        if layout.iter().any(|p| p.regs.contains(&'x')) {
            return None;
        }
        // for のループ変数に X を残しておく
        let mut has_for = false;
        crate::check::walk_block(body, &mut |x: &Expr| {
            if let ExprKind::For(..) = &x.kind {
                has_for = true;
            }
        });
        if has_for || !self.body_allows_reg(body, 'x') {
            return None;
        }
        for (i, pl) in layout.iter().enumerate() {
            if pl.regs != vec!['a'] || pl.size != 1 || !matches!(pl.slot, Slot::Mem(_)) {
                continue;
            }
            let li = match fi.locals.get(i) {
                Some(l) => l,
                None => continue,
            };
            if li.addr_taken || li.asm_ref {
                continue;
            }
            let mut as_index = false;
            crate::check::walk_block(body, &mut |x: &Expr| {
                if let ExprKind::Index(_, ix) = &x.kind {
                    if let ExprKind::Path(_) = &ix.kind {
                        if let Some(Res::Local(l)) = self.ctx.res.get(&ix.id) {
                            if *l == i {
                                as_index = true;
                            }
                        }
                    }
                }
            });
            if as_index {
                return Some(i);
            }
        }
        None
    }

    /// 関数 f が壊すレジスタ（まだ作っていなければすべて）
    pub fn clob_of(&self, f: usize) -> u8 {
        let fi = &self.ctx.funcs[f];
        if fi.kind == FnKind::Extern {
            let mut m = 0u8;
            if let Some(d) = &fi.decl {
                if d.clobbers.is_empty() {
                    return 7;
                }
                for c in &d.clobbers {
                    match c.to_ascii_lowercase().as_str() {
                        "a" => m |= 1,
                        "x" => m |= 2,
                        "y" => m |= 4,
                        _ => {}
                    }
                }
                return m;
            }
            return 7;
        }
        self.clob.get(&f).copied().unwrap_or(7)
    }

    fn gen_fn(&mut self, fid: usize) {
        let fi = self.ctx.funcs[fid].clone();
        match fi.kind {
            FnKind::Extern | FnKind::Comptime => return,
            _ => {}
        }
        self.begin_fn(fid);
        let label = fi.label.clone();
        self.label(&label);
        // 引数の置き場所
        let layout = self.param_layout(fid);
        for (i, pl) in layout.iter().enumerate() {
            self.f.slots.insert(i, pl.slot.clone());
            match &pl.slot {
                Slot::Mem(o) => self.f.frame_top = self.f.frame_top.max(o + pl.size),
                Slot::Zp(o) => self.f.ztop = self.f.ztop.max(o + pl.size),
                _ => {}
            }
        }
        self.f.frame_max = self.f.frame_top;
        self.f.zmax = self.f.ztop;
        if fi.kind == FnKind::Asm {
            self.gen_asm_fn(fid, &layout);
            self.end_fn(false);
            return;
        }
        if fi.interrupt.is_some() {
            self.f.items.push(Item::Mark("SAVE".to_string()));
            self.forget_a();
        }
        // 添字に使う 1 バイトの引数は、X に置いたままにできる（関数の中で X を壊さないとき）
        let keep_x = self.param_for_x(&fi, &layout);
        // レジスタで届いた引数をしまう
        for (i, pl) in layout.iter().enumerate() {
            let base = self.slot_base(&label, &pl.slot);
            let pname = fi.params[i].0.clone();
            if keep_x == Some(i) {
                continue;
            }
            for (k, r) in pl.regs.iter().enumerate() {
                let mn = match r {
                    'a' => "sta",
                    'x' => "stx",
                    _ => "sty",
                };
                self.emit(mn, AMode::Mem(base.add(k as i64).expr()), &format!("引数 {} を受け取る", pname));
            }
        }
        if let Some(i) = keep_x {
            let pname = fi.params[i].0.clone();
            self.emit0("tax", &format!("引数 {} を X に置く（関数の中で X を壊さないので）", pname));
            self.f.slots.insert(i, Slot::RegX);
            self.f.xres = true;
        }
        let body = match (&fi.decl, &fi.init_body) {
            (Some(d), _) => d.body.clone(),
            (None, Some(b)) => Some(b.clone()),
            _ => None,
        };
        if let Some(b) = body {
            if fi.interrupt.is_some() || fi.init_body.is_some() {
                self.gen_block(&b, &Want::Effect);
                self.label(&format!("{}@exit", label));
                if fi.interrupt.is_some() {
                    self.f.items.push(Item::Mark("RESTORE".to_string()));
                    self.emit("rti", AMode::Imp, "割り込みから戻る");
                } else {
                    self.emit("rts", AMode::Imp, "init の終わり");
                }
            } else {
                self.gen_block(&b, &Want::Tail);
            }
        }
        self.end_fn(fi.init_body.is_some());
    }

    // ---- 起動処理（15.1節）----
    fn gen_start(&mut self) {
        self.f = FnState::default();
        self.f.fid = usize::MAX;
        self.f.label = "start".to_string();
        self.f.last_line = (usize::MAX, 0);
        self.label("start");
        self.f.items.push(Item::Comment("; ---- 起動処理（コンパイラが作る。15.1節）----".to_string()));
        self.emit("sei", AMode::Imp, "1. 割り込みを止める");
        if self.ctx.target.exit_rts() {
            // exit = rts：モニタとスタックを共用するので、スタックポインタは設定しない（1.1。16.4節）
            self.emit("cld", AMode::Imp, "10進モードを解除する（exit = rts なので、スタックポインタは設定しない）");
        } else {
            self.emit("cld", AMode::Imp, "10進モードを解除する");
            self.imm("ldx", 0xFF, "スタックポインタの初期値");
            self.emit("txs", AMode::Imp, "スタックポインタを設定する");
        }
        self.static_init();
        // 割り込みベクタ
        let mut irq_h = None;
        let mut nmi_h = None;
        let mut brk_h = None;
        for code in &self.funcs {
            match code.interrupt.as_deref() {
                Some("irq") => irq_h = Some(code.label.clone()),
                Some("nmi") => nmi_h = Some(code.label.clone()),
                Some("brk") => brk_h = Some(code.label.clone()),
                _ => {}
            }
        }
        if brk_h.is_some() {
            // IRQ と BRK を振り分ける入口を作る（10.1節）
            let disp = "irq_dispatch".to_string();
            let irq_target = irq_h.clone();
            let brk_target = brk_h.clone().unwrap_or_default();
            self.gen_brk_dispatch(&disp, irq_target, &brk_target);
            irq_h = Some(disp);
        }
        let irq_mode = self.ctx.target.irq.clone();
        let nmi_mode = self.ctx.target.nmi.clone();
        if let Some(h) = irq_h.clone() {
            self.write_vector(&irq_mode, &h, "IRQ");
        }
        if let Some(h) = nmi_h.clone() {
            self.write_vector(&nmi_mode, &h, "NMI");
        }
        // init（読み込まれる側から順に。8.5節）
        let order = self.init_order();
        for fid in order {
            let l = match self.funcs.iter().find(|c| c.fid == fid) {
                Some(code) => code.label.clone(),
                None => continue,
            };
            let mname = self.ctx.modules[self.ctx.funcs[fid].module].name.clone();
            self.f.items.push(Item::Comment(format!("; ---- {} の init（ここに展開される）----", mname)));
            self.emit("jsr", AMode::Mem(OpExpr::sym(&l)), "4. init");
            self.f.calls.push(fid);
        }
        // main
        let main = self.ctx.funcs.iter().position(|f| f.name == "main" && f.module == 0 && f.init_body.is_none());
        let exit_rts = matches!(self.ctx.target.exit, Some(Exit::Rts));
        if let Some(m) = main {
            let l = self.ctx.funcs[m].label.clone();
            if exit_rts {
                // exit = rts：main の rts がそのままモニタへ戻るので、末尾呼び出しにする（1.1。15.1節）
                self.emit("jmp", AMode::Mem(OpExpr::sym(&l)), "5. main を呼ぶ。6. main の rts でモニタへ戻る（exit = rts）");
            } else {
                self.emit("jsr", AMode::Mem(OpExpr::sym(&l)), "5. main を呼ぶ");
            }
            self.f.calls.push(m);
        }
        match self.ctx.target.exit.clone() {
            Some(Exit::Rts) => {
                if main.is_none() {
                    self.emit("rts", AMode::Imp, "6. exit = rts：起動処理を呼んだモニタへ戻る");
                }
            }
            Some(Exit::Monitor(a)) => self.emit("jmp", AMode::Mem(OpExpr::num(a as i64)), "6. main が終わったらモニタへ戻る（exit = monitor）"),
            _ => {
                let l = "start@halt".to_string();
                self.label(&l);
                self.emit("jmp", AMode::Mem(OpExpr::sym(&l)), "6. その場で止まる（exit = halt / never）");
            }
        }
        let st = std::mem::take(&mut self.f);
        let mut calls = st.calls.clone();
        calls.sort();
        calls.dedup();
        self.funcs.insert(
            0,
            FnCode {
                fid: usize::MAX,
                label: "start".to_string(),
                items: st.items,
                frame: st.frame_max,
                zframe: st.zmax,
                calls,
                force_inline: false,
                interrupt: None,
                is_start: true,
                ext_stack: 0,
                emitted: true,
            },
        );
    }

    fn gen_brk_dispatch(&mut self, disp: &str, irq: Option<String>, brk: &str) {
        let saved = std::mem::take(&mut self.f);
        self.f.label = disp.to_string();
        self.f.fid = usize::MAX;
        self.label(disp);
        self.emit("pha", AMode::Imp, "A を退避する");
        self.emit("phx", AMode::Imp, "X を退避する");
        self.emit("tsx", AMode::Imp, "スタックの位置を X に");
        self.emit("lda", AMode::MemX(OpExpr::num(0x103)), "積まれたフラグを読む");
        self.imm("and", 0x10, "B ビット（brk 命令か）");
        let l = format!("{}@brk", disp);
        self.br("bne", &l, "brk なら brk の処理へ");
        self.emit("plx", AMode::Imp, "X を戻す");
        self.emit("pla", AMode::Imp, "A を戻す");
        match irq {
            Some(h) => self.emit("jmp", AMode::Mem(OpExpr::sym(&h)), "IRQ の処理へ"),
            None => self.emit("rti", AMode::Imp, "IRQ の処理はない"),
        }
        self.label(&l);
        self.emit("plx", AMode::Imp, "X を戻す");
        self.emit("pla", AMode::Imp, "A を戻す");
        self.emit("jmp", AMode::Mem(OpExpr::sym(brk)), "BRK の処理へ");
        let st = std::mem::replace(&mut self.f, saved);
        self.funcs.push(FnCode {
            fid: usize::MAX,
            label: disp.to_string(),
            items: st.items,
            frame: 0,
            zframe: 0,
            calls: Vec::new(),
            force_inline: false,
            interrupt: Some("dispatch".to_string()),
            is_start: false,
            ext_stack: 0,
            emitted: true,
        });
    }

    fn write_vector(&mut self, mode: &Option<VecMode>, h: &str, what: &str) {
        match mode {
            Some(VecMode::Indirect(a)) => {
                let a = *a as i64;
                self.emit("lda", AMode::Imm(OpExpr::sym(h).lo()), &format!("3. {} ベクタ（indirect(${:04X})）の下位", what, a));
                self.emit("sta", AMode::Mem(OpExpr::num(a)), "書く");
                self.emit("lda", AMode::Imm(OpExpr::sym(h).hi()), "上位");
                self.emit("sta", AMode::Mem(OpExpr::num(a + 1)), "書く");
            }
            Some(VecMode::Jump(a)) => {
                let a = *a as i64;
                self.imm("lda", 0x4C, &format!("3. {} の jmp 命令（jump(${:04X})）", what, a));
                self.emit("sta", AMode::Mem(OpExpr::num(a)), "書く");
                self.emit("lda", AMode::Imm(OpExpr::sym(h).lo()), "飛び先の下位");
                self.emit("sta", AMode::Mem(OpExpr::num(a + 1)), "書く");
                self.emit("lda", AMode::Imm(OpExpr::sym(h).hi()), "飛び先の上位");
                self.emit("sta", AMode::Mem(OpExpr::num(a + 2)), "書く");
            }
            Some(VecMode::Hw) => {
                if !self.ctx.target.run_rom() {
                    let sp = Span::new(self.ctx.target.file, 1, 1, 1);
                    self.ctx.diags.err("E1108", &format!("{} = hw は run = rom のターゲットだけで使えます", what.to_lowercase()), sp);
                }
                // ROM のベクタは出力の段階で書く
            }
            // none（省略を含む）：割り込み処理を書くと検査の段階で E1110 になるので、ここには来ない（1.1）
            Some(VecMode::None) | None => {}
        }
    }

    /// init の実行順：use で読み込まれる側が先（8.5節）
    fn init_order(&mut self) -> Vec<usize> {
        let nm = self.ctx.modules.len();
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); nm];
        for m in 0..nm {
            for (u, _, _) in self.ctx.modules[m].uses.clone() {
                let segs = u.path.segs.clone();
                let mut n = segs.len();
                while n >= 1 {
                    let name = segs[..n].join("::");
                    if let Some(i) = self.ctx.mod_index.get(&name) {
                        let eff = self.ctx.effective_module(*i).unwrap_or(*i);
                        deps[m].push(*i);
                        if eff != *i {
                            deps[m].push(eff);
                        }
                        break;
                    }
                    n -= 1;
                }
            }
        }
        let mut order_mods = Vec::new();
        let mut state = vec![0u8; nm];
        fn visit(m: usize, deps: &Vec<Vec<usize>>, state: &mut Vec<u8>, out: &mut Vec<usize>) -> bool {
            if state[m] == 2 {
                return true;
            }
            if state[m] == 1 {
                return false;
            }
            state[m] = 1;
            let mut ok = true;
            for d in &deps[m] {
                if !visit(*d, deps, state, out) {
                    ok = false;
                }
            }
            state[m] = 2;
            out.push(m);
            ok
        }
        let mut cyc = false;
        for m in 0..nm {
            if !visit(m, &deps, &mut state, &mut order_mods) {
                cyc = true;
            }
        }
        let mut out = Vec::new();
        for m in order_mods {
            for (i, f) in self.ctx.funcs.iter().enumerate() {
                if f.module == m && f.init_body.is_some() {
                    out.push(i);
                }
            }
        }
        if cyc {
            // init を持つモジュールが輪の中にあるときだけエラー
            let with_init: Vec<usize> = out.clone();
            if with_init.len() > 1 {
                let sp = Span::new(self.ctx.modules[0].file, 1, 1, 1);
                self.ctx.diags.err("E0205", "モジュールの use が循環していて、init の順序が決まりません", sp);
            }
        }
        out
    }

    /// static と shared の初期値（4.6節）
    fn static_init(&mut self) {
        let gids: Vec<usize> = self.used_globals.iter().cloned().collect();
        let mut first = true;
        for gid in gids {
            let g = self.ctx.globals[gid].clone();
            match g.kind {
                GKind::Static | GKind::Shared(_) => {}
                _ => continue,
            }
            if find_attr(&g.attrs, "noinit").is_some() {
                continue;
            }
            let bytes = match &g.value {
                Some(v) => self.ctx.value_bytes(v, &g.ty),
                None => vec![0u8; self.sz(&g.ty) as usize],
            };
            if bytes.is_empty() {
                continue;
            }
            if first {
                first = false;
            }
            let base = Base::Sym(g.label.clone(), 0);
            let n = bytes.len();
            let nonzero = bytes.iter().filter(|b| **b != 0).count();
            if nonzero == 0 {
                self.zero_fill(&base, n, &g.name);
            } else if nonzero <= 8 && n <= 16 {
                let mut last: Option<u8> = None;
                for (k, b) in bytes.iter().enumerate() {
                    let addr = base.add(k as i64).expr();
                    if *b == 0 {
                        self.emit("stz", AMode::Mem(addr), &format!("2. 初期値：{} = 0", g.name));
                    } else {
                        if last != Some(*b) {
                            self.imm("lda", *b as i64, &format!("2. 初期値：{}", g.name));
                            last = Some(*b);
                        }
                        self.emit("sta", AMode::Mem(addr), "書く");
                    }
                }
            } else {
                // 元データからコピーする
                let src = format!("I_{}", g.label);
                self.data.push(Item::Label(src.clone()));
                self.data.push(Item::Bytes(bytes.clone(), format!("{} の初期値", g.name)));
                self.copy_loop(&Base::Sym(src, 0), &base, n, &g.name);
            }
        }
    }

    pub fn zero_fill(&mut self, base: &Base, n: usize, name: &str) {
        if n <= 4 {
            for k in 0..n {
                self.emit("stz", AMode::Mem(base.add(k as i64).expr()), &format!("2. 初期値：{} = 0", name));
            }
            return;
        }
        let mut done = 0usize;
        while done < n {
            let chunk = (n - done).min(256);
            let b = base.add(done as i64);
            let l = self.new_label("zero");
            if chunk <= 128 {
                self.imm("ldx", (chunk - 1) as i64, &format!("2. {} の {} バイトをまとめて 0 にする", name, chunk));
                self.label(&l);
                self.emit("stz", AMode::MemX(b.expr()), "0 を書く");
                self.emit("dex", AMode::Imp, "次のバイトへ");
                self.br("bpl", &l, "0 番目まで繰り返す");
            } else if chunk < 256 {
                self.imm("ldx", chunk as i64, &format!("2. {} の {} バイトをまとめて 0 にする", name, chunk));
                self.label(&l);
                self.emit("stz", AMode::MemX(b.add(-1).expr()), "0 を書く");
                self.emit("dex", AMode::Imp, "次のバイトへ");
                self.br("bne", &l, "繰り返す");
            } else {
                self.imm("ldx", 0, &format!("2. {} の 256 バイトをまとめて 0 にする", name));
                self.label(&l);
                self.emit("stz", AMode::MemX(b.expr()), "0 を書く");
                self.emit("inx", AMode::Imp, "次のバイトへ");
                self.br("bne", &l, "256 回繰り返す");
            }
            done += chunk;
        }
    }

    pub fn copy_loop(&mut self, src: &Base, dst: &Base, n: usize, name: &str) {
        let mut done = 0usize;
        while done < n {
            let chunk = (n - done).min(256);
            let s = src.add(done as i64);
            let d = dst.add(done as i64);
            let l = self.new_label("copy");
            if chunk < 256 {
                self.imm("ldx", chunk as i64, &format!("2. {} の初期値をコピーする（{} バイト）", name, chunk));
                self.label(&l);
                self.emit("lda", AMode::MemX(s.add(-1).expr()), "元データを読む");
                self.emit("sta", AMode::MemX(d.add(-1).expr()), "書く");
                self.emit("dex", AMode::Imp, "次のバイトへ");
                self.br("bne", &l, "繰り返す");
            } else {
                self.imm("ldx", 0, &format!("2. {} の初期値をコピーする（256 バイト）", name));
                self.label(&l);
                self.emit("lda", AMode::MemX(s.expr()), "元データを読む");
                self.emit("sta", AMode::MemX(d.expr()), "書く");
                self.emit("inx", AMode::Imp, "次のバイトへ");
                self.br("bne", &l, "256 回繰り返す");
            }
            done += chunk;
        }
    }

    fn gen_fnset_tables(&mut self) {
        let sets: Vec<usize> = self.fnset_used.iter().cloned().collect();
        for s in sets {
            let def = self.ctx.types.fnsets[s].clone();
            let tbl = format!("FS_{}", def.name);
            let disp = format!("FS_{}_call", def.name);
            self.data.push(Item::Label(tbl.clone()));
            let mut ex = Vec::new();
            for f in &def.fns {
                let l = self.ctx.funcs[*f].label.clone();
                ex.push(OpExpr::sym(&l).lo());
                ex.push(OpExpr::sym(&l).hi());
            }
            self.data.push(Item::Exprs(ex, format!("関数集合 {} の飛び先の表", def.name)));
            // 呼び出しの入口
            let saved = std::mem::take(&mut self.f);
            self.f.label = disp.clone();
            self.f.fid = usize::MAX;
            self.label(&disp);
            self.emit("jmp", AMode::IndAbsX(OpExpr::sym(&tbl)), "X 番目の関数へ飛ぶ（jmp (abs,x)）");
            let st = std::mem::replace(&mut self.f, saved);
            self.funcs.push(FnCode {
                fid: usize::MAX,
                label: disp.clone(),
                items: st.items,
                frame: 0,
                zframe: 0,
                calls: def.fns.clone(),
                force_inline: false,
                interrupt: None,
                is_start: false,
                ext_stack: 0,
                emitted: true,
            });
            self.notes.push(format!("関数集合 {}：jmp ({},x) のジャンプテーブルで呼ぶ（{} 個）", def.name, tbl, def.fns.len()));
        }
    }

    // ---- ブロックと文 ----
    pub fn gen_block(&mut self, b: &Block, want: &Want) {
        let m = self.mark();
        let saved_slots: Vec<usize> = self.f.slots.keys().cloned().collect();
        let mut diverged = false;
        for s in &b.stmts {
            self.gen_stmt(s);
            if let StmtKind::Expr(e, _) = &s.kind {
                if self.ty_of(e) == Ty::Never {
                    diverged = true;
                }
            }
        }
        match &b.tail {
            Some(t) => {
                if diverged {
                    self.gen(t, &Want::Effect);
                } else {
                    self.src_line(&t.span);
                    self.gen(t, want);
                }
            }
            None => {
                if !diverged {
                    self.finish_unit(want);
                }
            }
        }
        // ブロックの中のローカル変数の場所を解放する
        let keep: HashSet<usize> = saved_slots.into_iter().collect();
        let all: Vec<usize> = self.f.slots.keys().cloned().collect();
        for l in all {
            if !keep.contains(&l) {
                if let Some(Slot::Mem(_)) | Some(Slot::Zp(_)) = self.f.slots.get(&l) {
                    self.f.slots.remove(&l);
                }
            }
        }
        self.release(m);
    }

    /// 値のない式で、値が求められたとき
    pub fn finish_unit(&mut self, want: &Want) {
        match want {
            Want::Tail => self.emit_ret(),
            Want::Cond(_, _) | Want::A | Want::Store(_) | Want::Effect => {}
        }
    }

    pub fn gen_stmt(&mut self, s: &Stmt) {
        self.src_line(&s.span);
        let m = self.mark();
        match &s.kind {
            StmtKind::Let(name, _, init) => {
                match name {
                    Some(_) => {
                        let l = match self.ctx.node_local.get(&s.id) {
                            Some(l) => *l,
                            None => return,
                        };
                        let lt = self.ctx.funcs[self.f.fid].locals[l].ty.clone();
                        let slot = self.new_slot(&lt);
                        self.f.slots.insert(l, slot);
                        let m2 = self.mark();
                        if let Some(e) = init {
                            let v = self.local_val(l);
                            self.gen(e, &Want::Store(v));
                        }
                        self.release(m2);
                        return;
                    }
                    None => {
                        if let Some(e) = init {
                            self.gen_discard_read(e);
                        }
                    }
                }
            }
            StmtKind::Expr(e, _) => {
                self.gen(e, &Want::Effect);
            }
            StmtKind::Item(it) => {
                // 関数の中の static：使われたものとして登録する
                if let ItemKind::Static(_) = &it.kind {
                    if let Some(n) = it.name() {
                        for (i, g) in self.ctx.globals.iter().enumerate() {
                            if g.fn_local == Some(self.f.fid) && g.name == n {
                                self.used_globals.insert(i);
                            }
                        }
                    }
                }
            }
            StmtKind::ComptimeAssert(_, _) => {}
        }
        self.release(m);
    }

    /// let _ = X;（11.3節：I/O は bit で読み捨てる）
    fn gen_discard_read(&mut self, e: &Expr) {
        let is_io = match &e.kind {
            ExprKind::Path(_) => match self.ctx.res.get(&e.id) {
                Some(Res::Global(g)) => matches!(self.ctx.globals[*g].kind, GKind::Io(_)),
                _ => false,
            },
            _ => false,
        };
        if is_io {
            let v = self.val_of(e);
            if let Val::Mem { idx: Idx::None, .. } = &v {
                self.emit_byte("bit", &v, 0, "値を使わない読み込み：bit で読む（A は変わらない）");
                return;
            }
            self.emit_byte("lda", &v, 0, "値を使わない読み込み");
            return;
        }
        self.gen(e, &Want::Effect);
    }

    // ---- 式の振り分け ----
    pub fn gen(&mut self, e: &Expr, want: &Want) {
        match &e.kind {
            ExprKind::Block(b) => self.gen_block(b, want),
            ExprKind::If(c, then_b, els) => self.gen_if(c, then_b, els, want),
            ExprKind::IfLet(p, s, then_b, els) => self.gen_if_let(p, s, then_b, els, want),
            ExprKind::Match(s, arms, attrs) => self.gen_match(e, s, arms, attrs, want),
            ExprKind::Loop(label, body) => self.gen_loop(e, label, body, want),
            ExprKind::While(label, c, body, attrs) => {
                self.gen_while(label, c, body, attrs);
                self.finish_unit(want);
            }
            ExprKind::For(label, _, iter, body, attrs) => {
                self.gen_for(e, label, iter, body, attrs);
                self.finish_unit(want);
            }
            ExprKind::Atomic(b) => self.gen_atomic(b, want),
            ExprKind::Timed(k, n, b) => {
                self.gen_timed(e, k, n, b);
                self.finish_unit(want);
            }
            ExprKind::Asm(a) => {
                self.gen_asm_block(e, a);
                self.finish_unit(want);
            }
            ExprKind::Break(label, v) => self.gen_break(e, label, v),
            ExprKind::Continue(label) => self.gen_continue(e, label),
            ExprKind::Return(v) => self.gen_return(v),
            ExprKind::ComptimeIf(_, then_b, els) => match self.ctx.ct_choice.get(&e.id).cloned() {
                Some(Some(0)) => self.gen_block(then_b, want),
                Some(Some(_)) => match els {
                    Some(x) => self.gen(x, want),
                    None => self.finish_unit(want),
                },
                _ => self.finish_unit(want),
            },
            ExprKind::ComptimeMatch(_, arms) => match self.ctx.ct_choice.get(&e.id).cloned() {
                Some(Some(i)) => {
                    let body = arms[i].body.clone();
                    self.gen(&body, want)
                }
                _ => self.finish_unit(want),
            },
            _ => self.gen_leaf(e, want),
        }
    }

    // ---- if ----
    fn gen_if(&mut self, c: &Expr, then_b: &Block, els: &Option<Box<Expr>>, want: &Want) {
        let l_else = self.new_label("else");
        let l_end = self.new_label("endif");
        // 関数の最後の、else のない if：本体の後ろと条件が成り立たないときで、同じ rts を使う
        if els.is_none() && matches!(want, Want::Tail) {
            self.gen_cond(c, &l_else, false);
            self.gen_block(then_b, &Want::Effect);
            self.label(&l_else);
            self.emit_ret();
            return;
        }
        self.gen_cond(c, &l_else, false);
        self.gen_block(then_b, want);
        let then_div = self.ty_of_block(then_b) == Ty::Never || matches!(want, Want::Tail);
        match els {
            Some(x) => {
                if !then_div {
                    self.jmp(&l_end, "else を飛ばす");
                }
                self.label(&l_else);
                self.gen(x, want);
                self.label(&l_end);
            }
            None => {
                self.label(&l_else);
                if let Want::Tail = want {
                    self.emit_ret();
                }
            }
        }
    }

    pub fn ty_of_block(&self, b: &Block) -> Ty {
        self.ctx.ety.get(&b.id).cloned().unwrap_or(Ty::Unit)
    }

    fn gen_if_let(&mut self, p: &Pattern, s: &Expr, then_b: &Block, els: &Option<Box<Expr>>, want: &Want) {
        let l_else = self.new_label("else");
        let l_end = self.new_label("endif");
        let st = self.ty_of(s);
        match (&p.kind, &st) {
            (PatKind::Some(inner), Ty::Opt(it)) => {
                let it = (**it).clone();
                let v = self.gen_opt_test(s, &l_else);
                self.bind_or_test(inner, &v, &it, &l_else);
            }
            (PatKind::None, Ty::Opt(_)) => {
                let l_some = l_else.clone();
                let l_none = self.new_label("none");
                let _ = self.gen_opt_test(s, &l_none);
                self.jmp(&l_some, "Some なら else へ");
                self.label(&l_none);
            }
            _ => {
                // 一般のパターン：値を 1 バイトで比べる
                if self.sz(&st) == 1 {
                    self.gen_a(s);
                    let ok = self.new_label("pat");
                    self.pattern_test_a(p, &st, &ok, &l_else);
                    self.label(&ok);
                    self.bind_a(p);
                } else {
                    self.unimpl("1 バイトでない値の if let", &s.span);
                }
            }
        }
        self.gen_block(then_b, want);
        let then_div = self.ty_of_block(then_b) == Ty::Never || matches!(want, Want::Tail);
        match els {
            Some(x) => {
                if !then_div {
                    self.jmp(&l_end, "else を飛ばす");
                }
                self.label(&l_else);
                self.gen(x, want);
                self.label(&l_end);
            }
            None => {
                self.label(&l_else);
                if let Want::Tail = want {
                    self.emit_ret();
                }
            }
        }
    }

    /// Some(p) の p：束縛するか、値を比べる
    fn bind_or_test(&mut self, p: &Pattern, v: &Val, t: &Ty, fail: &str) {
        match &p.kind {
            PatKind::Wild => {}
            PatKind::Ident(_) => match self.ctx.res.get(&p.id).cloned() {
                Some(Res::Value(c)) => {
                    let x = value_int(&c).unwrap_or(0);
                    self.test_val_eq_const(v, t, x, fail);
                }
                _ => {
                    if let Some(l) = self.ctx.node_local.get(&p.id).cloned() {
                        let dst = self.local_val(l);
                        let n = self.sz(t);
                        self.copy_val(v, &dst, n);
                    }
                }
            },
            PatKind::Lit(e) => {
                let x = self.ctx.econst.get(&e.id).and_then(value_int).unwrap_or(0);
                self.test_val_eq_const(v, t, x, fail);
            }
            PatKind::Range(_, _, _) => {
                if self.sz(t) == 1 {
                    self.emit_byte("lda", v, 0, "値を読む");
                    let ok = self.new_label("pat");
                    self.pattern_test_a(p, t, &ok, fail);
                    self.label(&ok);
                } else {
                    self.unimpl("1 バイトでない値の範囲パターン", &p.span);
                }
            }
            PatKind::Some(_) | PatKind::None => self.unimpl("入れ子の Option のパターン", &p.span),
        }
    }

    fn test_val_eq_const(&mut self, v: &Val, t: &Ty, x: i128, fail: &str) {
        let n = self.sz(t).max(1);
        for k in 0..n {
            self.emit_byte("lda", v, k, "値を読む");
            let b = ((x >> (8 * k)) & 0xFF) as i64;
            self.imm("cmp", b, "パターンの値と比べる");
            self.br("bne", fail, "違えば次へ");
        }
    }

    /// A の値をパターンで調べる（1 バイト）。合えば ok へ、合わなければ fail へ
    pub fn pattern_test_a(&mut self, p: &Pattern, t: &Ty, ok: &str, fail: &str) {
        let signed = self.ctx.types.is_signed(t);
        let adj = |x: i128| -> i64 {
            let b = (x & 0xFF) as i64;
            if signed {
                b ^ 0x80
            } else {
                b
            }
        };
        match &p.kind {
            PatKind::Wild => self.jmp(ok, "どの値でも合う"),
            PatKind::Ident(_) => match self.ctx.res.get(&p.id).cloned() {
                Some(Res::Value(c)) => {
                    let x = value_int(&c).unwrap_or(0);
                    self.imm("cmp", adj(x), "パターンの値と比べる");
                    self.br("beq", ok, "等しければ合う");
                    self.jmp(fail, "違う");
                }
                _ => self.jmp(ok, "どの値でも合う（束縛）"),
            },
            PatKind::Lit(e) => {
                let x = self.ctx.econst.get(&e.id).and_then(value_int).unwrap_or(0);
                self.imm("cmp", adj(x), "パターンの値と比べる");
                self.br("beq", ok, "等しければ合う");
                self.jmp(fail, "違う");
            }
            PatKind::Range(a, b, incl) => {
                let lo = self.ctx.econst.get(&a.id).and_then(value_int).unwrap_or(0);
                let mut hi = self.ctx.econst.get(&b.id).and_then(value_int).unwrap_or(0);
                if !*incl {
                    hi -= 1;
                }
                let (lo2, hi2) = (adj(lo), adj(hi));
                self.imm("cmp", lo2, "範囲の下限と比べる");
                self.br("bcc", fail, "下限より小さければ違う");
                if hi2 < 255 {
                    self.imm("cmp", hi2 + 1, "範囲の上限と比べる");
                    self.br("bcc", ok, "上限以下なら合う");
                    self.jmp(fail, "違う");
                } else {
                    self.jmp(ok, "上限は 255");
                }
            }
            _ => self.jmp(fail, "合わない"),
        }
    }

    fn bind_a(&mut self, p: &Pattern) {
        if let PatKind::Ident(_) = &p.kind {
            if let Some(l) = self.ctx.node_local.get(&p.id).cloned() {
                let v = self.local_val(l);
                self.store_byte(&v, 0, "パターンの値を変数に入れる");
            }
        }
    }

    // ---- match（6.6節）----
    fn gen_match(&mut self, e: &Expr, s: &Expr, arms: &[Arm], attrs: &[Attr], want: &Want) {
        let st = self.ty_of(s);
        let l_end = self.new_label("endm");
        let body_labels: Vec<String> = (0..arms.len()).map(|i| self.new_label(&format!("arm{}_", i))).collect();
        let size = self.sz(&st);
        let jump_table = find_attr(attrs, "jump_table").is_some();
        match &st {
            Ty::Opt(inner) => {
                let inner = (**inner).clone();
                let l_none = self.new_label("none");
                let v = self.gen_opt_test(s, &l_none);
                // Some の腕
                let mut none_arm = None;
                let isz = self.sz(&inner);
                if isz == 1 {
                    self.emit_byte("lda", &v, 0, "Some の中身");
                }
                for (i, arm) in arms.iter().enumerate() {
                    for p in &arm.pats {
                        match &p.kind {
                            PatKind::None => {
                                if none_arm.is_none() {
                                    none_arm = Some(i);
                                }
                            }
                            PatKind::Wild => {
                                if none_arm.is_none() {
                                    none_arm = Some(i);
                                }
                                self.jmp(&body_labels[i], "どの値でも合う");
                            }
                            PatKind::Some(ip) => {
                                if isz == 1 {
                                    let next = self.new_label("pat");
                                    self.pattern_test_a(ip, &inner, &body_labels[i], &next);
                                    self.label(&next);
                                    self.emit_byte("lda", &v, 0, "Some の中身");
                                } else {
                                    match &ip.kind {
                                        PatKind::Wild | PatKind::Ident(_) => self.jmp(&body_labels[i], "Some(_) に合う"),
                                        _ => self.unimpl("1 バイトでない値の Some(定数) パターン", &ip.span),
                                    }
                                }
                            }
                            _ => self.jmp(&body_labels[i], "合う"),
                        }
                    }
                }
                self.jmp(&l_end, "どれにも合わない");
                self.label(&l_none);
                match none_arm {
                    Some(i) => self.jmp(&body_labels[i], "None の腕へ"),
                    None => self.jmp(&l_end, "None の腕はない"),
                }
                // 腕の本体
                for (i, arm) in arms.iter().enumerate() {
                    self.label(&body_labels[i]);
                    for p in &arm.pats {
                        if let PatKind::Some(ip) = &p.kind {
                            self.bind_or_test(ip, &v, &inner, &l_end);
                        }
                    }
                    self.gen_arm_body(&arm.body, want, &l_end);
                }
                self.label(&l_end);
                self.notes.push(format!("match（{}行）：Option の有無はキャリー、Some の値は比較の連鎖", e.span.line));
                return;
            }
            _ => {}
        }
        if size != 1 {
            // 多バイトの値：一時領域に置いて比べる
            let v = self.val_of(s);
            for (i, arm) in arms.iter().enumerate() {
                for p in &arm.pats {
                    match &p.kind {
                        PatKind::Wild => self.jmp(&body_labels[i], "どの値でも合う"),
                        PatKind::Ident(_) if !matches!(self.ctx.res.get(&p.id), Some(Res::Value(_))) => {
                            if let Some(l) = self.ctx.node_local.get(&p.id).cloned() {
                                let dst = self.local_val(l);
                                self.copy_val(&v, &dst, size);
                            }
                            self.jmp(&body_labels[i], "束縛");
                        }
                        PatKind::Lit(_) | PatKind::Ident(_) => {
                            let x = match &p.kind {
                                PatKind::Lit(le) => self.ctx.econst.get(&le.id).and_then(value_int).unwrap_or(0),
                                _ => match self.ctx.res.get(&p.id) {
                                    Some(Res::Value(c)) => value_int(c).unwrap_or(0),
                                    _ => 0,
                                },
                            };
                            let next = self.new_label("pat");
                            self.test_val_eq_const(&v, &st, x, &next);
                            self.jmp(&body_labels[i], "等しい");
                            self.label(&next);
                        }
                        _ => self.unimpl("1 バイトでない値の範囲パターン", &p.span),
                    }
                }
            }
            self.jmp(&l_end, "どれにも合わない");
            for (i, arm) in arms.iter().enumerate() {
                self.label(&body_labels[i]);
                self.gen_arm_body(&arm.body, want, &l_end);
            }
            self.label(&l_end);
            self.notes.push(format!("match（{}行）：比較の連鎖（{} バイトの値）", e.span.line, size));
            return;
        }
        // 1 バイトの値：A に置いて比べる（A にもうあれば読まない）
        let held = self.is_simple(s) && {
            let sv = self.val_of(s);
            self.a_holds(&sv)
        };
        if !held {
            self.gen_a(s);
        }
        if arms.is_empty() {
            self.label(&l_end);
            return;
        }
        let signed = self.ctx.types.is_signed(&st);
        if signed {
            self.imm("eor", 0x80, "符号付きの比較のために最上位ビットを反転する");
        }
        if jump_table && !signed {
            if self.gen_jump_table(arms, &body_labels, &l_end) {
                for (i, arm) in arms.iter().enumerate() {
                    self.label(&body_labels[i]);
                    self.bind_arm_a_from_tmp(arm);
                    self.gen_arm_body(&arm.body, want, &l_end);
                }
                self.label(&l_end);
                self.notes.push(format!("match（{}行）：ジャンプテーブル（#[jump_table]）", e.span.line));
                return;
            }
        }
        // 束縛があるときは値を保存しておく
        let needs_bind = arms.iter().any(|a| a.pats.iter().any(|p| matches!(p.kind, PatKind::Ident(_)) && !matches!(self.ctx.res.get(&p.id), Some(Res::Value(_)))));
        // どの値でも合う腕（_ と束縛）。なければ最後の腕（網羅性は検査済みなので、ほかの腕に合わなければ必ず合う）
        let catch_all = |g: &Gen, p: &Pattern| -> bool {
            match &p.kind {
                PatKind::Wild => true,
                PatKind::Ident(_) => !matches!(g.ctx.res.get(&p.id), Some(Res::Value(_))),
                _ => false,
            }
        };
        let fall = arms.iter().position(|a| a.pats.iter().any(|p| catch_all(self, p))).unwrap_or(arms.len().saturating_sub(1));
        for (i, arm) in arms.iter().enumerate().take(fall) {
            for p in &arm.pats {
                let next = self.new_label("pat");
                self.pattern_test_a(p, &st, &body_labels[i], &next);
                self.label(&next);
            }
        }
        // 比べ終わったら、残りの腕へそのまま進む（その後ろの腕には届かない）
        let mut order: Vec<usize> = vec![fall];
        order.extend((0..fall).collect::<Vec<usize>>());
        for i in order {
            let arm = &arms[i];
            self.label(&body_labels[i]);
            if needs_bind {
                for p in &arm.pats {
                    if let PatKind::Ident(_) = &p.kind {
                        if !matches!(self.ctx.res.get(&p.id), Some(Res::Value(_))) {
                            if signed {
                                self.imm("eor", 0x80, "符号を戻す");
                            }
                            self.bind_a(p);
                        }
                    }
                }
            }
            self.gen_arm_body(&arm.body, want, &l_end);
        }
        self.label(&l_end);
        self.notes.push(format!("match（{}行）：比較の連鎖（{} 本の腕）", e.span.line, arms.len()));
    }

    fn bind_arm_a_from_tmp(&mut self, arm: &Arm) {
        let _ = arm;
    }

    /// #[jump_table]：A の値で飛び先の表を引く
    fn gen_jump_table(&mut self, arms: &[Arm], labels: &[String], l_end: &str) -> bool {
        let mut table: Vec<Option<usize>> = vec![None; 128];
        let mut default = None;
        for (i, arm) in arms.iter().enumerate() {
            for p in &arm.pats {
                match &p.kind {
                    PatKind::Wild => default = Some(i),
                    PatKind::Ident(_) => match self.ctx.res.get(&p.id) {
                        Some(Res::Value(c)) => {
                            let x = value_int(c).unwrap_or(-1);
                            if !(0..128).contains(&x) {
                                return false;
                            }
                            if table[x as usize].is_none() {
                                table[x as usize] = Some(i);
                            }
                        }
                        _ => return false,
                    },
                    PatKind::Lit(e) => {
                        let x = self.ctx.econst.get(&e.id).and_then(value_int).unwrap_or(-1);
                        if !(0..128).contains(&x) {
                            return false;
                        }
                        if table[x as usize].is_none() {
                            table[x as usize] = Some(i);
                        }
                    }
                    PatKind::Range(a, b, incl) => {
                        let lo = self.ctx.econst.get(&a.id).and_then(value_int).unwrap_or(-1);
                        let mut hi = self.ctx.econst.get(&b.id).and_then(value_int).unwrap_or(-1);
                        if !*incl {
                            hi -= 1;
                        }
                        if lo < 0 || hi > 127 {
                            return false;
                        }
                        for x in lo..=hi {
                            if table[x as usize].is_none() {
                                table[x as usize] = Some(i);
                            }
                        }
                    }
                    _ => return false,
                }
            }
        }
        let max = match table.iter().rposition(|x| x.is_some()) {
            Some(m) => m,
            None => return false,
        };
        let dflt = match default {
            Some(d) => labels[d].clone(),
            None => l_end.to_string(),
        };
        let tbl = self.new_label("jt");
        let tbl = tbl.replace('@', "_");
        self.imm("cmp", (max + 1) as i64, "表の範囲か");
        self.br("bcs", &dflt, "範囲の外は既定の腕へ");
        self.emit0("asl", "表の位置（2 倍）");
        self.emit0("tax", "X に置く");
        self.emit("jmp", AMode::IndAbsX(OpExpr::sym(&tbl)), "表の飛び先へ（jmp (abs,x)）");
        let mut ex = Vec::new();
        for x in table.iter().take(max + 1) {
            let l = match x {
                Some(i) => labels[*i].clone(),
                None => dflt.clone(),
            };
            ex.push(OpExpr::sym(&l).lo());
            ex.push(OpExpr::sym(&l).hi());
        }
        self.data.push(Item::Label(tbl));
        self.data.push(Item::Exprs(ex, "match のジャンプテーブル".to_string()));
        true
    }

    fn gen_arm_body(&mut self, body: &Expr, want: &Want, l_end: &str) {
        let m = self.mark();
        self.src_line(&body.span);
        self.gen(body, want);
        self.release(m);
        let div = self.ty_of(body) == Ty::Never || matches!(want, Want::Tail);
        if !div {
            self.jmp(l_end, "match の後ろへ");
        }
    }

    // ---- ループ ----
    fn gen_loop(&mut self, e: &Expr, label: &Option<String>, body: &Block, want: &Want) {
        let t = self.ty_of(e);
        let result = if t != Ty::Unit && t != Ty::Never && !matches!(want, Want::Effect) {
            let n = self.sz(&t);
            Some(self.tmp_val(n.max(1)))
        } else {
            None
        };
        let top = self.new_label("loop");
        let brk = self.new_label("brk");
        self.label(&top);
        self.f.loops.push(GLoop { label: label.clone(), brk: brk.clone(), cont: top.clone(), result: result.clone(), atomic: self.f.atomic });
        self.gen_block(body, &Want::Effect);
        self.f.loops.pop();
        self.jmp(&top, "loop の先頭へ");
        self.label(&brk);
        match result {
            Some(r) => self.deliver(&r, &t, want),
            None => {
                if t != Ty::Never {
                    self.finish_unit(want);
                }
            }
        }
    }

    fn gen_while(&mut self, label: &Option<String>, c: &Expr, body: &Block, attrs: &[Attr]) {
        let top = self.new_label("while");
        let brk = self.new_label("brk");
        let max = find_attr(attrs, "max").and_then(attr_int_arg);
        let counter = match max {
            Some(n) => {
                let v = self.tmp_val(2);
                self.imm("lda", (n & 0xFF) as i64, "#[max(n)]：回数の上限");
                self.store_byte(&v, 0, "数える");
                self.imm("lda", ((n >> 8) & 0xFF) as i64, "上位");
                self.store_byte(&v, 1, "数える");
                Some(v)
            }
            None => None,
        };
        self.label(&top);
        self.gen_cond(c, &brk, false);
        let cont = self.new_label("cont");
        self.f.loops.push(GLoop { label: label.clone(), brk: brk.clone(), cont: cont.clone(), result: None, atomic: self.f.atomic });
        self.gen_block(body, &Want::Effect);
        self.f.loops.pop();
        self.label(&cont);
        if let Some(v) = counter {
            // 上限に達したら抜ける
            let skip = self.new_label("max");
            self.emit_byte("lda", &v, 0, "残りの回数（下位）");
            self.br("bne", &skip, "0 でなければ");
            self.emit_rmw("dec", &v, 1, "上位を 1 減らす");
            self.label(&skip);
            self.emit_rmw("dec", &v, 0, "下位を 1 減らす");
            self.emit_byte("lda", &v, 0, "残り");
            self.emit_byte("ora", &v, 1, "上位と合わせて");
            self.br("beq", &brk, "上限に達したら抜ける");
        }
        self.jmp(&top, "while の先頭へ");
        self.label(&brk);
    }

    pub fn gen_break(&mut self, e: &Expr, label: &Option<String>, v: &Option<Box<Expr>>) {
        let idx = self.find_gloop(label);
        let lp = match idx {
            Some(i) => self.f.loops[i].clone(),
            None => {
                self.unimpl("ループの外の break", &e.span);
                return;
            }
        };
        if let Some(x) = v {
            match &lp.result {
                Some(r) => {
                    let r = r.clone();
                    self.gen(x, &Want::Store(r));
                }
                None => self.gen(x, &Want::Effect),
            }
        }
        for _ in lp.atomic..self.f.atomic {
            if !self.f.interrupt {
                self.emit("plp", AMode::Imp, "atomic を抜ける");
            }
        }
        self.jmp(&lp.brk, "break");
    }

    fn gen_continue(&mut self, e: &Expr, label: &Option<String>) {
        let idx = self.find_gloop(label);
        let lp = match idx {
            Some(i) => self.f.loops[i].clone(),
            None => {
                self.unimpl("ループの外の continue", &e.span);
                return;
            }
        };
        for _ in lp.atomic..self.f.atomic {
            if !self.f.interrupt {
                self.emit("plp", AMode::Imp, "atomic を抜ける");
            }
        }
        self.jmp(&lp.cont, "continue");
    }

    fn find_gloop(&self, label: &Option<String>) -> Option<usize> {
        if self.f.loops.is_empty() {
            return None;
        }
        match label {
            None => Some(self.f.loops.len() - 1),
            Some(l) => (0..self.f.loops.len()).rev().find(|i| self.f.loops[*i].label.as_ref() == Some(l)),
        }
    }

    fn gen_return(&mut self, v: &Option<Box<Expr>>) {
        match v {
            Some(x) => self.gen(x, &Want::Tail),
            None => self.emit_ret(),
        }
    }

    /// 関数から戻る（atomic の中なら plp を先に）
    pub fn leave_prefix(&mut self) {
        if !self.f.interrupt {
            for _ in 0..self.f.atomic {
                self.emit("plp", AMode::Imp, "atomic を抜ける");
            }
        }
    }
    pub fn emit_rts(&mut self) {
        if self.f.interrupt {
            let l = self.f.epi.clone();
            self.jmp(&l, "割り込み処理の出口へ");
        } else {
            self.emit("rts", AMode::Imp, "呼び出し元に戻る");
        }
    }
    pub fn emit_ret(&mut self) {
        self.leave_prefix();
        self.emit_rts();
    }

    // ---- atomic（10.5節）----
    fn gen_atomic(&mut self, b: &Block, want: &Want) {
        // 割り込み処理の中と、irq・nmi がどちらも none のターゲットでは何も生成しない（10.5節）
        if self.f.interrupt || !self.ctx.target.has_interrupts() {
            self.gen_block(b, want);
            return;
        }
        let t = self.ty_of_block(b);
        let flagged = matches!(want, Want::Tail | Want::Cond(_, _));
        self.emit("php", AMode::Imp, "atomic の始まり：割り込みの許可状態を退避する");
        self.emit("sei", AMode::Imp, "割り込みを止める");
        self.f.atomic += 1;
        match want {
            Want::Tail => {
                // 値を返す atomic：plp はキャリーを戻すので、戻る直前に置く
                self.gen_block(b, &Want::Tail);
                self.f.atomic -= 1;
                return;
            }
            Want::Cond(l, j) => {
                let v = self.tmp_val(1);
                self.gen_block(b, &Want::Store(v.clone()));
                self.f.atomic -= 1;
                self.emit("plp", AMode::Imp, "atomic の終わり：許可状態を元に戻す");
                let (l, j) = (l.clone(), *j);
                self.emit_byte("lda", &v, 0, "atomic の値");
                self.br(if j { "bne" } else { "beq" }, &l, "条件");
                let _ = flagged;
                return;
            }
            _ => {}
        }
        let _ = t;
        self.gen_block(b, want);
        self.f.atomic -= 1;
        self.emit("plp", AMode::Imp, "atomic の終わり：許可状態を元に戻す");
    }

    // ---- cycles / io_span / max_cycles（12.1節）----
    fn gen_timed(&mut self, e: &Expr, k: &TimedKind, n: &Expr, b: &Block) {
        let target = self.ctx.econst.get(&n.id).and_then(value_int).unwrap_or(0);
        if *k == TimedKind::IoSpan {
            self.unimpl("io_span() のサイクル数の調整", &e.span);
            self.gen_block(b, &Want::Effect);
            return;
        }
        let atomic = *k == TimedKind::Cycles && !self.f.interrupt && self.ctx.target.has_interrupts();
        if atomic {
            self.emit("php", AMode::Imp, "cycles() は atomic を兼ねる");
            self.emit("sei", AMode::Imp, "割り込みを止める");
            self.f.atomic += 1;
        }
        self.keep_begin();
        let start = self.f.items.len();
        self.gen_block(b, &Want::Effect);
        let body: Vec<Item> = self.f.items[start..].to_vec();
        match self.straight_cycles(&body) {
            Some(c) => {
                if c as i128 > target {
                    self.ctx.diags.err("E0906", &format!("ブロックの中が {} サイクルで、指定の {} サイクルを超えています", c, target), e.span.clone());
                } else if *k == TimedKind::Cycles {
                    let mut rest = (target - c as i128) as u32;
                    if rest == 1 {
                        self.ctx.diags.err("E0907", "1 サイクルだけ足りません（nop は 2 サイクル）。そろえられません", e.span.clone());
                    }
                    if rest % 2 == 1 && rest >= 3 {
                        let l = self.new_label("pad");
                        self.emit("jmp", AMode::Mem(OpExpr::sym(&l)), "3 サイクル埋める（コンパイラが入れた）");
                        self.label(&l);
                        rest -= 3;
                    }
                    let nops = rest / 2;
                    for _ in 0..nops {
                        self.emit("nop", AMode::Imp, "2 サイクル埋める（コンパイラが入れた）");
                    }
                    self.notes.push(format!("cycles({})（{}行）：本体 {} サイクル、nop {} 個を埋めた", target, e.span.line, c, nops));
                } else {
                    self.notes.push(format!("max_cycles({})（{}行）：最長 {} サイクル", target, e.span.line, c));
                }
            }
            None => {
                self.unimpl("分岐・ループ・関数呼び出しを含む cycles()・max_cycles() のサイクル数の計算", &e.span);
            }
        }
        self.keep_end();
        if atomic {
            self.f.atomic -= 1;
            self.emit("plp", AMode::Imp, "atomic の終わり");
        }
    }

    /// 分岐のない命令の並びのサイクル数
    pub fn straight_cycles(&self, items: &[Item]) -> Option<u32> {
        let asm = crate::asm6502::Assembler::new();
        let known: HashMap<String, i64> = HashMap::new();
        let mut total = 0;
        for it in items {
            match it {
                Item::Ins(ins) => {
                    if crate::asm6502::is_branch(&ins.mn) || ins.mn == "jsr" || ins.mn == "rts" {
                        return None;
                    }
                    if ins.mn == "jmp" {
                        // パディングの jmp だけ
                        total += 3;
                        continue;
                    }
                    let (c, page) = asm.ins_cycles(ins, &known);
                    let is_store = matches!(ins.mn.as_str(), "sta" | "stz" | "inc" | "dec" | "asl" | "lsr" | "rol" | "ror");
                    if page && !is_store {
                        return None;
                    }
                    total += c;
                }
                Item::Label(_) => return None,
                _ => {}
            }
        }
        Some(total)
    }

    // ---- インラインアセンブリ（14章）----
    fn gen_asm_block(&mut self, e: &Expr, a: &AsmBlock) {
        // 受け渡し：in
        let mut ins_x = None;
        let mut ins_y = None;
        let mut ins_a = None;
        for b in &a.binds {
            if let AsmBind::In(r, x) = b {
                match r.to_lowercase().as_str() {
                    "x" => ins_x = Some(x.clone()),
                    "y" => ins_y = Some(x.clone()),
                    _ => ins_a = Some(x.clone()),
                }
            }
        }
        if let Some(x) = ins_x {
            self.gen_a(&x);
            self.emit0("tax", "asm の入力を X に");
        }
        if let Some(y) = ins_y {
            if self.f.xres {
                self.unimpl("X を使うループの中での asm (in Y = …)", &e.span);
            }
            self.gen_a(&y);
            self.emit0("tay", "asm の入力を Y に");
        }
        if let Some(x) = ins_a {
            self.gen_a(&x);
        }
        let prefix = self.new_label("asm");
        self.f.items.push(Item::Comment("; ---- asm ブロック（ソースのとおり）----".to_string()));
        self.keep_begin();
        self.translate_asm(&a.text, &prefix, false, a.is_unsafe, &e.span);
        self.keep_end();
        // アセンブリが何を呼ぶかは分からないので、すべてのレジスタを壊すとみなす
        self.f.callee_clob |= 7;
        self.f.xcache = None;
        self.f.ycache = None;
        // out
        for b in &a.binds {
            if let AsmBind::Out(r, name, sp) = b {
                let v = self.find_named_local(name);
                match v {
                    Some(v) => {
                        let mn = match r.to_lowercase().as_str() {
                            "x" => "stx",
                            "y" => "sty",
                            _ => "sta",
                        };
                        if let Val::Mem { base, idx: Idx::None, .. } = &v {
                            self.emit(mn, AMode::Mem(base.expr()), &format!("asm の出力を {} に", name));
                        } else {
                            self.unimpl("レジスタに置いた変数への asm の出力", sp);
                        }
                    }
                    None => self.unimpl("asm の出力先が見つかりません", sp),
                }
            }
        }
    }

    pub fn find_named_local(&mut self, name: &str) -> Option<Val> {
        let fid = self.f.fid;
        if fid == usize::MAX {
            return None;
        }
        let locals = self.ctx.funcs[fid].locals.clone();
        // 後から宣言されたものを優先する
        for (i, l) in locals.iter().enumerate().rev() {
            if l.name == name && self.f.slots.contains_key(&i) {
                return Some(self.local_val(i));
            }
        }
        for (i, l) in locals.iter().enumerate().rev() {
            if l.name == name {
                return Some(self.local_val(i));
            }
        }
        None
    }

    fn gen_asm_fn(&mut self, fid: usize, layout: &[ParamLoc]) {
        let fi = self.ctx.funcs[fid].clone();
        let d = match &fi.decl {
            Some(d) => d.clone(),
            None => return,
        };
        let text = match &d.asm_body {
            Some(t) => t.clone(),
            None => return,
        };
        // {引数} を参照するなら、入口でしまう
        let refs = crate::check::asm_ref_names(&text.text);
        for (i, pl) in layout.iter().enumerate() {
            let pname = fi.params[i].0.clone();
            let used = refs.iter().any(|(n, _)| n == &pname || n.starts_with(&format!("{}.", pname)));
            if used {
                let base = self.slot_base(&fi.label, &pl.slot);
                for (k, r) in pl.regs.iter().enumerate() {
                    let mn = match r {
                        'a' => "sta",
                        'x' => "stx",
                        _ => "sty",
                    };
                    self.emit(mn, AMode::Mem(base.add(k as i64).expr()), &format!("引数 {} を受け取る", pname));
                }
            }
        }
        let prefix = format!("{}@", fi.label);
        self.keep_begin();
        self.translate_asm(&text, &prefix, true, false, &d.span);
        self.keep_end();
    }
}

