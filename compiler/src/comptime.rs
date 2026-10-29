// コンパイル時の計算（13章）
use crate::ast::*;
use crate::diag::Span;
use crate::lexer::{CharUnit, StrPrefix};
use crate::program::{CfgVal, Ctx, GKind, PathRes};
use crate::types::{IntK, StrK, Ty};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i128, Option<IntK>),
    Real(f64),
    Char(u8, Option<String>),
    Bcd(u64, u32),
    Enum(usize, i128),
    Range(usize, i128),
    Flags(usize, i128),
    Array(Vec<Value>),
    Struct(usize, Vec<Value>),
    Str(StrK, Vec<u8>, Option<String>),
    Opt(Option<Box<Value>>),
    Func(usize),
    Cpu(String),
}

#[derive(Clone, Debug)]
pub struct EvalErr {
    pub code: String,
    pub msg: String,
    pub span: Span,
}

pub fn ev_err(code: &str, msg: &str, sp: &Span) -> EvalErr {
    EvalErr { code: code.to_string(), msg: msg.to_string(), span: sp.clone() }
}

pub type EResult<T> = Result<T, EvalErr>;

enum Flow {
    Val(Value),
    Break(Option<String>, Value),
    Continue(Option<String>),
    Return(Value),
}

pub struct Env {
    scopes: Vec<HashMap<String, Value>>,
    pub module: usize,
    pub fn_ret: Option<Ty>,
}

impl Env {
    pub fn new(module: usize) -> Env {
        Env { scopes: vec![HashMap::new()], module, fn_ret: None }
    }
    fn get(&self, n: &str) -> Option<Value> {
        for s in self.scopes.iter().rev() {
            if let Some(v) = s.get(n) {
                return Some(v.clone());
            }
        }
        None
    }
    fn set(&mut self, n: &str, v: Value) -> bool {
        for s in self.scopes.iter_mut().rev() {
            if s.contains_key(n) {
                s.insert(n.to_string(), v);
                return true;
            }
        }
        false
    }
    fn define(&mut self, n: &str, v: Value) {
        if let Some(s) = self.scopes.last_mut() {
            s.insert(n.to_string(), v);
        }
    }
    fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }
    fn pop(&mut self) {
        self.scopes.pop();
    }
}

pub fn bcd_encode(d: u64, bytes: u32) -> u64 {
    let mut out = 0u64;
    let mut x = d;
    for k in 0..(bytes * 2) {
        let digit = x % 10;
        x /= 10;
        out |= digit << (4 * k);
    }
    out
}

pub fn bcd_modulus(bytes: u32) -> u64 {
    10u64.pow(bytes * 2)
}

/// 値の整数としての中身（列挙型・範囲型・文字・bool なども）
pub fn value_int(v: &Value) -> Option<i128> {
    match v {
        Value::Int(i, _) => Some(*i),
        Value::Bool(b) => Some(if *b { 1 } else { 0 }),
        Value::Char(c, _) => Some(*c as i128),
        Value::Enum(_, x) | Value::Range(_, x) | Value::Flags(_, x) => Some(*x),
        Value::Bcd(d, n) => Some(bcd_encode(*d, *n) as i128),
        Value::Func(i) => Some(*i as i128),
        _ => None,
    }
}

pub fn cfg_to_value(c: &CfgVal) -> Value {
    match c {
        CfgVal::Int(i) => Value::Int(*i, None),
        CfgVal::Bool(b) => Value::Bool(*b),
        CfgVal::Str(s) => Value::Str(StrK::P, s.as_bytes().to_vec(), None),
        CfgVal::Cpu(s) => Value::Cpu(s.clone()),
    }
}

impl Ctx {
    fn step(&mut self, sp: &Span) -> EResult<()> {
        self.steps += 1;
        if self.steps > self.step_limit {
            return Err(ev_err(
                "E1003",
                &format!("コンパイル時の計算の手順数が上限（{}）を超えました", self.step_limit),
                sp,
            ));
        }
        Ok(())
    }

    /// 文字の変換（3.3節・16.5節）
    pub fn encode_char(&mut self, u: &CharUnit, enc: &Option<String>, sp: &Span) -> EResult<u8> {
        match enc {
            None => match u {
                CharUnit::B(b) => Ok(*b),
                CharUnit::U(_) => Err(ev_err(
                    "E0106",
                    "ASCII 以外の文字は、変換表を付けた文字・文字列の中だけに書けます",
                    sp,
                )),
            },
            Some(e) => {
                let table = match self.target.encodings.get(e) {
                    Some(t) => t.clone(),
                    None => {
                        return Err(ev_err("E1106", &format!("ターゲット定義に変換表 {} がありません", e), sp));
                    }
                };
                let code = match u {
                    CharUnit::B(b) => *b as u32,
                    CharUnit::U(c) => *c as u32,
                };
                match table.get(&code) {
                    Some(v) => Ok(*v),
                    None => {
                        let shown = match u {
                            CharUnit::B(b) => format!("{}", *b as char),
                            CharUnit::U(c) => format!("{}", c),
                        };
                        Err(ev_err("E1106", &format!("文字 '{}' は変換表 {} にありません", shown, e), sp))
                    }
                }
            }
        }
    }

    pub fn string_value(&mut self, p: &StrPrefix, units: &[CharUnit], enc: &Option<String>, sp: &Span) -> EResult<Value> {
        let mut bytes = Vec::new();
        for u in units {
            bytes.push(self.encode_char(u, enc, sp)?);
        }
        match p {
            StrPrefix::Raw => Ok(Value::Array(bytes.iter().map(|b| Value::Char(*b, enc.clone())).collect())),
            StrPrefix::P => {
                if bytes.len() > 255 {
                    return Err(ev_err("E0510", "長さ付きの文字列は 255 文字までです", sp));
                }
                Ok(Value::Str(StrK::P, bytes, enc.clone()))
            }
            StrPrefix::Z => Ok(Value::Str(StrK::Z, bytes, enc.clone())),
            StrPrefix::Hi => {
                if bytes.is_empty() {
                    return Err(ev_err("E0510", "hi\"\" には 1 文字以上が必要です", sp));
                }
                Ok(Value::Str(StrK::Hi, bytes, enc.clone()))
            }
        }
    }

    /// 定数の値（キャッシュ付き）
    pub fn global_value(&mut self, gid: usize, sp: &Span) -> EResult<Value> {
        self.ensure_global(gid);
        match &self.globals[gid].value {
            Some(v) => Ok(v.clone()),
            None => Err(ev_err("E1001", &format!("{} の値をコンパイル時に計算できません", self.globals[gid].name), sp)),
        }
    }

    /// 値を型に合わせる（int・real からは範囲を検査、実行時の型同士は型が同じであること）
    pub fn convert_value(&mut self, v: Value, ty: &Ty, sp: &Span) -> EResult<Value> {
        match (v, ty) {
            (Value::Int(i, None), Ty::Int(k)) => {
                if i < k.min() || i > k.max() {
                    Err(ev_err("E0510", &format!("{} は {} に収まりません", i, k.name()), sp))
                } else {
                    Ok(Value::Int(i, Some(*k)))
                }
            }
            (Value::Int(i, Some(k1)), Ty::Int(k)) => {
                if k1 == *k {
                    Ok(Value::Int(i, Some(k1)))
                } else {
                    Err(ev_err("E0401", &format!("型が違います（{} の値を {} に入れています）", k1.name(), k.name()), sp))
                }
            }
            (Value::Int(i, None), Ty::Bcd(n)) => {
                let m = bcd_modulus(*n) as i128;
                if i < 0 || i >= m {
                    Err(ev_err("E0509", &format!("{} は bcd{} に収まりません", i, n * 8), sp))
                } else {
                    Ok(Value::Bcd(i as u64, *n))
                }
            }
            (Value::Bcd(d, n1), Ty::Bcd(n)) => {
                if n1 == *n {
                    Ok(Value::Bcd(d, n1))
                } else {
                    Err(ev_err("E0401", "BCD の大きさが違います", sp))
                }
            }
            (Value::Int(i, None), Ty::CInt) => Ok(Value::Int(i, None)),
            (Value::Int(i, Some(_)), Ty::CInt) => Ok(Value::Int(i, None)),
            (Value::Int(i, _), Ty::CReal) => Ok(Value::Real(i as f64)),
            (Value::Real(r), Ty::CReal) => Ok(Value::Real(r)),
            (Value::Int(i, None), Ty::Range(r)) => {
                let d = self.types.ranges[*r].clone();
                if i < d.lo || i >= d.hi {
                    Err(ev_err("E0510", &format!("{} は範囲型 {}（{}..{}）に収まりません", i, d.name, d.lo, d.hi), sp))
                } else {
                    Ok(Value::Range(*r, i))
                }
            }
            (Value::Range(r1, x), Ty::Range(r)) => {
                if r1 == *r {
                    Ok(Value::Range(r1, x))
                } else {
                    Err(ev_err("E0401", "範囲型が違います", sp))
                }
            }
            (Value::Enum(e1, x), Ty::Enum(e)) if e1 == *e => Ok(Value::Enum(e1, x)),
            (Value::Flags(f1, x), Ty::Flags(f)) if f1 == *f => Ok(Value::Flags(f1, x)),
            (Value::Bool(b), Ty::Bool) => Ok(Value::Bool(b)),
            (Value::Char(c, e1), Ty::Char(e)) => {
                if &e1 == e {
                    Ok(Value::Char(c, e1))
                } else {
                    Err(ev_err("E0401", "文字の変換表が違います", sp))
                }
            }
            (Value::Str(k1, b, e1), Ty::Str(k, e)) => {
                if &k1 == k && &e1 == e {
                    Ok(Value::Str(k1, b, e1))
                } else {
                    Err(ev_err("E0401", "文字列の型が違います", sp))
                }
            }
            (Value::Array(items), Ty::Array(et, l)) => {
                let n = self.types.len_count(l) as usize;
                if items.len() != n {
                    return Err(ev_err(
                        "E0508",
                        &format!("配列の値の個数（{}）が配列の長さ（{}）と違います", items.len(), n),
                        sp,
                    ));
                }
                let et2 = (**et).clone();
                let mut out = Vec::new();
                for x in items {
                    out.push(self.convert_value(x, &et2, sp)?);
                }
                Ok(Value::Array(out))
            }
            (Value::Struct(s1, f), Ty::Struct(s)) if s1 == *s => Ok(Value::Struct(s1, f)),
            (Value::Opt(None), Ty::Opt(_)) => Ok(Value::Opt(None)),
            (Value::Opt(Some(x)), Ty::Opt(t)) => {
                let t2 = (**t).clone();
                let x2 = self.convert_value(*x, &t2, sp)?;
                Ok(Value::Opt(Some(Box::new(x2))))
            }
            (Value::Func(f), Ty::FnSet(s)) => {
                let set = self.types.fnsets[*s].clone();
                if set.fns.contains(&f) {
                    Ok(Value::Func(f))
                } else {
                    Err(ev_err("E0602", &format!("{} は関数集合 {} にありません", self.funcs[f].name, set.name), sp))
                }
            }
            (Value::Int(i, Some(IntK::U8)), Ty::FnSet(_)) => Ok(Value::Int(i, Some(IntK::U8))),
            (Value::Int(i, None), Ty::Flags(f)) => {
                let size = self.types.flags[*f].size();
                let k = IntK::unsigned_of_size(size);
                if i < 0 || i > k.max() {
                    return Err(ev_err("E0510", "値がビットフラグ型に収まりません", sp));
                }
                Ok(Value::Flags(*f, i))
            }
            (Value::Cpu(c), Ty::Cpu) => Ok(Value::Cpu(c)),
            (Value::Unit, Ty::Unit) => Ok(Value::Unit),
            (v, t) => {
                let tn = self.types.name(t);
                Err(ev_err("E0401", &format!("値（{}）を型 {} に変換できません", short_value(&v), tn), sp))
            }
        }
    }

    /// as による変換（3.14節）
    pub fn cast_value(&mut self, v: Value, ty: &Ty, sp: &Span) -> EResult<Value> {
        match (&v, ty) {
            (Value::Int(i, None), Ty::Int(k)) => {
                if *i < k.min() || *i > k.max() {
                    return Err(ev_err("E1002", &format!("{} は {} に収まりません（int から実行時の型への変換）", i, k.name()), sp));
                }
                Ok(Value::Int(*i, Some(*k)))
            }
            (Value::Real(r), Ty::Int(k)) => {
                let t = r.trunc();
                if !t.is_finite() || t < k.min() as f64 || t > k.max() as f64 {
                    return Err(ev_err("E1002", &format!("{} は {} に収まりません（real から実行時の型への変換）", r, k.name()), sp));
                }
                Ok(Value::Int(t as i128, Some(*k)))
            }
            (Value::Real(r), Ty::CInt) => Ok(Value::Int(r.trunc() as i128, None)),
            (Value::Int(i, _), Ty::CReal) => Ok(Value::Real(*i as f64)),
            (Value::Real(r), Ty::CReal) => Ok(Value::Real(*r)),
            (Value::Int(i, _), Ty::CInt) => Ok(Value::Int(*i, None)),
            (Value::Int(i, Some(_)), Ty::Int(k)) => Ok(Value::Int(k.wrap(*i), Some(*k))),
            (Value::Bcd(_, _), Ty::Int(_)) | (Value::Int(_, _), Ty::Bcd(_)) => Err(ev_err(
                "E0411",
                "BCD と整数の間は as で変換できません（bcd::to_u16 などを使ってください）",
                sp,
            )),
            (Value::Enum(_, x), Ty::Int(k)) | (Value::Range(_, x), Ty::Int(k)) | (Value::Flags(_, x), Ty::Int(k)) => {
                Ok(Value::Int(k.wrap(*x), Some(*k)))
            }
            (Value::Enum(_, x), Ty::CInt) | (Value::Range(_, x), Ty::CInt) | (Value::Flags(_, x), Ty::CInt) => {
                Ok(Value::Int(*x, None))
            }
            (Value::Char(c, _), Ty::Int(k)) => Ok(Value::Int(k.wrap(*c as i128), Some(*k))),
            (Value::Char(c, _), Ty::CInt) => Ok(Value::Int(*c as i128, None)),
            (Value::Int(i, _), Ty::Char(e)) => Ok(Value::Char((*i & 0xFF) as u8, e.clone())),
            (Value::Char(c, _), Ty::Char(e)) => Ok(Value::Char(*c, e.clone())),
            (Value::Int(i, _), Ty::Flags(f)) => {
                let size = self.types.flags[*f].size();
                Ok(Value::Flags(*f, IntK::unsigned_of_size(size).wrap(*i)))
            }
            (Value::Bool(b), Ty::Int(k)) => Ok(Value::Int(if *b { 1 } else { 0 }, Some(*k))),
            (Value::Int(_, _), Ty::Range(_)) | (Value::Int(_, _), Ty::Enum(_)) => Err(ev_err(
                "E0410",
                "整数から範囲型・列挙型へは as で変換できません（check() などを使ってください）",
                sp,
            )),
            _ => {
                if let Ok(x) = self.convert_value(v.clone(), ty, sp) {
                    return Ok(x);
                }
                let tn = self.types.name(ty);
                Err(ev_err("E0419", &format!("{} を {} に変換できません", short_value(&v), tn), sp))
            }
        }
    }

    pub fn ct_binop(&mut self, op: BinOp, a: Value, b: Value, sp: &Span) -> EResult<Value> {
        use crate::ast::BinOp::*;
        // 比較
        if op.is_cmp() {
            let any_real = matches!(a, Value::Real(_)) || matches!(b, Value::Real(_));
            let r = match (&a, &b) {
                _ if any_real => {
                    let fx = real_of(&a).unwrap_or(0.0);
                    let fy = real_of(&b).unwrap_or(0.0);
                    match op {
                        Eq => fx == fy,
                        Ne => fx != fy,
                        Lt => fx < fy,
                        Gt => fx > fy,
                        Le => fx <= fy,
                        _ => fx >= fy,
                    }
                }
                (Value::Cpu(x), Value::Cpu(y)) => match op {
                    Eq => x == y,
                    Ne => x != y,
                    _ => return Err(ev_err("E0401", "Cpu は == と != だけで比べられます", sp)),
                },
                (Value::Str(_, x, _), Value::Str(_, y, _)) => match op {
                    Eq => x == y,
                    Ne => x != y,
                    _ => return Err(ev_err("E0401", "文字列は == と != だけで比べられます", sp)),
                },
                _ => {
                    let x = match &a {
                        Value::Bcd(d, _) => *d as i128,
                        _ => value_int(&a).ok_or_else(|| ev_err("E1001", "比べられない値です", sp))?,
                    };
                    let y = match &b {
                        Value::Bcd(d, _) => *d as i128,
                        _ => value_int(&b).ok_or_else(|| ev_err("E1001", "比べられない値です", sp))?,
                    };
                    match op {
                        Eq => x == y,
                        Ne => x != y,
                        Lt => x < y,
                        Gt => x > y,
                        Le => x <= y,
                        _ => x >= y,
                    }
                }
            };
            return Ok(Value::Bool(r));
        }
        match (&a, &b) {
            (Value::Bool(x), Value::Bool(y)) => {
                return match op {
                    LAnd => Ok(Value::Bool(*x && *y)),
                    LOr => Ok(Value::Bool(*x || *y)),
                    And => Ok(Value::Bool(*x & *y)),
                    Or => Ok(Value::Bool(*x | *y)),
                    Xor => Ok(Value::Bool(*x ^ *y)),
                    _ => Err(ev_err("E0401", "bool にこの演算は使えません", sp)),
                };
            }
            _ => {}
        }
        // 小数
        if matches!(a, Value::Real(_)) || matches!(b, Value::Real(_)) {
            let x = real_of(&a).ok_or_else(|| ev_err("E0401", "real の演算に使えない値です", sp))?;
            let y = real_of(&b).ok_or_else(|| ev_err("E0401", "real の演算に使えない値です", sp))?;
            let r = match op {
                Add => x + y,
                Sub => x - y,
                Mul => x * y,
                Div => x / y,
                Rem => x % y,
                _ => return Err(ev_err("E0401", "real にこの演算は使えません", sp)),
            };
            return Ok(Value::Real(r));
        }
        // BCD
        if let Value::Bcd(x, n) = &a {
            let y = match &b {
                Value::Bcd(y, _) => *y as i128,
                Value::Int(y, None) => *y,
                _ => return Err(ev_err("E0401", "BCD と他の型を混ぜて計算できません", sp)),
            };
            let m = bcd_modulus(*n) as i128;
            let r = match op {
                Add => (*x as i128 + y).rem_euclid(m),
                Sub => (*x as i128 - y).rem_euclid(m),
                _ => return Err(ev_err("E0401", "BCD に使える演算は + と - と比較だけです", sp)),
            };
            return Ok(Value::Bcd(r as u64, *n));
        }
        // ビットフラグ
        if let (Value::Flags(f, x), Value::Flags(_, y)) = (&a, &b) {
            let r = match op {
                Or => x | y,
                And => x & y,
                Xor => x ^ y,
                _ => return Err(ev_err("E0401", "ビットフラグに使える演算は | & ^ です", sp)),
            };
            return Ok(Value::Flags(*f, r));
        }
        // 整数
        let (x, kx) = match &a {
            Value::Int(i, k) => (*i, *k),
            Value::Range(_, i) | Value::Enum(_, i) => (*i, Some(IntK::U8)),
            Value::Char(c, _) => (*c as i128, Some(IntK::U8)),
            Value::Flags(_, i) => (*i, Some(IntK::U8)),
            _ => return Err(ev_err("E1001", &format!("{} にこの演算は使えません", short_value(&a)), sp)),
        };
        let (y, ky) = match &b {
            Value::Int(i, k) => (*i, *k),
            Value::Range(_, i) | Value::Enum(_, i) => (*i, Some(IntK::U8)),
            Value::Char(c, _) => (*c as i128, Some(IntK::U8)),
            Value::Flags(_, i) => (*i, Some(IntK::U8)),
            _ => return Err(ev_err("E1001", &format!("{} にこの演算は使えません", short_value(&b)), sp)),
        };
        // u8 + i8 → u8 などは左の型
        let k = if kx.is_some() { kx } else { ky };
        let r = match op {
            Add => x + y,
            Sub => x - y,
            Mul => x * y,
            Div => {
                if y == 0 {
                    return Err(ev_err("E1001", "0 で割っています", sp));
                }
                if k.is_none() {
                    x.div_euclid(y)
                } else {
                    x / y
                }
            }
            Rem => {
                if y == 0 {
                    return Err(ev_err("E1001", "0 で割っています", sp));
                }
                if k.is_none() {
                    x.rem_euclid(y)
                } else {
                    x % y
                }
            }
            And => x & y,
            Or => x | y,
            Xor => x ^ y,
            Shl => {
                if !(0..128).contains(&y) {
                    return Err(ev_err("E1001", "シフトの回数が大きすぎます", sp));
                }
                x << y
            }
            Shr => {
                if !(0..128).contains(&y) {
                    return Err(ev_err("E1001", "シフトの回数が大きすぎます", sp));
                }
                match k {
                    Some(kk) if !kk.signed() => (x & kk.max()) >> y,
                    _ => x >> y,
                }
            }
            _ => return Err(ev_err("E1001", "この演算はコンパイル時に計算できません", sp)),
        };
        Ok(match k {
            Some(kk) => Value::Int(kk.wrap(r), Some(kk)),
            None => Value::Int(r, None),
        })
    }

    pub fn ct_unop(&mut self, op: UnOp, a: Value, sp: &Span) -> EResult<Value> {
        match (op, a) {
            (UnOp::Neg, Value::Int(i, k)) => Ok(match k {
                Some(kk) => Value::Int(kk.wrap(-i), Some(kk)),
                None => Value::Int(-i, None),
            }),
            (UnOp::Neg, Value::Real(r)) => Ok(Value::Real(-r)),
            (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
            (UnOp::Not, Value::Int(i, k)) => Ok(match k {
                Some(kk) => Value::Int(kk.wrap(!i), Some(kk)),
                None => Value::Int(!i, None),
            }),
            (UnOp::Not, Value::Flags(f, x)) => {
                let size = self.types.flags[f].size();
                Ok(Value::Flags(f, IntK::unsigned_of_size(size).wrap(!x)))
            }
            (_, v) => Err(ev_err("E1001", &format!("{} にこの演算は使えません", short_value(&v)), sp)),
        }
    }

    /// 式をコンパイル時に計算する
    pub fn eval_ct(&mut self, e: &Expr, env: &mut Env, expect: Option<&Ty>) -> EResult<Value> {
        match self.eval_flow(e, env, expect)? {
            Flow::Val(v) => Ok(v),
            Flow::Return(v) => Ok(v),
            Flow::Break(_, _) | Flow::Continue(_) => Err(ev_err("E0704", "ループの外で break・continue を使っています", &e.span)),
        }
    }

    fn eval_val(&mut self, e: &Expr, env: &mut Env, expect: Option<&Ty>) -> EResult<Result<Value, Flow>> {
        match self.eval_flow(e, env, expect)? {
            Flow::Val(v) => Ok(Ok(v)),
            f => Ok(Err(f)),
        }
    }

    fn eval_flow(&mut self, e: &Expr, env: &mut Env, expect: Option<&Ty>) -> EResult<Flow> {
        self.step(&e.span)?;
        // 値を求める。break・continue・return なら、そのまま外へ伝える
        macro_rules! val {
            ($x:expr, $exp:expr) => {
                match self.eval_val($x, &mut *env, $exp)? {
                    Ok(v) => v,
                    Err(f) => return Ok(f),
                }
            };
        }
        let sp = e.span.clone();
        let v = match &e.kind {
            ExprKind::Int(i, hex) => match expect {
                Some(Ty::Bcd(n)) => {
                    if *hex {
                        return Err(ev_err("E0509", "BCD の型に 16進のリテラルは書けません（10進で書いてください）", &sp));
                    }
                    let m = bcd_modulus(*n) as i128;
                    if *i < 0 || *i >= m {
                        return Err(ev_err("E0509", &format!("{} は bcd{} に収まりません", i, n * 8), &sp));
                    }
                    Value::Bcd(*i as u64, *n)
                }
                Some(Ty::Int(k)) => {
                    if *i < k.min() || *i > k.max() {
                        return Err(ev_err("E0510", &format!("{} は {} に収まりません", i, k.name()), &sp));
                    }
                    Value::Int(*i, Some(*k))
                }
                Some(Ty::Range(r)) => {
                    let d = self.types.ranges[*r].clone();
                    if *i < d.lo || *i >= d.hi {
                        return Err(ev_err("E0510", &format!("{} は範囲型 {} に収まりません", i, d.name), &sp));
                    }
                    Value::Range(*r, *i)
                }
                Some(Ty::CReal) => Value::Real(*i as f64),
                _ => Value::Int(*i, None),
            },
            ExprKind::Real(r) => Value::Real(*r),
            ExprKind::Bool(b) => Value::Bool(*b),
            ExprKind::Char(u, enc) => {
                let b = self.encode_char(u, enc, &sp)?;
                Value::Char(b, enc.clone())
            }
            ExprKind::Str(p, units, enc) => self.string_value(p, units, enc, &sp)?,
            ExprKind::Unit => Value::Unit,
            ExprKind::Path(p) => {
                if p.segs.len() == 1 {
                    if let Some(v) = env.get(&p.segs[0]) {
                        return Ok(Flow::Val(v));
                    }
                    if p.segs[0] == "PI" {
                        return Ok(Flow::Val(Value::Real(std::f64::consts::PI)));
                    }
                    if p.segs[0] == "None" {
                        return Ok(Flow::Val(Value::Opt(None)));
                    }
                }
                if p.segs.len() == 2 && p.segs[0] == "Cpu" {
                    return Ok(Flow::Val(Value::Cpu(p.segs[1].clone())));
                }
                let m = env.module;
                match self.resolve_value_path(m, &p.segs, &sp) {
                    PathRes::Global(g) => {
                        match self.globals[g].kind {
                            GKind::Const => {}
                            _ => {
                                return Err(ev_err(
                                    "E1001",
                                    &format!("{} は const ではないので、コンパイル時に読めません", self.globals[g].name),
                                    &sp,
                                ))
                            }
                        }
                        self.global_value(g, &sp)?
                    }
                    PathRes::Value(v) => v,
                    PathRes::Func(f) => Value::Func(f),
                    PathRes::NotFound(msg) => return Err(ev_err("E0201", &msg, &sp)),
                    _ => return Err(ev_err("E1001", &format!("{} はコンパイル時に計算できる値ではありません", p.text()), &sp)),
                }
            }
            ExprKind::Unary(op, x) => {
                if *op == UnOp::Addr {
                    return Err(ev_err("E1001", "アドレスはコンパイル時の値として使えません", &sp));
                }
                let a = val!(x, expect);
                self.ct_unop(*op, a, &sp)?
            }
            ExprKind::Binary(op, a, b) => {
                if *op == BinOp::LAnd || *op == BinOp::LOr {
                    let x = val!(a, None);
                    let xb = match x {
                        Value::Bool(v) => v,
                        _ => return Err(ev_err("E0403", "&& と || には bool を使います", &sp)),
                    };
                    if (*op == BinOp::LAnd && !xb) || (*op == BinOp::LOr && xb) {
                        return Ok(Flow::Val(Value::Bool(xb)));
                    }
                    let y = val!(b, None);
                    return match y {
                        Value::Bool(v) => Ok(Flow::Val(Value::Bool(v))),
                        _ => Err(ev_err("E0403", "&& と || には bool を使います", &sp)),
                    };
                }
                let sub_expect = if op.is_cmp() || *op == BinOp::Shl || *op == BinOp::Shr { None } else { expect };
                let x = val!(a, sub_expect);
                let y_expect = if op.is_cmp() { None } else if *op == BinOp::Shl || *op == BinOp::Shr { None } else { sub_expect };
                let y = val!(b, y_expect);
                // 片方だけ型がある整数リテラルは、もう片方の型に合わせる
                self.ct_binop(*op, x, y, &sp)?
            }
            ExprKind::Cast(x, t) => {
                let m = env.module;
                let ty = self.resolve_type(m, t);
                let a = val!(x, None);
                self.cast_value(a, &ty, &sp)?
            }
            ExprKind::Field(base, name) => {
                if let ExprKind::Path(p) = &base.kind {
                    if p.segs.len() == 1 && (p.segs[0] == "target" || p.segs[0] == "build") && env.get(&p.segs[0]).is_none() {
                        if p.segs[0] == "build" {
                            return Ok(Flow::Val(match self.build.get(name) {
                                Some(c) => cfg_to_value(c),
                                None => Value::Bool(false),
                            }));
                        }
                        let t = self.target.clone();
                        return match crate::program::target_field(&t, name, &sp) {
                            Ok(c) => Ok(Flow::Val(cfg_to_value(&c))),
                            Err((code, msg, s2)) => Err(ev_err(&code, &msg, &s2)),
                        };
                    }
                }
                let b = val!(base, None);
                match b {
                    Value::Struct(sid, fields) => {
                        let def = self.types.structs[sid].clone();
                        match def.fields.iter().position(|f| &f.0 == name) {
                            Some(i) => fields[i].clone(),
                            None => return Err(ev_err("E0416", &format!("{} にフィールド {} はありません", def.name, name), &sp)),
                        }
                    }
                    _ => return Err(ev_err("E1001", "フィールドを読めない値です", &sp)),
                }
            }
            ExprKind::MethodCall(base, name, args) => {
                if let ExprKind::Path(p) = &base.kind {
                    if p.segs.len() == 1 && p.segs[0] == "target" && name == "has" && env.get("target").is_none() {
                        if let Some(a) = args.first() {
                            if let ExprKind::Path(ip) = &a.value.kind {
                                let n = ip.segs.join("::");
                                if !self.known_ifaces.contains(&n) {
                                    return Err(ev_err("E1006", &format!("target.has() に知らないインターフェース {} を書いています", n), &sp));
                                }
                                return Ok(Flow::Val(Value::Bool(self.target.binds.contains_key(&n))));
                            }
                        }
                    }
                }
                let b = val!(base, None);
                let mut av = Vec::new();
                for a in args {
                    av.push(val!(&a.value, None));
                }
                self.ct_method(b, name, av, &sp)?
            }
            ExprKind::Call(f, args) => {
                return self.eval_call(f, args, env, expect, &sp).map(Flow::Val);
            }
            ExprKind::Index(base, idx) => {
                let b = val!(base, None);
                let i = val!(idx, None);
                let iv = value_int(&i).ok_or_else(|| ev_err("E1001", "添字が整数ではありません", &sp))?;
                match b {
                    Value::Array(items) => {
                        if iv < 0 || iv as usize >= items.len() {
                            return Err(ev_err("E0412", &format!("添字 {} が配列の範囲（0〜{}）の外です", iv, items.len() as i128 - 1), &sp));
                        }
                        items[iv as usize].clone()
                    }
                    Value::Str(_, bytes, enc) => {
                        if iv < 0 || iv as usize >= bytes.len() {
                            return Err(ev_err("E0412", "添字が文字列の範囲の外です", &sp));
                        }
                        Value::Char(bytes[iv as usize], enc)
                    }
                    _ => return Err(ev_err("E1001", "添字を使えない値です", &sp)),
                }
            }
            ExprKind::StructLit(path, fields, base) => {
                let m = env.module;
                let ty = match self.resolve_type_segs(m, &path.segs, &path.span) {
                    Some(t) => t,
                    None => return Err(ev_err("E0201", &format!("構造体 {} が見つかりません", path.text()), &sp)),
                };
                let sid = match ty {
                    Ty::Struct(s) => s,
                    _ => return Err(ev_err("E0401", &format!("{} は構造体ではありません", path.text()), &sp)),
                };
                let def = self.types.structs[sid].clone();
                let mut vals: Vec<Option<Value>> = vec![None; def.fields.len()];
                for (fname, fe, fsp) in fields {
                    match def.fields.iter().position(|f| &f.0 == fname) {
                        Some(i) => {
                            let ft = def.fields[i].1.clone();
                            let v = val!(fe, Some(&ft));
                            vals[i] = Some(self.convert_value(v, &ft, fsp)?);
                        }
                        None => return Err(ev_err("E0416", &format!("{} にフィールド {} はありません", def.name, fname), fsp)),
                    }
                }
                if let Some(b) = base {
                    let bv = val!(b, None);
                    match bv {
                        Value::Struct(s2, bf) if s2 == sid => {
                            for i in 0..vals.len() {
                                if vals[i].is_none() {
                                    vals[i] = Some(bf[i].clone());
                                }
                            }
                        }
                        _ => return Err(ev_err("E0401", "..の後ろには同じ構造体の定数を書きます", &sp)),
                    }
                }
                let mut out = Vec::new();
                for (i, v) in vals.into_iter().enumerate() {
                    match v {
                        Some(v) => out.push(v),
                        None => {
                            return Err(ev_err(
                                "E0507",
                                &format!("構造体 {} の値にフィールド {} がありません", def.name, def.fields[i].0),
                                &sp,
                            ))
                        }
                    }
                }
                Value::Struct(sid, out)
            }
            ExprKind::ArrayList(items) => {
                let et = match expect {
                    Some(Ty::Array(t, _)) => Some((**t).clone()),
                    _ => None,
                };
                let mut out = Vec::new();
                for x in items {
                    out.push(val!(x, et.as_ref()));
                }
                Value::Array(out)
            }
            ExprKind::ArrayRepeat(x, n) => {
                let et = match expect {
                    Some(Ty::Array(t, _)) => Some((**t).clone()),
                    _ => None,
                };
                let count = self.eval_count(n, env)?;
                let v = val!(x, et.as_ref());
                if count > 65536 {
                    return Err(ev_err("E1001", "配列が大きすぎます", &sp));
                }
                Value::Array(vec![v; count as usize])
            }
            ExprKind::ArrayFor(name, range, body) => {
                let et = match expect {
                    Some(Ty::Array(t, _)) => Some((**t).clone()),
                    _ => None,
                };
                let (lo, hi) = self.eval_int_range(range, env)?;
                let mut out = Vec::new();
                let mut i = lo;
                while i < hi {
                    env.push();
                    env.define(name, Value::Int(i, None));
                    let r = self.eval_ct(body, env, None);
                    env.pop();
                    let mut v = r?;
                    if let Some(t) = &et {
                        v = self.cast_or_convert(v, t, &body.span)?;
                    }
                    out.push(v);
                    i += 1;
                }
                Value::Array(out)
            }
            ExprKind::Block(b) => return self.eval_block(b, env, expect),
            ExprKind::If(c, then_b, els) => {
                let cv = val!(c, None);
                match cv {
                    Value::Bool(true) => return self.eval_block(then_b, env, expect),
                    Value::Bool(false) => match els {
                        Some(x) => return self.eval_flow(x, env, expect),
                        None => Value::Unit,
                    },
                    _ => return Err(ev_err("E0403", "if の条件は bool でなければなりません", &c.span)),
                }
            }
            ExprKind::IfLet(pat, scrut, then_b, els) => {
                let sv = val!(scrut, None);
                env.push();
                let ok = self.ct_pattern(pat, &sv, env)?;
                if ok {
                    let r = self.eval_block(then_b, env, expect);
                    env.pop();
                    return r;
                }
                env.pop();
                match els {
                    Some(x) => return self.eval_flow(x, env, expect),
                    None => Value::Unit,
                }
            }
            ExprKind::Match(scrut, arms, _) => {
                let sv = val!(scrut, None);
                for arm in arms {
                    for p in &arm.pats {
                        env.push();
                        let ok = self.ct_pattern(p, &sv, env)?;
                        if ok {
                            let r = self.eval_flow(&arm.body, env, expect);
                            env.pop();
                            return r;
                        }
                        env.pop();
                    }
                }
                return Err(ev_err("E0701", "match のどの腕にも当てはまりませんでした", &sp));
            }
            ExprKind::ComptimeIf(c, then_b, els) => {
                let cv = val!(c, None);
                match cv {
                    Value::Bool(true) => return self.eval_block(then_b, env, expect),
                    Value::Bool(false) => match els {
                        Some(x) => return self.eval_flow(x, env, expect),
                        None => Value::Unit,
                    },
                    _ => return Err(ev_err("E0403", "comptime if の条件は bool でなければなりません", &c.span)),
                }
            }
            ExprKind::Loop(label, body) => loop {
                match self.eval_block(body, env, None)? {
                    Flow::Val(_) => {}
                    Flow::Break(l, v) => {
                        if l.is_none() || &l == label {
                            break v;
                        }
                        return Ok(Flow::Break(l, v));
                    }
                    Flow::Continue(l) => {
                        if l.is_none() || &l == label {
                            continue;
                        }
                        return Ok(Flow::Continue(l));
                    }
                    Flow::Return(v) => return Ok(Flow::Return(v)),
                }
            },
            ExprKind::While(label, c, body, _) => {
                loop {
                    let cv = val!(c, None);
                    match cv {
                        Value::Bool(true) => {}
                        Value::Bool(false) => break,
                        _ => return Err(ev_err("E0403", "while の条件は bool でなければなりません", &c.span)),
                    }
                    match self.eval_block(body, env, None)? {
                        Flow::Val(_) => {}
                        Flow::Break(l, _) => {
                            if l.is_none() || &l == label {
                                break;
                            }
                            return Ok(Flow::Break(l, Value::Unit));
                        }
                        Flow::Continue(l) => {
                            if l.is_none() || &l == label {
                                continue;
                            }
                            return Ok(Flow::Continue(l));
                        }
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                    }
                }
                Value::Unit
            }
            ExprKind::For(label, name, iter, body, _) => {
                let items = self.ct_iter_values(iter, env)?;
                for it in items {
                    env.push();
                    env.define(name, it);
                    let r = self.eval_block(body, env, None);
                    env.pop();
                    match r? {
                        Flow::Val(_) => {}
                        Flow::Break(l, _) => {
                            if l.is_none() || &l == label {
                                break;
                            }
                            return Ok(Flow::Break(l, Value::Unit));
                        }
                        Flow::Continue(l) => {
                            if l.is_none() || &l == label {
                                continue;
                            }
                            return Ok(Flow::Continue(l));
                        }
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                    }
                }
                Value::Unit
            }
            ExprKind::Break(label, v) => {
                let x = match v {
                    Some(x) => val!(x, None),
                    None => Value::Unit,
                };
                return Ok(Flow::Break(label.clone(), x));
            }
            ExprKind::Continue(label) => return Ok(Flow::Continue(label.clone())),
            ExprKind::Return(v) => {
                let rt = env.fn_ret.clone();
                let x = match v {
                    Some(x) => val!(x, rt.as_ref()),
                    None => Value::Unit,
                };
                return Ok(Flow::Return(x));
            }
            ExprKind::Assign(op, lhs, rhs) => {
                let rv = val!(rhs, None);
                let newv = match op {
                    None => rv,
                    Some(o) => {
                        let cur = val!(lhs, None);
                        self.ct_binop(*o, cur, rv, &sp)?
                    }
                };
                self.ct_assign(lhs, newv, env)?;
                Value::Unit
            }
            ExprKind::Try(x) => {
                let v = val!(x, None);
                match v {
                    Value::Opt(Some(inner)) => *inner,
                    Value::Opt(None) => return Ok(Flow::Return(Value::Opt(None))),
                    _ => return Err(ev_err("E0406", "? は Option の値にだけ使えます", &sp)),
                }
            }
            ExprKind::Range(_, _, _) => return Err(ev_err("E1001", "範囲はこの場所では値になりません", &sp)),
            _ => return Err(ev_err("E1001", "この式はコンパイル時に計算できません", &sp)),
        };
        Ok(Flow::Val(v))
    }

    fn cast_or_convert(&mut self, v: Value, t: &Ty, sp: &Span) -> EResult<Value> {
        // 先に判定してから v を渡す（借用中の値を動かさない）
        let use_cast = matches!((&v, t), (Value::Int(_, None), Ty::Int(_)) | (Value::Real(_), Ty::Int(_)));
        if use_cast {
            self.cast_value(v, t, sp)
        } else {
            self.convert_value(v, t, sp)
        }
    }

    pub fn eval_count(&mut self, n: &Expr, env: &mut Env) -> EResult<i128> {
        if let ExprKind::Path(p) = &n.kind {
            let m = env.module;
            if env.get(&p.segs[0]).is_none() {
                if let Some(t) = self.try_type_path(m, &p.segs) {
                    match t {
                        Ty::Range(r) => return Ok(self.types.ranges[r].count()),
                        Ty::Enum(e) => return Ok(self.types.enums[e].variants.len() as i128),
                        _ => {}
                    }
                }
            }
        }
        let v = self.eval_ct(n, env, None)?;
        value_int(&v).ok_or_else(|| ev_err("E1001", "個数が整数ではありません", &n.span))
    }

    fn eval_int_range(&mut self, r: &Expr, env: &mut Env) -> EResult<(i128, i128)> {
        match &r.kind {
            ExprKind::Range(Some(a), Some(b), incl) => {
                let av = self.eval_ct(a, env, None)?;
                let bv = self.eval_ct(b, env, None)?;
                let x = value_int(&av).ok_or_else(|| ev_err("E1001", "範囲の端が整数ではありません", &a.span))?;
                let y = value_int(&bv).ok_or_else(|| ev_err("E1001", "範囲の端が整数ではありません", &b.span))?;
                Ok((x, if *incl { y + 1 } else { y }))
            }
            _ => Err(ev_err("E1001", "範囲（a..b）が必要です", &r.span)),
        }
    }

    fn ct_iter_values(&mut self, iter: &Expr, env: &mut Env) -> EResult<Vec<Value>> {
        // (a..b).rev()
        if let ExprKind::MethodCall(base, name, _) = &iter.kind {
            if name == "rev" {
                let mut v = self.ct_iter_values(base, env)?;
                v.reverse();
                return Ok(v);
            }
        }
        if let ExprKind::Range(_, _, _) = &iter.kind {
            let (lo, hi) = self.eval_int_range(iter, env)?;
            // 端の型を受け継ぐ
            let k = match &iter.kind {
                ExprKind::Range(Some(a), _, _) => {
                    let av = self.eval_ct(a, env, None)?;
                    match av {
                        Value::Int(_, k) => k,
                        _ => None,
                    }
                }
                _ => None,
            };
            let mut out = Vec::new();
            let mut i = lo;
            while i < hi {
                out.push(Value::Int(i, k));
                i += 1;
            }
            return Ok(out);
        }
        if let ExprKind::Path(p) = &iter.kind {
            let m = env.module;
            if env.get(&p.segs[0]).is_none() {
                if let Some(t) = self.try_type_path(m, &p.segs) {
                    match t {
                        Ty::Range(r) => {
                            let d = self.types.ranges[r].clone();
                            return Ok((d.lo..d.hi).map(|x| Value::Range(r, x)).collect());
                        }
                        Ty::Enum(e) => {
                            let d = self.types.enums[e].clone();
                            return Ok(d.variants.iter().map(|x| Value::Enum(e, x.1)).collect());
                        }
                        _ => {}
                    }
                }
            }
        }
        let v = self.eval_ct(iter, env, None)?;
        match v {
            Value::Array(items) => Ok(items),
            Value::Str(_, bytes, enc) => Ok(bytes.iter().map(|b| Value::Char(*b, enc.clone())).collect()),
            _ => Err(ev_err("E1001", "for で回せない値です", &iter.span)),
        }
    }

    fn ct_assign(&mut self, lhs: &Expr, v: Value, env: &mut Env) -> EResult<()> {
        match &lhs.kind {
            ExprKind::Path(p) if p.segs.len() == 1 => {
                if env.set(&p.segs[0], v) {
                    Ok(())
                } else {
                    Err(ev_err("E1001", &format!("{} はコンパイル時に書き換えられません", p.segs[0]), &lhs.span))
                }
            }
            ExprKind::Index(base, idx) => {
                let iv = self.eval_ct(idx, env, None)?;
                let i = value_int(&iv).ok_or_else(|| ev_err("E1001", "添字が整数ではありません", &idx.span))?;
                let cur = self.eval_ct(base, env, None)?;
                match cur {
                    Value::Array(mut items) => {
                        if i < 0 || i as usize >= items.len() {
                            return Err(ev_err("E0412", "添字が配列の範囲の外です", &idx.span));
                        }
                        items[i as usize] = v;
                        self.ct_assign(base, Value::Array(items), env)
                    }
                    _ => Err(ev_err("E1001", "添字を使えない値です", &lhs.span)),
                }
            }
            ExprKind::Field(base, name) => {
                let cur = self.eval_ct(base, env, None)?;
                match cur {
                    Value::Struct(sid, mut fields) => {
                        let def = self.types.structs[sid].clone();
                        match def.fields.iter().position(|f| &f.0 == name) {
                            Some(i) => {
                                fields[i] = v;
                                self.ct_assign(base, Value::Struct(sid, fields), env)
                            }
                            None => Err(ev_err("E0416", "フィールドがありません", &lhs.span)),
                        }
                    }
                    _ => Err(ev_err("E1001", "フィールドを持たない値です", &lhs.span)),
                }
            }
            _ => Err(ev_err("E0512", "代入できない式です", &lhs.span)),
        }
    }

    fn ct_pattern(&mut self, p: &Pattern, v: &Value, env: &mut Env) -> EResult<bool> {
        match &p.kind {
            PatKind::Wild => Ok(true),
            PatKind::Ident(n) => {
                // 定数・列挙型の値なら比べる。そうでなければ束縛する
                let m = env.module;
                match self.resolve_value_path(m, &[n.clone()], &p.span) {
                    PathRes::Global(g) if self.globals[g].kind == GKind::Const => {
                        let c = self.global_value(g, &p.span)?;
                        Ok(value_int(&c) == value_int(v) && value_int(v).is_some())
                    }
                    PathRes::Value(c) => Ok(value_int(&c) == value_int(v)),
                    _ => {
                        env.define(n, v.clone());
                        Ok(true)
                    }
                }
            }
            PatKind::Lit(e) => {
                let c = self.eval_ct(e, env, None)?;
                Ok(match (&c, v) {
                    (Value::Cpu(a), Value::Cpu(b)) => a == b,
                    (Value::Bcd(a, _), Value::Bcd(b, _)) => a == b,
                    _ => value_int(&c).is_some() && value_int(&c) == value_int(v),
                })
            }
            PatKind::Range(a, b, incl) => {
                let x = self.eval_ct(a, env, None)?;
                let y = self.eval_ct(b, env, None)?;
                let (x, y, z) = (value_int(&x), value_int(&y), value_int(v));
                match (x, y, z) {
                    (Some(x), Some(y), Some(z)) => Ok(if *incl { z >= x && z <= y } else { z >= x && z < y }),
                    _ => Ok(false),
                }
            }
            PatKind::Some(inner) => match v {
                Value::Opt(Some(x)) => {
                    let xx = (**x).clone();
                    self.ct_pattern(inner, &xx, env)
                }
                _ => Ok(false),
            },
            PatKind::None => Ok(matches!(v, Value::Opt(None))),
        }
    }

    fn eval_block(&mut self, b: &Block, env: &mut Env, expect: Option<&Ty>) -> EResult<Flow> {
        env.push();
        let r = self.eval_block_inner(b, env, expect);
        env.pop();
        r
    }

    fn eval_block_inner(&mut self, b: &Block, env: &mut Env, expect: Option<&Ty>) -> EResult<Flow> {
        for s in &b.stmts {
            match &s.kind {
                StmtKind::Let(name, ty, init) => {
                    let m = env.module;
                    let t = ty.as_ref().map(|t| self.resolve_type(m, t));
                    let v = match init {
                        Some(x) => match self.eval_flow(x, env, t.as_ref())? {
                            Flow::Val(v) => v,
                            f => return Ok(f),
                        },
                        None => Value::Unit,
                    };
                    let v = match (&t, init) {
                        (Some(tt), Some(_)) => self.cast_or_convert(v, tt, &s.span)?,
                        _ => v,
                    };
                    if let Some(n) = name {
                        env.define(n, v);
                    }
                }
                StmtKind::Expr(x, _) => match self.eval_flow(x, env, None)? {
                    Flow::Val(_) => {}
                    f => return Ok(f),
                },
                StmtKind::Item(_) => {}
                StmtKind::ComptimeAssert(c, msg) => {
                    let v = self.eval_ct(c, env, None)?;
                    if v != Value::Bool(true) {
                        return Err(ev_err("E1005", &format!("comptime assert が成り立ちません：{}", msg), &c.span));
                    }
                }
            }
        }
        match &b.tail {
            Some(t) => self.eval_flow(t, env, expect),
            None => Ok(Flow::Val(Value::Unit)),
        }
    }

    fn ct_method(&mut self, b: Value, name: &str, args: Vec<Value>, sp: &Span) -> EResult<Value> {
        match (name, &b) {
            ("len", Value::Array(items)) => Ok(Value::Int(items.len() as i128, None)),
            ("len", Value::Str(_, bytes, _)) => Ok(Value::Int(bytes.len() as i128, None)),
            ("contains", Value::Flags(_, x)) => {
                let m = args.first().and_then(value_int).unwrap_or(0);
                Ok(Value::Bool(x & m == m))
            }
            ("checked_add", Value::Int(x, Some(k))) | ("checked_sub", Value::Int(x, Some(k))) => {
                let y = args.first().and_then(value_int).unwrap_or(0);
                let r = if name == "checked_add" { x + y } else { x - y };
                if r < k.min() || r > k.max() {
                    Ok(Value::Opt(None))
                } else {
                    Ok(Value::Opt(Some(Box::new(Value::Int(r, Some(*k))))))
                }
            }
            ("saturating_add", Value::Int(x, Some(k))) | ("saturating_sub", Value::Int(x, Some(k))) => {
                let y = args.first().and_then(value_int).unwrap_or(0);
                let r = if name == "saturating_add" { x + y } else { x - y };
                Ok(Value::Int(r.max(k.min()).min(k.max()), Some(*k)))
            }
            ("rev", _) => Err(ev_err("E1001", "rev() は for の中だけで使えます", sp)),
            _ => Err(ev_err("E1001", &format!("メソッド {} はコンパイル時に計算できません", name), sp)),
        }
    }

    fn eval_call(&mut self, f: &Expr, args: &[Arg], env: &mut Env, expect: Option<&Ty>, sp: &Span) -> EResult<Value> {
        let m = env.module;
        if let ExprKind::Path(p) = &f.kind {
            if p.segs.len() == 1 && env.get(&p.segs[0]).is_none() {
                let n = p.segs[0].as_str();
                match n {
                    "Some" => {
                        let inner_exp = match expect {
                            Some(Ty::Opt(t)) => Some((**t).clone()),
                            _ => None,
                        };
                        let a = args.first().ok_or_else(|| ev_err("E0418", "Some には値が 1 つ必要です", sp))?;
                        let v = self.eval_ct(&a.value, env, inner_exp.as_ref())?;
                        return Ok(Value::Opt(Some(Box::new(v))));
                    }
                    "sin" | "cos" | "sqrt" | "round" | "floor" | "abs" => {
                        let a = args.first().ok_or_else(|| ev_err("E0418", "引数が 1 つ必要です", sp))?;
                        let v = self.eval_ct(&a.value, env, None)?;
                        if n == "abs" {
                            if let Value::Int(i, k) = v {
                                return Ok(Value::Int(i.abs(), k));
                            }
                        }
                        let x = real_of(&v).ok_or_else(|| ev_err("E0401", "数学関数には数を渡します", sp))?;
                        let r = match n {
                            "sin" => x.sin(),
                            "cos" => x.cos(),
                            "sqrt" => x.sqrt(),
                            "round" => x.round(),
                            "floor" => x.floor(),
                            _ => x.abs(),
                        };
                        return Ok(Value::Real(r));
                    }
                    "size_of" => {
                        let a = args.first().ok_or_else(|| ev_err("E0418", "size_of には型が 1 つ必要です", sp))?;
                        let t = match &a.value.kind {
                            ExprKind::Path(tp) => self.resolve_type_segs(m, &tp.segs, &tp.span),
                            _ => None,
                        };
                        return match t {
                            Some(t) => Ok(Value::Int(self.types.size_of(&t) as i128, None)),
                            None => Err(ev_err("E1001", "size_of には型の名前を書きます", sp)),
                        };
                    }
                    "include_bin" => return self.include_bin(args, env, sp),
                    _ => {}
                }
            }
        }
        // 型に付いた関数（Eid::check など）
        if let ExprKind::Path(p) = &f.kind {
            if p.segs.len() >= 2 {
                let (tsegs, fname) = (&p.segs[..p.segs.len() - 1], p.segs[p.segs.len() - 1].clone());
                if let Some(t) = self.try_type_path(m, tsegs) {
                    let a = args.first().ok_or_else(|| ev_err("E0418", "引数が必要です", sp))?;
                    let v = self.eval_ct(&a.value, env, None)?;
                    let x = value_int(&v).ok_or_else(|| ev_err("E0401", "整数が必要です", sp))?;
                    return match (&t, fname.as_str()) {
                        (Ty::Range(r), "check") => {
                            let d = self.types.ranges[*r].clone();
                            Ok(Value::Opt(if x >= d.lo && x < d.hi { Some(Box::new(Value::Range(*r, x))) } else { None }))
                        }
                        (Ty::Range(r), "wrap") => {
                            let d = self.types.ranges[*r].clone();
                            Ok(Value::Range(*r, d.lo + (x - d.lo).rem_euclid(d.count())))
                        }
                        (Ty::Range(r), "unchecked") => Ok(Value::Range(*r, x)),
                        (Ty::Enum(e), "check") => {
                            let d = self.types.enums[*e].clone();
                            Ok(Value::Opt(if d.variants.iter().any(|v| v.1 == x) {
                                Some(Box::new(Value::Enum(*e, x)))
                            } else {
                                None
                            }))
                        }
                        _ => Err(ev_err("E1001", &format!("{} はコンパイル時に計算できません", p.text()), sp)),
                    };
                }
            }
        }
        let fv = self.eval_ct(f, env, None)?;
        let fid = match fv {
            Value::Func(i) => i,
            _ => return Err(ev_err("E1001", "関数ではない値を呼んでいます", sp)),
        };
        let kind = self.funcs[fid].kind.clone();
        if kind != FnKind::Const && kind != FnKind::Comptime {
            return Err(ev_err(
                "E1001",
                &format!("{} は const fn でも comptime fn でもないので、コンパイル時に呼べません", self.funcs[fid].name),
                sp,
            ));
        }
        self.ensure_sig(fid);
        let params = self.funcs[fid].params.clone();
        if params.len() != args.len() {
            return Err(ev_err(
                "E0418",
                &format!("{} の引数の数は {} ですが、{} 個渡しています", self.funcs[fid].name, params.len(), args.len()),
                sp,
            ));
        }
        let mut vals = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let pt = params[i].1.clone();
            let v = self.eval_ct(&a.value, env, Some(&pt))?;
            let use_cast = matches!((&v, &pt), (Value::Int(_, Some(_)), Ty::CInt));
            let v = if use_cast { self.cast_value(v, &pt, &a.value.span)? } else { self.convert_value(v, &pt, &a.value.span)? };
            vals.push(v);
        }
        self.call_ct(fid, vals, sp)
    }

    pub fn call_ct(&mut self, fid: usize, vals: Vec<Value>, sp: &Span) -> EResult<Value> {
        let decl = match &self.funcs[fid].decl {
            Some(d) => d.clone(),
            None => return Err(ev_err("E1001", "本体のない関数です", sp)),
        };
        let body = match &decl.body {
            Some(b) => b.clone(),
            None => return Err(ev_err("E1001", "本体のない関数です", sp)),
        };
        let m = self.funcs[fid].module;
        let ret = self.funcs[fid].ret.clone();
        let params = self.funcs[fid].params.clone();
        let mut env = Env::new(m);
        env.fn_ret = Some(ret.clone());
        for (i, v) in vals.into_iter().enumerate() {
            env.define(&params[i].0, v);
        }
        let r = match self.eval_block(&body, &mut env, Some(&ret))? {
            Flow::Val(v) | Flow::Return(v) => v,
            _ => return Err(ev_err("E0704", "ループの外で break・continue を使っています", sp)),
        };
        // core::us は、ちょうどにならなければ警告（12.2節）
        let r = self.cast_or_convert(r, &ret, sp)?;
        Ok(r)
    }

    fn include_bin(&mut self, args: &[Arg], env: &mut Env, sp: &Span) -> EResult<Value> {
        let m = env.module;
        let pa = args.first().ok_or_else(|| ev_err("E0418", "include_bin にはファイルの名前が必要です", sp))?;
        let pv = self.eval_ct(&pa.value, env, None)?;
        let path = match pv {
            Value::Array(items) => items.iter().filter_map(value_int).map(|b| b as u8 as char).collect::<String>(),
            Value::Str(_, b, _) => String::from_utf8_lossy(&b).to_string(),
            _ => return Err(ev_err("E1001", "include_bin のファイルの名前は文字列で書きます", sp)),
        };
        let mut offset = 0usize;
        for a in args.iter().skip(1) {
            if a.name.as_deref() == Some("offset") {
                let ov = self.eval_ct(&a.value, env, None)?;
                offset = value_int(&ov).unwrap_or(0).max(0) as usize;
            }
        }
        let full = self.modules[m].dir.join(&path);
        match std::fs::read(&full) {
            Ok(data) => {
                if offset > data.len() {
                    return Err(ev_err("E1004", "include_bin の offset がファイルの大きさを超えています", sp));
                }
                Ok(Value::Array(data[offset..].iter().map(|b| Value::Int(*b as i128, Some(IntK::U8))).collect()))
            }
            Err(_) => Err(ev_err("E1004", &format!("include_bin のファイル {} を読めません", full.display()), sp)),
        }
    }

    /// 定数の式を、モジュール m の中で計算する
    pub fn eval_in_module(&mut self, e: &Expr, m: usize, expect: Option<&Ty>) -> EResult<Value> {
        let mut env = Env::new(m);
        self.eval_ct(e, &mut env, expect)
    }

    /// 値をメモリ上のバイト列にする（配列はフィールドごと・下位上位に分けて並べる。3.8節）
    pub fn value_bytes(&self, v: &Value, t: &Ty) -> Vec<u8> {
        let size = self.types.size_of(t) as usize;
        match (v, t) {
            (Value::Array(items), Ty::Array(et, _)) => {
                let es = self.types.size_of(et) as usize;
                let n = items.len();
                let packed_elem = match &**et {
                    Ty::Struct(s) => self.types.structs[*s].packed,
                    Ty::Array(_, _) => true,
                    _ => es == 1,
                };
                let elems: Vec<Vec<u8>> = items.iter().map(|x| self.value_bytes(x, et)).collect();
                let mut out = vec![0u8; n * es];
                if packed_elem {
                    for (i, b) in elems.iter().enumerate() {
                        for (k, x) in b.iter().enumerate() {
                            if i * es + k < out.len() {
                                out[i * es + k] = *x;
                            }
                        }
                    }
                } else {
                    // バイトごとの面（plane）：要素 i のバイト k は k*n + i
                    for (i, b) in elems.iter().enumerate() {
                        for (k, x) in b.iter().enumerate() {
                            if k * n + i < out.len() {
                                out[k * n + i] = *x;
                            }
                        }
                    }
                }
                out
            }
            (Value::Struct(sid, fields), _) => {
                let def = &self.types.structs[*sid];
                let mut out = Vec::new();
                for (i, f) in fields.iter().enumerate() {
                    let ft = &def.fields[i].1;
                    out.extend(self.value_bytes(f, ft));
                }
                out
            }
            (Value::Str(k, bytes, _), _) => {
                // 文字列の定数そのもの（データ）
                let mut out = Vec::new();
                match k {
                    StrK::P => {
                        out.push(bytes.len() as u8);
                        out.extend(bytes.iter());
                    }
                    StrK::Z => {
                        out.extend(bytes.iter());
                        out.push(0);
                    }
                    StrK::Hi => {
                        out.extend(bytes.iter());
                        if let Some(l) = out.last_mut() {
                            *l |= 0x80;
                        }
                    }
                }
                out
            }
            (Value::Opt(o), Ty::Opt(inner)) => {
                let mut out = match o {
                    Some(x) => self.value_bytes(x, inner),
                    None => vec![0u8; self.types.size_of(inner) as usize],
                };
                out.push(if o.is_some() { 1 } else { 0 });
                out
            }
            (Value::Func(f), Ty::FnSet(s)) => {
                let idx = self.types.fnsets[*s].fns.iter().position(|x| x == f).unwrap_or(0);
                vec![idx as u8]
            }
            _ => {
                let x = value_int(v).unwrap_or(0);
                let mut out = Vec::new();
                for k in 0..size {
                    out.push(((x >> (8 * k)) & 0xFF) as u8);
                }
                out
            }
        }
    }
}

pub fn real_of(v: &Value) -> Option<f64> {
    match v {
        Value::Real(r) => Some(*r),
        Value::Int(i, _) => Some(*i as f64),
        _ => None,
    }
}

pub fn short_value(v: &Value) -> String {
    match v {
        Value::Unit => "()".to_string(),
        Value::Bool(b) => format!("{}", b),
        Value::Int(i, _) => format!("{}", i),
        Value::Real(r) => format!("{}", r),
        Value::Char(c, _) => format!("'{}'", *c as char),
        Value::Bcd(d, _) => format!("{}", d),
        Value::Enum(_, x) | Value::Range(_, x) | Value::Flags(_, x) => format!("{}", x),
        Value::Array(v) => format!("配列（{} 個）", v.len()),
        Value::Struct(_, _) => "構造体".to_string(),
        Value::Str(_, b, _) => format!("文字列（{} 文字）", b.len()),
        Value::Opt(None) => "None".to_string(),
        Value::Opt(Some(_)) => "Some(…)".to_string(),
        Value::Func(_) => "関数".to_string(),
        Value::Cpu(c) => format!("Cpu::{}", c),
    }
}
