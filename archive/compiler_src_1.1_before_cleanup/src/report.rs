// レポート（18.4節）
use crate::codegen::Gen;
use crate::opt::{code_sizes, Linked};
use crate::program::GKind;
use std::collections::BTreeMap;

pub fn build_report(g: &Gen, l: &Linked, title: &str) -> String {
    let mut s = String::new();
    let t = &g.ctx.target;
    let out = &l.out;
    s.push_str(&format!("AX65 レポート — {}\n", title));
    s.push_str(&format!(
        "ターゲット：{}（{}、{} Hz）　最適化：{}　実行：{}\n\n",
        t.name,
        t.cpu.clone().unwrap_or_default(),
        t.clock.unwrap_or(0),
        g.ctx.opt.name(),
        if t.run_rom() { "ROM" } else { "RAM" }
    ));

    // ---- メモリ ----
    s.push_str("== メモリ ==\n");
    let code = out.code_end.saturating_sub(out.code_start);
    let data = out.data_end.saturating_sub(out.code_end);
    let vars = out.bss_end.saturating_sub(out.bss_start);
    s.push_str(&format!("  コード：${:04X}〜${:04X}（{} バイト）\n", out.code_start, out.code_end.saturating_sub(1).max(out.code_start), code));
    s.push_str(&format!("  定数・初期値の元データ：{} バイト（${:04X}〜）\n", data, out.code_end));
    s.push_str(&format!("  変数（RAM）：{} バイト（${:04X}〜${:04X}）\n", vars, out.bss_start, out.bss_end.saturating_sub(1).max(out.bss_start)));
    let zp_used: u32 = l.var_report.iter().filter(|v| v.3.starts_with("ゼロページ")).map(|v| v.2).sum::<u32>();
    s.push_str(&format!("  ゼロページ：変数 {} バイト（ほかにポインタの領域）\n", zp_used));
    for (kind, ranges) in &t.regions {
        for r in ranges {
            let mut used = 0u32;
            for a in r.lo..=r.hi {
                if out.image.contains_key(&a) || (a >= out.bss_start && a < out.bss_end) {
                    used += 1;
                }
            }
            if kind == "ram" || kind == "rom" {
                s.push_str(&format!("  区画 {} ${:04X}〜${:04X}：使用 {} / {} バイト\n", kind, r.lo, r.hi, used, r.size()));
            }
        }
    }
    // モジュールごと
    let sizes = code_sizes(l);
    let mut per_mod: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for (label, m) in &l.fn_order {
        let mname = if label == "start" { "（起動処理）".to_string() } else { g.ctx.modules[*m].name.clone() };
        let e = per_mod.entry(mname).or_insert((0, 0));
        e.0 += sizes.get(label).copied().unwrap_or(0);
    }
    for gid in &g.used_globals {
        let gi = &g.ctx.globals[*gid];
        if let GKind::Static | GKind::Shared(_) = gi.kind {
            let e = per_mod.entry(g.ctx.modules[gi.module].name.clone()).or_insert((0, 0));
            e.1 += g.ctx.types.size_of(&gi.ty);
        }
    }
    s.push_str("  モジュールごと：\n");
    for (m, (c, v)) in &per_mod {
        s.push_str(&format!("    {}：コード {} バイト、変数 {} バイト\n", m, c, v));
    }
    s.push_str("  関数ごとのコード：\n");
    for (label, _) in &l.fn_order {
        s.push_str(&format!("    {}：{} バイト（${:04X}）\n", label, sizes.get(label).copied().unwrap_or(0), out.symbols.get(label).copied().unwrap_or(0)));
    }
    s.push('\n');

    // ---- 変数の配置 ----
    s.push_str("== 変数の配置 ==\n");
    let mut vr = l.var_report.clone();
    vr.sort_by(|a, b| a.1.cmp(&b.1));
    for (name, addr, size, place) in &vr {
        let end = addr + size.saturating_sub(1);
        if *size > 1 {
            s.push_str(&format!("  {:16} ${:04X}〜${:04X}（{} バイト）{}\n", name, addr, end, size, place));
        } else {
            s.push_str(&format!("  {:16} ${:04X}（1 バイト）{}\n", name, addr, place));
        }
    }
    for (a, size, addr) in &l.areas {
        if *size > 0 {
            s.push_str(&format!("  {:16} ${:04X}（{} バイト）ローカル変数の重ね合わせ領域\n", a, addr, size));
        }
    }
    s.push('\n');
    s.push_str("== ゼロページ（自動の配置） ==\n");
    if l.zp_report.is_empty() {
        s.push_str("  なし\n");
    }
    for z in &l.zp_report {
        s.push_str(&format!("  {}\n", z));
    }
    s.push('\n');

    // ---- ローカル変数 ----
    s.push_str("== ローカル変数（重ね合わせ。4.8節） ==\n");
    for x in &l.locals_report {
        s.push_str(&format!("  {}\n", x));
    }
    let total: u32 = l.areas.iter().map(|a| a.1).sum();
    s.push_str(&format!("  合計：{} バイト\n\n", total));

    // ---- スタック ----
    let st = &l.stack;
    s.push_str("== スタック ==\n");
    s.push_str(&format!("  最大 {} バイト（予算 {}）\n", st.main + st.irq + st.nmi, t.stack.unwrap_or(0)));
    s.push_str(&format!("  メイン：{}（{}）\n", st.main_path.join(" → "), st.main));
    if st.irq > 0 {
        s.push_str(&format!("  IRQ：{}（{}、CPU が積む 3 バイトを含む）\n", st.irq_path.join(" → "), st.irq));
    }
    if st.nmi > 0 {
        s.push_str(&format!("  NMI：{}（{}）\n", st.nmi_path.join(" → "), st.nmi));
    }
    s.push('\n');

    // ---- 割り込み ----
    s.push_str("== 割り込み ==\n");
    if l.irq_cycles.is_empty() {
        s.push_str("  割り込み処理なし\n");
    }
    for x in &l.irq_cycles {
        s.push_str(&format!("  {}\n", x));
    }
    s.push_str("  最も長い atomic：未計算（未実装）\n\n");

    // ---- 起動 ----
    s.push_str("== 起動（15.1節） ==\n");
    if l.init_report.is_empty() {
        s.push_str("  init なし\n");
    }
    for (i, x) in l.init_report.iter().enumerate() {
        s.push_str(&format!("  {}. {}\n", i + 1, x));
    }
    s.push('\n');

    // ---- 展開 ----
    s.push_str("== 展開（7.7節・19章） ==\n");
    if l.inline_report.is_empty() {
        s.push_str("  なし\n");
    }
    for x in &l.inline_report {
        s.push_str(&format!("  {}\n", x));
    }
    s.push('\n');

    // ---- 呼び出し ----
    s.push_str("== 呼び出し（15.2節・15.3節） ==\n");
    for (fid, f) in g.ctx.funcs.iter().enumerate() {
        if !g.generated.contains(&fid) || f.init_body.is_some() {
            continue;
        }
        let layout = g.param_layout(fid);
        let mut parts = Vec::new();
        for (i, p) in layout.iter().enumerate() {
            let name = f.params.get(i).map(|x| x.0.clone()).unwrap_or_default();
            if p.regs.is_empty() {
                parts.push(format!("{}：引数領域", name));
            } else {
                let r: Vec<String> = p.regs.iter().map(|c| c.to_ascii_uppercase().to_string()).collect();
                parts.push(format!("{}：{}", name, r.join("")));
            }
        }
        let ret = match &f.ret {
            crate::types::Ty::Unit => "なし".to_string(),
            crate::types::Ty::Bool => "キャリー".to_string(),
            crate::types::Ty::Opt(_) => "有無はキャリー、値はレジスタ".to_string(),
            t => {
                let n = g.ctx.types.size_of(t);
                match n {
                    1 => "A".to_string(),
                    2 => "A・X".to_string(),
                    3 => "A・X・Y".to_string(),
                    _ => "結果領域".to_string(),
                }
            }
        };
        s.push_str(&format!("  {}（{}）→ {}\n", f.name, if parts.is_empty() { "引数なし".to_string() } else { parts.join("、") }, ret));
    }
    s.push_str("  構造体の戻り値：結果領域のまま返す（最初のフィールドのレジスタ返しは未実装）\n\n");

    // ---- コードの選択 ----
    s.push_str("== コードの選択 ==\n");
    s.push_str("  Option：戻り値は有無をキャリー、変数は値＋有無の 1 バイト（使われない値の流用は未実装）\n");
    for x in &g.notes {
        s.push_str(&format!("  {}\n", x));
    }
    s.push('\n');

    // ---- アセンブリ ----
    s.push_str("== アセンブリ ==\n");
    if g.asm_notes.is_empty() {
        s.push_str("  なし\n");
    }
    for x in &g.asm_notes {
        s.push_str(&format!("  {}\n", x));
    }
    s.push('\n');

    // ---- unsafe ----
    s.push_str("== unsafe（1.2節の保証の対象外） ==\n");
    if g.ctx.unsafe_list.is_empty() {
        s.push_str("  なし\n");
    }
    let mut seen = std::collections::HashSet::new();
    for x in &g.ctx.unsafe_list {
        if seen.insert(x.clone()) {
            s.push_str(&format!("  {}\n", x));
        }
    }
    s.push('\n');

    // ---- 最適化 ----
    s.push_str("== 最適化 ==\n");
    s.push_str(&format!("  段階：{}\n", g.ctx.opt.name()));
    s.push_str("  どの段階でも行うこと：末尾呼び出しの jmp、読み捨ての I/O の bit、初期値 0 の stz\n");
    match g.ctx.opt {
        crate::program::OptLevel::O0 => s.push_str("  -O0：展開は #[inline] と init だけ。ゼロページの自動の配置はしない\n"),
        _ => {
            s.push_str("  行ったこと：ゼロページへの自動の配置、1 か所からしか呼ばれない関数の展開（-O1・-Os は小さな関数も）\n");
            s.push_str("  未実装：引数の渡し方の変更、不要な書き込みの削除、共通部分のくくり出し、ループの自動の逆順化、ループの配置、構造体の戻り値のレジスタ返し\n");
        }
    }
    s
}
