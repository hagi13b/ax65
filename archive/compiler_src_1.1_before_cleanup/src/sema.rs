// 意味解析：宣言の登録、型と名前の解決、プログラム全体の検査
use crate::ast::*;
use crate::comptime::{value_int, Value};
use crate::diag::{Diag, Diags, LintLevel, SourceMap, Span};
use crate::program::*;
use crate::project::{Project, Target};
use crate::types::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cx {
    Main,
    Irq,
    Nmi,
}

impl Cx {
    pub fn name(&self) -> &'static str {
        match self {
            Cx::Main => "メイン",
            Cx::Irq => "IRQ 処理",
            Cx::Nmi => "NMI 処理",
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_ctx(
    loaded: Loaded,
    proj: Project,
    target: Target,
    build: BTreeMap<String, CfgVal>,
    opt: OptLevel,
    step_limit: u64,
    std_dir: PathBuf,
    known: HashSet<String>,
    sm: SourceMap,
    diags: Diags,
) -> Ctx {
    let nmod = loaded.modules.len();
    let mut ctx = Ctx {
        sm,
        diags,
        proj,
        target,
        build,
        opt,
        step_limit,
        std_dir,
        known_ifaces: known,
        modules: loaded.modules,
        mod_index: loaded.index,
        bound_impl: loaded.bound_impl,
        items: Vec::new(),
        mod_scope: vec![HashMap::new(); nmod],
        imports: vec![HashMap::new(); nmod],
        used_imports: HashSet::new(),
        types: TypeTable::default(),
        item_type: HashMap::new(),
        type_busy: HashSet::new(),
        globals: Vec::new(),
        item_global: HashMap::new(),
        funcs: Vec::new(),
        item_func: HashMap::new(),
        const_busy: HashSet::new(),
        ety: HashMap::new(),
        econst: HashMap::new(),
        res: HashMap::new(),
        calls: HashMap::new(),
        ct_choice: HashMap::new(),
        node_local: HashMap::new(),
        events: Vec::new(),
        call_edges: Vec::new(),
        unsafe_list: Vec::new(),
        notes: Vec::new(),
        next_id: loaded.next_id,
        steps: 0,
    };
    ctx.register_items();
    ctx.register_imports();
    ctx.assign_labels();
    ctx
}

impl Ctx {
    fn register_items(&mut self) {
        for m in 0..self.modules.len() {
            let items = self.modules[m].items.clone();
            let impl_items = self.modules[m].impl_items.clone();
            for it in items {
                let name = match it.name() {
                    Some(n) => n,
                    None => {
                        if let ItemKind::ComptimeAssert(_, _) = it.kind {
                            // 後で検査する
                        }
                        continue;
                    }
                };
                if let ItemKind::Interface(_) = it.kind {
                    continue;
                }
                if self.mod_scope[m].contains_key(&name) {
                    self.err("E0206", &format!("同じ名前 {} が 2 回宣言されています", name), &it.span);
                    continue;
                }
                let id = self.items.len();
                let in_impl = impl_items.contains(&name);
                self.items.push(ItemEntry { module: m, name: name.clone(), item: it.clone(), in_impl });
                self.mod_scope[m].insert(name.clone(), id);
                self.register_item_info(id, m, &it, in_impl, None);
            }
            // init
            let inits = self.modules[m].inits.clone();
            for b in inits {
                let fid = self.funcs.len();
                let sp = b.span.clone();
                self.funcs.push(FnInfo {
                    name: format!("init({})", self.modules[m].name),
                    module: m,
                    item: None,
                    decl: None,
                    init_body: Some(b),
                    kind: FnKind::Normal,
                    attrs: Vec::new(),
                    params: Vec::new(),
                    ret: Ty::Unit,
                    interrupt: None,
                    locals: Vec::new(),
                    label: String::new(),
                    span: sp,
                    sig_done: true,
                    checked: false,
                    calls_irq_enable: false,
                    has_unsafe: false,
                    is_pub: false,
                    iface_member: false,
                });
                let _ = fid;
            }
        }
    }

    /// 宣言の種類ごとの情報を作る。fn_local は関数の中の static のとき
    pub fn register_item_info(&mut self, id: usize, m: usize, it: &Item, in_impl: bool, fn_local: Option<usize>) -> Option<usize> {
        let mk_global = |name: &str, kind: GKind, ty: Option<Type>, init: Option<Expr>| GlobalInfo {
            name: name.to_string(),
            module: m,
            kind,
            ty: Ty::Error,
            attrs: it.attrs.clone(),
            span: it.span.clone(),
            label: String::new(),
            init_expr: init,
            decl_ty: ty,
            value: None,
            io_addr: None,
            fn_local,
            is_pub: it.is_pub,
            asm_ref: false,
            resolved: false,
        };
        let g = match &it.kind {
            ItemKind::Const(d) => Some(mk_global(&d.name, GKind::Const, d.ty.clone(), d.value.clone())),
            ItemKind::Static(d) => Some(mk_global(&d.name, GKind::Static, Some(d.ty.clone()), d.value.clone())),
            ItemKind::Shared(d) => Some(mk_global(&d.name, GKind::Shared(d.writer.clone()), Some(d.ty.clone()), d.value.clone())),
            ItemKind::Io(d) => {
                let mut gi = mk_global(&d.name, GKind::Io(d.dir.clone()), Some(d.ty.clone()), Some(d.addr.clone()));
                gi.init_expr = Some(d.addr.clone());
                Some(gi)
            }
            _ => None,
        };
        if let Some(gi) = g {
            let gid = self.globals.len();
            self.globals.push(gi);
            if id != usize::MAX {
                self.item_global.insert(id, gid);
            }
            return Some(gid);
        }
        if let ItemKind::Fn(d) = &it.kind {
            let fid = self.funcs.len();
            let interrupt = find_attr(&it.attrs, "interrupt").and_then(attr_ident_arg);
            self.funcs.push(FnInfo {
                name: d.name.clone(),
                module: m,
                item: if id == usize::MAX { None } else { Some(id) },
                decl: Some(d.clone()),
                init_body: None,
                kind: d.kind.clone(),
                attrs: it.attrs.clone(),
                params: Vec::new(),
                ret: Ty::Unit,
                interrupt,
                locals: Vec::new(),
                label: String::new(),
                span: d.span.clone(),
                sig_done: false,
                checked: false,
                calls_irq_enable: false,
                has_unsafe: d.kind == FnKind::Extern,
                is_pub: it.is_pub,
                iface_member: in_impl,
            });
            if id != usize::MAX {
                self.item_func.insert(id, fid);
            }
            return Some(fid);
        }
        None
    }

    fn register_imports(&mut self) {
        for m in 0..self.modules.len() {
            let uses = self.modules[m].uses.clone();
            for (u, sp, _attrs) in uses {
                let segs = u.path.segs.clone();
                // 読み込まれたモジュールのうち最も長い前方一致
                let mut found: Option<(usize, Vec<String>)> = None;
                let mut n = segs.len();
                while n >= 1 {
                    let name = segs[..n].join("::");
                    if let Some(i) = self.mod_index.get(&name) {
                        found = Some((*i, segs[n..].to_vec()));
                        break;
                    }
                    n -= 1;
                }
                let (mm, rest) = match found {
                    Some(x) => x,
                    None => continue, // 読み込みのときにエラーを出している
                };
                let eff = self.effective_module(mm);
                let add = |ctx: &mut Ctx, name: String, imp: Import, sp: &Span| {
                    if name == "_" {
                        return;
                    }
                    if ctx.imports[m].contains_key(&name) || ctx.mod_scope[m].contains_key(&name) {
                        ctx.err("E0202", &format!("use で取り込んだ名前 {} が重なっています", name), sp);
                        return;
                    }
                    ctx.imports[m].insert(name, imp);
                };
                if let Some(names) = &u.names {
                    let base = if rest.is_empty() { Some(mm) } else { None };
                    if base.is_none() {
                        self.err("E0204", &format!("モジュール {} が見つかりません", segs.join("::")), &sp);
                        continue;
                    }
                    for (nm, nsp) in names {
                        match &eff {
                            Ok(em) => {
                                let item = self.mod_scope[*em].get(nm).copied();
                                match item {
                                    Some(it) => {
                                        if !self.item_visible_from(it, m) {
                                            self.err("E0203", &format!("{} は公開されていません（pub がありません）", nm), nsp);
                                        }
                                        add(&mut *self, nm.clone(), Import { item: Some(it), module: None, span: nsp.clone(), is_underscore: false }, nsp);
                                    }
                                    None => self.err(
                                        "E0201",
                                        &format!("モジュール {} に {} がありません", self.modules[*em].name, nm),
                                        nsp,
                                    ),
                                }
                            }
                            Err(_) => {
                                // bind されていないインターフェース：使ったときにエラー
                                add(&mut *self, nm.clone(), Import { item: None, module: Some(mm), span: nsp.clone(), is_underscore: false }, nsp);
                            }
                        }
                    }
                } else if !rest.is_empty() {
                    if rest.len() > 1 {
                        self.err("E0204", &format!("モジュール {} が見つかりません", segs.join("::")), &sp);
                        continue;
                    }
                    let nm = rest[0].clone();
                    let alias = u.alias.clone().unwrap_or_else(|| nm.clone());
                    match &eff {
                        Ok(em) => match self.mod_scope[*em].get(&nm).copied() {
                            Some(it) => {
                                if !self.item_visible_from(it, m) {
                                    self.err("E0203", &format!("{} は公開されていません（pub がありません）", nm), &sp);
                                }
                                add(&mut *self, alias, Import { item: Some(it), module: None, span: sp.clone(), is_underscore: false }, &sp);
                            }
                            None => self.err("E0201", &format!("モジュール {} に {} がありません", self.modules[*em].name, nm), &sp),
                        },
                        Err(_) => add(&mut *self, alias, Import { item: None, module: Some(mm), span: sp.clone(), is_underscore: false }, &sp),
                    }
                } else {
                    let alias = u.alias.clone().unwrap_or_else(|| segs[segs.len() - 1].clone());
                    if alias == "_" {
                        continue;
                    }
                    add(&mut *self, alias, Import { item: None, module: Some(mm), span: sp.clone(), is_underscore: false }, &sp);
                }
            }
        }
    }

    pub fn assign_labels(&mut self) {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for g in &self.globals {
            if g.fn_local.is_none() {
                *counts.entry(g.name.clone()).or_insert(0) += 1;
            }
        }
        for f in &self.funcs {
            if f.init_body.is_none() {
                *counts.entry(f.name.clone()).or_insert(0) += 1;
            }
        }
        let reserved = ["a", "x", "y", "A", "X", "Y", "start", "LOCALS", "ZLOCALS"];
        for i in 0..self.globals.len() {
            if self.globals[i].fn_local.is_some() || !self.globals[i].label.is_empty() {
                continue;
            }
            let n = self.globals[i].name.clone();
            let m = self.globals[i].module;
            let unique = counts.get(&n).copied().unwrap_or(0) <= 1 && !reserved.contains(&n.as_str());
            self.globals[i].label = if unique { n } else { format!("{}_{}", module_label_prefix(&self.modules[m].name), n) };
        }
        for i in 0..self.funcs.len() {
            if !self.funcs[i].label.is_empty() {
                continue;
            }
            let n = self.funcs[i].name.clone();
            let m = self.funcs[i].module;
            if self.funcs[i].init_body.is_some() {
                self.funcs[i].label = format!("init_{}", module_label_prefix(&self.modules[m].name));
                continue;
            }
            let unique = counts.get(&n).copied().unwrap_or(0) <= 1 && !reserved.contains(&n.as_str());
            self.funcs[i].label = if unique { n } else { format!("{}_{}", module_label_prefix(&self.modules[m].name), n) };
        }
    }

    // ---- 名前の解決 ----

    fn item_res(&mut self, it: usize) -> PathRes {
        if let Some(g) = self.item_global.get(&it) {
            return PathRes::Global(*g);
        }
        if let Some(f) = self.item_func.get(&it) {
            return PathRes::Func(*f);
        }
        match self.item_type_of(it) {
            Some(t) => PathRes::Type(t),
            None => PathRes::NotFound(format!("{} は値として使えません", self.items[it].name)),
        }
    }

    fn assoc_res(&mut self, t: &Ty, name: &str) -> PathRes {
        match t {
            Ty::Enum(e) => {
                let d = self.types.enums[*e].clone();
                if let Some(v) = d.variants.iter().find(|v| v.0 == name) {
                    return PathRes::Value(Value::Enum(*e, v.1));
                }
                if name == "check" {
                    return PathRes::TypeFn(t.clone(), name.to_string());
                }
                PathRes::NotFound(format!("列挙型 {} に {} はありません", d.name, name))
            }
            Ty::Flags(f) => {
                let d = self.types.flags[*f].clone();
                if let Some(i) = d.names.iter().position(|n| n == name) {
                    return PathRes::Value(Value::Flags(*f, 1i128 << i));
                }
                PathRes::NotFound(format!("ビットフラグ型 {} に {} はありません", d.name, name))
            }
            Ty::Range(_) => match name {
                "check" | "wrap" | "unchecked" | "range" => PathRes::TypeFn(t.clone(), name.to_string()),
                _ => PathRes::NotFound(format!("範囲型 {} に関数 {} はありません", self.types.name(t), name)),
            },
            Ty::StrBuf(_) => {
                if name == "new" {
                    PathRes::TypeFn(t.clone(), name.to_string())
                } else {
                    PathRes::NotFound(format!("strbuf に関数 {} はありません", name))
                }
            }
            _ => PathRes::NotFound(format!("型 {} に {} はありません", self.types.name(t), name)),
        }
    }

    /// 値の位置のパスを解決する（エラーは出さず、結果で返す）
    pub fn resolve_value_path(&mut self, m: usize, segs: &[String], _sp: &Span) -> PathRes {
        if segs.is_empty() {
            return PathRes::NotFound("名前がありません".to_string());
        }
        if segs.len() == 1 {
            let s = &segs[0];
            if let Some(it) = self.lookup_name(m, s) {
                return self.item_res(it);
            }
            if let Some(imp) = self.imports[m].get(s).cloned() {
                if imp.item.is_none() {
                    if let Some(mm) = imp.module {
                        self.used_imports.insert((m, s.clone()));
                        if let Err(iname) = self.effective_module(mm) {
                            if self.modules[mm].iface.is_some() && imp.span.line > 0 && !self.mod_scope[mm].contains_key(s) {
                                // use iface::{name} の name
                                if self.modules[mm].name != *s {
                                    return PathRes::Unbound(iname);
                                }
                            }
                        }
                        return PathRes::Module(mm);
                    }
                }
            }
            if let Some(mm) = self.lookup_module(m, s) {
                return PathRes::Module(mm);
            }
            if let Some(t) = builtin_type(s) {
                return PathRes::Type(t);
            }
            return PathRes::NotFound(format!("名前 {} が見つかりません", s));
        }
        // 型に付いた名前（Dir::Up、Eid::check など）
        if let Some(it) = self.lookup_name(m, &segs[0]) {
            if let Some(t) = self.item_type_of(it) {
                if segs.len() == 2 {
                    return self.assoc_res(&t, &segs[1]);
                }
                return PathRes::NotFound(format!("{} が見つかりません", segs.join("::")));
            }
        }
        if segs.len() == 2 && segs[0] == "strbuf" {
            return self.assoc_res(&Ty::StrBuf(0), &segs[1]);
        }
        // モジュールのパス
        let mut found: Option<(usize, usize)> = None;
        if let Some(mm) = self.lookup_module(m, &segs[0]) {
            found = Some((mm, 1));
        } else {
            let mut n = segs.len() - 1;
            while n >= 1 {
                let name = segs[..n].join("::");
                if let Some(i) = self.mod_index.get(&name) {
                    found = Some((*i, n));
                    break;
                }
                n -= 1;
            }
        }
        let (mm, k) = match found {
            Some(x) => x,
            None => return PathRes::NotFound(format!("{} が見つかりません", segs.join("::"))),
        };
        let em = match self.effective_module(mm) {
            Ok(e) => e,
            Err(iname) => return PathRes::Unbound(iname),
        };
        let rest = &segs[k..];
        if rest.is_empty() {
            return PathRes::Module(em);
        }
        let it = match self.mod_scope[em].get(&rest[0]).copied() {
            Some(i) => i,
            None => {
                return PathRes::NotFound(format!("モジュール {} に {} がありません", self.modules[em].name, rest[0]));
            }
        };
        if !self.item_visible_from(it, m) {
            return PathRes::Private(format!("{} は公開されていません（pub がありません）", rest.join("::")));
        }
        if rest.len() == 1 {
            return self.item_res(it);
        }
        if rest.len() == 2 {
            if let Some(t) = self.item_type_of(it) {
                return self.assoc_res(&t, &rest[1]);
            }
        }
        PathRes::NotFound(format!("{} が見つかりません", segs.join("::")))
    }

    pub fn try_type_path(&mut self, m: usize, segs: &[String]) -> Option<Ty> {
        if segs.len() == 1 {
            if let Some(t) = builtin_type(&segs[0]) {
                // ローカルの名前で隠されることはない（型の位置）
                if self.mod_scope[m].get(&segs[0]).is_none() {
                    return Some(t);
                }
            }
        }
        match self.resolve_value_path(m, segs, &Span::none()) {
            PathRes::Type(t) => Some(t),
            _ => None,
        }
    }

    pub fn resolve_type_segs(&mut self, m: usize, segs: &[String], sp: &Span) -> Option<Ty> {
        if let Some(t) = self.try_type_path(m, segs) {
            return Some(t);
        }
        match self.resolve_value_path(m, segs, sp) {
            PathRes::Unbound(i) => self.err(
                "E1104",
                &format!("インターフェース {} はこのターゲットで bind されていません", i),
                sp,
            ),
            PathRes::Private(msg) => self.err("E0203", &msg, sp),
            _ => self.err("E0201", &format!("型 {} が見つかりません", segs.join("::")), sp),
        }
        None
    }

    /// 型の宣言（struct・enum・range・flags・fnset・type）の型
    pub fn item_type_of(&mut self, it: usize) -> Option<Ty> {
        if let Some(t) = self.item_type.get(&it) {
            return Some(t.clone());
        }
        let entry = self.items[it].clone();
        let m = entry.module;
        let attrs = entry.item.attrs.clone();
        match &entry.item.kind {
            ItemKind::Struct(d) => {
                let sid = self.types.structs.len();
                self.types.structs.push(StructDef {
                    name: d.name.clone(),
                    module: m,
                    fields: Vec::new(),
                    packed: find_attr(&attrs, "packed").is_some(),
                });
                let t = Ty::Struct(sid);
                self.item_type.insert(it, t.clone());
                let mut fields = Vec::new();
                for (fname, fty, fsp) in &d.fields {
                    let ft = self.resolve_type(m, fty);
                    if ft == Ty::Struct(sid) {
                        self.err("E0401", "構造体を自分自身のフィールドにはできません", fsp);
                    }
                    if fields.iter().any(|(n, _): &(String, Ty)| n == fname) {
                        self.err("E0206", &format!("フィールド {} が 2 回あります", fname), fsp);
                    }
                    fields.push((fname.clone(), ft));
                }
                self.types.structs[sid].fields = fields;
                Some(t)
            }
            ItemKind::Enum(d) => {
                let eid = self.types.enums.len();
                self.types.enums.push(EnumDef { name: d.name.clone(), module: m, variants: Vec::new() });
                let t = Ty::Enum(eid);
                self.item_type.insert(it, t.clone());
                let mut next: i128 = 0;
                let mut vars: Vec<(String, i128)> = Vec::new();
                for (vn, ve, vsp) in &d.variants {
                    let v = match ve {
                        Some(e) => match self.eval_in_module(e, m, None) {
                            Ok(v) => value_int(&v).unwrap_or(0),
                            Err(er) => {
                                self.diags.err(&er.code, &er.msg, er.span);
                                0
                            }
                        },
                        None => next,
                    };
                    if !(0..=255).contains(&v) {
                        self.err("E0510", "列挙型の値は 0〜255 です", vsp);
                    }
                    if vars.iter().any(|x| x.1 == v) {
                        self.err("E0206", &format!("列挙型の値 {} が重なっています", v), vsp);
                    }
                    if vars.iter().any(|x| &x.0 == vn) {
                        self.err("E0206", &format!("列挙型の名前 {} が重なっています", vn), vsp);
                    }
                    vars.push((vn.clone(), v));
                    next = v + 1;
                }
                self.types.enums[eid].variants = vars;
                Some(t)
            }
            ItemKind::Range(d) => {
                let lo = self.eval_in_module(&d.lo, m, Some(&Ty::CInt));
                let hi = self.eval_in_module(&d.hi, m, Some(&Ty::CInt));
                let (lo, hi) = match (lo, hi) {
                    (Ok(a), Ok(b)) => (value_int(&a).unwrap_or(0), value_int(&b).unwrap_or(0)),
                    (Err(e), _) | (_, Err(e)) => {
                        self.diags.err(&e.code, &e.msg, e.span);
                        (0, 1)
                    }
                };
                let hi = if d.inclusive { hi + 1 } else { hi };
                if hi <= lo || lo < 0 || hi > 65536 {
                    self.err("E0510", "範囲型の範囲が正しくありません（0〜65535 の、空でない範囲）", &entry.item.span);
                }
                let rid = self.types.ranges.len();
                self.types.ranges.push(RangeDef { name: d.name.clone(), module: m, lo, hi: hi.max(lo + 1) });
                let t = Ty::Range(rid);
                self.item_type.insert(it, t.clone());
                Some(t)
            }
            ItemKind::Flags(d) => {
                if d.names.len() > 16 {
                    self.err("E0401", "ビットフラグは 16 ビットまでです", &entry.item.span);
                }
                let fid = self.types.flags.len();
                self.types.flags.push(FlagsDef {
                    name: d.name.clone(),
                    module: m,
                    names: d.names.iter().map(|x| x.0.clone()).collect(),
                });
                let t = Ty::Flags(fid);
                self.item_type.insert(it, t.clone());
                Some(t)
            }
            ItemKind::FnSet(d) => {
                let sid = self.types.fnsets.len();
                self.types.fnsets.push(FnSetDef { name: d.name.clone(), module: m, fns: Vec::new() });
                let t = Ty::FnSet(sid);
                self.item_type.insert(it, t.clone());
                let mut fns = Vec::new();
                let mut sig: Option<(Vec<Ty>, Ty)> = None;
                for (fname, fsp) in &d.fns {
                    match self.resolve_value_path(m, &[fname.clone()], fsp) {
                        PathRes::Func(f) => {
                            self.ensure_sig(f);
                            let s = (
                                self.funcs[f].params.iter().map(|p| p.1.clone()).collect::<Vec<Ty>>(),
                                self.funcs[f].ret.clone(),
                            );
                            let mismatch = match &sig {
                                Some(s0) => s0 != &s,
                                None => false,
                            };
                            if sig.is_none() {
                                sig = Some(s);
                            } else if mismatch {
                                self.err("E0602", &format!("関数集合の {} の引数と戻り値の型が他と違います", fname), fsp);
                            }
                            if fns.len() >= 128 {
                                self.err("E0602", "関数集合に入れられる関数は 128 個までです", fsp);
                            }
                            fns.push(f);
                        }
                        _ => self.err("E0201", &format!("関数 {} が見つかりません", fname), fsp),
                    }
                }
                self.types.fnsets[sid].fns = fns;
                Some(t)
            }
            ItemKind::TypeAlias(d) => {
                if self.type_busy.contains(&it) {
                    self.err("E0401", "型の別名が循環しています", &entry.item.span);
                    return Some(Ty::Error);
                }
                self.type_busy.insert(it);
                let t = self.resolve_type(m, &d.ty);
                self.type_busy.remove(&it);
                self.item_type.insert(it, t.clone());
                Some(t)
            }
            _ => None,
        }
    }

    pub fn resolve_type(&mut self, m: usize, t: &Type) -> Ty {
        match &t.kind {
            TypeKind::Unit => Ty::Unit,
            TypeKind::Path(p) => match self.resolve_type_segs(m, &p.segs, &t.span) {
                Some(x) => x,
                None => Ty::Error,
            },
            TypeKind::Ptr(inner) => {
                let i = self.resolve_type(m, inner);
                Ty::Ptr(Box::new(i))
            }
            TypeKind::Option(inner) => {
                let i = self.resolve_type(m, inner);
                Ty::Opt(Box::new(i))
            }
            TypeKind::StrBuf(n) => match self.eval_in_module(n, m, Some(&Ty::CInt)) {
                Ok(v) => {
                    let x = value_int(&v).unwrap_or(0);
                    if !(1..=255).contains(&x) {
                        self.err("E0510", "strbuf の長さは 1〜255 です", &t.span);
                    }
                    Ty::StrBuf(x.max(1).min(255) as u32)
                }
                Err(e) => {
                    self.diags.err(&e.code, &e.msg, e.span);
                    Ty::Error
                }
            },
            TypeKind::Enc(base, enc) => {
                if !self.target.encodings.contains_key(enc) {
                    self.err("E1106", &format!("ターゲット定義に変換表 {} がありません", enc), &t.span);
                }
                let b = self.resolve_type(m, base);
                match b {
                    Ty::Char(None) => Ty::Char(Some(enc.clone())),
                    Ty::Str(k, None) => Ty::Str(k, Some(enc.clone())),
                    Ty::Array(e, l) if *e == Ty::Char(None) => Ty::Array(Box::new(Ty::Char(Some(enc.clone()))), l),
                    Ty::Error => Ty::Error,
                    _ => {
                        self.err("E0401", "変換表を付けられるのは char・str・zstr・histr だけです", &t.span);
                        Ty::Error
                    }
                }
            }
            TypeKind::Array(elem, n) => {
                let et = self.resolve_type(m, elem);
                let len = self.array_len(m, n);
                match len {
                    Some(l) => Ty::Array(Box::new(et), l),
                    None => Ty::Error,
                }
            }
        }
    }

    fn array_len(&mut self, m: usize, n: &Expr) -> Option<Len> {
        if let ExprKind::Path(p) = &n.kind {
            if let Some(t) = self.try_type_path(m, &p.segs) {
                return match t {
                    Ty::Range(r) => {
                        let d = self.types.ranges[r].clone();
                        if d.lo != 0 || d.hi > 256 {
                            self.err("E0401", &format!("配列の添字にできる範囲型は 0 から始まり 256 以下のものだけです（{}）", d.name), &n.span);
                        }
                        Some(Len::R(r))
                    }
                    Ty::Enum(e) => {
                        if !self.types.enums[e].contiguous() {
                            self.err("E0401", "0 から連続した値の列挙型だけが配列の添字になります", &n.span);
                        }
                        Some(Len::E(e))
                    }
                    _ => {
                        self.err("E0401", "配列の長さには数か、範囲型・列挙型の名前を書きます", &n.span);
                        None
                    }
                };
            }
        }
        match self.eval_in_module(n, m, Some(&Ty::CInt)) {
            Ok(v) => {
                let x = value_int(&v).unwrap_or(-1);
                if !(1..=256).contains(&x) {
                    self.err("E0510", &format!("配列の長さは 1〜256 です（{}）", x), &n.span);
                    return None;
                }
                Some(Len::N(x as u32))
            }
            Err(e) => {
                self.diags.err(&e.code, &e.msg, e.span);
                None
            }
        }
    }

    // ---- 定数・変数・関数の型 ----

    pub fn ensure_global(&mut self, gid: usize) {
        if self.globals[gid].resolved {
            return;
        }
        if self.const_busy.contains(&gid) {
            let sp = self.globals[gid].span.clone();
            let n = self.globals[gid].name.clone();
            self.err("E1001", &format!("{} の定義が循環しています", n), &sp);
            return;
        }
        self.const_busy.insert(gid);
        let g = self.globals[gid].clone();
        let m = g.module;
        let ty = match &g.decl_ty {
            Some(t) => self.resolve_type(m, t),
            None => {
                self.err("E0505", &format!("const {} には型を書く必要があります", g.name), &g.span);
                Ty::Error
            }
        };
        self.globals[gid].ty = ty.clone();
        match &g.kind {
            GKind::Io(_) => {
                if let Some(a) = &g.init_expr {
                    match self.eval_in_module(a, m, Some(&Ty::CInt)) {
                        Ok(v) => {
                            let x = value_int(&v).unwrap_or(-1);
                            if !(0..=0xFFFF).contains(&x) {
                                self.err("E0304", "I/O のアドレスは $0000〜$FFFF です", &a.span);
                            } else {
                                self.globals[gid].io_addr = Some(x as u32);
                                let size = self.types.size_of(&ty).max(1);
                                let in_io = (0..size).all(|k| self.target.in_region("io", x as u32 + k));
                                if !in_io {
                                    self.err("E0304", &format!("I/O 変数 {} のアドレス ${:04X} が io の区画の外です", g.name, x), &a.span);
                                }
                                // reserve の中には I/O 変数を宣言できない（1.1。16.3節）
                                let hit = (0..size).map(|k| x as u32 + k).find(|ad| self.target.is_reserved(*ad));
                                if let Some(ad) = hit {
                                    self.diags.error(
                                        Diag::error("E1111", &format!("I/O 変数 {} のアドレス ${:04X} は、ターゲット定義の reserve の中です", g.name, ad), a.span.clone())
                                            .note("reserve の範囲のレジスタは、モニタなどが使っています（16.3節）"),
                                    );
                                }
                            }
                        }
                        Err(e) => self.diags.err(&e.code, &e.msg, e.span),
                    }
                }
                let ok = match &ty {
                    Ty::Array(e, _) => io_elem_ok(&self.types, e),
                    t => io_elem_ok(&self.types, t),
                };
                if !ok && ty != Ty::Error {
                    self.err("E0905", "I/O 変数の型は u8・i8・char・8 ビット以下のビットフラグ型（とその配列）だけです", &g.span);
                }
                if g.is_pub {
                    self.err("E0904", "I/O 変数には pub を付けられません", &g.span);
                }
            }
            _ => {
                if let Some(e) = &g.init_expr {
                    if ty != Ty::Error {
                        match self.eval_in_module(e, m, Some(&ty)) {
                            Ok(v) => match self.convert_init(v, &ty, &e.span) {
                                Ok(v2) => self.globals[gid].value = Some(v2),
                                Err(er) => self.diags.err(&er.code, &er.msg, er.span),
                            },
                            Err(er) => self.diags.err(&er.code, &er.msg, er.span),
                        }
                    }
                } else if g.kind == GKind::Const {
                    self.err("E0505", &format!("const {} に値がありません", g.name), &g.span);
                } else if find_attr(&g.attrs, "noinit").is_none() {
                    self.err("E0513", &format!("static {} には初期値が必要です（初期化しないなら #[noinit]）", g.name), &g.span);
                }
                if let Ty::Array(_, _) = &ty {
                    self.check_array_layout(&ty, &g.span);
                }
                if let GKind::Shared(w) = &g.kind {
                    if w.as_deref() == Some("nmi") && self.types.size_of(&ty) != 1 {
                        self.err("E0805", "NMI と共有する変数は 1 バイトの型だけです", &g.span);
                    }
                    // 割り込みのないターゲットの shared（1.1。16.4節）
                    let tn = self.target.name.clone();
                    let msg = if !self.target.has_interrupts() {
                        Some(format!("ターゲット {} は irq・nmi がどちらも none なので、shared は使えません（static を使ってください）", tn))
                    } else if w.as_deref() == Some("irq") && !self.target.has_irq() {
                        Some(format!("ターゲット {} は irq = none なので、shared(irq) は使えません", tn))
                    } else if w.as_deref() == Some("nmi") && !self.target.has_nmi() {
                        Some(format!("ターゲット {} は nmi = none なので、shared(nmi) は使えません", tn))
                    } else {
                        None
                    };
                    if let Some(msg) = msg {
                        self.diags.error(Diag::error("E1110", &msg, g.span.clone()).note("irq・nmi を省略すると none になります（16.4節）"));
                    }
                }
            }
        }
        self.const_busy.remove(&gid);
        self.globals[gid].resolved = true;
    }

    fn check_array_layout(&mut self, ty: &Ty, sp: &Span) {
        if let Ty::Array(e, l) = ty {
            if let Ty::Struct(s) = &**e {
                if self.types.structs[*s].packed {
                    let total = self.types.size_of(ty);
                    if total > 256 {
                        self.err("E0302", "#[packed] の構造体の配列は全体で 256 バイト以下です", sp);
                    }
                }
            }
            let _ = l;
        }
    }

    /// 初期値の変換：int のリテラルは範囲を検査し、それ以外は型が同じであること
    fn convert_init(&mut self, v: Value, ty: &Ty, sp: &Span) -> Result<Value, crate::comptime::EvalErr> {
        self.convert_value(v, ty, sp)
    }

    pub fn ensure_sig(&mut self, fid: usize) {
        if self.funcs[fid].sig_done {
            return;
        }
        self.funcs[fid].sig_done = true;
        let decl = match &self.funcs[fid].decl {
            Some(d) => d.clone(),
            None => return,
        };
        let m = self.funcs[fid].module;
        let mut params = Vec::new();
        for p in &decl.params {
            let t = self.resolve_type(m, &p.ty);
            if p.reg.is_some() && decl.kind != FnKind::Extern {
                self.err("E0606", "in によるレジスタの指定は extern fn にだけ書けます", &p.span);
            }
            if decl.kind == FnKind::Extern {
                self.check_extern_param(p, &t);
            }
            if let Ty::Array(_, _) = t {
                if decl.kind != FnKind::Comptime && decl.kind != FnKind::Const {
                    self.err("E0415", "配列は引数にできません（ポインタか構造体を使ってください）", &p.span);
                }
            }
            params.push((p.name.clone(), t));
        }
        let ret = match &decl.ret {
            Some(t) => self.resolve_type(m, t),
            None => Ty::Unit,
        };
        if let Ty::Array(_, _) = ret {
            if decl.kind != FnKind::Comptime && decl.kind != FnKind::Const {
                self.err("E0415", "配列を返す関数は書けません（構造体に入れてください）", &decl.span);
            }
        }
        if decl.ret_reg.is_some() && decl.kind != FnKind::Extern {
            self.err("E0606", "in によるレジスタの指定は extern fn にだけ書けます", &decl.span);
        }
        if decl.kind == FnKind::Extern {
            self.check_extern_sig(&decl, &ret);
        }
        if self.funcs[fid].interrupt.is_some() && (!params.is_empty() || ret != Ty::Unit) {
            self.err("E0808", "割り込み処理の関数は、引数と戻り値を持てません", &decl.span);
        }
        if let Some(k) = self.funcs[fid].interrupt.clone() {
            if k != "irq" && k != "nmi" && k != "brk" {
                let sp = decl.span.clone();
                self.err("E0102", "#[interrupt(…)] には irq・nmi・brk のどれかを書きます", &sp);
            }
            // 割り込みのないターゲットの割り込み処理（1.1。10.1節・16.4節）
            let tn = self.target.name.clone();
            let bad = match k.as_str() {
                "irq" | "brk" => !self.target.has_irq(),
                "nmi" => !self.target.has_nmi(),
                _ => false,
            };
            if bad {
                let what = if k == "nmi" { "nmi" } else { "irq" };
                self.diags.error(
                    Diag::error("E1110", &format!("ターゲット {} は {} = none なので、#[interrupt({})] は使えません", tn, what, k), decl.span.clone())
                        .note("irq・nmi を省略すると none になります（16.4節）")
                        .help("割り込みのあるターゲットだけで使うなら comptime if target.has_irq { … } で囲んでください"),
                );
            }
        }
        self.funcs[fid].params = params;
        self.funcs[fid].ret = ret;
    }

    /// extern fn の引数のレジスタ（7.6節）：A・X・Y は 1 バイト、AX は 2 バイト
    fn check_extern_param(&mut self, p: &Param, t: &Ty) {
        let size = self.types.size_of(t);
        match p.reg.as_deref() {
            None => self.err("E0607", &format!("extern fn の引数 {} には in でレジスタを書きます（例：{}: u8 in A）", p.name, p.name), &p.span),
            Some("A") | Some("X") | Some("Y") => {
                if size != 1 && *t != Ty::Error {
                    self.err("E0607", &format!("引数 {} は {} バイトなので、A・X・Y の 1 つでは渡せません（2 バイトは AX）", p.name, size), &p.span);
                }
            }
            Some("AX") => {
                if size != 2 && *t != Ty::Error {
                    self.err("E0607", &format!("AX で渡せるのは 2 バイトの値だけです（引数 {} は {} バイト）", p.name, size), &p.span);
                }
            }
            Some("C") => self.err("E0607", "C（キャリー）は extern fn の戻り値（bool）だけに使えます", &p.span),
            Some("flags") => self.err("E0607", "flags は clobbers の中だけに書けます", &p.span),
            Some(r) => self.err("E0607", &format!("レジスタ {} はありません（引数に書けるのは A・X・Y・AX）", r), &p.span),
        }
    }

    /// extern fn の戻り値と clobbers（7.6節）
    fn check_extern_sig(&mut self, d: &FnDecl, ret: &Ty) {
        let sp = d.span.clone();
        if *ret != Ty::Unit && *ret != Ty::Error {
            let size = self.types.size_of(ret);
            let rn = self.types.name(ret);
            match d.ret_reg.as_deref() {
                None => self.err("E0607", &format!("extern fn の戻り値には in でレジスタを書きます（例：-> {} in A）", rn), &sp),
                Some("C") => {
                    if *ret != Ty::Bool {
                        self.err("E0607", &format!("C（キャリー）で返せるのは bool だけです（{} です）", rn), &sp);
                    }
                }
                Some("A") => {
                    let ok = match ret {
                        Ty::Bool => false,
                        // Option<u8> in A：有無はキャリー、値は A（1.1）
                        Ty::Opt(inner) => self.types.size_of(inner) == 1 && **inner != Ty::Bool,
                        _ => size == 1,
                    };
                    if !ok {
                        let hint = if *ret == Ty::Bool { "（bool は -> bool in C）" } else { "" };
                        self.err("E0607", &format!("A で返せるのは 1 バイトの値と Option<u8> だけです（{} です）{}", rn, hint), &sp);
                    }
                }
                Some("AX") => {
                    if size != 2 || matches!(ret, Ty::Opt(_) | Ty::Bool) {
                        self.err("E0607", &format!("AX で返せるのは 2 バイトの値だけです（{} です）", rn), &sp);
                    }
                }
                Some("flags") => self.err("E0607", "flags は clobbers の中だけに書けます", &sp),
                Some(r) => self.err("E0607", &format!("戻り値のレジスタ {} は使えません（A・AX・C）", r), &sp),
            }
        } else if d.ret_reg.is_some() && *ret == Ty::Unit {
            self.err("E0607", "戻り値のない extern fn に in は書けません", &sp);
        }
        for c in &d.clobbers {
            if !["A", "X", "Y", "flags"].contains(&c.as_str()) {
                self.err("E0607", &format!("clobbers に {} は書けません（A・X・Y・flags）", c), &sp);
            }
        }
    }

    // ---- 全体の検査 ----

    pub fn check_program(&mut self) {
        for g in 0..self.globals.len() {
            self.ensure_global(g);
        }
        for f in 0..self.funcs.len() {
            self.ensure_sig(f);
        }
        // 型の宣言を全部解決しておく
        for it in 0..self.items.len() {
            let _ = self.item_type_of(it);
        }
        self.check_items_misc();
        self.check_interfaces();
        // comptime assert（モジュールの直下）
        for m in 0..self.modules.len() {
            let items = self.modules[m].items.clone();
            for it in &items {
                if let ItemKind::ComptimeAssert(c, msg) = &it.kind {
                    match self.eval_in_module(c, m, None) {
                        Ok(Value::Bool(true)) => {}
                        Ok(Value::Bool(false)) => self.err("E1005", &format!("comptime assert が成り立ちません：{}", msg), &c.span),
                        Ok(_) => self.err("E0403", "comptime assert の条件は bool でなければなりません", &c.span),
                        Err(e) => self.diags.err(&e.code, &e.msg, e.span),
                    }
                }
            }
        }
        // 関数の本体
        let n = self.funcs.len();
        for f in 0..n {
            self.check_fn(f);
        }
        // 関数の中で見つかった static も解決する
        for g in 0..self.globals.len() {
            self.ensure_global(g);
        }
        self.check_main_exists();
        self.check_call_graph();
        self.check_unused_imports();
    }

    fn check_main_exists(&mut self) {
        let main_mod = 0usize;
        let has = self.funcs.iter().any(|f| f.name == "main" && f.module == main_mod && f.init_body.is_none());
        if !has {
            let sp = Span::new(self.modules[main_mod].file, 1, 1, 1);
            self.err("E0201", "main 関数がありません", &sp);
        }
    }

    fn check_items_misc(&mut self) {
        // I/O のアドレスの重なり（11.4節）
        let mut seen: HashMap<u32, (usize, String)> = HashMap::new();
        for g in self.globals.clone() {
            if let (GKind::Io(_), Some(a)) = (&g.kind, g.io_addr) {
                let size = self.types.size_of(&g.ty).max(1);
                for k in 0..size {
                    let addr = a + k;
                    if let Some((m0, n0)) = seen.get(&addr).cloned() {
                        if m0 != g.module {
                            self.err(
                                "E0903",
                                &format!("I/O のアドレス ${:04X} を {} と {} の 2 つのモジュールが宣言しています", addr, self.modules[m0].name, self.modules[g.module].name),
                                &g.span,
                            );
                        } else if n0 != g.name {
                            // 同じモジュールの中なら別名として許す
                        }
                        break;
                    }
                    seen.insert(addr, (g.module, g.name.clone()));
                }
            }
        }
        // 名前の慣習（2.3節）
        let empty: HashMap<String, LintLevel> = HashMap::new();
        for it in self.items.clone() {
            if self.modules[it.module].is_std {
                continue;
            }
            let ok = match &it.item.kind {
                ItemKind::Fn(_) => naming_ok_snake(&it.name),
                ItemKind::Const(_) | ItemKind::Static(_) | ItemKind::Shared(_) | ItemKind::Io(_) => naming_ok_upper(&it.name),
                ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Range(_) | ItemKind::Flags(_) | ItemKind::FnSet(_) | ItemKind::TypeAlias(_) => {
                    naming_ok_camel(&it.name)
                }
                _ => true,
            };
            if !ok {
                let what = match &it.item.kind {
                    ItemKind::Fn(_) => "関数は snake_case",
                    ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Range(_) | ItemKind::Flags(_) | ItemKind::FnSet(_) | ItemKind::TypeAlias(_) => "型は CamelCase",
                    _ => "定数・static・shared・I/O 変数は UPPER_CASE",
                };
                let lints = lint_overrides(&it.item.attrs);
                let d = Diag::warning("naming", &format!("{} は名前の慣習に合いません（{}）", it.name, what), it.item.span.clone());
                self.diags.warn_with(d, if lints.is_empty() { &empty } else { &lints });
            }
            if let ItemKind::Enum(d) = &it.item.kind {
                for (vn, _, vsp) in &d.variants {
                    if !naming_ok_camel(vn) {
                        let d = Diag::warning("naming", &format!("{} は名前の慣習に合いません（列挙型の値は CamelCase）", vn), vsp.clone());
                        self.diags.warn(d);
                    }
                }
            }
            if let ItemKind::Flags(d) = &it.item.kind {
                for (vn, vsp) in &d.names {
                    if !naming_ok_camel(vn) {
                        let d = Diag::warning("naming", &format!("{} は名前の慣習に合いません（ビットフラグの名前は CamelCase）", vn), vsp.clone());
                        self.diags.warn(d);
                    }
                }
            }
        }
    }

    fn check_interfaces(&mut self) {
        for m in 0..self.modules.len() {
            let iname = match &self.modules[m].impl_of {
                Some(n) => n.clone(),
                None => continue,
            };
            let impl_span = self.modules[m].impl_span.clone();
            // インターフェースの宣言を探す
            let mut iface: Option<(usize, InterfaceDecl)> = None;
            for (i, mm) in self.modules.iter().enumerate() {
                if let Some(d) = &mm.iface {
                    if d.name == iname {
                        iface = Some((i, d.clone()));
                    }
                }
            }
            let (im, decl) = match iface {
                Some(x) => x,
                None => {
                    // インターフェースのファイルを探して読む必要はない（use されていない実装）
                    continue;
                }
            };
            let mut member_names = HashSet::new();
            for mem in &decl.members {
                match mem {
                    IfaceMember::Fn(f) => {
                        member_names.insert(f.name.clone());
                        let want_params: Vec<Ty> = f.params.iter().map(|p| self.resolve_type(im, &p.ty)).collect();
                        let want_ret = match &f.ret {
                            Some(t) => self.resolve_type(im, t),
                            None => Ty::Unit,
                        };
                        let it = self.mod_scope[m].get(&f.name).copied();
                        match it.and_then(|i| self.item_func.get(&i).copied()) {
                            Some(fid) => {
                                self.ensure_sig(fid);
                                let got: Vec<Ty> = self.funcs[fid].params.iter().map(|p| p.1.clone()).collect();
                                if got != want_params || self.funcs[fid].ret != want_ret {
                                    let sp = self.funcs[fid].span.clone();
                                    self.err("E1101", &format!("{} の型がインターフェース {} の宣言と違います", f.name, iname), &sp);
                                }
                            }
                            None => self.err("E1101", &format!("impl {} に関数 {} がありません", iname, f.name), &impl_span),
                        }
                    }
                    IfaceMember::Const(n, t, _) => {
                        member_names.insert(n.clone());
                        let want = self.resolve_type(im, t);
                        let it = self.mod_scope[m].get(n).copied();
                        match it.and_then(|i| self.item_global.get(&i).copied()) {
                            Some(gid) => {
                                self.ensure_global(gid);
                                if self.globals[gid].ty != want || self.globals[gid].kind != GKind::Const {
                                    let sp = self.globals[gid].span.clone();
                                    self.err("E1101", &format!("{} の型がインターフェース {} の宣言と違います", n, iname), &sp);
                                }
                            }
                            None => self.err("E1101", &format!("impl {} に定数 {} がありません", iname, n), &impl_span),
                        }
                    }
                }
            }
            let pubs = self.modules[m].impl_pub_spans.clone();
            for (n, sp) in pubs {
                if member_names.contains(&n) {
                    self.err("E1102", "impl の中のインターフェースの関数・定数には pub を書けません（自動で公開されます）", &sp);
                } else {
                    self.err("E1102", "impl の中に pub は書けません", &sp);
                }
            }
        }
    }

    fn check_unused_imports(&mut self) {
        for m in 0..self.modules.len() {
            if self.modules[m].is_std {
                continue;
            }
            let uses = self.modules[m].uses.clone();
            for (u, sp, attrs) in uses {
                let mut names: Vec<(String, Span)> = Vec::new();
                if let Some(ns) = &u.names {
                    for (n, s) in ns {
                        names.push((n.clone(), s.clone()));
                    }
                } else {
                    let alias = u.alias.clone().unwrap_or_else(|| u.path.segs[u.path.segs.len() - 1].clone());
                    if alias == "_" {
                        continue;
                    }
                    names.push((alias, sp.clone()));
                }
                for (n, s) in names {
                    let cnt = self.modules[m].ident_counts.get(&n).copied().unwrap_or(0);
                    let used = self.used_imports.contains(&(m, n.clone())) || cnt >= 2;
                    if !used {
                        // init を持つモジュールか
                        let segs = u.path.segs.clone();
                        let mname = segs.join("::");
                        let has_init = self.mod_index.get(&mname).map(|i| !self.modules[*i].inits.is_empty()).unwrap_or(false);
                        let mut d = Diag::warning("unused_imports", &format!("use した {} を使っていません", n), s);
                        if has_init {
                            d = d.note(&format!("モジュール {} の init は起動時に実行されます", mname)).help(&format!("init のためだけなら use {} as _; と書いてください", mname));
                        }
                        let lints = lint_overrides(&attrs);
                        self.diags.warn_with(d, &lints);
                    }
                }
            }
        }
    }

    /// 呼び出しグラフ：文脈、再帰、irq_enable の検査（7.5節・10.2節・10.3節）
    pub fn contexts(&self) -> HashMap<usize, HashSet<Cx>> {
        let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
        for e in &self.call_edges {
            adj.entry(e.from).or_default().push(e.to);
        }
        let mut res: HashMap<usize, HashSet<Cx>> = HashMap::new();
        for (fid, f) in self.funcs.iter().enumerate() {
            let cx = if f.init_body.is_some() || (f.name == "main" && f.module == 0) {
                Some(Cx::Main)
            } else {
                match f.interrupt.as_deref() {
                    Some("irq") | Some("brk") => Some(Cx::Irq),
                    Some("nmi") => Some(Cx::Nmi),
                    _ => None,
                }
            };
            if let Some(c) = cx {
                let mut stack = vec![fid];
                let mut seen = HashSet::new();
                while let Some(x) = stack.pop() {
                    if !seen.insert(x) {
                        continue;
                    }
                    res.entry(x).or_default().insert(c);
                    if let Some(v) = adj.get(&x) {
                        for y in v {
                            stack.push(*y);
                        }
                    }
                }
            }
        }
        res
    }

    fn check_call_graph(&mut self) {
        let mut adj: HashMap<usize, Vec<(usize, Span, bool)>> = HashMap::new();
        for e in &self.call_edges {
            adj.entry(e.from).or_default().push((e.to, e.span.clone(), e.in_atomic));
        }
        // 再帰（輪）
        let n = self.funcs.len();
        let mut color = vec![0u8; n];
        let mut reported = false;
        for start in 0..n {
            if color[start] != 0 || reported {
                continue;
            }
            let mut path: Vec<usize> = Vec::new();
            if let Some((cycle, sp)) = dfs_cycle(start, &adj, &mut color, &mut path) {
                let names: Vec<String> = cycle.iter().map(|f| self.funcs[*f].name.clone()).collect();
                self.diags.error(
                    Diag::error("E0601", "再帰呼び出しがあります", sp)
                        .label(&format!("{} の輪ができています", names.join(" → ")))
                        .note("ax65 では再帰を使えません（7.5節）"),
                );
                reported = true;
            }
        }
        // 文脈（10.2節）
        let cxs = self.contexts();
        let mut is_handler = HashSet::new();
        for (i, f) in self.funcs.iter().enumerate() {
            if f.interrupt.is_some() {
                is_handler.insert(i);
            }
        }
        for (fid, set) in &cxs {
            if set.len() > 1 {
                let mut names: Vec<&str> = set.iter().map(|c| c.name()).collect();
                names.sort();
                let sp = self.funcs[*fid].span.clone();
                let fname = self.funcs[*fid].name.clone();
                self.diags.error(
                    Diag::error("E0801", &format!("{} は 2 つ以上の文脈（{}）から呼ばれています", fname, names.join("・")), sp)
                        .note("1 つの関数は、メイン・IRQ 処理・NMI 処理のうち 1 つの文脈からだけ呼べます（10.2節）"),
                );
            }
        }
        // irq_enable の伝播
        let mut enables: HashSet<usize> = HashSet::new();
        for (i, f) in self.funcs.iter().enumerate() {
            if f.calls_irq_enable {
                enables.insert(i);
            }
        }
        loop {
            let mut changed = false;
            for e in &self.call_edges {
                if enables.contains(&e.to) && !enables.contains(&e.from) {
                    enables.insert(e.from);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for (fid, set) in &cxs {
            if (set.contains(&Cx::Irq) || set.contains(&Cx::Nmi)) && self.funcs[*fid].calls_irq_enable {
                let sp = self.funcs[*fid].span.clone();
                self.err("E0802", "割り込み処理（そこから呼ぶ関数を含む）の中で irq_enable() を呼んでいます", &sp);
            }
        }
        for e in self.call_edges.clone() {
            if e.in_atomic && enables.contains(&e.to) {
                self.err(
                    "E0802",
                    &format!("atomic・cycles()・io_span() の中から呼ぶ {} で irq_enable() を呼んでいます", self.funcs[e.to].name),
                    &e.span,
                );
            }
        }
        if !is_handler.is_empty() || !self.target.has_irq() {
            // irq = none のときは irq_enable() 自体がエラー（E1110）
        } else {
            for (i, f) in self.funcs.clone().iter().enumerate() {
                if f.calls_irq_enable && cxs.contains_key(&i) {
                    let d = Diag::warning("irq_without_handler", "割り込み処理を宣言していないプログラムで irq_enable() を呼んでいます", f.span.clone());
                    self.diags.warn(d);
                }
            }
        }
        // exit = never
        if self.target.exit == Some(crate::project::Exit::Never) {
            for (i, f) in self.funcs.iter().enumerate() {
                if f.name == "main" && f.module == 0 && f.init_body.is_none() {
                    let ok = match &f.decl {
                        Some(d) => match &d.body {
                            Some(b) => self.ety.get(&b.id) == Some(&Ty::Never),
                            None => false,
                        },
                        None => false,
                    };
                    if !ok {
                        let sp = self.funcs[i].span.clone();
                        self.diags.err("E1107", "ターゲットの exit = never ですが、main が終わる可能性があります", sp);
                    }
                }
            }
        }
        self.check_shared_rules(&cxs);
    }

    /// shared と static の規則（10.4節）
    fn check_shared_rules(&mut self, cxs: &HashMap<usize, HashSet<Cx>>) {
        let events = self.events.clone();
        // static を複数の文脈で使っていないか
        let mut static_cx: HashMap<usize, (HashSet<Cx>, Span)> = HashMap::new();
        for ev in &events {
            let g = self.globals[ev.gid].clone();
            let fcx = match cxs.get(&ev.fid) {
                Some(s) => s.clone(),
                None => continue,
            };
            match &g.kind {
                GKind::Static => {
                    let e = static_cx.entry(ev.gid).or_insert((HashSet::new(), ev.span.clone()));
                    for c in fcx {
                        e.0.insert(c);
                    }
                }
                GKind::Shared(writer) => {
                    let size = self.types.size_of(&g.ty);
                    let elem_size = match &g.ty {
                        Ty::Array(e, _) => self.types.size_of(e),
                        _ => size,
                    };
                    let nmi_shared = writer.as_deref() == Some("nmi") || self.shared_with_nmi(ev.gid, &events, cxs);
                    for c in fcx {
                        // 書き込む側の指定
                        if ev.write {
                            let allowed = match writer.as_deref() {
                                Some("irq") => c == Cx::Irq,
                                Some("main") => c == Cx::Main,
                                Some("nmi") => c == Cx::Nmi,
                                _ => true,
                            };
                            if !allowed {
                                self.diags.error(
                                    Diag::error("E0804", &format!("{} は shared({}) なので、{}からは書き込めません", g.name, writer.clone().unwrap_or_default(), c.name()), ev.span.clone()),
                                );
                                continue;
                            }
                        }
                        if nmi_shared {
                            if writer.is_none() {
                                self.err("E0805", &format!("NMI と共有する {} には書き込む側の指定（shared(nmi) など）が必要です", g.name), &g.span);
                            }
                            if elem_size != 1 {
                                self.err("E0805", &format!("NMI と共有する {} は 1 バイトの型でなければなりません", g.name), &g.span);
                            }
                            if c != Cx::Nmi && !ev.single {
                                self.diags.error(
                                    Diag::error("E0803", &format!("NMI と共有する {} には、1 命令の操作だけを使えます", g.name), ev.span.clone())
                                        .note("atomic でも NMI は止められません（10.4節）"),
                                );
                            }
                            if ev.in_atomic && c != Cx::Nmi {
                                let d = Diag::warning("nmi_in_atomic", &format!("NMI と共有する {} を atomic の中で使っています（NMI は止められません）", g.name), ev.span.clone());
                                self.diags.warn(d);
                            }
                            continue;
                        }
                        if c != Cx::Main || ev.in_atomic || ev.single {
                            continue;
                        }
                        // メインでの 2 命令以上の操作
                        let writer_main = writer.as_deref() == Some("main");
                        let writer_irq = writer.as_deref() == Some("irq");
                        let ok = if writer_main {
                            // 書き込む側：読むのは不要、1 バイトの書き込みは不要
                            !ev.write || elem_size == 1
                        } else if writer_irq {
                            // 読む側：2 バイト以上を読むには必要
                            !ev.write && elem_size == 1
                        } else {
                            false
                        };
                        if !ok {
                            self.diags.error(
                                Diag::error("E0803", &format!("{} への 2 命令以上の操作には atomic が必要です", g.name), ev.span.clone())
                                    .note("割り込み処理と共有する変数の規則（10.4節）")
                                    .help("atomic { … } で囲んでください"),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        for (gid, (set, sp)) in static_cx {
            if set.len() > 1 {
                let n = self.globals[gid].name.clone();
                self.diags.error(
                    Diag::error("E0806", &format!("static {} を割り込み処理とメインで共有しています", n), sp)
                        .help("割り込み処理と共有する変数は shared で宣言してください（10.4節）"),
                );
            }
        }
    }

    fn shared_with_nmi(&self, gid: usize, events: &[AccessEvent], cxs: &HashMap<usize, HashSet<Cx>>) -> bool {
        events.iter().any(|e| e.gid == gid && cxs.get(&e.fid).map(|s| s.contains(&Cx::Nmi)).unwrap_or(false))
    }
}

fn dfs_cycle(
    v: usize,
    adj: &HashMap<usize, Vec<(usize, Span, bool)>>,
    color: &mut Vec<u8>,
    path: &mut Vec<usize>,
) -> Option<(Vec<usize>, Span)> {
    color[v] = 1;
    path.push(v);
    if let Some(list) = adj.get(&v) {
        for (w, sp, _) in list {
            if color[*w] == 1 {
                let pos = path.iter().position(|x| x == w).unwrap_or(0);
                let mut cyc: Vec<usize> = path[pos..].to_vec();
                cyc.push(*w);
                return Some((cyc, sp.clone()));
            }
            if color[*w] == 0 {
                if let Some(r) = dfs_cycle(*w, adj, color, path) {
                    return Some(r);
                }
            }
        }
    }
    path.pop();
    color[v] = 2;
    None
}

fn io_elem_ok(types: &TypeTable, t: &Ty) -> bool {
    match t {
        Ty::Int(IntK::U8) | Ty::Int(IntK::I8) | Ty::Char(_) => true,
        Ty::Flags(f) => types.flags[*f].size() == 1,
        Ty::Array(e, _) => io_elem_ok(types, e),
        _ => false,
    }
}

pub fn builtin_type(s: &str) -> Option<Ty> {
    if let Some(k) = IntK::from_name(s) {
        return Some(Ty::Int(k));
    }
    match s {
        "bool" => Some(Ty::Bool),
        "char" => Some(Ty::Char(None)),
        "str" => Some(Ty::Str(StrK::P, None)),
        "zstr" => Some(Ty::Str(StrK::Z, None)),
        "histr" => Some(Ty::Str(StrK::Hi, None)),
        "bcd8" => Some(Ty::Bcd(1)),
        "bcd16" => Some(Ty::Bcd(2)),
        "bcd32" => Some(Ty::Bcd(4)),
        "int" => Some(Ty::CInt),
        "real" => Some(Ty::CReal),
        "Cpu" => Some(Ty::Cpu),
        _ => None,
    }
}

pub fn lint_overrides(attrs: &[Attr]) -> HashMap<String, LintLevel> {
    let mut m = HashMap::new();
    for a in attrs {
        let lvl = match a.name.as_str() {
            "allow" => LintLevel::Allow,
            "warn" => LintLevel::Warn,
            "deny" => LintLevel::Deny,
            _ => continue,
        };
        for arg in &a.args {
            if let AttrArg::Ident(n) = arg {
                m.insert(n.clone(), lvl);
            }
        }
    }
    m
}
