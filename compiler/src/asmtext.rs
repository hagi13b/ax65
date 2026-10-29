// インラインアセンブリのテキストの解析（14章）

#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    Num(i64),
    Ident(String),
    Ref(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Sel {
    Full,
    Lo,
    Hi,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AExpr {
    pub sel: Sel,
    pub terms: Vec<(i64, Term)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    Imp,
    Acc,
    Imm(AExpr),
    Abs(AExpr),
    AbsX(AExpr),
    AbsY(AExpr),
    IndY(AExpr),
    IndX(AExpr),
    Ind(AExpr),
}

#[derive(Clone, Debug)]
pub struct AsmLine {
    pub line: usize,
    pub label: Option<String>,
    pub smc: bool,
    pub mn: Option<String>,
    pub operand: Operand,
    pub comment: String,
}

fn parse_term(s: &str) -> Result<Term, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("式がありません".to_string());
    }
    if let Some(h) = s.strip_prefix('$') {
        return i64::from_str_radix(&h.replace('_', ""), 16).map(Term::Num).map_err(|_| format!("16進の数 {} が正しくありません", s));
    }
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return i64::from_str_radix(&h.replace('_', ""), 16).map(Term::Num).map_err(|_| format!("16進の数 {} が正しくありません", s));
    }
    if let Some(h) = s.strip_prefix("0b") {
        return i64::from_str_radix(&h.replace('_', ""), 2).map(Term::Num).map_err(|_| format!("2進の数 {} が正しくありません", s));
    }
    if s.starts_with('\'') && s.ends_with('\'') && s.chars().count() == 3 {
        let c = s.chars().nth(1).unwrap_or(' ');
        return Ok(Term::Num(c as i64));
    }
    if s.starts_with('{') && s.ends_with('}') {
        return Ok(Term::Ref(s[1..s.len() - 1].trim().to_string()));
    }
    if s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        return s.replace('_', "").parse::<i64>().map(Term::Num).map_err(|_| format!("数 {} が正しくありません", s));
    }
    if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '@' || c == '.') {
        return Ok(Term::Ident(s.to_string()));
    }
    Err(format!("式 {} が正しくありません", s))
}

pub fn parse_expr(s: &str) -> Result<AExpr, String> {
    let mut t = s.trim();
    let mut sel = Sel::Full;
    if let Some(r) = t.strip_prefix('<') {
        sel = Sel::Lo;
        t = r.trim();
    } else if let Some(r) = t.strip_prefix('>') {
        sel = Sel::Hi;
        t = r.trim();
    }
    let mut terms = Vec::new();
    let mut cur = String::new();
    let mut sign = 1i64;
    let mut depth = 0;
    let chars: Vec<char> = t.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' {
            depth += 1;
        }
        if c == '}' {
            depth -= 1;
        }
        if c == '\'' && i + 2 < chars.len() && chars[i + 2] == '\'' {
            cur.push(c);
            cur.push(chars[i + 1]);
            cur.push(chars[i + 2]);
            i += 3;
            continue;
        }
        if depth == 0 && (c == '+' || c == '-') && !cur.trim().is_empty() {
            terms.push((sign, parse_term(&cur)?));
            cur.clear();
            sign = if c == '+' { 1 } else { -1 };
            i += 1;
            continue;
        }
        if depth == 0 && c == '-' && cur.trim().is_empty() {
            sign = -sign;
            i += 1;
            continue;
        }
        cur.push(c);
        i += 1;
    }
    terms.push((sign, parse_term(&cur)?));
    Ok(AExpr { sel, terms })
}

fn split_index(s: &str) -> Option<(String, char)> {
    // "expr,x" / "expr,y"（{…} の中のカンマは区切りにしない）
    let mut depth = 0;
    let chars: Vec<char> = s.chars().collect();
    for i in (0..chars.len()).rev() {
        let c = chars[i];
        if c == '}' {
            depth += 1;
        }
        if c == '{' {
            depth -= 1;
        }
        if c == ',' && depth == 0 {
            let reg: String = chars[i + 1..].iter().collect::<String>().trim().to_lowercase();
            let base: String = chars[..i].iter().collect();
            if reg == "x" || reg == "y" {
                return Some((base, reg.chars().next().unwrap_or('x')));
            }
            return None;
        }
    }
    None
}

pub fn parse_operand(text: &str) -> Result<Operand, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(Operand::Imp);
    }
    if t == "a" || t == "A" {
        return Ok(Operand::Acc);
    }
    if let Some(r) = t.strip_prefix('#') {
        return Ok(Operand::Imm(parse_expr(r)?));
    }
    if t.starts_with('(') {
        // (zp),y  (zp,x)  (zp)  (abs)
        let tl = t.to_lowercase().replace(' ', "");
        if tl.ends_with("),y") {
            let inner = &t[1..t.rfind(')').unwrap_or(t.len())];
            return Ok(Operand::IndY(parse_expr(inner)?));
        }
        if tl.ends_with(",x)") {
            let close = t.rfind(')').unwrap_or(t.len());
            let inner = &t[1..close];
            let comma = inner.rfind(',').unwrap_or(inner.len());
            return Ok(Operand::IndX(parse_expr(&inner[..comma])?));
        }
        if tl.ends_with(')') {
            let inner = &t[1..t.len() - 1];
            return Ok(Operand::Ind(parse_expr(inner)?));
        }
        return Err(format!("オペランド {} が正しくありません", t));
    }
    if let Some((base, r)) = split_index(t) {
        let e = parse_expr(&base)?;
        return Ok(if r == 'x' { Operand::AbsX(e) } else { Operand::AbsY(e) });
    }
    Ok(Operand::Abs(parse_expr(t)?))
}

/// テキストを行ごとに分ける。ラベル・smc・命令・オペランド・コメント
pub fn parse_lines(text: &str) -> Result<Vec<AsmLine>, (usize, String)> {
    let mut out = Vec::new();
    for (ln, raw) in text.lines().enumerate() {
        // コメントは ; の後ろ（同じ行に AX65 の // コメントがあってもよい）
        let cut = match (raw.find(';'), raw.find("//")) {
            (Some(a), Some(b)) => Some((a.min(b), if a < b { 1 } else { 2 })),
            (Some(a), None) => Some((a, 1)),
            (None, Some(b)) => Some((b, 2)),
            (None, None) => None,
        };
        let (code, comment) = match cut {
            Some((i, w)) => (&raw[..i], raw[i + w..].trim().to_string()),
            None => (raw, String::new()),
        };
        let mut rest = code.trim().to_string();
        if rest.is_empty() {
            if !comment.is_empty() {
                out.push(AsmLine { line: ln, label: None, smc: false, mn: None, operand: Operand::Imp, comment });
            }
            continue;
        }
        let mut smc = false;
        if rest.starts_with("smc ") || rest.starts_with("smc\t") {
            smc = true;
            rest = rest[3..].trim().to_string();
        }
        let mut label = None;
        // ラベル name:
        if let Some(i) = rest.find(':') {
            let cand = rest[..i].trim();
            if !cand.is_empty() && cand.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '@' || c == '.') && !cand.contains(' ') {
                label = Some(cand.trim_start_matches('@').to_string());
                rest = rest[i + 1..].trim().to_string();
            }
        }
        if rest.is_empty() {
            out.push(AsmLine { line: ln, label, smc, mn: None, operand: Operand::Imp, comment });
            continue;
        }
        let (mn, op) = match rest.find(|c: char| c.is_whitespace()) {
            Some(i) => (rest[..i].to_lowercase(), rest[i..].trim().to_string()),
            None => (rest.to_lowercase(), String::new()),
        };
        let operand = parse_operand(&op).map_err(|m| (ln, m))?;
        out.push(AsmLine { line: ln, label, smc, mn: Some(mn), operand, comment });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operands() {
        assert_eq!(parse_operand("a").unwrap(), Operand::Acc);
        assert!(matches!(parse_operand("#$1D").unwrap(), Operand::Imm(_)));
        assert!(matches!(parse_operand("{BUF},x").unwrap(), Operand::AbsX(_)));
        assert!(matches!(parse_operand("(ptr),y").unwrap(), Operand::IndY(_)));
        assert!(matches!(parse_operand("($80,x)").unwrap(), Operand::IndX(_)));
        assert!(matches!(parse_operand("load+2").unwrap(), Operand::Abs(_)));
    }

    #[test]
    fn lines() {
        let v = parse_lines("    asl a   ; c\ndone:\n  rts\nloop: lsr a\n").unwrap();
        assert_eq!(v.len(), 4);
        assert_eq!(v[0].mn.as_deref(), Some("asl"));
        assert_eq!(v[1].label.as_deref(), Some("done"));
        assert_eq!(v[3].label.as_deref(), Some("loop"));
        assert_eq!(v[3].mn.as_deref(), Some("lsr"));
    }

    #[test]
    fn exprs() {
        let e = parse_expr("{SCORE.hi}+1").unwrap();
        assert_eq!(e.terms.len(), 2);
        assert_eq!(e.terms[0].1, Term::Ref("SCORE.hi".to_string()));
        let e2 = parse_expr("<{TITLE}").unwrap();
        assert_eq!(e2.sel, Sel::Lo);
    }
}
