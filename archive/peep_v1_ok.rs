// のぞき穴最適化（-O1 以上。展開の後、記号を番地に置き換える前の命令の並びに行う）
//
//   1. 分岐の整理：分岐先の分岐をたどる、すぐ次への分岐を消す、分岐を飛び越す分岐を反転する、
//      到達しない命令を消す、参照されないラベルを消す
//   2. レジスタの中身の追跡：同じ値の読み直しを消す、別のレジスタにある値は転送命令にする、
//      同じ値の書き戻しを消す、分かっているキャリーの分岐を決める、要らない clc・sec・cmp #0 を消す
//   3. ローカル変数（フレーム F_・Z_）への、後で読まれない書き込みを消す
//
// 次のものは変えない：アセンブリのブロックと asm fn の本体、cycles()・max_cycles() の中（Item::Mark の
// "KEEP{" 〜 "}KEEP" で囲まれる）、I/O（記号のない番地）と shared 変数の読み書きの回数と順序。
use crate::asm6502::{invert_branch, is_branch, AMode, Ins, Item, OpExpr, Part};
use std::collections::{HashMap, HashSet};

pub const KEEP_BEGIN: &str = "KEEP{";
pub const KEEP_END: &str = "}KEEP";

/// のぞき穴最適化に渡す情報
pub struct PeepCx<'a> {
    /// この関数の外（ほかの関数・データ）から参照されるラベル
    pub ext_refs: &'a HashSet<String>,
    /// 読み直しを省けない記号（shared 変数など）
    pub volatile: &'a HashSet<String>,
    /// どこかから jsr・jmp・アドレスで参照される関数のラベル（そのフレームへの書き込みは消さない）
    pub called: &'a HashSet<String>,
    /// この関数のラベル
    pub host: &'a str,
    /// この関数が値をキャリーで返すか（bool・Option）。返さなければ rts の時点でキャリーは要らない
    pub ret_c: bool,
}

#[derive(Clone, PartialEq, Debug)]
enum Val {
    Imm(OpExpr),
    Mem(OpExpr),
}

#[derive(Clone, Default)]
struct St {
    a: Option<Val>,
    x: Option<Val>,
    y: Option<Val>,
    /// N と Z が、どのレジスタの値を表しているか
    nz: Option<char>,
    /// キャリーが分かっていればその値
    c: Option<bool>,
}

impl St {
    fn reg(&self, r: char) -> &Option<Val> {
        match r {
            'a' => &self.a,
            'x' => &self.x,
            _ => &self.y,
        }
    }
    fn set(&mut self, r: char, v: Option<Val>) {
        match r {
            'a' => self.a = v,
            'x' => self.x = v,
            _ => self.y = v,
        }
    }
    /// 番地 e への書き込み：e の中身を持っているレジスタを忘れる
    fn forget_mem(&mut self, e: &OpExpr) {
        for r in ['a', 'x', 'y'] {
            if let Some(Val::Mem(m)) = self.reg(r) {
                if m == e {
                    self.set(r, None);
                }
            }
        }
    }
    /// 添字付きの書き込み：同じ記号の中身を持っているレジスタを忘れる（記号がなければすべて）
    fn forget_sym(&mut self, e: &OpExpr) {
        for r in ['a', 'x', 'y'] {
            if let Some(Val::Mem(m)) = self.reg(r) {
                if e.sym.is_none() || m.sym == e.sym {
                    self.set(r, None);
                }
            }
        }
    }
    fn forget_all_mem(&mut self) {
        for r in ['a', 'x', 'y'] {
            if let Some(Val::Mem(_)) = self.reg(r) {
                self.set(r, None);
            }
        }
    }
}

/// のぞき穴最適化の本体。変えたら true
pub fn peephole(items: &mut Vec<Item>, cx: &PeepCx, uniq: &mut usize) -> bool {
    let mut any = false;
    for _round in 0..64 {
        let mut changed = false;
        changed |= thread_jumps(items);
        changed |= drop_jumps_to_next(items);
        changed |= branch_over_branch(items);
        changed |= remove_unreachable(items);
        changed |= remove_unused_labels(items, cx);
        changed |= track(items, cx, uniq);
        changed |= dead_stores(items, cx);
        changed |= fold_commutative(items, cx);
        if !changed {
            break;
        }
        any = true;
    }
    any
}

// ---- 共通の道具 ----------------------------------------------------------

fn keep_mask(items: &[Item]) -> Vec<bool> {
    let mut v = vec![false; items.len()];
    let mut depth = 0;
    for (i, it) in items.iter().enumerate() {
        if let Item::Mark(m) = it {
            if m == KEEP_BEGIN {
                depth += 1;
            } else if m == KEEP_END && depth > 0 {
                depth -= 1;
                v[i] = true;
                continue;
            }
        }
        v[i] = depth > 0;
    }
    v
}

fn label_pos(items: &[Item]) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for (i, it) in items.iter().enumerate() {
        if let Item::Label(l) = it {
            m.insert(l.clone(), i);
        }
    }
    m
}

/// 分岐・jmp の飛び先のラベル（jmp は記号だけの番地のとき）
fn jump_target(ins: &Ins) -> Option<String> {
    match &ins.am {
        AMode::Rel(l) => Some(l.clone()),
        AMode::Mem(e) if ins.mn == "jmp" && e.off == 0 && e.part == Part::Full => e.sym.clone(),
        _ => None,
    }
}

fn set_jump_target(ins: &mut Ins, l: &str) {
    match &mut ins.am {
        AMode::Rel(x) => *x = l.to_string(),
        AMode::Mem(e) => e.sym = Some(l.to_string()),
        _ => {}
    }
}

fn is_uncond(ins: &Ins) -> bool {
    matches!(ins.mn.as_str(), "bra" | "jmp" | "rts" | "rti" | "stp")
}

/// i から先の最初の命令（ラベル・コメント・ソースは飛ばす。印やデータがあれば None）
fn next_ins_through_labels(items: &[Item], from: usize, removed: &[bool]) -> Option<usize> {
    let mut k = from;
    while k < items.len() {
        if removed[k] {
            k += 1;
            continue;
        }
        match &items[k] {
            Item::Ins(_) => return Some(k),
            Item::Label(_) | Item::Src(_) | Item::Comment(_) => k += 1,
            _ => return None,
        }
    }
    None
}

/// i から先の最初の命令（コメント・ソースだけを飛ばす。ラベルがあれば None）
fn next_ins_same_block(items: &[Item], from: usize, removed: &[bool]) -> Option<usize> {
    let mut k = from;
    while k < items.len() {
        if removed[k] {
            k += 1;
            continue;
        }
        match &items[k] {
            Item::Ins(_) => return Some(k),
            Item::Src(_) | Item::Comment(_) => k += 1,
            _ => return None,
        }
    }
    None
}

fn compact(items: &mut Vec<Item>, removed: &[bool]) {
    let mut out = Vec::with_capacity(items.len());
    for (i, it) in items.drain(..).enumerate() {
        if !removed[i] {
            out.push(it);
        }
    }
    *items = out;
}

/// 記号のない番地（I/O など）と shared 変数は、読み書きを省けない
fn is_volatile(e: &OpExpr, cx: &PeepCx) -> bool {
    match &e.sym {
        None => true,
        Some(s) => e.part != Part::Full || cx.volatile.contains(s),
    }
}

// ---- フラグ ----------------------------------------------------------------

fn sets_flag(ins: &Ins, f: char) -> bool {
    let m = ins.mn.as_str();
    let nz = matches!(
        m,
        "lda" | "ldx" | "ldy" | "and" | "ora" | "eor" | "adc" | "sbc" | "inc" | "dec" | "inx" | "iny" | "dex" | "dey" | "asl" | "lsr" | "rol"
            | "ror" | "tax" | "tay" | "txa" | "tya" | "tsx" | "pla" | "plx" | "ply" | "cmp" | "cpx" | "cpy" | "plp" | "rti"
    );
    match f {
        'n' => nz || (m == "bit" && !matches!(ins.am, AMode::Imm(_))),
        'z' => nz || matches!(m, "bit" | "trb" | "tsb"),
        _ => matches!(m, "adc" | "sbc" | "asl" | "lsr" | "rol" | "ror" | "cmp" | "cpx" | "cpy" | "sec" | "clc" | "plp" | "rti"),
    }
}

fn reads_flag(ins: &Ins, f: char) -> bool {
    let m = ins.mn.as_str();
    if matches!(m, "php" | "brk") {
        return true;
    }
    match f {
        'n' => matches!(m, "bmi" | "bpl"),
        'z' => matches!(m, "beq" | "bne"),
        _ => matches!(m, "bcc" | "bcs" | "adc" | "sbc" | "rol" | "ror"),
    }
}

/// from の位置でフラグ f の値が後で使われるか（分からなければ使われるとみなす）
fn flag_live(items: &[Item], removed: &[bool], from: usize, f: char, labels: &HashMap<String, usize>, ret_c: bool) -> bool {
    let mut stack = vec![from];
    let mut seen: HashSet<usize> = HashSet::new();
    while let Some(start) = stack.pop() {
        let mut i = start;
        loop {
            if i >= items.len() {
                return true;
            }
            if !seen.insert(i) {
                break;
            }
            if removed[i] {
                i += 1;
                continue;
            }
            match &items[i] {
                Item::Ins(ins) => {
                    let m = ins.mn.as_str();
                    if reads_flag(ins, f) {
                        return true;
                    }
                    if sets_flag(ins, f) {
                        break;
                    }
                    match m {
                        // 関数から戻る：bool・Option はキャリーで返る。N と Z は返さない
                        "rts" => {
                            if f == 'c' && ret_c {
                                return true;
                            }
                            break;
                        }
                        // 呼んだ関数はフラグを壊す（フラグで値を受け取る関数はない）
                        "jsr" => break,
                        "stp" | "wai" => return true,
                        "jmp" => match jump_target(ins) {
                            Some(l) => match labels.get(&l) {
                                Some(&p) => {
                                    i = p;
                                    continue;
                                }
                                // ほかの関数への jmp（末尾呼び出し）
                                None => break,
                            },
                            None => return true,
                        },
                        "bra" => match jump_target(ins).and_then(|l| labels.get(&l).copied()) {
                            Some(p) => {
                                i = p;
                                continue;
                            }
                            None => return true,
                        },
                        _ if is_branch(m) => {
                            match jump_target(ins).and_then(|l| labels.get(&l).copied()) {
                                Some(p) => stack.push(p),
                                None => return true,
                            }
                            i += 1;
                            continue;
                        }
                        _ => {
                            i += 1;
                            continue;
                        }
                    }
                }
                Item::Label(_) | Item::Src(_) | Item::Comment(_) => {
                    i += 1;
                    continue;
                }
                _ => return true,
            }
        }
    }
    false
}

// ---- 1. 分岐の整理 ---------------------------------------------------------

/// 分岐先が無条件の分岐なら、その先へ直接飛ぶ。分岐先が rts なら bra・jmp を rts にする
fn thread_jumps(items: &mut Vec<Item>) -> bool {
    let keep = keep_mask(items);
    let labels = label_pos(items);
    let none = vec![false; items.len()];
    let mut changed = false;
    for i in 0..items.len() {
        if keep[i] {
            continue;
        }
        let (mn, t) = match &items[i] {
            Item::Ins(ins) if is_branch(&ins.mn) || ins.mn == "jmp" => match jump_target(ins) {
                Some(t) => (ins.mn.clone(), t),
                None => continue,
            },
            _ => continue,
        };
        let p = match labels.get(&t) {
            Some(&p) => p,
            None => continue,
        };
        let q = match next_ins_through_labels(items, p + 1, &none) {
            Some(q) => q,
            None => continue,
        };
        if keep[q] {
            continue;
        }
        let target = match &items[q] {
            Item::Ins(x) => x.clone(),
            _ => continue,
        };
        if (mn == "bra" || mn == "jmp") && target.mn == "rts" {
            let c = match &items[i] {
                Item::Ins(ins) => ins.comment.clone(),
                _ => String::new(),
            };
            items[i] = Item::Ins(Ins::new("rts", AMode::Imp, &c));
            changed = true;
            continue;
        }
        if target.mn == "bra" || target.mn == "jmp" {
            if let Some(t2) = jump_target(&target) {
                if t2 != t && labels.contains_key(&t2) {
                    if let Item::Ins(ins) = &mut items[i] {
                        set_jump_target(ins, &t2);
                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

/// すぐ次のラベルへの bra・jmp を消す
fn drop_jumps_to_next(items: &mut Vec<Item>) -> bool {
    let keep = keep_mask(items);
    let mut removed = vec![false; items.len()];
    let mut changed = false;
    for i in 0..items.len() {
        if keep[i] {
            continue;
        }
        let t = match &items[i] {
            Item::Ins(ins) if ins.mn == "bra" || ins.mn == "jmp" => match jump_target(ins) {
                Some(t) => t,
                None => continue,
            },
            _ => continue,
        };
        let mut k = i + 1;
        while k < items.len() {
            match &items[k] {
                Item::Label(l) if *l == t => {
                    removed[i] = true;
                    changed = true;
                    break;
                }
                Item::Label(_) | Item::Src(_) | Item::Comment(_) => k += 1,
                _ => break,
            }
        }
    }
    compact(items, &removed);
    changed
}

/// Bcc L1; bra L2; L1:  →  B!cc L2; L1:
fn branch_over_branch(items: &mut Vec<Item>) -> bool {
    let keep = keep_mask(items);
    let labels = label_pos(items);
    let mut removed = vec![false; items.len()];
    let mut changed = false;
    let mut i = 0;
    while i < items.len() {
        if keep[i] || removed[i] {
            i += 1;
            continue;
        }
        let (mn, l1) = match &items[i] {
            Item::Ins(ins) if is_branch(&ins.mn) && ins.mn != "bra" => match &ins.am {
                AMode::Rel(l) => (ins.mn.clone(), l.clone()),
                _ => {
                    i += 1;
                    continue;
                }
            },
            _ => {
                i += 1;
                continue;
            }
        };
        let j = match next_ins_same_block(items, i + 1, &removed) {
            Some(j) => j,
            None => {
                i += 1;
                continue;
            }
        };
        if keep[j] {
            i += 1;
            continue;
        }
        let l2 = match &items[j] {
            Item::Ins(x) if x.mn == "bra" || x.mn == "jmp" => match jump_target(x) {
                // jmp はこの関数の中のラベルへのときだけ（分岐の届かない分は、アセンブラが直す）
                Some(t) if labels.contains_key(&t) => t,
                _ => {
                    i += 1;
                    continue;
                }
            },
            _ => {
                i += 1;
                continue;
            }
        };
        // j の次（ラベルだけを飛ばした先）に L1 があるか
        let mut k = j + 1;
        let mut hit = false;
        while k < items.len() {
            match &items[k] {
                Item::Label(l) => {
                    if *l == l1 {
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
            if let Item::Ins(ins) = &mut items[i] {
                ins.mn = invert_branch(&mn).to_string();
                ins.am = AMode::Rel(l2);
                if !ins.comment.contains("（分岐を反転）") {
                    ins.comment = format!("{}（分岐を反転）", ins.comment);
                }
            }
            removed[j] = true;
            changed = true;
        }
        i += 1;
    }
    compact(items, &removed);
    changed
}

/// 無条件の分岐・rts の後ろの、ラベルのない命令を消す
fn remove_unreachable(items: &mut Vec<Item>) -> bool {
    let keep = keep_mask(items);
    let mut removed = vec![false; items.len()];
    let mut changed = false;
    let mut dead = false;
    for i in 0..items.len() {
        if keep[i] {
            dead = false;
            continue;
        }
        match &items[i] {
            Item::Ins(ins) => {
                if dead {
                    removed[i] = true;
                    changed = true;
                } else if is_uncond(ins) {
                    dead = true;
                }
            }
            Item::Src(_) | Item::Comment(_) => {}
            _ => dead = false,
        }
    }
    compact(items, &removed);
    changed
}

/// 参照されないラベル（コンパイラが作った @・~ を含む名前だけ）を消す
fn remove_unused_labels(items: &mut Vec<Item>, cx: &PeepCx) -> bool {
    let keep = keep_mask(items);
    let mut refs: HashSet<String> = HashSet::new();
    for it in items.iter() {
        match it {
            Item::Ins(ins) => {
                if let AMode::Rel(l) = &ins.am {
                    refs.insert(l.clone());
                }
                if let Some(e) = ins.expr() {
                    if let Some(s) = &e.sym {
                        refs.insert(s.clone());
                    }
                }
            }
            Item::Exprs(v, _) => {
                for e in v {
                    if let Some(s) = &e.sym {
                        refs.insert(s.clone());
                    }
                }
            }
            _ => {}
        }
    }
    let mut removed = vec![false; items.len()];
    let mut changed = false;
    for (i, it) in items.iter().enumerate() {
        if keep[i] {
            continue;
        }
        if let Item::Label(l) = it {
            if (l.contains('@') || l.contains('~')) && !refs.contains(l) && !cx.ext_refs.contains(l) && l != cx.host {
                removed[i] = true;
                changed = true;
            }
        }
    }
    compact(items, &removed);
    changed
}

// ---- 2. レジスタの中身の追跡 -----------------------------------------------

fn transfer(src: char, dst: char) -> Option<&'static str> {
    match (src, dst) {
        ('a', 'x') => Some("tax"),
        ('a', 'y') => Some("tay"),
        ('x', 'a') => Some("txa"),
        ('y', 'a') => Some("tya"),
        _ => None,
    }
}

fn nz_dead(items: &[Item], removed: &[bool], i: usize, labels: &HashMap<String, usize>) -> bool {
    !flag_live(items, removed, i + 1, 'n', labels, false) && !flag_live(items, removed, i + 1, 'z', labels, false)
}

fn track(items: &mut Vec<Item>, cx: &PeepCx, uniq: &mut usize) -> bool {
    let keep = keep_mask(items);
    let labels = label_pos(items);
    let n = items.len();
    let mut removed = vec![false; n];
    let mut changed = false;
    let mut st = St::default();
    let mut i = 0;
    while i < n {
        if removed[i] {
            i += 1;
            continue;
        }
        if keep[i] {
            st = St::default();
            i += 1;
            continue;
        }
        let ins = match &items[i] {
            Item::Ins(x) => x.clone(),
            Item::Src(_) | Item::Comment(_) => {
                i += 1;
                continue;
            }
            _ => {
                st = St::default();
                i += 1;
                continue;
            }
        };
        let m = ins.mn.as_str();
        match m {
            "lda" | "ldx" | "ldy" => {
                let r = m.chars().nth(2).unwrap_or('a');
                let v = match &ins.am {
                    AMode::Imm(e) => Some(Val::Imm(e.clone())),
                    AMode::Mem(e) if !is_volatile(e, cx) => Some(Val::Mem(e.clone())),
                    _ => None,
                };
                match v {
                    Some(v) => {
                        if st.reg(r).as_ref() == Some(&v) && (st.nz == Some(r) || nz_dead(items, &removed, i, &labels)) {
                            removed[i] = true;
                            changed = true;
                            i += 1;
                            continue;
                        }
                        let src = ['a', 'x', 'y'].into_iter().find(|s| *s != r && st.reg(*s).as_ref() == Some(&v) && transfer(*s, r).is_some());
                        if let Some(s) = src {
                            let t = transfer(s, r).unwrap_or("nop");
                            items[i] = Item::Ins(Ins::new(t, AMode::Imp, &format!("{}（{} にある値を移す）", ins.comment, s.to_ascii_uppercase())));
                            changed = true;
                        }
                        st.set(r, Some(v));
                    }
                    None => st.set(r, None),
                }
                st.nz = Some(r);
            }
            "sta" | "stx" | "sty" | "stz" => {
                let r = m.chars().nth(2).unwrap_or('z');
                match &ins.am {
                    AMode::Mem(e) if !is_volatile(e, cx) => {
                        if r != 'z' && st.reg(r).as_ref() == Some(&Val::Mem(e.clone())) {
                            removed[i] = true;
                            changed = true;
                            i += 1;
                            continue;
                        }
                        st.forget_mem(e);
                        if r != 'z' && !matches!(st.reg(r), Some(Val::Imm(_))) {
                            st.set(r, Some(Val::Mem(e.clone())));
                        }
                    }
                    AMode::Mem(_) => {}
                    AMode::MemX(e) | AMode::MemY(e) => st.forget_sym(e),
                    _ => st.forget_all_mem(),
                }
            }
            "tax" | "tay" | "txa" | "tya" => {
                let s = m.chars().nth(1).unwrap_or('a');
                let d = m.chars().nth(2).unwrap_or('a');
                let v = st.reg(s).clone();
                if v.is_some() && *st.reg(d) == v && (st.nz == Some(d) || nz_dead(items, &removed, i, &labels)) {
                    removed[i] = true;
                    changed = true;
                    i += 1;
                    continue;
                }
                st.set(d, v);
                st.nz = Some(d);
            }
            "inx" | "dex" => {
                st.x = None;
                st.nz = Some('x');
            }
            "iny" | "dey" => {
                st.y = None;
                st.nz = Some('y');
            }
            "inc" | "dec" | "asl" | "lsr" | "rol" | "ror" => {
                match &ins.am {
                    AMode::Imp | AMode::Acc => {
                        st.a = None;
                        st.nz = Some('a');
                    }
                    AMode::Mem(e) => {
                        st.forget_mem(e);
                        st.nz = None;
                    }
                    AMode::MemX(e) | AMode::MemY(e) => {
                        st.forget_sym(e);
                        st.nz = None;
                    }
                    _ => {
                        st.forget_all_mem();
                        st.nz = None;
                    }
                }
                if !matches!(m, "inc" | "dec") {
                    st.c = None;
                }
            }
            "and" | "ora" | "eor" => {
                st.a = None;
                st.nz = Some('a');
            }
            "adc" | "sbc" => {
                st.a = None;
                st.nz = Some('a');
                st.c = None;
            }
            "cmp" | "cpx" | "cpy" => {
                // cmp #0：N と Z は A の値のまま。キャリーが使われなければ要らない
                if m == "cmp" && st.nz == Some('a') {
                    if let AMode::Imm(e) = &ins.am {
                        if e.sym.is_none() && e.off == 0 && !flag_live(items, &removed, i + 1, 'c', &labels, cx.ret_c) {
                            removed[i] = true;
                            changed = true;
                            i += 1;
                            continue;
                        }
                        // cmp #1; bcc L  →  beq L（A が 0 かどうか。キャリーが使われないとき）
                        if e.sym.is_none() && e.off == 1 {
                            if let Some(j) = next_ins_same_block(items, i + 1, &removed) {
                                let br = match &items[j] {
                                    Item::Ins(b) if (b.mn == "bcc" || b.mn == "bcs") && !keep[j] => Some(b.clone()),
                                    _ => None,
                                };
                                if let Some(b) = br {
                                    let tl = jump_target(&b).and_then(|l| labels.get(&l).copied());
                                    if let Some(tp) = tl {
                                        if !flag_live(items, &removed, j + 1, 'c', &labels, cx.ret_c) && !flag_live(items, &removed, tp, 'c', &labels, cx.ret_c) {
                                            removed[i] = true;
                                            let nm = if b.mn == "bcc" { "beq" } else { "bne" };
                                            items[j] = Item::Ins(Ins::new(nm, b.am.clone(), &format!("{}（0 かどうかで分かれる）", b.comment)));
                                            changed = true;
                                            i += 1;
                                            continue;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                st.nz = None;
                st.c = None;
            }
            "bit" => st.nz = None,
            "trb" | "tsb" => {
                if let Some(e) = ins.expr() {
                    st.forget_mem(e);
                }
                st.nz = None;
            }
            "clc" | "sec" => {
                let v = m == "sec";
                if st.c == Some(v) || !flag_live(items, &removed, i + 1, 'c', &labels, cx.ret_c) {
                    removed[i] = true;
                    changed = true;
                    i += 1;
                    continue;
                }
                st.c = Some(v);
            }
            "bcc" | "bcs" => {
                if let Some(cv) = st.c {
                    let taken = (m == "bcs") == cv;
                    if taken {
                        items[i] = Item::Ins(Ins::new("bra", ins.am.clone(), &format!("{}（キャリーは分かっている）", ins.comment)));
                        st = St::default();
                    } else {
                        removed[i] = true;
                    }
                    changed = true;
                    i += 1;
                    continue;
                }
            }
            "bra" => {
                // キャリーが分かっていて、飛び先が bcc・bcs なら、その結果の先へ直接飛ぶ
                if let Some(cv) = st.c {
                    if let Some(t) = jump_target(&ins) {
                        if let Some(&p) = labels.get(&t) {
                            if let Some(q) = next_ins_through_labels(items, p + 1, &removed) {
                                let qb = match &items[q] {
                                    Item::Ins(b) if (b.mn == "bcc" || b.mn == "bcs") && !keep[q] => Some(b.clone()),
                                    _ => None,
                                };
                                if let Some(b) = qb {
                                    let taken = (b.mn == "bcs") == cv;
                                    if taken {
                                        if let Some(t2) = jump_target(&b) {
                                            if t2 != t {
                                                if let Item::Ins(x) = &mut items[i] {
                                                    set_jump_target(x, &t2);
                                                }
                                                changed = true;
                                            }
                                        }
                                    } else {
                                        // 分岐しない側：bcc・bcs の次に新しいラベルを置いて、そこへ飛ぶ
                                        *uniq += 1;
                                        let nl = format!("{}@nc{}", t, uniq);
                                        if let Item::Ins(x) = &mut items[i] {
                                            set_jump_target(x, &nl);
                                        }
                                        compact(items, &removed);
                                        // 消した分だけ q の位置がずれる
                                        let shift = removed[..q].iter().filter(|r| **r).count();
                                        items.insert(q - shift + 1, Item::Label(nl));
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
                st = St::default();
            }
            "pha" | "phx" | "phy" | "php" | "sed" | "cld" | "sei" | "cli" | "nop" => {}
            "clv" => {}
            "pla" => {
                st.a = None;
                st.nz = Some('a');
            }
            "plx" => {
                st.x = None;
                st.nz = Some('x');
            }
            "ply" => {
                st.y = None;
                st.nz = Some('y');
            }
            _ if is_branch(m) => {}
            _ => st = St::default(),
        }
        i += 1;
    }
    compact(items, &removed);
    changed
}

// ---- 3. ローカル変数への、読まれない書き込み ------------------------------

/// フレームの記号（F_関数、Z_関数）の 1 バイト
fn frame_slot(e: &OpExpr, cx: &PeepCx) -> Option<(String, i64)> {
    let s = e.sym.as_ref()?;
    if e.part != Part::Full {
        return None;
    }
    let f = s.strip_prefix("F_").or_else(|| s.strip_prefix("Z_"))?;
    // ほかから呼ばれる関数のフレームは、その関数が読むかもしれない（自分自身は除く）
    if f != cx.host && cx.called.contains(f) {
        return None;
    }
    Some((s.clone(), e.off))
}

fn is_store(m: &str) -> bool {
    matches!(m, "sta" | "stx" | "sty" | "stz")
}

/// from の位置で、フレームの 1 バイト (s, o) の値が後で読まれるか
fn slot_live(items: &[Item], removed: &[bool], keep: &[bool], from: usize, s: &str, o: i64, labels: &HashMap<String, usize>) -> bool {
    let mut stack = vec![from];
    let mut seen: HashSet<usize> = HashSet::new();
    while let Some(start) = stack.pop() {
        let mut i = start;
        loop {
            if i >= items.len() {
                return true;
            }
            if !seen.insert(i) {
                break;
            }
            if removed[i] {
                i += 1;
                continue;
            }
            match &items[i] {
                Item::Ins(ins) => {
                    let m = ins.mn.as_str();
                    if keep[i] {
                        // アセンブリの中：この記号に触れていれば読まれるとみなす
                        if ins.expr().and_then(|e| e.sym.as_deref()) == Some(s) {
                            return true;
                        }
                    }
                    if let Some(e) = ins.expr() {
                        if e.sym.as_deref() == Some(s) {
                            match &ins.am {
                                AMode::Mem(_) => {
                                    if e.off == o {
                                        if is_store(m) {
                                            break;
                                        }
                                        return true;
                                    }
                                }
                                AMode::MemX(_) | AMode::MemY(_) => {
                                    if !is_store(m) {
                                        return true;
                                    }
                                }
                                _ => return true,
                            }
                        }
                    }
                    match m {
                        "rts" | "rti" => break,
                        "jsr" => {
                            // 呼ぶ関数自身のフレーム（引数）なら読まれる
                            if let Some(t) = jump_target(ins).or_else(|| ins.expr().and_then(|e| e.sym.clone())) {
                                if s == format!("F_{}", t) || s == format!("Z_{}", t) {
                                    return true;
                                }
                            }
                            i += 1;
                            continue;
                        }
                        "jmp" => match jump_target(ins) {
                            Some(l) => match labels.get(&l) {
                                Some(&p) => {
                                    i = p;
                                    continue;
                                }
                                None => {
                                    if s == format!("F_{}", l) || s == format!("Z_{}", l) {
                                        return true;
                                    }
                                    break;
                                }
                            },
                            None => return true,
                        },
                        "brk" | "stp" | "wai" => return true,
                        "bra" => match jump_target(ins).and_then(|l| labels.get(&l).copied()) {
                            Some(p) => {
                                i = p;
                                continue;
                            }
                            None => return true,
                        },
                        _ if is_branch(m) => {
                            match jump_target(ins).and_then(|l| labels.get(&l).copied()) {
                                Some(p) => stack.push(p),
                                None => return true,
                            }
                            i += 1;
                            continue;
                        }
                        _ => {
                            i += 1;
                            continue;
                        }
                    }
                }
                Item::Label(_) | Item::Src(_) | Item::Comment(_) => {
                    i += 1;
                    continue;
                }
                Item::Mark(mk) => {
                    if mk == KEEP_BEGIN || mk == KEEP_END {
                        i += 1;
                        continue;
                    }
                    return true;
                }
                _ => return true,
            }
        }
    }
    false
}

fn dead_stores(items: &mut Vec<Item>, cx: &PeepCx) -> bool {
    let keep = keep_mask(items);
    let labels = label_pos(items);
    let mut removed = vec![false; items.len()];
    let mut changed = false;
    // アドレスを取られたフレームの記号（&local）は対象外
    let mut escaped: HashSet<String> = HashSet::new();
    for it in items.iter() {
        if let Item::Ins(ins) = it {
            if let AMode::Imm(e) = &ins.am {
                if let Some(s) = &e.sym {
                    escaped.insert(s.clone());
                }
            }
        }
        if let Item::Exprs(v, _) = it {
            for e in v {
                if let Some(s) = &e.sym {
                    escaped.insert(s.clone());
                }
            }
        }
    }
    for i in 0..items.len() {
        if keep[i] {
            continue;
        }
        let (s, o) = match &items[i] {
            Item::Ins(ins) if is_store(&ins.mn) => match &ins.am {
                AMode::Mem(e) => match frame_slot(e, cx) {
                    Some(x) => x,
                    None => continue,
                },
                _ => continue,
            },
            _ => continue,
        };
        if escaped.contains(&s) {
            continue;
        }
        if !slot_live(items, &removed, &keep, i + 1, &s, o, &labels) {
            removed[i] = true;
            changed = true;
        }
    }
    compact(items, &removed);
    changed
}

/// sta T; lda V; [clc;] adc T  →  [clc;] adc V（T が後で読まれないとき。and・ora・eor も同じ）
fn fold_commutative(items: &mut Vec<Item>, cx: &PeepCx) -> bool {
    let keep = keep_mask(items);
    let labels = label_pos(items);
    let none = vec![false; items.len()];
    let mut i = 0;
    while i < items.len() {
        if keep[i] {
            i += 1;
            continue;
        }
        let t = match &items[i] {
            Item::Ins(ins) if ins.mn == "sta" => match &ins.am {
                AMode::Mem(e) => match frame_slot(e, cx) {
                    Some(_) => e.clone(),
                    None => {
                        i += 1;
                        continue;
                    }
                },
                _ => {
                    i += 1;
                    continue;
                }
            },
            _ => {
                i += 1;
                continue;
            }
        };
        let j = match next_ins_same_block(items, i + 1, &none) {
            Some(j) => j,
            None => {
                i += 1;
                continue;
            }
        };
        let v = match &items[j] {
            Item::Ins(x) if x.mn == "lda" && !keep[j] => match &x.am {
                AMode::Imm(e) => AMode::Imm(e.clone()),
                AMode::Mem(e) if !is_volatile(e, cx) && *e != t => AMode::Mem(e.clone()),
                _ => {
                    i += 1;
                    continue;
                }
            },
            _ => {
                i += 1;
                continue;
            }
        };
        let mut k = match next_ins_same_block(items, j + 1, &none) {
            Some(k) => k,
            None => {
                i += 1;
                continue;
            }
        };
        let mut has_clc = false;
        if let Item::Ins(x) = &items[k] {
            if x.mn == "clc" {
                has_clc = true;
                k = match next_ins_same_block(items, k + 1, &none) {
                    Some(k2) => k2,
                    None => {
                        i += 1;
                        continue;
                    }
                };
            }
        }
        let op = match &items[k] {
            Item::Ins(x) if matches!(x.mn.as_str(), "adc" | "and" | "ora" | "eor") && !keep[k] => match &x.am {
                AMode::Mem(e) if *e == t => x.clone(),
                _ => {
                    i += 1;
                    continue;
                }
            },
            _ => {
                i += 1;
                continue;
            }
        };
        if op.mn == "adc" && !has_clc {
            i += 1;
            continue;
        }
        let (s, o) = match frame_slot(&t, cx) {
            Some(x) => x,
            None => {
                i += 1;
                continue;
            }
        };
        if slot_live(items, &none, &keep, k + 1, &s, o, &labels) {
            i += 1;
            continue;
        }
        // 書き換え：sta と lda を消し、演算の相手を V にする
        let mut removed = vec![false; items.len()];
        removed[i] = true;
        removed[j] = true;
        items[k] = Item::Ins(Ins::new(&op.mn, v, &format!("{}（順序を入れ替えた）", op.comment)));
        compact(items, &removed);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ins(m: &str, am: AMode) -> Item {
        Item::Ins(Ins::new(m, am, ""))
    }
    fn mem(s: &str, o: i64) -> AMode {
        AMode::Mem(OpExpr::sym_off(s, o))
    }

    fn run(items: &mut Vec<Item>) {
        let e = HashSet::new();
        let v = HashSet::new();
        let c = HashSet::new();
        let cx = PeepCx { ext_refs: &e, volatile: &v, called: &c, host: "f", ret_c: false };
        let mut u = 0;
        peephole(items, &cx, &mut u);
    }

    fn mns(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .filter_map(|x| match x {
                Item::Ins(i) => Some(format!("{} {}", i.mn, i.operand_text()).trim().to_string()),
                Item::Label(l) => Some(format!("{}:", l)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn reload_and_temp() {
        // lda IO; sta k; lda k; and #$80; sta t; lda t; bne L; rts; L: lda k; rts
        let mut v = vec![
            Item::Label("f".to_string()),
            ins("lda", AMode::Mem(OpExpr::num(0x8800))),
            ins("sta", mem("F_f", 0)),
            ins("lda", mem("F_f", 0)),
            ins("and", AMode::Imm(OpExpr::num(0x80))),
            ins("sta", mem("F_f", 1)),
            ins("lda", mem("F_f", 1)),
            ins("bne", AMode::Rel("f@l".to_string())),
            ins("rts", AMode::Imp),
            Item::Label("f@l".to_string()),
            ins("lda", mem("F_f", 0)),
            ins("rts", AMode::Imp),
        ];
        run(&mut v);
        assert_eq!(mns(&v), vec!["f:", "lda $8800", "sta F_f", "and #$80", "bne f@l", "rts", "f@l:", "lda F_f", "rts"]);
    }

    #[test]
    fn io_reads_kept() {
        let mut v = vec![
            Item::Label("f".to_string()),
            ins("lda", AMode::Mem(OpExpr::num(0x8800))),
            ins("lda", AMode::Mem(OpExpr::num(0x8800))),
            ins("sta", AMode::Mem(OpExpr::num(0x8400))),
            ins("rts", AMode::Imp),
        ];
        run(&mut v);
        assert_eq!(mns(&v), vec!["f:", "lda $8800", "lda $8800", "sta $8400", "rts"]);
    }

    #[test]
    fn bool_through_carry() {
        // lda h; bne t; clc; bra e; t: sec; e: bcc no; jmp yes; no: rts; yes: lda #1; rts
        let mut v = vec![
            Item::Label("f".to_string()),
            ins("lda", mem("H", 0)),
            ins("bne", AMode::Rel("f@t".to_string())),
            ins("clc", AMode::Imp),
            ins("bra", AMode::Rel("f@e".to_string())),
            Item::Label("f@t".to_string()),
            ins("sec", AMode::Imp),
            Item::Label("f@e".to_string()),
            ins("bcc", AMode::Rel("f@no".to_string())),
            ins("jmp", AMode::Mem(OpExpr::sym("f@yes"))),
            Item::Label("f@no".to_string()),
            ins("lda", AMode::Imm(OpExpr::num(0))),
            ins("rts", AMode::Imp),
            Item::Label("f@yes".to_string()),
            ins("lda", AMode::Imm(OpExpr::num(1))),
            ins("rts", AMode::Imp),
        ];
        run(&mut v);
        assert_eq!(mns(&v), vec!["f:", "lda H", "bne f@yes", "lda #0", "rts", "f@yes:", "lda #1", "rts"]);
    }

    #[test]
    fn keep_region_untouched() {
        let mut v = vec![
            Item::Label("f".to_string()),
            Item::Mark(KEEP_BEGIN.to_string()),
            ins("lda", mem("A", 0)),
            ins("lda", mem("A", 0)),
            ins("bra", AMode::Rel("f@x".to_string())),
            Item::Label("f@x".to_string()),
            Item::Mark(KEEP_END.to_string()),
            ins("rts", AMode::Imp),
        ];
        run(&mut v);
        assert_eq!(mns(&v), vec!["f:", "lda A", "lda A", "bra f@x", "f@x:", "rts"]);
    }
}
