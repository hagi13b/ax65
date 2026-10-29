// 内蔵の 2 パス・アセンブラ（W65C02S と NMOS 6502）と命令表・サイクル数
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Imp,
    Acc,
    Imm,
    Zp,
    Zpx,
    Zpy,
    Izx,
    Izy,
    Izp,
    Rel,
    Abs,
    Abx,
    Aby,
    Ind,
    Iax,
}

pub fn mode_size(m: Mode) -> u32 {
    match m {
        Mode::Imp | Mode::Acc => 1,
        Mode::Imm | Mode::Zp | Mode::Zpx | Mode::Zpy | Mode::Izx | Mode::Izy | Mode::Izp | Mode::Rel => 2,
        Mode::Abs | Mode::Abx | Mode::Aby | Mode::Ind | Mode::Iax => 3,
    }
}

fn mode_from(s: &str) -> Mode {
    match s {
        "imp" => Mode::Imp,
        "acc" => Mode::Acc,
        "imm" => Mode::Imm,
        "zp" => Mode::Zp,
        "zpx" => Mode::Zpx,
        "zpy" => Mode::Zpy,
        "izx" => Mode::Izx,
        "izy" => Mode::Izy,
        "izp" => Mode::Izp,
        "rel" => Mode::Rel,
        "abs" => Mode::Abs,
        "abx" => Mode::Abx,
        "aby" => Mode::Aby,
        "ind" => Mode::Ind,
        _ => Mode::Iax,
    }
}

// 命令表（tools/asm65.py と同じ）：mnemonic mode opcode cycles(*はページ境界で +1)
const TABLE: &str = "\
brk imp 00 7|ora izx 01 6|tsb zp 04 5|ora zp 05 3|asl zp 06 5|php imp 08 3|ora imm 09 2|asl acc 0A 2|tsb abs 0C 6|ora abs 0D 4|asl abs 0E 6|\
bpl rel 10 2|ora izy 11 5*|ora izp 12 5|trb zp 14 5|ora zpx 15 4|asl zpx 16 6|clc imp 18 2|ora aby 19 4*|inc acc 1A 2|trb abs 1C 6|ora abx 1D 4*|asl abx 1E 6*|\
jsr abs 20 6|and izx 21 6|bit zp 24 3|and zp 25 3|rol zp 26 5|plp imp 28 4|and imm 29 2|rol acc 2A 2|bit abs 2C 4|and abs 2D 4|rol abs 2E 6|\
bmi rel 30 2|and izy 31 5*|and izp 32 5|bit zpx 34 4|and zpx 35 4|rol zpx 36 6|sec imp 38 2|and aby 39 4*|dec acc 3A 2|bit abx 3C 4*|and abx 3D 4*|rol abx 3E 6*|\
rti imp 40 6|eor izx 41 6|eor zp 45 3|lsr zp 46 5|pha imp 48 3|eor imm 49 2|lsr acc 4A 2|jmp abs 4C 3|eor abs 4D 4|lsr abs 4E 6|\
bvc rel 50 2|eor izy 51 5*|eor izp 52 5|eor zpx 55 4|lsr zpx 56 6|cli imp 58 2|eor aby 59 4*|phy imp 5A 3|eor abx 5D 4*|lsr abx 5E 6*|\
rts imp 60 6|adc izx 61 6|stz zp 64 3|adc zp 65 3|ror zp 66 5|pla imp 68 4|adc imm 69 2|ror acc 6A 2|jmp ind 6C 6|adc abs 6D 4|ror abs 6E 6|\
bvs rel 70 2|adc izy 71 5*|adc izp 72 5|stz zpx 74 4|adc zpx 75 4|ror zpx 76 6|sei imp 78 2|adc aby 79 4*|ply imp 7A 4|jmp iax 7C 6|adc abx 7D 4*|ror abx 7E 6*|\
bra rel 80 3|sta izx 81 6|sty zp 84 3|sta zp 85 3|stx zp 86 3|dey imp 88 2|bit imm 89 2|txa imp 8A 2|sty abs 8C 4|sta abs 8D 4|stx abs 8E 4|\
bcc rel 90 2|sta izy 91 6|sta izp 92 5|sty zpx 94 4|sta zpx 95 4|stx zpy 96 4|tya imp 98 2|sta aby 99 5|txs imp 9A 2|stz abs 9C 4|sta abx 9D 5|stz abx 9E 5|\
ldy imm A0 2|lda izx A1 6|ldx imm A2 2|ldy zp A4 3|lda zp A5 3|ldx zp A6 3|tay imp A8 2|lda imm A9 2|tax imp AA 2|ldy abs AC 4|lda abs AD 4|ldx abs AE 4|\
bcs rel B0 2|lda izy B1 5*|lda izp B2 5|ldy zpx B4 4|lda zpx B5 4|ldx zpy B6 4|clv imp B8 2|lda aby B9 4*|tsx imp BA 2|ldy abx BC 4*|lda abx BD 4*|ldx aby BE 4*|\
cpy imm C0 2|cmp izx C1 6|cpy zp C4 3|cmp zp C5 3|dec zp C6 5|iny imp C8 2|cmp imm C9 2|dex imp CA 2|wai imp CB 3|cpy abs CC 4|cmp abs CD 4|dec abs CE 6|\
bne rel D0 2|cmp izy D1 5*|cmp izp D2 5|cmp zpx D5 4|dec zpx D6 6|cld imp D8 2|cmp aby D9 4*|phx imp DA 3|stp imp DB 3|cmp abx DD 4*|dec abx DE 7|\
cpx imm E0 2|sbc izx E1 6|cpx zp E4 3|sbc zp E5 3|inc zp E6 5|inx imp E8 2|sbc imm E9 2|nop imp EA 2|cpx abs EC 4|sbc abs ED 4|inc abs EE 6|\
beq rel F0 2|sbc izy F1 5*|sbc izp F2 5|sbc zpx F5 4|inc zpx F6 6|sed imp F8 2|sbc aby F9 4*|plx imp FA 4|sbc abx FD 4*|inc abx FE 7";

#[derive(Clone, Copy, Debug)]
pub struct OpInfo {
    pub code: u8,
    pub cycles: u8,
    pub page: bool,
}

pub struct OpTable {
    map: HashMap<(String, Mode), OpInfo>,
    mnems: HashSet<String>,
}

impl OpTable {
    pub fn new() -> OpTable {
        let mut map = HashMap::new();
        let mut mnems = HashSet::new();
        for item in TABLE.split('|') {
            let parts: Vec<&str> = item.split_whitespace().collect();
            if parts.len() != 4 {
                continue;
            }
            let code = u8::from_str_radix(parts[2], 16).unwrap_or(0xEA);
            let page = parts[3].ends_with('*');
            let cycles: u8 = parts[3].trim_end_matches('*').parse().unwrap_or(2);
            map.insert((parts[0].to_string(), mode_from(parts[1])), OpInfo { code, cycles, page });
            mnems.insert(parts[0].to_string());
        }
        OpTable { map, mnems }
    }
    pub fn get(&self, m: &str, mode: Mode) -> Option<OpInfo> {
        self.map.get(&(m.to_string(), mode)).copied()
    }
    pub fn has(&self, m: &str, mode: Mode) -> bool {
        self.map.contains_key(&(m.to_string(), mode))
    }
    pub fn is_mnemonic(&self, m: &str) -> bool {
        self.mnems.contains(m)
    }
}

/// 65C02 で追加された命令・アドレッシング（NMOS 6502 にはない）
pub fn is_cmos_only(m: &str, mode: Mode) -> bool {
    match m {
        "bra" | "phx" | "phy" | "plx" | "ply" | "stz" | "trb" | "tsb" | "wai" | "stp" => true,
        "inc" | "dec" => mode == Mode::Acc,
        "bit" => matches!(mode, Mode::Imm | Mode::Zpx | Mode::Abx),
        "jmp" => mode == Mode::Iax,
        _ => mode == Mode::Izp,
    }
}

pub fn is_branch(m: &str) -> bool {
    matches!(m, "bpl" | "bmi" | "bvc" | "bvs" | "bcc" | "bcs" | "bne" | "beq" | "bra")
}

pub fn invert_branch(m: &str) -> &'static str {
    match m {
        "bpl" => "bmi",
        "bmi" => "bpl",
        "bvc" => "bvs",
        "bvs" => "bvc",
        "bcc" => "bcs",
        "bcs" => "bcc",
        "bne" => "beq",
        "beq" => "bne",
        _ => "bra",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Full,
    Lo,
    Hi,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpExpr {
    pub sym: Option<String>,
    pub off: i64,
    pub part: Part,
}

impl OpExpr {
    pub fn num(v: i64) -> OpExpr {
        OpExpr { sym: None, off: v, part: Part::Full }
    }
    pub fn sym(s: &str) -> OpExpr {
        OpExpr { sym: Some(s.to_string()), off: 0, part: Part::Full }
    }
    pub fn sym_off(s: &str, off: i64) -> OpExpr {
        OpExpr { sym: Some(s.to_string()), off, part: Part::Full }
    }
    pub fn lo(&self) -> OpExpr {
        OpExpr { sym: self.sym.clone(), off: self.off, part: Part::Lo }
    }
    pub fn hi(&self) -> OpExpr {
        OpExpr { sym: self.sym.clone(), off: self.off, part: Part::Hi }
    }
    pub fn text(&self) -> String {
        let base = match &self.sym {
            Some(s) => {
                if self.off > 0 {
                    format!("{}+{}", s, self.off)
                } else if self.off < 0 {
                    format!("{}-{}", s, -self.off)
                } else {
                    s.clone()
                }
            }
            None => {
                if self.part == Part::Full {
                    if self.off < 0 {
                        format!("-{}", -self.off)
                    } else if self.off < 10 {
                        format!("{}", self.off)
                    } else if self.off < 256 {
                        format!("${:02X}", self.off)
                    } else {
                        format!("${:04X}", self.off)
                    }
                } else {
                    format!("${:04X}", self.off)
                }
            }
        };
        match self.part {
            Part::Full => base,
            Part::Lo => format!("<{}", base),
            Part::Hi => format!(">{}", base),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AMode {
    Imp,
    Acc,
    Imm(OpExpr),
    Mem(OpExpr),
    MemX(OpExpr),
    MemY(OpExpr),
    IndY(OpExpr),
    IndX(OpExpr),
    Ind(OpExpr),
    IndAbsX(OpExpr),
    Rel(String),
}

#[derive(Clone, Debug)]
pub struct Ins {
    pub mn: String,
    pub am: AMode,
    pub comment: String,
}

impl Ins {
    pub fn new(mn: &str, am: AMode, comment: &str) -> Ins {
        Ins { mn: mn.to_string(), am, comment: comment.to_string() }
    }
    pub fn operand_text(&self) -> String {
        match &self.am {
            AMode::Imp => String::new(),
            AMode::Acc => "a".to_string(),
            AMode::Imm(e) => format!("#{}", e.text()),
            AMode::Mem(e) => e.text(),
            AMode::MemX(e) => format!("{},x", e.text()),
            AMode::MemY(e) => format!("{},y", e.text()),
            AMode::IndY(e) => format!("({}),y", e.text()),
            AMode::IndX(e) => format!("({},x)", e.text()),
            AMode::Ind(e) => format!("({})", e.text()),
            AMode::IndAbsX(e) => format!("({},x)", e.text()),
            AMode::Rel(l) => l.clone(),
        }
    }
    pub fn text(&self) -> String {
        let o = self.operand_text();
        if o.is_empty() {
            self.mn.clone()
        } else {
            format!("{} {}", self.mn, o)
        }
    }
    pub fn expr(&self) -> Option<&OpExpr> {
        match &self.am {
            AMode::Imm(e) | AMode::Mem(e) | AMode::MemX(e) | AMode::MemY(e) | AMode::IndY(e) | AMode::IndX(e) | AMode::Ind(e) | AMode::IndAbsX(e) => Some(e),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Item {
    Label(String),
    Ins(Ins),
    Bytes(Vec<u8>, String),
    Exprs(Vec<OpExpr>, String),
    Src(String),
    Comment(String),
    Res(String, u32, String),
    /// 内部用の印（インライン展開や退避の場所）
    Mark(String),
}

#[derive(Clone, Debug)]
pub struct ListLine {
    pub addr: Option<u32>,
    pub bytes: Vec<u8>,
    pub cycles: String,
    pub text: String,
    pub comment: String,
    pub is_src: bool,
}

pub struct AsmInput {
    pub code: Vec<Item>,
    pub data: Vec<Item>,
    pub bss: Vec<Item>,
    pub known: HashMap<String, i64>,
    pub org: u32,
    pub bss_org: Option<u32>,
    pub nmos: bool,
    pub fixed: Vec<(u32, Vec<Item>)>,
}

pub struct AsmOutput {
    pub image: BTreeMap<u32, u8>,
    pub symbols: BTreeMap<String, u32>,
    pub listing: Vec<ListLine>,
    pub code_start: u32,
    pub code_end: u32,
    pub data_end: u32,
    pub bss_start: u32,
    pub bss_end: u32,
    pub errors: Vec<String>,
}

struct Layout {
    labels: HashMap<String, i64>,
    long: HashSet<(usize, usize)>,
}

pub struct Assembler {
    pub table: OpTable,
}

/// 命令の説明（リスティング用。17.7節）
pub fn explain(m: &str) -> &'static str {
    match m {
        "lda" => "A に読む",
        "ldx" => "X に読む",
        "ldy" => "Y に読む",
        "sta" => "A を書く",
        "stx" => "X を書く",
        "sty" => "Y を書く",
        "stz" => "0 を書く",
        "adc" => "キャリーとともに足す",
        "sbc" => "借りとともに引く",
        "and" => "AND を取る",
        "ora" => "OR を取る",
        "eor" => "XOR を取る",
        "cmp" => "A と比べる",
        "cpx" => "X と比べる",
        "cpy" => "Y と比べる",
        "bit" => "ビットを調べる（N・V・Z）",
        "inc" => "1 増やす",
        "dec" => "1 減らす",
        "inx" => "X を 1 増やす",
        "iny" => "Y を 1 増やす",
        "dex" => "X を 1 減らす",
        "dey" => "Y を 1 減らす",
        "asl" => "左に 1 ビットずらす",
        "lsr" => "右に 1 ビットずらす",
        "rol" => "キャリーを通して左に回す",
        "ror" => "キャリーを通して右に回す",
        "tax" => "A を X に移す",
        "tay" => "A を Y に移す",
        "txa" => "X を A に移す",
        "tya" => "Y を A に移す",
        "tsx" => "スタックポインタを X に移す",
        "txs" => "X をスタックポインタに移す",
        "pha" => "A を積む",
        "phx" => "X を積む",
        "phy" => "Y を積む",
        "php" => "フラグを積む",
        "pla" => "A を戻す",
        "plx" => "X を戻す",
        "ply" => "Y を戻す",
        "plp" => "フラグを戻す",
        "clc" => "キャリーを 0 にする",
        "sec" => "キャリーを 1 にする",
        "cli" => "割り込みを許可する",
        "sei" => "割り込みを止める",
        "cld" => "10進モードを解除する",
        "sed" => "10進モードにする",
        "clv" => "V を 0 にする",
        "bpl" => "正（N=0）なら飛ぶ",
        "bmi" => "負（N=1）なら飛ぶ",
        "bvc" => "V=0 なら飛ぶ",
        "bvs" => "V=1 なら飛ぶ",
        "bcc" => "キャリー 0 なら飛ぶ",
        "bcs" => "キャリー 1 なら飛ぶ",
        "bne" => "等しくなければ飛ぶ",
        "beq" => "等しければ飛ぶ",
        "bra" => "飛ぶ",
        "jmp" => "飛ぶ",
        "jsr" => "呼ぶ",
        "rts" => "戻る",
        "rti" => "割り込みから戻る",
        "brk" => "ソフトウェア割り込み",
        "nop" => "何もしない",
        "wai" => "割り込みを待つ",
        "stp" => "止まる",
        "tsb" => "ビットを立てる",
        "trb" => "ビットを消す",
        _ => "",
    }
}

impl Assembler {
    pub fn new() -> Assembler {
        Assembler { table: OpTable::new() }
    }

    fn eval(&self, e: &OpExpr, lay: &Layout, known: &HashMap<String, i64>) -> Option<i64> {
        let base = match &e.sym {
            Some(s) => match known.get(s) {
                Some(v) => *v,
                None => *lay.labels.get(s)?,
            },
            None => 0,
        };
        let v = base + e.off;
        Some(match e.part {
            Part::Full => v,
            Part::Lo => v & 0xFF,
            Part::Hi => (v >> 8) & 0xFF,
        })
    }

    fn early(&self, e: &OpExpr, known: &HashMap<String, i64>) -> Option<i64> {
        let base = match &e.sym {
            Some(s) => *known.get(s)?,
            None => 0,
        };
        Some(base + e.off)
    }

    /// 命令の実際のアドレッシングモード（ゼロページか絶対か）
    pub fn choose_mode(&self, ins: &Ins, known: &HashMap<String, i64>) -> Mode {
        let m = ins.mn.as_str();
        match &ins.am {
            AMode::Imp => Mode::Imp,
            AMode::Acc => Mode::Acc,
            AMode::Imm(_) => Mode::Imm,
            AMode::Rel(_) => Mode::Rel,
            AMode::Mem(e) => {
                if m == "jmp" || m == "jsr" {
                    return Mode::Abs;
                }
                match self.early(e, known) {
                    Some(v) if (0..256).contains(&v) && self.table.has(m, Mode::Zp) => Mode::Zp,
                    _ => Mode::Abs,
                }
            }
            AMode::MemX(e) => match self.early(e, known) {
                Some(v) if (0..256).contains(&v) && self.table.has(m, Mode::Zpx) => Mode::Zpx,
                _ => Mode::Abx,
            },
            AMode::MemY(e) => match self.early(e, known) {
                Some(v) if (0..256).contains(&v) && self.table.has(m, Mode::Zpy) => Mode::Zpy,
                _ => Mode::Aby,
            },
            AMode::IndY(_) => Mode::Izy,
            AMode::IndX(_) => Mode::Izx,
            AMode::Ind(_) => {
                if m == "jmp" {
                    Mode::Ind
                } else {
                    Mode::Izp
                }
            }
            AMode::IndAbsX(_) => Mode::Iax,
        }
    }

    fn ins_size(&self, ins: &Ins, known: &HashMap<String, i64>, long: bool, nmos: bool) -> u32 {
        if let AMode::Rel(_) = ins.am {
            if long || (nmos && ins.mn == "bra") {
                return if ins.mn == "bra" { 3 } else { 5 };
            }
            return 2;
        }
        mode_size(self.choose_mode(ins, known))
    }

    fn sections<'a>(&self, inp: &'a AsmInput) -> Vec<(u32, &'a Vec<Item>, bool)> {
        // (開始アドレス、項目、続けて置くか)
        let mut v: Vec<(u32, &'a Vec<Item>, bool)> = vec![(inp.org, &inp.code, false), (0, &inp.data, true)];
        for (a, items) in &inp.fixed {
            v.push((*a, items, false));
        }
        v
    }

    fn layout_pass(&self, inp: &AsmInput, lay: &mut Layout) -> (u32, u32, u32, u32) {
        lay.labels.clear();
        let mut code_end = inp.org;
        let mut data_end = inp.org;
        let secs = self.sections(inp);
        let mut pc = inp.org;
        for (si, (start, items, cont)) in secs.iter().enumerate() {
            if !*cont {
                pc = *start;
            }
            for (ii, it) in items.iter().enumerate() {
                match it {
                    Item::Label(n) => {
                        lay.labels.insert(n.clone(), pc as i64);
                    }
                    Item::Ins(ins) => {
                        let long = lay.long.contains(&(si, ii));
                        pc += self.ins_size(ins, &inp.known, long, inp.nmos);
                    }
                    Item::Bytes(b, _) => pc += b.len() as u32,
                    Item::Exprs(v, _) => pc += v.len() as u32,
                    _ => {}
                }
            }
            if si == 0 {
                code_end = pc;
            }
            if si == 1 {
                data_end = pc;
            }
        }
        let bss_start = inp.bss_org.unwrap_or(data_end);
        let mut b = bss_start;
        for it in &inp.bss {
            match it {
                Item::Label(n) => {
                    lay.labels.insert(n.clone(), b as i64);
                }
                Item::Res(n, size, _) => {
                    lay.labels.insert(n.clone(), b as i64);
                    b += size;
                }
                _ => {}
            }
        }
        (code_end, data_end, bss_start, b)
    }

    pub fn assemble(&self, inp: &AsmInput) -> AsmOutput {
        let mut lay = Layout { labels: HashMap::new(), long: HashSet::new() };
        let mut errors = Vec::new();
        let mut ends = (0, 0, 0, 0);
        for _iter in 0..64 {
            ends = self.layout_pass(inp, &mut lay);
            // 届かない分岐を書き換える
            let mut changed = false;
            let secs = self.sections(inp);
            let mut pc = inp.org;
            for (si, (start, items, cont)) in secs.iter().enumerate() {
                if !*cont {
                    pc = *start;
                }
                for (ii, it) in items.iter().enumerate() {
                    if let Item::Ins(ins) = it {
                        let long = lay.long.contains(&(si, ii));
                        let size = self.ins_size(ins, &inp.known, long, inp.nmos);
                        if let AMode::Rel(l) = &ins.am {
                            if !long && !(inp.nmos && ins.mn == "bra") {
                                if let Some(t) = lay.labels.get(l) {
                                    let d = *t - (pc as i64 + 2);
                                    if !(-128..=127).contains(&d) {
                                        lay.long.insert((si, ii));
                                        changed = true;
                                    }
                                }
                            }
                        }
                        pc += size;
                    } else if let Item::Bytes(b, _) = it {
                        pc += b.len() as u32;
                    } else if let Item::Exprs(v, _) = it {
                        pc += v.len() as u32;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        // 出力
        let mut image = BTreeMap::new();
        let mut listing = Vec::new();
        let secs = self.sections(inp);
        let mut pc = inp.org;
        for (si, (start, items, cont)) in secs.iter().enumerate() {
            if !*cont {
                pc = *start;
            }
            for (ii, it) in items.iter().enumerate() {
                match it {
                    Item::Label(n) => listing.push(ListLine {
                        addr: Some(pc),
                        bytes: Vec::new(),
                        cycles: String::new(),
                        text: format!("{}:", n),
                        comment: String::new(),
                        is_src: false,
                    }),
                    Item::Src(s) => listing.push(ListLine { addr: None, bytes: Vec::new(), cycles: String::new(), text: String::new(), comment: s.clone(), is_src: true }),
                    Item::Comment(s) => listing.push(ListLine { addr: None, bytes: Vec::new(), cycles: String::new(), text: String::new(), comment: s.clone(), is_src: true }),
                    Item::Mark(_) | Item::Res(_, _, _) => {}
                    Item::Bytes(b, c) => {
                        for (k, x) in b.iter().enumerate() {
                            image.insert(pc + k as u32, *x);
                        }
                        for (k, chunk) in b.chunks(8).enumerate() {
                            let txt: Vec<String> = chunk.iter().map(|x| format!("${:02X}", x)).collect();
                            listing.push(ListLine {
                                addr: Some(pc + (k * 8) as u32),
                                bytes: chunk.to_vec(),
                                cycles: String::new(),
                                text: format!(".byte {}", txt.join(",")),
                                comment: if k == 0 { c.clone() } else { String::new() },
                                is_src: false,
                            });
                        }
                        pc += b.len() as u32;
                    }
                    Item::Exprs(v, c) => {
                        let mut bytes = Vec::new();
                        for e in v {
                            match self.eval(e, &lay, &inp.known) {
                                Some(x) => bytes.push((x & 0xFF) as u8),
                                None => {
                                    errors.push(format!("未定義の記号 {}", e.text()));
                                    bytes.push(0);
                                }
                            }
                        }
                        for (k, x) in bytes.iter().enumerate() {
                            image.insert(pc + k as u32, *x);
                        }
                        let txt: Vec<String> = v.iter().map(|e| e.text()).collect();
                        listing.push(ListLine { addr: Some(pc), bytes: bytes.clone(), cycles: String::new(), text: format!(".byte {}", txt.join(",")), comment: c.clone(), is_src: false });
                        pc += bytes.len() as u32;
                    }
                    Item::Ins(ins) => {
                        let long = lay.long.contains(&(si, ii));
                        let (bytes, lines) = self.encode(ins, pc, &lay, &inp.known, long, inp.nmos, &mut errors);
                        for (k, x) in bytes.iter().enumerate() {
                            image.insert(pc + k as u32, *x);
                        }
                        let mut a = pc;
                        for l in lines {
                            let n = l.bytes.len() as u32;
                            listing.push(ListLine { addr: Some(a), ..l });
                            a += n;
                        }
                        pc += bytes.len() as u32;
                    }
                }
            }
        }
        // 変数の領域（初期値のない領域。イメージには含めず、リスティングに番地と大きさを示す）
        let mut b = ends.2;
        if inp.bss.iter().any(|it| matches!(it, Item::Res(_, _, _))) {
            listing.push(ListLine {
                addr: None,
                bytes: Vec::new(),
                cycles: String::new(),
                text: String::new(),
                comment: "; 変数の領域（初期値なし。読み込むファイルには含まない）".to_string(),
                is_src: true,
            });
        }
        for it in &inp.bss {
            match it {
                Item::Label(n) => listing.push(ListLine {
                    addr: Some(b),
                    bytes: Vec::new(),
                    cycles: String::new(),
                    text: format!("{}:", n),
                    comment: String::new(),
                    is_src: false,
                }),
                Item::Res(n, size, c) => {
                    listing.push(ListLine {
                        addr: Some(b),
                        bytes: Vec::new(),
                        cycles: String::new(),
                        text: format!("{}: .res {}", n, size),
                        comment: c.clone(),
                        is_src: false,
                    });
                    b += size;
                }
                _ => {}
            }
        }
        let mut symbols = BTreeMap::new();
        for (k, v) in &lay.labels {
            symbols.insert(k.clone(), *v as u32);
        }
        for (k, v) in &inp.known {
            symbols.insert(k.clone(), *v as u32);
        }
        AsmOutput {
            image,
            symbols,
            listing,
            code_start: inp.org,
            code_end: ends.0,
            data_end: ends.1,
            bss_start: ends.2,
            bss_end: ends.3,
            errors,
        }
    }

    pub fn cycles_text(&self, m: &str, mode: Mode, info: &OpInfo) -> String {
        if mode == Mode::Rel {
            if m == "bra" {
                return "3".to_string();
            }
            return format!("{}/{}", info.cycles, info.cycles + 1);
        }
        let stores = matches!(m, "sta" | "stz") || matches!(m, "inc" | "dec" | "asl" | "lsr" | "rol" | "ror");
        if info.page && !stores {
            format!("{}*", info.cycles)
        } else {
            format!("{}", info.cycles)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn encode(
        &self,
        ins: &Ins,
        pc: u32,
        lay: &Layout,
        known: &HashMap<String, i64>,
        long: bool,
        nmos: bool,
        errors: &mut Vec<String>,
    ) -> (Vec<u8>, Vec<ListLine>) {
        let m = ins.mn.as_str();
        let comment = if ins.comment.is_empty() { explain(m).to_string() } else { ins.comment.clone() };
        if let AMode::Rel(l) = &ins.am {
            let target = match lay.labels.get(l) {
                Some(t) => *t,
                None => {
                    errors.push(format!("未定義のラベル {}", l));
                    pc as i64
                }
            };
            if (nmos && m == "bra") || (long && m == "bra") {
                let info = self.table.get("jmp", Mode::Abs).unwrap();
                let b = vec![info.code, (target & 0xFF) as u8, ((target >> 8) & 0xFF) as u8];
                let line = ListLine { addr: None, bytes: b.clone(), cycles: "3".to_string(), text: format!("jmp {}", l), comment, is_src: false };
                return (b, vec![line]);
            }
            if long {
                // 逆の条件で jmp を飛び越す
                let inv = invert_branch(m);
                let bi = self.table.get(inv, Mode::Rel).unwrap();
                let ji = self.table.get("jmp", Mode::Abs).unwrap();
                let b = vec![bi.code, 3, ji.code, (target & 0xFF) as u8, ((target >> 8) & 0xFF) as u8];
                let l1 = ListLine { addr: None, bytes: b[..2].to_vec(), cycles: format!("{}/{}", bi.cycles, bi.cycles + 1), text: format!("{} *+5", inv), comment: format!("{}（遠いので書き換えた）", comment), is_src: false };
                let l2 = ListLine { addr: None, bytes: b[2..].to_vec(), cycles: "3".to_string(), text: format!("jmp {}", l), comment: String::new(), is_src: false };
                return (b, vec![l1, l2]);
            }
            let info = match self.table.get(m, Mode::Rel) {
                Some(i) => i,
                None => {
                    errors.push(format!("{} に相対アドレスはありません", m));
                    return (vec![0xEA, 0xEA], Vec::new());
                }
            };
            let d = target - (pc as i64 + 2);
            if !(-128..=127).contains(&d) {
                errors.push(format!("分岐 {} {} が届きません", m, l));
            }
            let b = vec![info.code, (d & 0xFF) as u8];
            let line = ListLine { addr: None, bytes: b.clone(), cycles: self.cycles_text(m, Mode::Rel, &info), text: ins.text(), comment, is_src: false };
            return (b, vec![line]);
        }
        let mode = self.choose_mode(ins, known);
        let info = match self.table.get(m, mode) {
            Some(i) => i,
            None => {
                errors.push(format!("命令 {} にこのアドレッシング（{}）はありません", m, ins.text()));
                return (vec![0xEA], Vec::new());
            }
        };
        if nmos && is_cmos_only(m, mode) {
            errors.push(format!("{} は NMOS 6502 にない命令です", ins.text()));
        }
        let mut b = vec![info.code];
        if let Some(e) = ins.expr() {
            let v = match self.eval(e, lay, known) {
                Some(v) => v,
                None => {
                    errors.push(format!("未定義の記号 {}", e.text()));
                    0
                }
            };
            match mode_size(mode) {
                2 => {
                    if matches!(mode, Mode::Zp | Mode::Zpx | Mode::Zpy | Mode::Izx | Mode::Izy | Mode::Izp) && !(0..256).contains(&v) {
                        errors.push(format!("{} のオペランドはゼロページでなければなりません", ins.text()));
                    }
                    b.push((v & 0xFF) as u8);
                }
                3 => {
                    b.push((v & 0xFF) as u8);
                    b.push(((v >> 8) & 0xFF) as u8);
                }
                _ => {}
            }
        }
        let line = ListLine { addr: None, bytes: b.clone(), cycles: self.cycles_text(m, mode, &info), text: ins.text(), comment, is_src: false };
        (b, vec![line])
    }

    /// 命令のサイクル数（分岐は飛ばない場合）
    pub fn ins_cycles(&self, ins: &Ins, known: &HashMap<String, i64>) -> (u32, bool) {
        let mode = self.choose_mode(ins, known);
        match self.table.get(&ins.mn, mode) {
            Some(i) => (i.cycles as u32, i.page),
            None => (2, false),
        }
    }
}

/// リスティングの 1 行の書式（17.7節）
pub fn format_list_line(l: &ListLine, plain: bool) -> String {
    if l.is_src {
        return format!("{:25}{}", "", l.comment);
    }
    let addr = match l.addr {
        Some(a) => format!("{:04X}", a),
        None => "    ".to_string(),
    };
    let bytes: Vec<String> = l.bytes.iter().map(|b| format!("{:02X}", b)).collect();
    let bs = bytes.join(" ");
    if l.text.ends_with(':') && l.bytes.is_empty() {
        return format!("{}  {:10} {:6} {}", addr, "", "", l.text);
    }
    let mut s = format!("{}  {:10} {:6} {:16}", addr, bs, l.cycles, l.text);
    if !plain && !l.comment.is_empty() {
        s.push_str(" ; ");
        s.push_str(&l.comment);
    }
    s.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asm(code: Vec<Item>) -> AsmOutput {
        let a = Assembler::new();
        let inp = AsmInput { code, data: Vec::new(), bss: Vec::new(), known: HashMap::new(), org: 0x300, bss_org: None, nmos: false, fixed: Vec::new() };
        a.assemble(&inp)
    }

    fn bytes(o: &AsmOutput) -> Vec<u8> {
        o.image.values().cloned().collect()
    }

    #[test]
    fn basic_encoding() {
        let o = asm(vec![
            Item::Ins(Ins::new("sei", AMode::Imp, "")),
            Item::Ins(Ins::new("lda", AMode::Imm(OpExpr::num(10)), "")),
            Item::Ins(Ins::new("sta", AMode::Mem(OpExpr::num(0x30)), "")),
            Item::Ins(Ins::new("sta", AMode::Mem(OpExpr::num(0x6004)), "")),
            Item::Ins(Ins::new("lda", AMode::IndY(OpExpr::num(0)), "")),
        ]);
        assert!(o.errors.is_empty(), "{:?}", o.errors);
        assert_eq!(bytes(&o), vec![0x78, 0xA9, 0x0A, 0x85, 0x30, 0x8D, 0x04, 0x60, 0xB1, 0x00]);
    }

    #[test]
    fn labels_and_branches() {
        let o = asm(vec![
            Item::Label("top".to_string()),
            Item::Ins(Ins::new("dex", AMode::Imp, "")),
            Item::Ins(Ins::new("bne", AMode::Rel("top".to_string()), "")),
            Item::Ins(Ins::new("jsr", AMode::Mem(OpExpr::sym("top")), "")),
        ]);
        assert!(o.errors.is_empty());
        assert_eq!(bytes(&o), vec![0xCA, 0xD0, 0xFD, 0x20, 0x00, 0x03]);
    }

    #[test]
    fn long_branch_relaxation() {
        let mut code = vec![Item::Ins(Ins::new("beq", AMode::Rel("far".to_string()), ""))];
        code.push(Item::Bytes(vec![0xEA; 200], String::new()));
        code.push(Item::Label("far".to_string()));
        code.push(Item::Ins(Ins::new("rts", AMode::Imp, "")));
        let o = asm(code);
        assert!(o.errors.is_empty(), "{:?}", o.errors);
        let b = bytes(&o);
        assert_eq!(b[0], 0xD0); // bne *+5
        assert_eq!(b[1], 3);
        assert_eq!(b[2], 0x4C); // jmp far
        let far = 0x300 + 5 + 200;
        assert_eq!(b[3] as u32 | ((b[4] as u32) << 8), far);
    }

    #[test]
    fn lo_hi_parts() {
        let o = asm(vec![
            Item::Label("x".to_string()),
            Item::Ins(Ins::new("lda", AMode::Imm(OpExpr::sym("x").lo()), "")),
            Item::Ins(Ins::new("ldx", AMode::Imm(OpExpr::sym("x").hi()), "")),
        ]);
        assert_eq!(bytes(&o), vec![0xA9, 0x00, 0xA2, 0x03]);
    }
}
