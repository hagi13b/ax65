// 型（仕様書 3章）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntK {
    U8,
    I8,
    U16,
    I16,
    U24,
    I24,
    U32,
    I32,
}

impl IntK {
    pub fn size(&self) -> u32 {
        match self {
            IntK::U8 | IntK::I8 => 1,
            IntK::U16 | IntK::I16 => 2,
            IntK::U24 | IntK::I24 => 3,
            IntK::U32 | IntK::I32 => 4,
        }
    }
    pub fn signed(&self) -> bool {
        matches!(self, IntK::I8 | IntK::I16 | IntK::I24 | IntK::I32)
    }
    pub fn name(&self) -> &'static str {
        match self {
            IntK::U8 => "u8",
            IntK::I8 => "i8",
            IntK::U16 => "u16",
            IntK::I16 => "i16",
            IntK::U24 => "u24",
            IntK::I24 => "i24",
            IntK::U32 => "u32",
            IntK::I32 => "i32",
        }
    }
    pub fn min(&self) -> i128 {
        if self.signed() {
            -(1i128 << (self.size() * 8 - 1))
        } else {
            0
        }
    }
    pub fn max(&self) -> i128 {
        if self.signed() {
            (1i128 << (self.size() * 8 - 1)) - 1
        } else {
            (1i128 << (self.size() * 8)) - 1
        }
    }
    /// 一周して戻す（ビットの並びを保ったまま範囲に入れる）
    pub fn wrap(&self, v: i128) -> i128 {
        let bits = self.size() * 8;
        let m = 1i128 << bits;
        let mut x = v.rem_euclid(m);
        if self.signed() && x >= (m >> 1) {
            x -= m;
        }
        x
    }
    pub fn from_name(s: &str) -> Option<IntK> {
        match s {
            "u8" => Some(IntK::U8),
            "i8" => Some(IntK::I8),
            "u16" => Some(IntK::U16),
            "i16" => Some(IntK::I16),
            "u24" => Some(IntK::U24),
            "i24" => Some(IntK::I24),
            "u32" => Some(IntK::U32),
            "i32" => Some(IntK::I32),
            _ => None,
        }
    }
    pub fn unsigned_of_size(n: u32) -> IntK {
        match n {
            1 => IntK::U8,
            2 => IntK::U16,
            3 => IntK::U24,
            _ => IntK::U32,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Len {
    N(u32),
    R(usize),
    E(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub enum StrK {
    P,
    Z,
    Hi,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Unit,
    Never,
    Bool,
    Error,
    Int(IntK),
    Bcd(u32),
    Char(Option<String>),
    Range(usize),
    Enum(usize),
    Flags(usize),
    Struct(usize),
    FnSet(usize),
    Array(Box<Ty>, Len),
    Str(StrK, Option<String>),
    StrBuf(u32),
    Ptr(Box<Ty>),
    Opt(Box<Ty>),
    CInt,
    CReal,
    IntLit,
    FnItem(usize),
    TypeName(Box<Ty>),
    RangeIter(Box<Ty>),
    Cpu,
}

#[derive(Clone, Debug)]
pub struct StructDef {
    pub name: String,
    pub module: usize,
    pub fields: Vec<(String, Ty)>,
    pub packed: bool,
}

#[derive(Clone, Debug)]
pub struct EnumDef {
    pub name: String,
    pub module: usize,
    pub variants: Vec<(String, i128)>,
}

impl EnumDef {
    /// 0 から連続した値か（配列の添字にできるか）
    pub fn contiguous(&self) -> bool {
        let mut vals: Vec<i128> = self.variants.iter().map(|v| v.1).collect();
        vals.sort();
        for (i, v) in vals.iter().enumerate() {
            if *v != i as i128 {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Debug)]
pub struct RangeDef {
    pub name: String,
    pub module: usize,
    pub lo: i128,
    pub hi: i128, // 含まない上限
}

impl RangeDef {
    pub fn count(&self) -> i128 {
        self.hi - self.lo
    }
    pub fn base_size(&self) -> u32 {
        if self.hi - 1 > 255 || self.lo < 0 {
            2
        } else {
            1
        }
    }
}

#[derive(Clone, Debug)]
pub struct FlagsDef {
    pub name: String,
    pub module: usize,
    pub names: Vec<String>,
}

impl FlagsDef {
    pub fn size(&self) -> u32 {
        if self.names.len() > 8 {
            2
        } else {
            1
        }
    }
}

#[derive(Clone, Debug)]
pub struct FnSetDef {
    pub name: String,
    pub module: usize,
    pub fns: Vec<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct TypeTable {
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub ranges: Vec<RangeDef>,
    pub flags: Vec<FlagsDef>,
    pub fnsets: Vec<FnSetDef>,
}

impl TypeTable {
    pub fn len_count(&self, l: &Len) -> u32 {
        match l {
            Len::N(n) => *n,
            Len::R(r) => self.ranges[*r].count().max(0) as u32,
            Len::E(e) => self.enums[*e].variants.len() as u32,
        }
    }

    pub fn size_of(&self, t: &Ty) -> u32 {
        match t {
            Ty::Unit | Ty::Never | Ty::Error => 0,
            Ty::Bool => 1,
            Ty::Int(k) => k.size(),
            Ty::Bcd(n) => *n,
            Ty::Char(_) => 1,
            Ty::Range(r) => self.ranges[*r].base_size(),
            Ty::Enum(_) => 1,
            Ty::Flags(f) => self.flags[*f].size(),
            Ty::Struct(s) => self.structs[*s].fields.iter().map(|f| self.size_of(&f.1)).sum(),
            Ty::FnSet(_) => 1,
            Ty::Array(e, l) => self.size_of(e) * self.len_count(l),
            Ty::Str(_, _) => 2,
            Ty::StrBuf(n) => n + 1,
            Ty::Ptr(_) => 2,
            Ty::Opt(inner) => self.size_of(inner) + 1,
            Ty::CInt | Ty::CReal | Ty::IntLit => 1,
            Ty::FnItem(_) => 1,
            Ty::TypeName(_) | Ty::RangeIter(_) | Ty::Cpu => 0,
        }
    }

    /// 構造体のフィールドの位置（バイト）と型
    pub fn field(&self, sid: usize, name: &str) -> Option<(u32, Ty)> {
        let mut off = 0;
        for (n, t) in &self.structs[sid].fields {
            if n == name {
                return Some((off, t.clone()));
            }
            off += self.size_of(t);
        }
        None
    }

    pub fn name(&self, t: &Ty) -> String {
        match t {
            Ty::Unit => "()".to_string(),
            Ty::Never => "!".to_string(),
            Ty::Bool => "bool".to_string(),
            Ty::Error => "{エラー}".to_string(),
            Ty::Int(k) => k.name().to_string(),
            Ty::Bcd(n) => format!("bcd{}", n * 8),
            Ty::Char(None) => "char".to_string(),
            Ty::Char(Some(e)) => format!("char@{}", e),
            Ty::Range(r) => self.ranges[*r].name.clone(),
            Ty::Enum(e) => self.enums[*e].name.clone(),
            Ty::Flags(f) => self.flags[*f].name.clone(),
            Ty::Struct(s) => self.structs[*s].name.clone(),
            Ty::FnSet(f) => self.fnsets[*f].name.clone(),
            Ty::Array(e, l) => {
                let ln = match l {
                    Len::N(n) => format!("{}", n),
                    Len::R(r) => self.ranges[*r].name.clone(),
                    Len::E(x) => self.enums[*x].name.clone(),
                };
                format!("[{}; {}]", self.name(e), ln)
            }
            Ty::Str(k, enc) => {
                let b = match k {
                    StrK::P => "str",
                    StrK::Z => "zstr",
                    StrK::Hi => "histr",
                };
                match enc {
                    Some(e) => format!("{}@{}", b, e),
                    None => b.to_string(),
                }
            }
            Ty::StrBuf(n) => format!("strbuf<{}>", n),
            Ty::Ptr(i) => format!("ptr<{}>", self.name(i)),
            Ty::Opt(i) => format!("Option<{}>", self.name(i)),
            Ty::CInt => "int".to_string(),
            Ty::CReal => "real".to_string(),
            Ty::IntLit => "{整数}".to_string(),
            Ty::FnItem(_) => "関数".to_string(),
            Ty::TypeName(t) => format!("型 {}", self.name(t)),
            Ty::RangeIter(t) => format!("範囲（{}）", self.name(t)),
            Ty::Cpu => "Cpu".to_string(),
        }
    }

    /// 値の取りうる個数（1 バイトの型の網羅性の検査に使う）
    pub fn value_set(&self, t: &Ty) -> Option<Vec<i128>> {
        match t {
            Ty::Bool => Some(vec![0, 1]),
            Ty::Enum(e) => Some(self.enums[*e].variants.iter().map(|v| v.1).collect()),
            Ty::Range(r) => {
                let d = &self.ranges[*r];
                if d.count() <= 4096 {
                    Some((d.lo..d.hi).collect())
                } else {
                    None
                }
            }
            Ty::Int(IntK::U8) | Ty::Char(_) | Ty::Bcd(1) => Some((0..256).collect()),
            Ty::Int(IntK::I8) => Some((-128..128).collect()),
            _ => None,
        }
    }

    pub fn is_integer(&self, t: &Ty) -> bool {
        matches!(t, Ty::Int(_) | Ty::IntLit | Ty::CInt)
    }

    /// 1 バイトで表せる、添字や比較に使える値の型
    pub fn is_byte_value(&self, t: &Ty) -> bool {
        self.size_of(t) == 1
            && matches!(t, Ty::Int(_) | Ty::Char(_) | Ty::Range(_) | Ty::Enum(_) | Ty::Flags(_) | Ty::Bool | Ty::Bcd(_) | Ty::FnSet(_))
    }

    pub fn is_signed(&self, t: &Ty) -> bool {
        matches!(t, Ty::Int(k) if k.signed())
    }

    /// Option の値をレジスタで返せるか（1〜3 バイト）
    pub fn reg_bytes(&self, t: &Ty) -> u32 {
        self.size_of(t)
    }
}
