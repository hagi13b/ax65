// コード生成の後の処理：展開（インライン化）、末尾呼び出し、レジスタの退避、
// ローカル変数の重ね合わせ（4.8節）、ゼロページの配置（4.7節）、アセンブル、スタックの予算（15.5節）
use crate::asm6502::{AMode, AsmInput, AsmOutput, Assembler, Ins, Item, OpExpr};
use crate::ast::find_attr;
use crate::codegen::{frame_sym, zframe_sym, Gen};
use crate::diag::{Diag, Span};
use crate::program::{GKind, OptLevel};
use crate::sema::Cx;
use crate::types::Ty;
use std::collections::{BTreeMap, HashMap, HashSet};

pub fn for_each_expr_mut(it: &mut Item, f: &mut dyn FnMut(&mut OpExpr)) {
    match it {
        Item::Ins(ins) => match &mut ins.am {
            AMode::Imm(e) | AMode::Mem(e) | AMode::MemX(e) | AMode::MemY(e) | AMode::IndY(e) | AMode::IndX(e) | AMode::Ind(e) | AMode::IndAbsX(e) => f(e),
            _ => {}
        },
        Item::Exprs(v, _) => {
            for e in v.iter_mut() {
                f(e);
            }
        }
        _ => {}
    }
}

/// 記号 from を、記号 to＋add に置き換える
pub fn rename_sym_in_item(it: &mut Item, from: &str, to: &str, add: i64) {
    for_each_expr_mut(it, &mut |e: &mut OpExpr| {
        if e.sym.as_deref() == Some(from) {
            e.sym = Some(to.to_string());
            e.off += add;
        }
    });
}

fn sym_to_num(it: &mut Item, from: &str, addr: i64) {
    for_each_expr_mut(it, &mut |e: &mut OpExpr| {
        if e.sym.as_deref() == Some(from) {
            e.sym = None;
            e.off += addr;
        }
    });
}

#[derive(Clone, Debug, Default)]
pub struct StackInfo {
    pub main: u32,
    pub irq: u32,
    pub nmi: u32,
    pub main_path: Vec<String>,
    pub irq_path: Vec<String>,
    pub nmi_path: Vec<String>,
}

pub struct Linked {
    pub out: AsmOutput,
    pub inline_report: Vec<String>,
    pub zp_report: Vec<String>,
    pub locals_report: Vec<String>,
    pub var_report: Vec<(String, u32, u32, String)>,
    pub stack: StackInfo,
    pub irq_cycles: Vec<String>,
    pub fn_order: Vec<(String, usize)>,
    pub areas: Vec<(String, u32, u32)>,
    pub init_report: Vec<String>,
}

fn writes_reg(ins: &Ins) -> Vec<char> {
    let m = ins.mn.as_str();
    match m {
        "lda" | "pla" | "txa" | "tya" | "adc" | "sbc" | "and" | "ora" | "eor" => vec!['a'],
        "asl" | "lsr" | "rol" | "ror" | "inc" | "dec" => {
            if ins.am == AMode::Acc {
                vec!['a']
            } else {
                vec![]
            }
        }
        "ldx" | "tax" | "tsx" | "inx" | "dex" | "plx" => vec!['x'],
        "ldy" | "tay" | "iny" | "dey" | "ply" => vec!['y'],
        _ => vec![],
    }
}

fn target_label(ins: &Ins) -> Option<String> {
    if ins.mn == "jsr" || ins.mn == "jmp" {
        if let AMode::Mem(e) = &ins.am {
            if e.off == 0 && e.part == crate::asm6502::Part::Full {
                return e.sym.clone();
            }
        }
    }
    None
}

pub fn link(g: &mut Gen) -> Option<Linked> {
    let opt = g.ctx.opt;
    let mut inline_report = Vec::new();
    // ---- 展開（7.7節・19章）----
    let label_idx: HashMap<String, usize> = g.funcs.iter().enumerate().map(|(i, f)| (f.label.clone(), i)).collect();
    let mut sites: HashMap<String, usize> = HashMap::new();
    let mut addr_taken: HashSet<String> = HashSet::new();
    for f in &g.funcs {
        for it in &f.items {
            match it {
                Item::Ins(ins) => {
                    if let Some(l) = target_label(ins) {
                        if label_idx.contains_key(&l) {
                            *sites.entry(l).or_insert(0) += 1;
                        }
                    } else if let Some(e) = ins.expr() {
                        if let Some(s) = &e.sym {
                            if label_idx.contains_key(s) {
                                addr_taken.insert(s.clone());
                            }
                        }
                    }
                }
                Item::Exprs(v, _) => {
                    for e in v {
                        if let Some(s) = &e.sym {
                            addr_taken.insert(s.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    for it in &g.data {
        if let Item::Exprs(v, _) = it {
            for e in v {
                if let Some(s) = &e.sym {
                    addr_taken.insert(s.clone());
                }
            }
        }
    }
    let n = g.funcs.len();
    let mut inl = vec![false; n];
    for i in 0..n {
        let f = &g.funcs[i];
        if f.is_start || f.fid == usize::MAX || f.interrupt.is_some() {
            continue;
        }
        let fi = &g.ctx.funcs[f.fid];
        let name = fi.name.clone();
        let attrs = fi.attrs.clone();
        let count = sites.get(&f.label).copied().unwrap_or(0);
        let ninstr = f.items.iter().filter(|x| matches!(x, Item::Ins(_))).count();
        let never = find_attr(&attrs, "inline").and_then(crate::ast::attr_ident_arg).as_deref() == Some("never");
        let always = find_attr(&attrs, "inline").and_then(crate::ast::attr_ident_arg).as_deref() == Some("always");
        let hint = find_attr(&attrs, "inline").is_some() && !never;
        let cold = find_attr(&attrs, "cold").is_some();
        let reason_no: Option<String> = if fi.kind == crate::ast::FnKind::Asm {
            Some("asm fn は展開しない".to_string())
        } else if fi.has_unsafe {
            Some("unsafe asm を含む".to_string())
        } else if never {
            Some("#[inline(never)]".to_string())
        } else if addr_taken.contains(&f.label) {
            Some("アドレスを使っている（関数集合など）".to_string())
        } else if count == 0 {
            Some("呼び出しがない".to_string())
        } else {
            None
        };
        match reason_no {
            Some(r) => {
                if always {
                    let sp = fi.span.clone();
                    g.ctx.diags.err("E0605", &format!("#[inline(always)] の {} を展開できません（{}）", name, r), sp);
                }
                if !f.force_inline && count > 0 {
                    inline_report.push(format!("{} → 展開しない（{}）", name, r));
                }
            }
            None => {
                let yes = f.force_inline
                    || hint
                    || always
                    || (opt != OptLevel::O0 && count == 1)
                    || ((opt == OptLevel::O1 || opt == OptLevel::Os) && ninstr <= 12 && !cold);
                inl[i] = yes;
                if f.force_inline {
                    inline_report.push(format!("{} → 起動処理に展開（init は常に展開する）", name));
                } else if yes {
                    let why = if hint || always {
                        "#[inline]".to_string()
                    } else if count == 1 {
                        "呼び出し 1 か所".to_string()
                    } else {
                        format!("小さな関数（{} 命令、呼び出し {} か所）", ninstr, count)
                    };
                    inline_report.push(format!("{} → 展開する（{}。{}）", name, why, opt.name()));
                } else {
                    let why = if cold { "#[cold]".to_string() } else if opt == OptLevel::O0 { "-O0".to_string() } else { format!("呼び出し {} か所", count) };
                    inline_report.push(format!("{} → 展開しない（{}）", name, why));
                }
            }
        }
    }
    // 呼ばれる側から順に展開する
    let order = post_order(g);
    let mut inst = 0usize;
    for i in order {
        let items = g.funcs[i].items.clone();
        let mut out: Vec<Item> = Vec::new();
        for it in items {
            if let Item::Ins(ins) = &it {
                if let Some(l) = target_label(ins) {
                    if let Some(&ci) = label_idx.get(&l) {
                        if inl[ci] && ci != i {
                            inst += 1;
                            let tail = ins.mn == "jmp";
                            let callee = g.funcs[ci].items.clone();
                            let cname = if g.funcs[ci].fid != usize::MAX { g.ctx.funcs[g.funcs[ci].fid].name.clone() } else { l.clone() };
                            out.push(Item::Comment(format!("; ---- {} を展開 ----", cname)));
                            splice(&mut out, &callee, inst, tail);
                            continue;
                        }
                    }
                }
            }
            out.push(it);
        }
        g.funcs[i].items = out;
    }
    for i in 0..n {
        if inl[i] {
            g.funcs[i].emitted = false;
        }
    }
    // ---- 末尾呼び出しを jmp に（19.5節）と、すぐ次への分岐の削除 ----
    for f in g.funcs.iter_mut() {
        // 起動処理の形は gen_start で決めている（15.1節）ので、ここでは変えない
        if f.interrupt.is_none() && !f.is_start {
            tail_calls(&mut f.items);
        }
        drop_jump_to_next(&mut f.items);
    }
    // ---- のぞき穴最適化（-O1 以上。peep.rs）----
    // -O1 以上：のぞき穴最適化。-O0 でも、読まれないローカル変数への書き込みだけは消す（コード生成の一部）
    peephole_all(g, opt != OptLevel::O0);
    // ---- 割り込み処理のレジスタの退避（10.1節）----
    let clob = clobbers(g);
    for f in g.funcs.iter_mut() {
        if f.interrupt.is_none() {
            continue;
        }
        let regs = clob.get(&f.label).cloned().unwrap_or_default();
        let mut out = Vec::new();
        let items: Vec<Item> = f.items.drain(..).collect();
        for it in items {
            let is_save = matches!(&it, Item::Mark(m) if m == "SAVE");
            let is_restore = matches!(&it, Item::Mark(m) if m == "RESTORE");
            if is_save {
                {
                    if regs.contains(&'a') {
                        out.push(Item::Ins(Ins::new("pha", AMode::Imp, "A を退避する")));
                    }
                    if regs.contains(&'x') {
                        out.push(Item::Ins(Ins::new("phx", AMode::Imp, "X を退避する")));
                    }
                    if regs.contains(&'y') {
                        out.push(Item::Ins(Ins::new("phy", AMode::Imp, "Y を退避する")));
                    }
                    if regs.is_empty() {
                        out.push(Item::Comment("; レジスタを使わないので退避しない".to_string()));
                    }
                }
            } else if is_restore {
                {
                    if regs.contains(&'y') {
                        out.push(Item::Ins(Ins::new("ply", AMode::Imp, "Y を戻す")));
                    }
                    if regs.contains(&'x') {
                        out.push(Item::Ins(Ins::new("plx", AMode::Imp, "X を戻す")));
                    }
                    if regs.contains(&'a') {
                        out.push(Item::Ins(Ins::new("pla", AMode::Imp, "A を戻す")));
                    }
                }
            } else {
                out.push(it);
            }
        }
        f.items = out;
    }
    // ---- ローカル変数の重ね合わせ ----
    let cxs = g.ctx.contexts();
    let (bases, areas) = overlay(g, &cxs, false);
    let (zbases, zareas) = overlay(g, &cxs, true);
    let mut locals_report = Vec::new();
    for f in &g.funcs {
        if f.fid == usize::MAX {
            continue;
        }
        let name = g.ctx.funcs[f.fid].name.clone();
        if let Some((cx, b)) = bases.get(&f.label) {
            if f.frame > 0 {
                locals_report.push(format!("{}：{} の領域の +{}〜+{}（{} バイト）", name, cx_name(*cx), b, b + f.frame - 1, f.frame));
            }
        }
        if let Some((cx, b)) = zbases.get(&f.label) {
            if f.zframe > 0 {
                locals_report.push(format!("{}：ゼロページの {} の領域の +{}（{} バイト、ポインタ）", name, cx_name(*cx), b, f.zframe));
            }
        }
    }
    // ---- ゼロページの配置 ----
    let mut known: HashMap<String, i64> = HashMap::new();
    let mut zp_free: Vec<bool> = vec![false; 256];
    for r in g.ctx.target.region("zp") {
        for a in r.lo..=r.hi.min(255) {
            zp_free[a as usize] = true;
        }
    }
    for r in g.ctx.target.reserved() {
        for a in r.lo..=r.hi {
            if a < 256 {
                zp_free[a as usize] = false;
            }
        }
    }
    let mut zp_report = Vec::new();
    let zp_alloc = |size: u32, free: &mut Vec<bool>| -> Option<u32> {
        if size == 0 {
            return Some(0);
        }
        let mut start = 0usize;
        while start + size as usize <= 256 {
            if (start..start + size as usize).all(|a| free[a]) {
                for a in start..start + size as usize {
                    free[a] = false;
                }
                return Some(start as u32);
            }
            start += 1;
        }
        None
    };
    let sp0 = Span::new(g.ctx.target.file, 1, 1, 1);
    let mut zarea_addr: HashMap<Cx, u32> = HashMap::new();
    for (cx, size) in &zareas {
        match zp_alloc(*size, &mut zp_free) {
            Some(a) => {
                zarea_addr.insert(*cx, a);
                if *size > 0 {
                    zp_report.push(format!("ポインタのローカル変数（{}）：${:02X}〜${:02X}（{} バイト）", cx_name(*cx), a, a + size - 1, size));
                }
            }
            None => {
                g.ctx.diags.err("E0303", &format!("ゼロページが足りません（ポインタのローカル変数 {} バイト）", size), sp0.clone());
                return None;
            }
        }
    }
    // static の置き場所
    let used: Vec<usize> = g.used_globals.iter().cloned().collect();
    let mut zp_statics: Vec<usize> = Vec::new();
    let mut ram_statics: Vec<usize> = Vec::new();
    for gid in &used {
        let gi = g.ctx.globals[*gid].clone();
        match gi.kind {
            GKind::Static | GKind::Shared(_) => {}
            _ => continue,
        }
        let is_ptr = matches!(gi.ty, Ty::Ptr(_) | Ty::Str(_, _));
        if is_ptr || find_attr(&gi.attrs, "zp").is_some() {
            zp_statics.push(*gid);
        } else {
            ram_statics.push(*gid);
        }
    }
    let mut var_report: Vec<(String, u32, u32, String)> = Vec::new();
    for gid in zp_statics {
        let gi = g.ctx.globals[gid].clone();
        let size = g.sz(&gi.ty).max(1);
        match zp_alloc(size, &mut zp_free) {
            Some(a) => {
                known.insert(gi.label.clone(), a as i64);
                var_report.push((gi.name.clone(), a, size, "ゼロページ（#[zp]・ポインタ）".to_string()));
            }
            None => {
                g.ctx.diags.err("E0303", &format!("ゼロページが足りません（{} の {} バイト）", gi.name, size), gi.span.clone());
                return None;
            }
        }
    }
    // -O1 以上：よく使う変数を自動でゼロページへ
    let mut ram_final: Vec<usize> = Vec::new();
    let mut locals_in_zp: HashSet<Cx> = HashSet::new();
    if opt != OptLevel::O0 {
        let mut uses: HashMap<String, usize> = HashMap::new();
        for f in &g.funcs {
            if !f.emitted {
                continue;
            }
            for it in &f.items {
                if let Item::Ins(ins) = it {
                    if let Some(e) = ins.expr() {
                        if let Some(s) = &e.sym {
                            if !matches!(ins.am, AMode::Imm(_)) {
                                *uses.entry(s.clone()).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }
        // ローカル変数の領域の使用回数
        let mut area_uses: HashMap<Cx, usize> = HashMap::new();
        for (l, (cx, _)) in &bases {
            let c = uses.get(&frame_sym(l)).copied().unwrap_or(0);
            *area_uses.entry(*cx).or_insert(0) += c;
        }
        let asm_local_cx: HashSet<Cx> = asm_ref_local_contexts(g, &cxs);
        let mut cands: Vec<(usize, String, u32, Option<usize>, Option<Cx>)> = Vec::new();
        for gid in &ram_statics {
            let gi = g.ctx.globals[*gid].clone();
            let size = g.sz(&gi.ty).max(1);
            if gi.asm_ref || find_attr(&gi.attrs, "fixed").is_some() {
                ram_final.push(*gid);
                continue;
            }
            let c = uses.get(&gi.label).copied().unwrap_or(0);
            cands.push((c, gi.label.clone(), size, Some(*gid), None));
        }
        for (cx, size) in &areas {
            if *size == 0 || asm_local_cx.contains(cx) {
                continue;
            }
            let c = area_uses.get(cx).copied().unwrap_or(0);
            cands.push((c, format!("LOCALS_{}", cx_key(*cx)), *size, None, Some(*cx)));
        }
        cands.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (c, label, size, gid, cx) in cands {
            if c == 0 {
                if let Some(gid) = gid {
                    ram_final.push(gid);
                }
                continue;
            }
            match zp_alloc(size, &mut zp_free) {
                Some(a) => {
                    known.insert(label.clone(), a as i64);
                    let name = match gid {
                        Some(gg) => g.ctx.globals[gg].name.clone(),
                        None => format!("ローカル変数の領域（{}）", cx.map(cx_name).unwrap_or("")),
                    };
                    zp_report.push(format!("{}（${:02X}〜${:02X}）：使用 {} 回、節約の見込み {} バイト・{} サイクル（自動の配置。{}）", name, a, a + size - 1, c, c, c, opt.name()));
                    if let Some(gg) = gid {
                        var_report.push((g.ctx.globals[gg].name.clone(), a, size, "ゼロページ（自動）".to_string()));
                    }
                    if let Some(cxx) = cx {
                        locals_in_zp.insert(cxx);
                    }
                }
                None => {
                    if let Some(gid) = gid {
                        ram_final.push(gid);
                    }
                }
            }
        }
    } else {
        ram_final = ram_statics.clone();
    }
    // ---- 記号の置き換え：フレーム → 領域 ----
    let mut frame_map: Vec<(String, String, i64, bool)> = Vec::new(); // (from, to, off, numeric)
    for (l, (cx, b)) in &bases {
        let area = format!("LOCALS_{}", cx_key(*cx));
        match known.get(&area) {
            Some(a) => frame_map.push((frame_sym(l), String::new(), *a + *b as i64, true)),
            None => frame_map.push((frame_sym(l), area, *b as i64, false)),
        }
    }
    for (l, (cx, b)) in &zbases {
        let a = zarea_addr.get(cx).copied().unwrap_or(0) as i64;
        frame_map.push((zframe_sym(l), String::new(), a + *b as i64, true));
    }
    let fm: HashMap<String, (String, i64, bool)> = frame_map.iter().map(|(f, t, o, n)| (f.clone(), (t.clone(), *o, *n))).collect();
    let rewrite = |it: &mut Item| {
        for_each_expr_mut(it, &mut |e: &mut OpExpr| {
            if let Some(s) = e.sym.clone() {
                if let Some((to, off, numeric)) = fm.get(&s) {
                    if *numeric {
                        e.sym = None;
                        e.off += *off;
                    } else {
                        e.sym = Some(to.clone());
                        e.off += *off;
                    }
                }
            }
        });
    };
    for f in g.funcs.iter_mut() {
        for it in f.items.iter_mut() {
            rewrite(it);
        }
    }
    for it in g.data.iter_mut() {
        rewrite(it);
    }
    // ゼロページの static は数に置き換える
    for f in g.funcs.iter_mut() {
        for it in f.items.iter_mut() {
            for (k, v) in &known {
                sym_to_num(it, k, *v);
            }
        }
    }
    // ---- 定数のデータ ----
    let mut data = g.data.clone();
    for gid in &used {
        let gi = g.ctx.globals[*gid].clone();
        if gi.kind != GKind::Const {
            continue;
        }
        if g.is_scalar_ty(&gi.ty) {
            continue;
        }
        if let Some(v) = &gi.value {
            let bytes = g.ctx.value_bytes(v, &gi.ty);
            data.push(Item::Label(gi.label.clone()));
            data.push(Item::Bytes(bytes, format!("const {}", gi.name)));
        }
    }
    for it in data.iter_mut() {
        for (k, v) in &known {
            sym_to_num(it, k, *v);
        }
    }
    // ---- 変数（RAM）----
    let mut bss = Vec::new();
    for gid in &ram_final {
        let gi = g.ctx.globals[*gid].clone();
        let size = g.sz(&gi.ty).max(1);
        let what = match gi.kind {
            GKind::Shared(_) => "shared",
            _ => "static",
        };
        bss.push(Item::Res(gi.label.clone(), size, format!("{} {}", what, gi.name)));
    }
    let mut area_list = Vec::new();
    for (cx, size) in &areas {
        let area = format!("LOCALS_{}", cx_key(*cx));
        if *size > 0 && !known.contains_key(&area) {
            bss.push(Item::Res(area.clone(), *size, format!("ローカル変数の重ね合わせ領域（{}）", cx_name(*cx))));
        }
        area_list.push((area, *size, 0u32));
    }
    let mut rareas: Vec<(String, u32)> = g.area_sizes.iter().map(|(k, v)| (k.clone(), *v)).collect();
    rareas.sort();
    for (l, s) in &rareas {
        bss.push(Item::Res(l.clone(), *s, "結果領域（15.3節）".to_string()));
    }
    // ---- アセンブル ----
    let mut code = Vec::new();
    let mut fn_order = Vec::new();
    for f in &g.funcs {
        if !f.emitted {
            continue;
        }
        if f.fid != usize::MAX {
            fn_order.push((f.label.clone(), g.ctx.funcs[f.fid].module));
        } else {
            fn_order.push((f.label.clone(), 0));
        }
        if !f.is_start {
            let title = if f.fid != usize::MAX {
                let fi = &g.ctx.funcs[f.fid];
                if fi.init_body.is_some() {
                    format!("; ==== {} ====", fi.name)
                } else {
                    format!("; ==== fn {}（{}）====", fi.name, g.ctx.modules[fi.module].name)
                }
            } else {
                format!("; ==== {} ====", f.label)
            };
            code.push(Item::Comment(title));
        }
        code.extend(f.items.iter().cloned());
    }
    let t = g.ctx.target.clone();
    let rom = t.run_rom();
    let org = if rom {
        let place = t.places.iter().find(|(w, _)| w == "code").map(|(_, r)| r.lo);
        place.unwrap_or_else(|| t.region("rom").first().map(|r| r.lo).unwrap_or(0x8000))
    } else {
        t.load.unwrap_or(0x0200)
    };
    let bss_org = if rom {
        let mut a = t.region("ram").first().map(|r| r.lo).unwrap_or(0x0200);
        while t.is_reserved(a) {
            a += 1;
        }
        Some(a)
    } else {
        None
    };
    let mut fixed = Vec::new();
    if rom {
        let irq = g.funcs.iter().find(|f| f.interrupt.as_deref() == Some("dispatch")).or_else(|| g.funcs.iter().find(|f| f.interrupt.as_deref() == Some("irq")));
        let nmi = g.funcs.iter().find(|f| f.interrupt.as_deref() == Some("nmi"));
        let irq_l = irq.map(|f| f.label.clone()).unwrap_or_else(|| "vec_rti".to_string());
        let nmi_l = nmi.map(|f| f.label.clone()).unwrap_or_else(|| "vec_rti".to_string());
        code.push(Item::Label("vec_rti".to_string()));
        code.push(Item::Ins(Ins::new("rti", AMode::Imp, "割り込み処理がないときの入口")));
        let ex = vec![
            OpExpr::sym(&nmi_l).lo(),
            OpExpr::sym(&nmi_l).hi(),
            OpExpr::sym("start").lo(),
            OpExpr::sym("start").hi(),
            OpExpr::sym(&irq_l).lo(),
            OpExpr::sym(&irq_l).hi(),
        ];
        fixed.push((0xFFFA, vec![Item::Label("VECTORS".to_string()), Item::Exprs(ex, "NMI・RESET・IRQ のベクタ".to_string())]));
    }
    let inp = AsmInput { code, data, bss, known: known.clone(), org, bss_org, nmos: g.opts.nmos, fixed };
    let asm = Assembler::new();
    let out = asm.assemble(&inp);
    for e in &out.errors {
        g.ctx.diags.err("E1204", &format!("アセンブルできません：{}", e), sp0.clone());
    }
    // ---- メモリの検査（16.3節）----
    check_memory(g, &out, rom);
    // ---- スタックの予算（15.5節）----
    let stack = stack_usage(g);
    let budget = g.ctx.target.stack.unwrap_or(256);
    let total = stack.main + stack.irq + stack.nmi;
    if total > budget {
        let mut d = Diag::error("E0301", &format!("スタック予算超過（必要 {} バイト / 予算 {} バイト）", total, budget), Span::none());
        d = d.note(&format!("メイン最深経路：{}（{} バイト）", stack.main_path.join(" → "), stack.main));
        if stack.irq > 0 {
            d = d.note(&format!("IRQ：{}（{} バイト）", stack.irq_path.join(" → "), stack.irq));
        }
        if stack.nmi > 0 {
            d = d.note(&format!("NMI：{}（{} バイト）", stack.nmi_path.join(" → "), stack.nmi));
        }
        g.ctx.diags.error(d);
    }
    // ---- 割り込み処理の時間（10.2節）----
    let irq_cycles = interrupt_cycles(g);
    let mut init_report = Vec::new();
    for f in &g.funcs {
        if f.fid != usize::MAX && g.ctx.funcs[f.fid].init_body.is_some() {
            let bytes = f.items.iter().filter(|x| matches!(x, Item::Ins(_))).count();
            init_report.push(format!("{}：{} 命令", g.ctx.funcs[f.fid].name, bytes));
        }
    }
    let _ = locals_in_zp;
    let mut areas_final = Vec::new();
    for (a, s, _) in area_list {
        let addr = known.get(&a).map(|x| *x as u32).or_else(|| out.symbols.get(&a).copied()).unwrap_or(0);
        areas_final.push((a, s, addr));
    }
    for gid in &ram_final {
        let gi = g.ctx.globals[*gid].clone();
        let size = g.sz(&gi.ty).max(1);
        let addr = out.symbols.get(&gi.label).copied().unwrap_or(0);
        var_report.push((gi.name.clone(), addr, size, "RAM".to_string()));
    }
    Some(Linked {
        out,
        inline_report,
        zp_report,
        locals_report,
        var_report,
        stack,
        irq_cycles,
        fn_order,
        areas: areas_final,
        init_report,
    })
}

/// 出力するすべての関数に、のぞき穴最適化をかける
fn peephole_all(g: &mut Gen, full: bool) {
    // 関数ごとの、参照している記号とラベル
    let refs_of = |items: &[Item]| -> HashSet<String> {
        let mut s = HashSet::new();
        for it in items {
            match it {
                Item::Ins(ins) => {
                    if let AMode::Rel(l) = &ins.am {
                        s.insert(l.clone());
                    }
                    if let Some(e) = ins.expr() {
                        if let Some(x) = &e.sym {
                            s.insert(x.clone());
                        }
                    }
                }
                Item::Exprs(v, _) => {
                    for e in v {
                        if let Some(x) = &e.sym {
                            s.insert(x.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        s
    };
    let data_refs = refs_of(&g.data);
    let per_fn: Vec<HashSet<String>> = g.funcs.iter().map(|f| if f.emitted { refs_of(&f.items) } else { HashSet::new() }).collect();
    let mut called: HashSet<String> = data_refs.clone();
    for s in &per_fn {
        called.extend(s.iter().cloned());
    }
    let mut volatile: HashSet<String> = HashSet::new();
    for gi in &g.ctx.globals {
        if matches!(gi.kind, GKind::Shared(_)) || gi.asm_ref {
            volatile.insert(gi.label.clone());
        }
    }
    let mut uniq = 0usize;
    for idx in 0..g.funcs.len() {
        if !g.funcs[idx].emitted {
            continue;
        }
        let mut ext: HashSet<String> = data_refs.clone();
        for (j, s) in per_fn.iter().enumerate() {
            if j != idx {
                ext.extend(s.iter().cloned());
            }
        }
        let host = g.funcs[idx].label.clone();
        let fid = g.funcs[idx].fid;
        let ret_c = fid != usize::MAX && matches!(g.ctx.funcs[fid].ret, Ty::Bool | Ty::Opt(_));
        let ret_regs = fid != usize::MAX && !matches!(g.ctx.funcs[fid].ret, Ty::Unit | Ty::Never | Ty::Bool);
        let cx = crate::peep::PeepCx { ext_refs: &ext, volatile: &volatile, called: &called, host: &host, ret_c, ret_regs };
        let mut items = std::mem::take(&mut g.funcs[idx].items);
        if full {
            crate::peep::peephole(&mut items, &cx, &mut uniq);
        } else {
            crate::peep::dead_frame_stores(&mut items, &cx);
        }
        g.funcs[idx].items = items;
    }
    if !full {
        return;
    }
    // 整理した後にできた jsr と rts の並びを、末尾呼び出しにする
    for f in g.funcs.iter_mut() {
        if f.interrupt.is_none() && f.emitted {
            tail_calls(&mut f.items);
            drop_jump_to_next(&mut f.items);
        }
    }
}

pub fn cx_name(c: Cx) -> &'static str {
    c.name()
}

fn cx_key(c: Cx) -> &'static str {
    match c {
        Cx::Main => "main",
        Cx::Irq => "irq",
        Cx::Nmi => "nmi",
    }
}

fn post_order(g: &Gen) -> Vec<usize> {
    let label_idx: HashMap<String, usize> = g.funcs.iter().enumerate().map(|(i, f)| (f.label.clone(), i)).collect();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); g.funcs.len()];
    for (i, f) in g.funcs.iter().enumerate() {
        for it in &f.items {
            if let Item::Ins(ins) = it {
                if let Some(l) = target_label(ins) {
                    if let Some(&j) = label_idx.get(&l) {
                        if j != i {
                            adj[i].push(j);
                        }
                    }
                }
            }
        }
    }
    let mut state = vec![0u8; g.funcs.len()];
    let mut out = Vec::new();
    fn dfs(v: usize, adj: &Vec<Vec<usize>>, state: &mut Vec<u8>, out: &mut Vec<usize>) {
        if state[v] != 0 {
            return;
        }
        state[v] = 1;
        for w in &adj[v] {
            dfs(*w, adj, state, out);
        }
        state[v] = 2;
        out.push(v);
    }
    for i in 0..g.funcs.len() {
        dfs(i, &adj, &mut state, &mut out);
    }
    out
}

/// 呼ばれる関数の命令を、呼ぶ場所に入れる
fn splice(out: &mut Vec<Item>, callee: &[Item], inst: usize, tail: bool) {
    let mut labels: HashSet<String> = HashSet::new();
    for it in callee {
        if let Item::Label(l) = it {
            labels.insert(l.clone());
        }
    }
    let ren = |s: &str| -> String { format!("{}~{}", s, inst) };
    let end = format!("inl~{}", inst);
    for it in callee {
        let mut x = it.clone();
        match &mut x {
            Item::Label(l) => {
                *l = ren(l.as_str());
            }
            Item::Ins(ins) => {
                if ins.mn == "rts" && !tail {
                    *ins = Ins::new("bra", AMode::Rel(end.clone()), "展開した関数の終わりへ");
                } else {
                    if let AMode::Rel(l) = &mut ins.am {
                        if labels.contains(l.as_str()) {
                            *l = ren(l.as_str());
                        }
                    }
                }
            }
            _ => {}
        }
        for_each_expr_mut(&mut x, &mut |e: &mut OpExpr| {
            if let Some(s) = e.sym.clone() {
                if labels.contains(&s) {
                    e.sym = Some(ren(s.as_str()));
                }
            }
        });
        out.push(x);
    }
    if !tail {
        out.push(Item::Label(end));
    }
}

fn next_ins(items: &[Item], from: usize) -> Option<usize> {
    let mut k = from;
    while k < items.len() {
        match &items[k] {
            Item::Ins(_) => return Some(k),
            Item::Src(_) | Item::Comment(_) => k += 1,
            _ => return None,
        }
    }
    None
}

fn tail_calls(items: &mut Vec<Item>) {
    let mut i = 0;
    while i < items.len() {
        let is_jsr = match &items[i] {
            Item::Ins(ins) => ins.mn == "jsr" && matches!(ins.am, AMode::Mem(_)),
            _ => false,
        };
        if is_jsr {
            if let Some(k) = next_ins(items, i + 1) {
                let is_rts = matches!(&items[k], Item::Ins(r) if r.mn == "rts");
                if is_rts {
                    if let Item::Ins(ins) = &mut items[i] {
                        ins.mn = "jmp".to_string();
                        ins.comment = format!("{}（末尾呼び出しなので jmp）", ins.comment);
                    }
                    items.remove(k);
                }
            }
        }
        i += 1;
    }
}

fn drop_jump_to_next(items: &mut Vec<Item>) {
    // 消すと、その前の分岐がすぐ次への分岐になることがあるので、変わらなくなるまで繰り返す
    loop {
        let before = items.len();
        drop_jump_to_next_once(items);
        if items.len() == before {
            break;
        }
    }
}

fn drop_jump_to_next_once(items: &mut Vec<Item>) {
    let mut i = 0;
    while i < items.len() {
        let target = match &items[i] {
            Item::Ins(ins) if ins.mn == "bra" => match &ins.am {
                AMode::Rel(l) => Some(l.clone()),
                _ => None,
            },
            _ => None,
        };
        if let Some(t) = target {
            // 次の命令までにそのラベルがあるか
            let mut k = i + 1;
            let mut hit = false;
            while k < items.len() {
                match &items[k] {
                    Item::Label(l) => {
                        if *l == t {
                            hit = true;
                            break;
                        }
                        k += 1;
                    }
                    Item::Src(_) | Item::Comment(_) => k += 1,
                    _ => break,
                }
            }
            if hit {
                items.remove(i);
                continue;
            }
        }
        i += 1;
    }
}

fn clobbers(g: &Gen) -> HashMap<String, Vec<char>> {
    let label_idx: HashMap<String, usize> = g.funcs.iter().enumerate().map(|(i, f)| (f.label.clone(), i)).collect();
    let mut memo: HashMap<usize, HashSet<char>> = HashMap::new();
    fn go(i: usize, g: &Gen, idx: &HashMap<String, usize>, memo: &mut HashMap<usize, HashSet<char>>, busy: &mut HashSet<usize>) -> HashSet<char> {
        if let Some(s) = memo.get(&i) {
            return s.clone();
        }
        if busy.contains(&i) {
            return ['a', 'x', 'y'].iter().cloned().collect();
        }
        busy.insert(i);
        let mut s: HashSet<char> = HashSet::new();
        for it in &g.funcs[i].items {
            if let Item::Ins(ins) = it {
                for r in writes_reg(ins) {
                    s.insert(r);
                }
                if ins.mn == "jsr" || ins.mn == "jmp" {
                    match target_label(ins).and_then(|l| idx.get(&l).copied()) {
                        Some(j) => {
                            for r in go(j, g, idx, memo, busy) {
                                s.insert(r);
                            }
                        }
                        None => {
                            if ins.mn == "jsr" || matches!(ins.am, AMode::IndAbsX(_)) {
                                s.insert('a');
                                s.insert('x');
                                s.insert('y');
                            }
                        }
                    }
                }
                if matches!(ins.am, AMode::IndAbsX(_)) {
                    for c in &g.funcs[i].calls {
                        let l = g.ctx.funcs[*c].label.clone();
                        if let Some(j) = idx.get(&l).copied() {
                            for r in go(j, g, idx, memo, busy) {
                                s.insert(r);
                            }
                        }
                    }
                }
            }
        }
        busy.remove(&i);
        memo.insert(i, s.clone());
        s
    }
    let mut out = HashMap::new();
    let mut busy = HashSet::new();
    for (i, f) in g.funcs.iter().enumerate() {
        let s = go(i, g, &label_idx, &mut memo, &mut busy);
        let mut v: Vec<char> = s.into_iter().collect();
        v.sort();
        out.insert(f.label.clone(), v);
    }
    out
}

/// 文脈ごとのフレームの位置（呼び出し元のフレームの後ろ）
fn overlay(g: &Gen, cxs: &HashMap<usize, HashSet<Cx>>, zp: bool) -> (HashMap<String, (Cx, u32)>, Vec<(Cx, u32)>) {
    let fid_idx: HashMap<usize, usize> = g.funcs.iter().enumerate().filter(|(_, f)| f.fid != usize::MAX).map(|(i, f)| (f.fid, i)).collect();
    let size = |i: usize| -> u32 {
        if zp {
            g.funcs[i].zframe
        } else {
            g.funcs[i].frame
        }
    };
    let mut bases: HashMap<String, (Cx, u32)> = HashMap::new();
    let mut areas: Vec<(Cx, u32)> = Vec::new();
    for cx in [Cx::Main, Cx::Irq, Cx::Nmi] {
        // この文脈の関数
        let members: Vec<usize> = g
            .funcs
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                f.fid != usize::MAX
                    && match cxs.get(&f.fid) {
                        Some(s) => s.contains(&cx),
                        // 呼び出しグラフに文脈がない関数はメインとして扱う
                        None => cx == Cx::Main,
                    }
            })
            .map(|(i, _)| i)
            .collect();
        let mset: HashSet<usize> = members.iter().cloned().collect();
        // 呼び出し元の一覧
        let mut callers: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in &members {
            for c in &g.funcs[*i].calls {
                if let Some(&j) = fid_idx.get(c) {
                    if mset.contains(&j) && j != *i {
                        callers.entry(j).or_default().push(*i);
                    }
                }
            }
        }
        let mut base: HashMap<usize, u32> = HashMap::new();
        fn calc(i: usize, callers: &HashMap<usize, Vec<usize>>, base: &mut HashMap<usize, u32>, size: &dyn Fn(usize) -> u32, depth: u32) -> u32 {
            if let Some(b) = base.get(&i) {
                return *b;
            }
            if depth > 200 {
                return 0;
            }
            let mut b = 0;
            if let Some(cs) = callers.get(&i) {
                for c in cs {
                    let cb = calc(*c, callers, base, size, depth + 1) + size(*c);
                    if cb > b {
                        b = cb;
                    }
                }
            }
            base.insert(i, b);
            b
        }
        let mut total = 0;
        for i in &members {
            let b = calc(*i, &callers, &mut base, &size, 0);
            if b + size(*i) > total {
                total = b + size(*i);
            }
            bases.insert(g.funcs[*i].label.clone(), (cx, b));
        }
        areas.push((cx, total));
    }
    (bases, areas)
}

fn asm_ref_local_contexts(g: &Gen, cxs: &HashMap<usize, HashSet<Cx>>) -> HashSet<Cx> {
    let mut s = HashSet::new();
    for (fid, f) in g.ctx.funcs.iter().enumerate() {
        if f.locals.iter().any(|l| l.asm_ref) {
            if let Some(set) = cxs.get(&fid) {
                for c in set {
                    s.insert(*c);
                }
            }
        }
    }
    s
}

fn check_memory(g: &mut Gen, out: &AsmOutput, rom: bool) {
    let t = g.ctx.target.clone();
    let sp = Span::new(t.file, 1, 1, 1);
    let rams = t.region("ram");
    let in_one = |lo: u32, hi: u32, regs: &[crate::project::RangeV]| regs.iter().any(|r| lo >= r.lo && hi <= r.hi);
    if rom {
        let roms = t.region("rom");
        if out.data_end > out.code_start && !in_one(out.code_start, out.data_end - 1, roms.as_slice()) {
            g.ctx.diags.err("E0302", &format!("コードと定数（${:04X}〜${:04X}）が rom の区画に収まりません", out.code_start, out.data_end - 1), sp.clone());
        }
    } else if out.data_end > out.code_start && !in_one(out.code_start, out.data_end - 1, rams.as_slice()) {
        g.ctx.diags.err("E0302", &format!("プログラム（${:04X}〜${:04X}）が ram の区画に収まりません", out.code_start, out.data_end - 1), sp.clone());
    }
    if out.bss_end > out.bss_start && !in_one(out.bss_start, out.bss_end - 1, rams.as_slice()) {
        g.ctx.diags.err("E0302", &format!("変数（${:04X}〜${:04X}）が ram の区画に収まりません（RAM の容量の超過）", out.bss_start, out.bss_end - 1), sp.clone());
    }
    for r in t.reserved() {
        let overlap = |lo: u32, hi: u32| hi > lo && !(hi - 1 < r.lo || lo > r.hi);
        if overlap(out.code_start, out.data_end) || overlap(out.bss_start, out.bss_end) {
            g.ctx.diags.err("E0304", &format!("プログラムか変数が reserve の範囲（${:04X}〜${:04X}）に重なっています", r.lo, r.hi), sp.clone());
        }
    }
}

fn stack_usage(g: &Gen) -> StackInfo {
    let label_idx: HashMap<String, usize> = g.funcs.iter().enumerate().map(|(i, f)| (f.label.clone(), i)).collect();
    let mut memo: HashMap<usize, (u32, Vec<String>)> = HashMap::new();
    fn go(i: usize, g: &Gen, idx: &HashMap<String, usize>, memo: &mut HashMap<usize, (u32, Vec<String>)>, depth: u32) -> (u32, Vec<String>) {
        if let Some(r) = memo.get(&i) {
            return r.clone();
        }
        if depth > 100 {
            return (0, Vec::new());
        }
        let f = &g.funcs[i];
        let name = if f.fid != usize::MAX { g.ctx.funcs[f.fid].name.clone() } else { f.label.clone() };
        let mut d: i32 = 0;
        let mut best: u32 = 0;
        let mut best_path: Vec<String> = Vec::new();
        for it in &f.items {
            if let Item::Ins(ins) = it {
                match ins.mn.as_str() {
                    "pha" | "php" | "phx" | "phy" => {
                        d += 1;
                        if d as u32 > best {
                            best = d as u32;
                        }
                    }
                    "pla" | "plp" | "plx" | "ply" => {
                        if d > 0 {
                            d -= 1;
                        }
                    }
                    "jsr" | "jmp" => {
                        let extra = if ins.mn == "jsr" { 2 } else { 0 };
                        match target_label(ins).and_then(|l| idx.get(&l).copied()) {
                            Some(j) => {
                                let (u, p) = go(j, g, idx, memo, depth + 1);
                                let tot = d.max(0) as u32 + extra + u;
                                if tot > best {
                                    best = tot;
                                    best_path = p;
                                }
                            }
                            None => {
                                if ins.mn == "jsr" {
                                    let tot = d.max(0) as u32 + f.ext_stack.max(2);
                                    if tot > best {
                                        best = tot;
                                        best_path = vec![ins.operand_text()];
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
                if matches!(ins.am, AMode::IndAbsX(_)) {
                    for c in &f.calls {
                        let l = g.ctx.funcs[*c].label.clone();
                        if let Some(j) = idx.get(&l).copied() {
                            let (u, p) = go(j, g, idx, memo, depth + 1);
                            let tot = d.max(0) as u32 + u;
                            if tot > best {
                                best = tot;
                                best_path = p;
                            }
                        }
                    }
                }
            }
        }
        let mut path = vec![name];
        path.extend(best_path);
        memo.insert(i, (best, path.clone()));
        (best, path)
    }
    let mut info = StackInfo::default();
    if let Some(&s) = label_idx.get("start") {
        let (u, p) = go(s, g, &label_idx, &mut memo, 0);
        info.main = u;
        info.main_path = p;
    }
    let irq = g.funcs.iter().position(|f| f.interrupt.as_deref() == Some("dispatch")).or_else(|| g.funcs.iter().position(|f| f.interrupt.as_deref() == Some("irq")));
    if let Some(i) = irq {
        let (u, p) = go(i, g, &label_idx, &mut memo, 0);
        info.irq = 3 + u;
        info.irq_path = p;
    }
    if let Some(i) = g.funcs.iter().position(|f| f.interrupt.as_deref() == Some("nmi")) {
        let (u, p) = go(i, g, &label_idx, &mut memo, 0);
        info.nmi = 3 + u;
        info.nmi_path = p;
    }
    info
}

fn interrupt_cycles(g: &mut Gen) -> Vec<String> {
    let asm = Assembler::new();
    let known: HashMap<String, i64> = HashMap::new();
    let mut out = Vec::new();
    let label_idx: HashMap<String, usize> = g.funcs.iter().enumerate().map(|(i, f)| (f.label.clone(), i)).collect();
    for (i, f) in g.funcs.iter().enumerate() {
        let kind = match f.interrupt.as_deref() {
            Some("irq") | Some("brk") | Some("nmi") => f.interrupt.clone().unwrap_or_default(),
            _ => continue,
        };
        // 後ろ向きの分岐（ループ）がなければ、全命令の合計が最長の上限
        let mut seen_labels: HashSet<String> = HashSet::new();
        let mut loops = false;
        let mut total: u32 = 7;
        let mut stack = vec![i];
        let mut visited = HashSet::new();
        while let Some(k) = stack.pop() {
            if !visited.insert(k) {
                continue;
            }
            seen_labels.clear();
            for it in &g.funcs[k].items {
                match it {
                    Item::Label(l) => {
                        seen_labels.insert(l.clone());
                    }
                    Item::Ins(ins) => {
                        if let AMode::Rel(l) = &ins.am {
                            if seen_labels.contains(l) {
                                loops = true;
                            }
                        }
                        let (c, page) = asm.ins_cycles(ins, &known);
                        let mut c = c + if page { 1 } else { 0 };
                        if crate::asm6502::is_branch(&ins.mn) {
                            c += 2;
                        }
                        total += c;
                        if let Some(l) = target_label(ins) {
                            if let Some(&j) = label_idx.get(&l) {
                                stack.push(j);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let name = if f.fid != usize::MAX { g.ctx.funcs[f.fid].name.clone() } else { f.label.clone() };
        if loops {
            out.push(format!("{}（{}）：ループを含むため、最長サイクル数は計算していません", name, kind));
        } else {
            out.push(format!("{}（{}）：最長 {} サイクル以下（割り込みの受け付け 7 サイクルを含む上限）", name, kind, total));
            if kind != "nmi" {
                if let Some(iv) = g.ctx.target.irq_interval {
                    if total as u64 > iv {
                        let sp = if f.fid != usize::MAX { g.ctx.funcs[f.fid].span.clone() } else { Span::none() };
                        g.ctx.diags.err("E0807", &format!("割り込み処理の最長 {} サイクルが irq_interval（{}）を超えています", total, iv), sp);
                    }
                }
            }
        }
    }
    out
}

pub fn code_sizes(l: &Linked) -> BTreeMap<String, u32> {
    let mut v: Vec<(u32, String)> = Vec::new();
    for (name, _) in &l.fn_order {
        if let Some(a) = l.out.symbols.get(name) {
            v.push((*a, name.clone()));
        }
    }
    v.sort();
    let mut m = BTreeMap::new();
    for k in 0..v.len() {
        let end = if k + 1 < v.len() { v[k + 1].0 } else { l.out.code_end };
        m.insert(v[k].1.clone(), end.saturating_sub(v[k].0));
    }
    m
}
