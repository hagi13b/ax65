// 構文解析（付録A。式の優先順位は Rust と同じ：5.1節）
use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::lexer::{CharUnit, IntStyle, Lexer, StrPrefix, Tok, Token};
use std::collections::VecDeque;

pub type PResult<T> = Result<T, ()>;

pub struct Parser {
    pub lx: Lexer,
    buf: VecDeque<Token>,
    pub errors: Vec<Diag>,
    pub next_id: usize,
    no_struct: bool,
    in_cast: bool,
    type_no_enc: bool,
    prev: Span,
    pub file: usize,
}

pub fn tok_text(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => s.clone(),
        Tok::Int(v, st) => match st {
            IntStyle::HexDollar => format!("${:X}", v),
            IntStyle::HexC => format!("0x{:X}", v),
            IntStyle::Bin => format!("0b{:b}", v),
            IntStyle::Dec => format!("{}", v),
        },
        Tok::Real(v) => format!("{}", v),
        Tok::Char(_) => String::from("文字"),
        Tok::Str(_, _) => String::from("文字列"),
        Tok::Label(l) => format!("'{}", l),
        Tok::Punct(p) => p.to_string(),
        Tok::Eof => String::from("ファイルの終わり"),
    }
}

pub fn units_to_string(v: &[CharUnit]) -> String {
    let mut s = String::new();
    for u in v {
        match u {
            CharUnit::B(b) => s.push(*b as char),
            CharUnit::U(c) => s.push(*c),
        }
    }
    s
}

fn binop_info(t: &Tok) -> Option<(BinOp, u8)> {
    match t {
        Tok::Punct(p) => match *p {
            "||" => Some((BinOp::LOr, 1)),
            "&&" => Some((BinOp::LAnd, 2)),
            "==" => Some((BinOp::Eq, 3)),
            "!=" => Some((BinOp::Ne, 3)),
            "<" => Some((BinOp::Lt, 3)),
            ">" => Some((BinOp::Gt, 3)),
            "<=" => Some((BinOp::Le, 3)),
            ">=" => Some((BinOp::Ge, 3)),
            "|" => Some((BinOp::Or, 4)),
            "^" => Some((BinOp::Xor, 5)),
            "&" => Some((BinOp::And, 6)),
            "<<" => Some((BinOp::Shl, 7)),
            ">>" => Some((BinOp::Shr, 7)),
            "+" => Some((BinOp::Add, 8)),
            "-" => Some((BinOp::Sub, 8)),
            "*" => Some((BinOp::Mul, 9)),
            "/" => Some((BinOp::Div, 9)),
            "%" => Some((BinOp::Rem, 9)),
            _ => None,
        },
        _ => None,
    }
}

fn assign_op(t: &Tok) -> Option<Option<BinOp>> {
    match t {
        Tok::Punct(p) => match *p {
            "=" => Some(None),
            "+=" => Some(Some(BinOp::Add)),
            "-=" => Some(Some(BinOp::Sub)),
            "*=" => Some(Some(BinOp::Mul)),
            "/=" => Some(Some(BinOp::Div)),
            "%=" => Some(Some(BinOp::Rem)),
            "&=" => Some(Some(BinOp::And)),
            "|=" => Some(Some(BinOp::Or)),
            "^=" => Some(Some(BinOp::Xor)),
            "<<=" => Some(Some(BinOp::Shl)),
            ">>=" => Some(Some(BinOp::Shr)),
            _ => None,
        },
        _ => None,
    }
}

impl Parser {
    pub fn new(src: &str, file: usize, id_start: usize) -> Parser {
        Parser {
            lx: Lexer::new(src, file),
            buf: VecDeque::new(),
            errors: Vec::new(),
            next_id: id_start,
            no_struct: false,
            in_cast: false,
            type_no_enc: false,
            prev: Span::new(file, 1, 1, 1),
            file,
        }
    }

    pub fn take_errors(&mut self) -> Vec<Diag> {
        let mut v: Vec<Diag> = std::mem::take(&mut self.lx.errors);
        let mut w: Vec<Diag> = std::mem::take(&mut self.errors);
        v.append(&mut w);
        v
    }

    // ---- トークン操作 ----
    fn fill(&mut self, n: usize) {
        while self.buf.len() <= n {
            let t = self.lx.next_token();
            self.buf.push_back(t);
        }
    }
    pub fn peek(&mut self) -> Tok {
        self.fill(0);
        self.buf[0].tok.clone()
    }
    pub fn peek_at(&mut self, n: usize) -> Tok {
        self.fill(n);
        self.buf[n].tok.clone()
    }
    pub fn peek_span(&mut self) -> Span {
        self.fill(0);
        self.buf[0].span.clone()
    }
    pub fn bump(&mut self) -> Token {
        self.fill(0);
        let t = self.buf.pop_front().unwrap();
        self.prev = t.span.clone();
        t
    }
    pub fn id(&mut self) -> NodeId {
        let i = self.next_id;
        self.next_id += 1;
        i
    }
    pub fn is_p(&mut self, p: &str) -> bool {
        match self.peek() {
            Tok::Punct(q) => q == p,
            _ => false,
        }
    }
    pub fn eat_p(&mut self, p: &str) -> bool {
        if self.is_p(p) {
            self.bump();
            true
        } else {
            false
        }
    }
    pub fn is_kw(&mut self, k: &str) -> bool {
        match self.peek() {
            Tok::Ident(s) => s == k,
            _ => false,
        }
    }
    pub fn is_kw_at(&mut self, n: usize, k: &str) -> bool {
        match self.peek_at(n) {
            Tok::Ident(s) => s == k,
            _ => false,
        }
    }
    pub fn is_p_at(&mut self, n: usize, p: &str) -> bool {
        match self.peek_at(n) {
            Tok::Punct(q) => q == p,
            _ => false,
        }
    }
    pub fn eat_kw(&mut self, k: &str) -> bool {
        if self.is_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }
    pub fn at_eof(&mut self) -> bool {
        self.peek() == Tok::Eof
    }
    pub fn err<T>(&mut self, code: &str, msg: &str, span: Span) -> PResult<T> {
        self.errors.push(Diag::error(code, msg, span));
        Err(())
    }
    pub fn expect_p(&mut self, p: &str) -> PResult<Span> {
        if self.is_p(p) {
            Ok(self.bump().span)
        } else {
            let t = self.peek();
            let sp = self.peek_span();
            let code = if p == ";" { "E0105" } else { "E0102" };
            let msg = if p == ";" {
                format!("文の区切りの「;」がありません（「{}」があります）", tok_text(&t))
            } else {
                format!("「{}」が必要です（「{}」があります）", p, tok_text(&t))
            };
            self.err(code, &msg, sp)
        }
    }
    pub fn expect_kw(&mut self, k: &str) -> PResult<Span> {
        if self.is_kw(k) {
            Ok(self.bump().span)
        } else {
            let t = self.peek();
            let sp = self.peek_span();
            self.err("E0102", &format!("「{}」が必要です（「{}」があります）", k, tok_text(&t)), sp)
        }
    }
    pub fn span_from(&self, start: &Span) -> Span {
        let len = if self.prev.line == start.line && self.prev.col + self.prev.len > start.col {
            self.prev.col + self.prev.len - start.col
        } else {
            start.len.max(1)
        };
        Span::new(start.file, start.line, start.col, len)
    }

    /// 名前（キーワードは使えない）
    pub fn parse_name(&mut self) -> PResult<(String, Span)> {
        let sp = self.peek_span();
        match self.peek() {
            Tok::Ident(s) => {
                if s == "mut" {
                    self.bump();
                    self.errors.push(
                        Diag::error("E0103", "mut は予約語で、使えません", sp)
                            .note("AX65 の let・引数・static は常に書き換えられます（4.4節）"),
                    );
                    return Err(());
                }
                if is_keyword(&s) {
                    return self.err("E0101", &format!("{} はキーワードなので、名前に使えません", s), sp);
                }
                self.bump();
                Ok((s, sp))
            }
            t => self.err("E0102", &format!("名前が必要です（「{}」があります）", tok_text(&t)), sp),
        }
    }
    /// パスの 2 番目以降・フィールド・メソッドの名前（キーワードも可）
    pub fn parse_any_ident(&mut self) -> PResult<(String, Span)> {
        let sp = self.peek_span();
        match self.peek() {
            Tok::Ident(s) => {
                self.bump();
                Ok((s, sp))
            }
            t => self.err("E0102", &format!("名前が必要です（「{}」があります）", tok_text(&t)), sp),
        }
    }

    fn raw_block(&mut self) -> PResult<AsmText> {
        self.fill(0);
        if !self.is_p("{") {
            let sp = self.peek_span();
            return self.err("E0102", "アセンブリの「{」が必要です", sp);
        }
        if self.buf.len() > 1 {
            let sp = self.peek_span();
            return self.err("E0101", "内部エラー：asm の読み込み位置が正しくありません", sp);
        }
        let t = self.bump();
        let (text, line0) = self.lx.raw_block(t.end, t.span.line, t.span.col);
        Ok(AsmText { text, line0, file: self.file })
    }

    // ---- 属性 ----
    pub fn parse_attrs(&mut self) -> PResult<Vec<Attr>> {
        let mut v = Vec::new();
        while self.is_p("#") {
            let start = self.bump().span;
            self.expect_p("[")?;
            let (name, _) = self.parse_any_ident()?;
            let mut args = Vec::new();
            if self.eat_p("(") {
                loop {
                    if self.eat_p(")") {
                        break;
                    }
                    match self.peek() {
                        Tok::Ident(s) => {
                            self.bump();
                            args.push(AttrArg::Ident(s));
                        }
                        Tok::Int(n, _) => {
                            self.bump();
                            args.push(AttrArg::Int(n));
                        }
                        Tok::Punct(",") => {
                            self.bump();
                        }
                        t => {
                            let sp = self.peek_span();
                            return self.err("E0102", &format!("属性の引数が正しくありません（「{}」）", tok_text(&t)), sp);
                        }
                    }
                }
            }
            self.expect_p("]")?;
            let span = self.span_from(&start);
            v.push(Attr { name, args, span });
        }
        Ok(v)
    }

    // ---- 項目 ----
    pub fn parse_file(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        loop {
            if self.at_eof() {
                break;
            }
            match self.parse_item() {
                Ok(it) => items.push(it),
                Err(()) => break,
            }
        }
        items
    }

    pub fn parse_item(&mut self) -> PResult<Item> {
        let attrs = self.parse_attrs()?;
        self.parse_item_rest(attrs)
    }

    fn is_item_start(&mut self) -> bool {
        match self.peek() {
            Tok::Ident(s) => match s.as_str() {
                "const" | "static" | "shared" | "io" | "fn" | "struct" | "enum" | "range" | "flags" | "fnset"
                | "type" | "use" | "interface" | "impl" | "extern" | "pub" => true,
                "comptime" => self.is_kw_at(1, "fn"),
                "asm" => self.is_kw_at(1, "fn"),
                _ => false,
            },
            _ => false,
        }
    }

    pub fn parse_item_rest(&mut self, attrs: Vec<Attr>) -> PResult<Item> {
        let start = self.peek_span();
        let is_pub = self.eat_kw("pub");
        let kw = match self.peek() {
            Tok::Ident(s) => s,
            t => {
                let sp = self.peek_span();
                return self.err("E0102", &format!("宣言が必要です（「{}」があります）", tok_text(&t)), sp);
            }
        };
        let kind = match kw.as_str() {
            "use" => ItemKind::Use(self.parse_use()?),
            "const" => {
                if self.is_kw_at(1, "fn") {
                    self.bump();
                    ItemKind::Fn(self.parse_fn(FnKind::Const)?)
                } else {
                    ItemKind::Const(self.parse_const()?)
                }
            }
            "comptime" => {
                if self.is_kw_at(1, "fn") {
                    self.bump();
                    ItemKind::Fn(self.parse_fn(FnKind::Comptime)?)
                } else if self.is_kw_at(1, "if") {
                    self.bump();
                    self.bump();
                    ItemKind::ComptimeIf(self.parse_cif_items_after_if()?)
                } else if self.is_kw_at(1, "match") {
                    self.bump();
                    self.bump();
                    ItemKind::ComptimeMatch(self.parse_cmatch_items_after_match()?)
                } else if self.is_kw_at(1, "assert") {
                    self.bump();
                    self.bump();
                    let (e, m) = self.parse_cassert_body()?;
                    ItemKind::ComptimeAssert(e, m)
                } else {
                    let sp = self.peek_span();
                    return self.err("E0102", "comptime の後ろには fn・if・match・assert が必要です", sp);
                }
            }
            "static" => ItemKind::Static(self.parse_static()?),
            "shared" => ItemKind::Shared(self.parse_shared()?),
            "io" => ItemKind::Io(self.parse_io()?),
            "fn" => ItemKind::Fn(self.parse_fn(FnKind::Normal)?),
            "asm" => {
                self.bump();
                ItemKind::Fn(self.parse_fn(FnKind::Asm)?)
            }
            "extern" => {
                self.bump();
                ItemKind::Fn(self.parse_fn(FnKind::Extern)?)
            }
            "struct" => ItemKind::Struct(self.parse_struct()?),
            "enum" => ItemKind::Enum(self.parse_enum()?),
            "range" => ItemKind::Range(self.parse_range_decl()?),
            "flags" => ItemKind::Flags(self.parse_flags()?),
            "fnset" => ItemKind::FnSet(self.parse_fnset()?),
            "type" => ItemKind::TypeAlias(self.parse_type_alias()?),
            "interface" => ItemKind::Interface(self.parse_interface()?),
            "impl" => ItemKind::Impl(self.parse_impl()?),
            "init" => {
                self.bump();
                ItemKind::Init(self.parse_block()?)
            }
            _ => {
                let sp = self.peek_span();
                return self.err("E0101", &format!("「{}」で始まる宣言はありません", kw), sp);
            }
        };
        let span = self.span_from(&start);
        Ok(Item { attrs, is_pub, kind, span })
    }

    fn parse_use(&mut self) -> PResult<UseDecl> {
        self.expect_kw("use")?;
        let start = self.peek_span();
        let (first, _) = self.parse_any_ident()?;
        let mut segs = vec![first];
        let mut names = None;
        loop {
            if self.is_p("::") {
                if self.is_p_at(1, "{") {
                    self.bump();
                    self.bump();
                    let mut v = Vec::new();
                    while !self.is_p("}") {
                        let (n, sp) = self.parse_any_ident()?;
                        v.push((n, sp));
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                    self.expect_p("}")?;
                    names = Some(v);
                    break;
                } else {
                    self.bump();
                    let (s, _) = self.parse_any_ident()?;
                    segs.push(s);
                }
            } else {
                break;
            }
        }
        let span = self.span_from(&start);
        let mut alias = None;
        if self.eat_kw("as") {
            let (a, _) = self.parse_any_ident()?;
            alias = Some(a);
        }
        self.expect_p(";")?;
        Ok(UseDecl { path: Path { segs, span }, names, alias })
    }

    fn parse_const(&mut self) -> PResult<ConstDecl> {
        self.expect_kw("const")?;
        let (name, _) = self.parse_name()?;
        let mut ty = None;
        if self.eat_p(":") {
            ty = Some(self.parse_type()?);
        }
        let mut value = None;
        if self.eat_p("=") {
            value = Some(self.parse_expr()?);
        }
        self.expect_p(";")?;
        Ok(ConstDecl { name, ty, value })
    }

    fn parse_static(&mut self) -> PResult<StaticDecl> {
        self.expect_kw("static")?;
        let (name, _) = self.parse_name()?;
        self.expect_p(":")?;
        let ty = self.parse_type()?;
        let mut value = None;
        if self.eat_p("=") {
            value = Some(self.parse_expr()?);
        }
        self.expect_p(";")?;
        Ok(StaticDecl { name, ty, value })
    }

    fn parse_shared(&mut self) -> PResult<SharedDecl> {
        self.expect_kw("shared")?;
        let mut writer = None;
        if self.eat_p("(") {
            let (w, sp) = self.parse_any_ident()?;
            if w != "irq" && w != "main" && w != "nmi" {
                return self.err("E0102", "shared(…) には irq・main・nmi のどれかを書きます", sp);
            }
            writer = Some(w);
            self.expect_p(")")?;
        }
        let (name, _) = self.parse_name()?;
        self.expect_p(":")?;
        let ty = self.parse_type()?;
        let mut value = None;
        if self.eat_p("=") {
            value = Some(self.parse_expr()?);
        }
        self.expect_p(";")?;
        Ok(SharedDecl { name, writer, ty, value })
    }

    fn parse_io(&mut self) -> PResult<IoDecl> {
        self.expect_kw("io")?;
        let mut dir = IoDir::Rw;
        if self.eat_p("(") {
            let (d, sp) = self.parse_any_ident()?;
            dir = match d.as_str() {
                "in" => IoDir::In,
                "out" => IoDir::Out,
                "rmw" => IoDir::Rmw,
                _ => return self.err("E0102", "io(…) には in・out・rmw のどれかを書きます", sp),
            };
            self.expect_p(")")?;
        }
        let (name, _) = self.parse_name()?;
        self.expect_p(":")?;
        let old = self.type_no_enc;
        self.type_no_enc = true;
        let ty_r = self.parse_type();
        self.type_no_enc = old;
        let mut ty = ty_r?;
        // 変換表付きの型：io X: char@enc @ $D000;
        if self.is_p("@") && self.is_p_at(2, "@") {
            if let Tok::Ident(enc) = self.peek_at(1) {
                self.bump();
                self.bump();
                let sp = ty.span.clone();
                ty = Type { kind: TypeKind::Enc(Box::new(ty), enc), span: sp };
            }
        }
        self.expect_p("@")?;
        let addr = self.parse_expr()?;
        self.expect_p(";")?;
        Ok(IoDecl { name, dir, ty, addr })
    }

    pub fn parse_fn(&mut self, kind: FnKind) -> PResult<FnDecl> {
        let start = self.peek_span();
        self.expect_kw("fn")?;
        let (name, _) = self.parse_name()?;
        self.expect_p("(")?;
        let mut params = Vec::new();
        while !self.is_p(")") {
            let (pn, ps) = self.parse_any_ident()?;
            if pn == "mut" {
                return self.err("E0103", "mut は予約語で、使えません", ps);
            }
            self.expect_p(":")?;
            let ty = self.parse_type()?;
            let mut reg = None;
            if self.is_kw("in") {
                self.bump();
                let (r, _) = self.parse_any_ident()?;
                reg = Some(r);
            }
            params.push(Param { name: pn, ty, reg, span: ps });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        let mut ret = None;
        let mut ret_reg = None;
        if self.eat_p("->") {
            ret = Some(self.parse_type()?);
            if self.is_kw("in") {
                self.bump();
                let (r, _) = self.parse_any_ident()?;
                ret_reg = Some(r);
            }
        }
        let mut body = None;
        let mut asm_body = None;
        let mut extern_addr = None;
        let mut clobbers = Vec::new();
        let mut stack = None;
        match kind {
            FnKind::Extern => {
                if self.eat_p("@") {
                    extern_addr = Some(self.parse_bin(0)?);
                }
                loop {
                    if self.is_kw("clobbers") {
                        self.bump();
                        self.expect_p("(")?;
                        while !self.is_p(")") {
                            let (c, _) = self.parse_any_ident()?;
                            clobbers.push(c);
                            if !self.eat_p(",") {
                                break;
                            }
                        }
                        self.expect_p(")")?;
                    } else if self.is_kw("stack") {
                        self.bump();
                        self.expect_p("(")?;
                        stack = Some(self.parse_expr()?);
                        self.expect_p(")")?;
                    } else {
                        break;
                    }
                }
                self.expect_p(";")?;
            }
            FnKind::Asm => {
                asm_body = Some(self.raw_block()?);
            }
            _ => {
                if self.is_p(";") {
                    self.bump();
                } else {
                    body = Some(self.parse_block()?);
                }
            }
        }
        let span = Span::new(start.file, start.line, start.col, 2);
        Ok(FnDecl { name, kind, params, ret, ret_reg, body, asm_body, extern_addr, clobbers, stack, span })
    }

    fn parse_struct(&mut self) -> PResult<StructDecl> {
        self.expect_kw("struct")?;
        let (name, _) = self.parse_name()?;
        self.expect_p("{")?;
        let mut fields = Vec::new();
        while !self.is_p("}") {
            let (f, sp) = self.parse_name()?;
            self.expect_p(":")?;
            let t = self.parse_type()?;
            fields.push((f, t, sp));
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p("}")?;
        Ok(StructDecl { name, fields })
    }

    fn parse_enum(&mut self) -> PResult<EnumDecl> {
        self.expect_kw("enum")?;
        let (name, _) = self.parse_name()?;
        self.expect_p("{")?;
        let mut variants = Vec::new();
        while !self.is_p("}") {
            let (v, sp) = self.parse_name()?;
            let mut val = None;
            if self.eat_p("=") {
                val = Some(self.parse_bin(0)?);
            }
            variants.push((v, val, sp));
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p("}")?;
        Ok(EnumDecl { name, variants })
    }

    fn parse_range_decl(&mut self) -> PResult<RangeDecl> {
        self.expect_kw("range")?;
        let (name, _) = self.parse_name()?;
        self.expect_p("=")?;
        let e = self.parse_expr()?;
        self.expect_p(";")?;
        match e.kind {
            ExprKind::Range(Some(lo), Some(hi), inclusive) => Ok(RangeDecl { name, lo: *lo, hi: *hi, inclusive }),
            _ => self.err("E0102", "範囲型には「下限..上限」か「下限..=上限」を書きます", e.span.clone()),
        }
    }

    fn parse_flags(&mut self) -> PResult<FlagsDecl> {
        self.expect_kw("flags")?;
        let (name, _) = self.parse_name()?;
        self.expect_p("{")?;
        let mut names = Vec::new();
        while !self.is_p("}") {
            let (v, sp) = self.parse_name()?;
            names.push((v, sp));
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p("}")?;
        Ok(FlagsDecl { name, names })
    }

    fn parse_fnset(&mut self) -> PResult<FnSetDecl> {
        self.expect_kw("fnset")?;
        let (name, _) = self.parse_name()?;
        self.expect_p("=")?;
        self.expect_p("{")?;
        let mut fns = Vec::new();
        while !self.is_p("}") {
            let (v, sp) = self.parse_name()?;
            fns.push((v, sp));
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p("}")?;
        self.expect_p(";")?;
        Ok(FnSetDecl { name, fns })
    }

    fn parse_type_alias(&mut self) -> PResult<TypeAliasDecl> {
        self.expect_kw("type")?;
        let (name, _) = self.parse_name()?;
        self.expect_p("=")?;
        let ty = self.parse_type()?;
        self.expect_p(";")?;
        Ok(TypeAliasDecl { name, ty })
    }

    fn parse_interface(&mut self) -> PResult<InterfaceDecl> {
        self.expect_kw("interface")?;
        let (name, _) = self.parse_name()?;
        self.expect_p("{")?;
        let mut members = Vec::new();
        while !self.is_p("}") {
            let _attrs = self.parse_attrs()?;
            if self.is_kw("fn") {
                let f = self.parse_fn(FnKind::Normal)?;
                members.push(IfaceMember::Fn(f));
            } else if self.is_kw("const") {
                self.bump();
                let (n, sp) = self.parse_name()?;
                self.expect_p(":")?;
                let t = self.parse_type()?;
                self.expect_p(";")?;
                members.push(IfaceMember::Const(n, t, sp));
            } else {
                let sp = self.peek_span();
                return self.err("E0102", "インターフェースには fn と const だけを書けます", sp);
            }
        }
        self.expect_p("}")?;
        Ok(InterfaceDecl { name, members })
    }

    fn parse_impl(&mut self) -> PResult<ImplDecl> {
        self.expect_kw("impl")?;
        let (name, _) = self.parse_name()?;
        self.expect_p("{")?;
        let mut items = Vec::new();
        while !self.is_p("}") {
            if self.at_eof() {
                let sp = self.peek_span();
                return self.err("E0102", "impl の「}」がありません", sp);
            }
            items.push(self.parse_item()?);
        }
        self.expect_p("}")?;
        Ok(ImplDecl { name, items })
    }

    fn parse_item_block(&mut self) -> PResult<Vec<Item>> {
        self.expect_p("{")?;
        let mut items = Vec::new();
        while !self.is_p("}") {
            if self.at_eof() {
                let sp = self.peek_span();
                return self.err("E0102", "「}」がありません", sp);
            }
            items.push(self.parse_item()?);
        }
        self.expect_p("}")?;
        Ok(items)
    }

    fn parse_cif_items_after_if(&mut self) -> PResult<CIfItems> {
        let cond = self.parse_expr_nostruct()?;
        let then_items = self.parse_item_block()?;
        let mut else_items = Vec::new();
        if self.eat_kw("else") {
            if self.is_kw("if") || (self.is_kw("comptime") && self.is_kw_at(1, "if")) {
                let start = self.peek_span();
                if self.is_kw("comptime") {
                    self.bump();
                }
                self.bump();
                let nested = self.parse_cif_items_after_if()?;
                let span = self.span_from(&start);
                else_items.push(Item { attrs: Vec::new(), is_pub: false, kind: ItemKind::ComptimeIf(nested), span });
            } else {
                else_items = self.parse_item_block()?;
            }
        }
        Ok(CIfItems { cond, then_items, else_items })
    }

    fn parse_cmatch_items_after_match(&mut self) -> PResult<CMatchItems> {
        let scrut = self.parse_expr_nostruct()?;
        self.expect_p("{")?;
        let mut arms = Vec::new();
        while !self.is_p("}") {
            let pats = self.parse_pats()?;
            self.expect_p("=>")?;
            let items = self.parse_item_block()?;
            self.eat_p(",");
            arms.push((pats, items));
        }
        self.expect_p("}")?;
        Ok(CMatchItems { scrut, arms })
    }

    fn parse_cassert_body(&mut self) -> PResult<(Expr, String)> {
        self.expect_p("(")?;
        let e = self.parse_expr()?;
        self.expect_p(",")?;
        let msg = match self.peek() {
            Tok::Str(_, v) => {
                self.bump();
                units_to_string(&v)
            }
            _ => {
                let sp = self.peek_span();
                return self.err("E0102", "comptime assert の 2 番目には説明の文字列を書きます", sp);
            }
        };
        self.expect_p(")")?;
        self.expect_p(";")?;
        Ok((e, msg))
    }

    // ---- 型 ----
    fn expect_gt(&mut self) -> PResult<()> {
        self.fill(0);
        let t = self.buf[0].tok.clone();
        match t {
            Tok::Punct(">") => {
                self.bump();
                Ok(())
            }
            Tok::Punct(">>") => {
                self.buf[0].tok = Tok::Punct(">");
                self.buf[0].span.col += 1;
                Ok(())
            }
            Tok::Punct(">=") => {
                self.buf[0].tok = Tok::Punct("=");
                self.buf[0].span.col += 1;
                Ok(())
            }
            Tok::Punct(">>=") => {
                self.buf[0].tok = Tok::Punct(">=");
                self.buf[0].span.col += 1;
                Ok(())
            }
            _ => {
                let sp = self.peek_span();
                self.err("E0102", &format!("「>」が必要です（「{}」があります）", tok_text(&t)), sp)
            }
        }
    }

    pub fn parse_type(&mut self) -> PResult<Type> {
        let start = self.peek_span();
        let t = self.peek();
        let mut ty = match t {
            Tok::Punct("[") => {
                self.bump();
                let inner = self.parse_type()?;
                self.expect_p(";")?;
                let old = self.no_struct;
                self.no_struct = false;
                let n = self.parse_expr();
                self.no_struct = old;
                let n = n?;
                self.expect_p("]")?;
                Type { kind: TypeKind::Array(Box::new(inner), Box::new(n)), span: self.span_from(&start) }
            }
            Tok::Punct("(") => {
                self.bump();
                self.expect_p(")")?;
                Type { kind: TypeKind::Unit, span: self.span_from(&start) }
            }
            Tok::Ident(s) => {
                if (s == "ptr" || s == "Option") && self.is_p_at(1, "<") {
                    self.bump();
                    self.bump();
                    let old = self.in_cast;
                    self.in_cast = false;
                    let inner = self.parse_type();
                    self.in_cast = old;
                    let inner = inner?;
                    self.expect_gt()?;
                    let kind = if s == "ptr" {
                        TypeKind::Ptr(Box::new(inner))
                    } else {
                        TypeKind::Option(Box::new(inner))
                    };
                    Type { kind, span: self.span_from(&start) }
                } else if s == "strbuf" && self.is_p_at(1, "<") {
                    self.bump();
                    self.bump();
                    let n = self.parse_unary()?;
                    self.expect_gt()?;
                    Type { kind: TypeKind::StrBuf(Box::new(n)), span: self.span_from(&start) }
                } else {
                    let (first, _) = self.parse_any_ident()?;
                    let mut segs = vec![first];
                    while self.is_p("::") {
                        self.bump();
                        let (s2, _) = self.parse_any_ident()?;
                        segs.push(s2);
                    }
                    let span = self.span_from(&start);
                    Type { kind: TypeKind::Path(Path { segs, span: span.clone() }), span }
                }
            }
            _ => {
                return self.err("E0102", &format!("型が必要です（「{}」があります）", tok_text(&t)), start);
            }
        };
        // 従来の書き方 T[N1][N2] は [[T; N2]; N1]
        let mut dims = Vec::new();
        while self.is_p("[") && !self.in_cast {
            self.bump();
            let old = self.no_struct;
            self.no_struct = false;
            let n = self.parse_expr();
            self.no_struct = old;
            dims.push(n?);
            self.expect_p("]")?;
        }
        for d in dims.into_iter().rev() {
            let sp = self.span_from(&start);
            ty = Type { kind: TypeKind::Array(Box::new(ty), Box::new(d)), span: sp };
        }
        if !self.type_no_enc && self.is_p("@") {
            if let Tok::Ident(enc) = self.peek_at(1) {
                self.bump();
                self.bump();
                let sp = self.span_from(&start);
                ty = Type { kind: TypeKind::Enc(Box::new(ty), enc), span: sp };
            }
        }
        Ok(ty)
    }

    // ---- 式 ----
    fn mk(&mut self, kind: ExprKind, start: &Span) -> Expr {
        let id = self.id();
        let span = self.span_from(start);
        Expr { id, kind, span }
    }

    pub fn parse_expr(&mut self) -> PResult<Expr> {
        self.parse_assign()
    }

    pub fn parse_expr_nostruct(&mut self) -> PResult<Expr> {
        let old = self.no_struct;
        self.no_struct = true;
        let r = self.parse_expr();
        self.no_struct = old;
        r
    }

    fn can_start_expr(&mut self) -> bool {
        match self.peek() {
            Tok::Punct(p) => !matches!(p, ";" | "}" | ")" | "]" | "," | "=>" | "=" | "{"),
            Tok::Eof => false,
            Tok::Ident(s) => s != "else",
            _ => true,
        }
    }

    fn parse_assign(&mut self) -> PResult<Expr> {
        let start = self.peek_span();
        let lhs = self.parse_range()?;
        let t = self.peek();
        if let Some(op) = assign_op(&t) {
            self.bump();
            let rhs = self.parse_assign()?;
            return Ok(self.mk(ExprKind::Assign(op, Box::new(lhs), Box::new(rhs)), &start));
        }
        Ok(lhs)
    }

    fn parse_range(&mut self) -> PResult<Expr> {
        let start = self.peek_span();
        if self.is_p("..") || self.is_p("..=") {
            let incl = self.is_p("..=");
            self.bump();
            let hi = if self.can_start_expr() { Some(Box::new(self.parse_bin(0)?)) } else { None };
            return Ok(self.mk(ExprKind::Range(None, hi, incl), &start));
        }
        let lhs = self.parse_bin(0)?;
        if self.is_p("..") || self.is_p("..=") {
            let incl = self.is_p("..=");
            self.bump();
            let hi = if self.can_start_expr() { Some(Box::new(self.parse_bin(0)?)) } else { None };
            return Ok(self.mk(ExprKind::Range(Some(Box::new(lhs)), hi, incl), &start));
        }
        Ok(lhs)
    }

    pub fn parse_bin(&mut self, min: u8) -> PResult<Expr> {
        let start = self.peek_span();
        let mut lhs = self.parse_cast()?;
        let mut last_cmp = false;
        loop {
            let t = self.peek();
            let (op, prec) = match binop_info(&t) {
                Some(x) => x,
                None => break,
            };
            if prec < min {
                break;
            }
            let opspan = self.peek_span();
            if op.is_cmp() && last_cmp {
                self.errors.push(
                    Diag::error("E0104", "比較をつなげて書くことはできません", opspan)
                        .help("(a < b) && (b < c) のように分けて書いてください"),
                );
                return Err(());
            }
            self.bump();
            let rhs = self.parse_bin(prec + 1)?;
            lhs = self.mk(ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), &start);
            last_cmp = op.is_cmp();
        }
        Ok(lhs)
    }

    fn parse_cast(&mut self) -> PResult<Expr> {
        let start = self.peek_span();
        let mut e = self.parse_unary()?;
        while self.is_kw("as") {
            self.bump();
            let old = self.in_cast;
            self.in_cast = true;
            let t = self.parse_type();
            self.in_cast = old;
            let t = t?;
            e = self.mk(ExprKind::Cast(Box::new(e), t), &start);
        }
        Ok(e)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        let start = self.peek_span();
        if self.is_p("-") {
            self.bump();
            let inner = self.parse_unary()?;
            if let ExprKind::Int(v, h) = &inner.kind {
                let (v, h) = (*v, *h);
                return Ok(self.mk(ExprKind::Int(-v, h), &start));
            }
            if let ExprKind::Real(v) = &inner.kind {
                let v = *v;
                return Ok(self.mk(ExprKind::Real(-v), &start));
            }
            return Ok(self.mk(ExprKind::Unary(UnOp::Neg, Box::new(inner)), &start));
        }
        if self.is_p("!") {
            self.bump();
            let inner = self.parse_unary()?;
            return Ok(self.mk(ExprKind::Unary(UnOp::Not, Box::new(inner)), &start));
        }
        if self.is_p("&") {
            self.bump();
            let inner = self.parse_unary()?;
            return Ok(self.mk(ExprKind::Unary(UnOp::Addr, Box::new(inner)), &start));
        }
        self.parse_postfix()
    }

    fn parse_args(&mut self) -> PResult<Vec<Arg>> {
        self.expect_p("(")?;
        let old = self.no_struct;
        self.no_struct = false;
        let mut args = Vec::new();
        while !self.is_p(")") {
            let mut name = None;
            if let Tok::Ident(n) = self.peek() {
                if self.is_p_at(1, "=") {
                    self.bump();
                    self.bump();
                    name = Some(n);
                }
            }
            let v = match self.parse_expr() {
                Ok(v) => v,
                Err(()) => {
                    self.no_struct = old;
                    return Err(());
                }
            };
            args.push(Arg { name, value: v });
            if !self.eat_p(",") {
                break;
            }
        }
        self.no_struct = old;
        self.expect_p(")")?;
        Ok(args)
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let start = self.peek_span();
        let mut e = self.parse_primary()?;
        loop {
            if self.is_p("?") {
                self.bump();
                e = self.mk(ExprKind::Try(Box::new(e)), &start);
            } else if self.is_p(".") {
                self.bump();
                let (name, _) = self.parse_any_ident()?;
                if self.is_p("(") {
                    let args = self.parse_args()?;
                    e = self.mk(ExprKind::MethodCall(Box::new(e), name, args), &start);
                } else {
                    e = self.mk(ExprKind::Field(Box::new(e), name), &start);
                }
            } else if self.is_p("(") {
                let args = self.parse_args()?;
                e = self.mk(ExprKind::Call(Box::new(e), args), &start);
            } else if self.is_p("[") {
                self.bump();
                let old = self.no_struct;
                self.no_struct = false;
                let idx = self.parse_expr();
                self.no_struct = old;
                let idx = idx?;
                self.expect_p("]")?;
                e = self.mk(ExprKind::Index(Box::new(e), Box::new(idx)), &start);
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_opt_enc(&mut self) -> Option<String> {
        if self.is_p("@") {
            if let Tok::Ident(enc) = self.peek_at(1) {
                self.bump();
                self.bump();
                return Some(enc);
            }
        }
        None
    }

    fn parse_label_opt(&mut self) -> Option<String> {
        if let Tok::Label(l) = self.peek() {
            self.bump();
            return Some(l);
        }
        None
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let start = self.peek_span();
        let t = self.peek();
        match t {
            Tok::Int(v, st) => {
                self.bump();
                let hex = st == IntStyle::HexDollar || st == IntStyle::HexC;
                Ok(self.mk(ExprKind::Int(v, hex), &start))
            }
            Tok::Real(v) => {
                self.bump();
                Ok(self.mk(ExprKind::Real(v), &start))
            }
            Tok::Char(u) => {
                self.bump();
                let enc = self.parse_opt_enc();
                Ok(self.mk(ExprKind::Char(u, enc), &start))
            }
            Tok::Str(p, v) => {
                self.bump();
                let enc = self.parse_opt_enc();
                Ok(self.mk(ExprKind::Str(p, v, enc), &start))
            }
            Tok::Label(_) => self.parse_block_like(Vec::new()),
            Tok::Punct("(") => {
                self.bump();
                if self.eat_p(")") {
                    return Ok(self.mk(ExprKind::Unit, &start));
                }
                let old = self.no_struct;
                self.no_struct = false;
                let e = self.parse_expr();
                self.no_struct = old;
                let e = e?;
                self.expect_p(")")?;
                Ok(e)
            }
            Tok::Punct("[") => self.parse_array_lit(),
            Tok::Punct("{") => {
                let b = self.parse_block()?;
                Ok(self.mk(ExprKind::Block(b), &start))
            }
            Tok::Punct("#") => {
                let attrs = self.parse_attrs()?;
                self.parse_block_like(attrs)
            }
            Tok::Ident(s) => match s.as_str() {
                "true" => {
                    self.bump();
                    Ok(self.mk(ExprKind::Bool(true), &start))
                }
                "false" => {
                    self.bump();
                    Ok(self.mk(ExprKind::Bool(false), &start))
                }
                "if" | "match" | "loop" | "while" | "for" | "atomic" | "cycles" | "io_span" | "max_cycles"
                | "asm" | "unsafe" | "comptime" => self.parse_block_like(Vec::new()),
                "break" => {
                    self.bump();
                    let label = self.parse_label_opt();
                    let v = if self.can_start_expr() { Some(Box::new(self.parse_expr()?)) } else { None };
                    Ok(self.mk(ExprKind::Break(label, v), &start))
                }
                "continue" => {
                    self.bump();
                    let label = self.parse_label_opt();
                    Ok(self.mk(ExprKind::Continue(label), &start))
                }
                "return" => {
                    self.bump();
                    let v = if self.can_start_expr() { Some(Box::new(self.parse_expr()?)) } else { None };
                    Ok(self.mk(ExprKind::Return(v), &start))
                }
                "let" => self.err("E0102", "let は文の先頭にだけ書けます", start),
                "mut" => {
                    self.bump();
                    self.err("E0103", "mut は予約語で、使えません", start)
                }
                _ => self.parse_path_expr(),
            },
            t => self.err("E0102", &format!("式が必要です（「{}」があります）", tok_text(&t)), start),
        }
    }

    fn parse_path_expr(&mut self) -> PResult<Expr> {
        let start = self.peek_span();
        let (first, _) = self.parse_any_ident()?;
        if is_keyword(&first) && first != "init" {
            return self.err("E0101", &format!("{} はキーワードなので、式には書けません", first), start);
        }
        let mut segs = vec![first];
        while self.is_p("::") {
            self.bump();
            let (s, _) = self.parse_any_ident()?;
            segs.push(s);
        }
        let pspan = self.span_from(&start);
        let path = Path { segs, span: pspan };
        // 構造体の値
        if !self.no_struct && self.is_p("{") {
            let is_struct = match self.peek_at(1) {
                Tok::Ident(_) => self.is_p_at(2, ":"),
                Tok::Punct("..") => true,
                _ => false,
            };
            if is_struct {
                return self.parse_struct_lit(path, &start);
            }
        }
        Ok(self.mk(ExprKind::Path(path), &start))
    }

    fn parse_struct_lit(&mut self, path: Path, start: &Span) -> PResult<Expr> {
        self.expect_p("{")?;
        let mut fields = Vec::new();
        let mut base = None;
        loop {
            if self.is_p("}") {
                break;
            }
            if self.eat_p("..") {
                base = Some(Box::new(self.parse_expr()?));
                break;
            }
            let (f, sp) = self.parse_any_ident()?;
            self.expect_p(":")?;
            let v = self.parse_expr()?;
            fields.push((f, v, sp));
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p("}")?;
        Ok(self.mk(ExprKind::StructLit(path, fields, base), start))
    }

    fn parse_array_lit(&mut self) -> PResult<Expr> {
        let start = self.peek_span();
        self.expect_p("[")?;
        let old = self.no_struct;
        self.no_struct = false;
        let r = self.parse_array_lit_inner(&start);
        self.no_struct = old;
        r
    }

    fn parse_array_lit_inner(&mut self, start: &Span) -> PResult<Expr> {
        if self.eat_p("]") {
            return Ok(self.mk(ExprKind::ArrayList(Vec::new()), start));
        }
        if self.is_kw("for") {
            self.bump();
            let (name, _) = self.parse_any_ident()?;
            self.expect_kw("in")?;
            let r = self.parse_range()?;
            self.expect_p("=>")?;
            let body = self.parse_expr()?;
            self.expect_p("]")?;
            return Ok(self.mk(ExprKind::ArrayFor(name, Box::new(r), Box::new(body)), start));
        }
        let first = self.parse_expr()?;
        if self.eat_p(";") {
            let n = self.parse_expr()?;
            self.expect_p("]")?;
            return Ok(self.mk(ExprKind::ArrayRepeat(Box::new(first), Box::new(n)), start));
        }
        let mut v = vec![first];
        while self.eat_p(",") {
            if self.is_p("]") {
                break;
            }
            v.push(self.parse_expr()?);
        }
        self.expect_p("]")?;
        Ok(self.mk(ExprKind::ArrayList(v), start))
    }

    fn starts_block_like(&mut self) -> bool {
        match self.peek() {
            Tok::Punct("{") => true,
            Tok::Label(_) => true,
            Tok::Ident(s) => match s.as_str() {
                "if" | "match" | "loop" | "while" | "for" | "atomic" | "cycles" | "io_span" | "max_cycles" | "asm"
                | "unsafe" => true,
                "comptime" => self.is_kw_at(1, "if") || self.is_kw_at(1, "match"),
                _ => false,
            },
            _ => false,
        }
    }

    /// ブロックで終わる式（属性・ラベル付き）
    fn parse_block_like(&mut self, attrs: Vec<Attr>) -> PResult<Expr> {
        let start = self.peek_span();
        let mut label = None;
        if let Tok::Label(l) = self.peek() {
            self.bump();
            self.expect_p(":")?;
            label = Some(l);
        }
        let t = self.peek();
        let kw = match t {
            Tok::Ident(s) => s,
            Tok::Punct("{") => {
                let b = self.parse_block()?;
                return Ok(self.mk(ExprKind::Block(b), &start));
            }
            t => return self.err("E0102", &format!("式が必要です（「{}」があります）", tok_text(&t)), start),
        };
        if label.is_some() && !(kw == "loop" || kw == "while" || kw == "for") {
            return self.err("E0102", "ラベルは loop・while・for の前にだけ付けられます", start);
        }
        match kw.as_str() {
            "if" => self.parse_if(),
            "match" => {
                self.bump();
                let scrut = self.parse_expr_nostruct()?;
                let arms = self.parse_arms()?;
                Ok(self.mk(ExprKind::Match(Box::new(scrut), arms, attrs), &start))
            }
            "loop" => {
                self.bump();
                let b = self.parse_block()?;
                Ok(self.mk(ExprKind::Loop(label, b), &start))
            }
            "while" => {
                self.bump();
                let c = self.parse_expr_nostruct()?;
                let b = self.parse_block()?;
                Ok(self.mk(ExprKind::While(label, Box::new(c), b, attrs), &start))
            }
            "for" => {
                self.bump();
                let (name, _) = self.parse_any_ident()?;
                self.expect_kw("in")?;
                let it = self.parse_expr_nostruct()?;
                let b = self.parse_block()?;
                Ok(self.mk(ExprKind::For(label, name, Box::new(it), b, attrs), &start))
            }
            "atomic" => {
                self.bump();
                let b = self.parse_block()?;
                Ok(self.mk(ExprKind::Atomic(b), &start))
            }
            "cycles" | "io_span" | "max_cycles" => {
                self.bump();
                let k = match kw.as_str() {
                    "cycles" => TimedKind::Cycles,
                    "io_span" => TimedKind::IoSpan,
                    _ => TimedKind::MaxCycles,
                };
                self.expect_p("(")?;
                let n = self.parse_expr()?;
                self.expect_p(")")?;
                let b = self.parse_block()?;
                Ok(self.mk(ExprKind::Timed(k, Box::new(n), b), &start))
            }
            "asm" => self.parse_asm(false),
            "unsafe" => {
                self.bump();
                if !self.is_kw("asm") {
                    let sp = self.peek_span();
                    return self.err("E0102", "unsafe の後ろには asm だけを書けます", sp);
                }
                self.parse_asm(true)
            }
            "comptime" => {
                self.bump();
                if self.eat_kw("if") {
                    self.parse_comptime_if_rest(&start)
                } else {
                    self.expect_kw("match")?;
                    let scrut = self.parse_expr_nostruct()?;
                    let arms = self.parse_arms()?;
                    Ok(self.mk(ExprKind::ComptimeMatch(Box::new(scrut), arms), &start))
                }
            }
            _ => self.parse_expr(),
        }
    }

    fn parse_comptime_if_rest(&mut self, start: &Span) -> PResult<Expr> {
        let cond = self.parse_expr_nostruct()?;
        let b = self.parse_block()?;
        let mut els = None;
        if self.eat_kw("else") {
            let s2 = self.peek_span();
            if self.is_kw("if") || (self.is_kw("comptime") && self.is_kw_at(1, "if")) {
                if self.is_kw("comptime") {
                    self.bump();
                }
                self.bump();
                els = Some(Box::new(self.parse_comptime_if_rest(&s2)?));
            } else {
                let eb = self.parse_block()?;
                els = Some(Box::new(self.mk(ExprKind::Block(eb), &s2)));
            }
        }
        Ok(self.mk(ExprKind::ComptimeIf(Box::new(cond), b, els), start))
    }

    fn parse_if(&mut self) -> PResult<Expr> {
        let start = self.peek_span();
        self.expect_kw("if")?;
        if self.eat_kw("let") {
            let pat = self.parse_pattern()?;
            self.expect_p("=")?;
            let scrut = self.parse_expr_nostruct()?;
            let b = self.parse_block()?;
            let els = self.parse_else()?;
            return Ok(self.mk(ExprKind::IfLet(pat, Box::new(scrut), b, els), &start));
        }
        let cond = self.parse_expr_nostruct()?;
        let b = self.parse_block()?;
        let els = self.parse_else()?;
        Ok(self.mk(ExprKind::If(Box::new(cond), b, els), &start))
    }

    fn parse_else(&mut self) -> PResult<Option<Box<Expr>>> {
        if !self.eat_kw("else") {
            return Ok(None);
        }
        let s2 = self.peek_span();
        if self.is_kw("if") {
            return Ok(Some(Box::new(self.parse_if()?)));
        }
        let b = self.parse_block()?;
        Ok(Some(Box::new(self.mk(ExprKind::Block(b), &s2))))
    }

    fn parse_arms(&mut self) -> PResult<Vec<Arm>> {
        self.expect_p("{")?;
        let old = self.no_struct;
        self.no_struct = false;
        let mut arms = Vec::new();
        loop {
            if self.is_p("}") {
                break;
            }
            if self.at_eof() {
                let sp = self.peek_span();
                self.no_struct = old;
                return self.err("E0102", "match の「}」がありません", sp);
            }
            let astart = self.peek_span();
            let pats = match self.parse_pats() {
                Ok(p) => p,
                Err(()) => {
                    self.no_struct = old;
                    return Err(());
                }
            };
            if self.expect_p("=>").is_err() {
                self.no_struct = old;
                return Err(());
            }
            let body = if self.is_p("{") {
                let bs = self.peek_span();
                let b = match self.parse_block() {
                    Ok(b) => b,
                    Err(()) => {
                        self.no_struct = old;
                        return Err(());
                    }
                };
                self.eat_p(",");
                self.mk(ExprKind::Block(b), &bs)
            } else {
                let e = match self.parse_expr() {
                    Ok(e) => e,
                    Err(()) => {
                        self.no_struct = old;
                        return Err(());
                    }
                };
                if !self.eat_p(",") && !self.is_p("}") {
                    if !e.is_block_like() {
                        let sp = self.peek_span();
                        self.no_struct = old;
                        return self.err("E0102", "match の腕の後ろには「,」が必要です", sp);
                    }
                }
                e
            };
            let span = self.span_from(&astart);
            arms.push(Arm { pats, body, span });
        }
        self.no_struct = old;
        self.expect_p("}")?;
        Ok(arms)
    }

    pub fn parse_pats(&mut self) -> PResult<Vec<Pattern>> {
        let mut v = vec![self.parse_pattern()?];
        while self.eat_p("|") {
            v.push(self.parse_pattern()?);
        }
        Ok(v)
    }

    fn parse_pat_value(&mut self) -> PResult<Expr> {
        let start = self.peek_span();
        match self.peek() {
            Tok::Punct("-") => {
                self.bump();
                match self.peek() {
                    Tok::Int(v, st) => {
                        self.bump();
                        let hex = st != IntStyle::Dec;
                        Ok(self.mk(ExprKind::Int(-v, hex), &start))
                    }
                    t => self.err("E0102", &format!("パターンが正しくありません（「{}」）", tok_text(&t)), start),
                }
            }
            Tok::Int(v, st) => {
                self.bump();
                let hex = st == IntStyle::HexDollar || st == IntStyle::HexC;
                Ok(self.mk(ExprKind::Int(v, hex), &start))
            }
            Tok::Char(u) => {
                self.bump();
                let enc = self.parse_opt_enc();
                Ok(self.mk(ExprKind::Char(u, enc), &start))
            }
            Tok::Ident(s) if s == "true" || s == "false" => {
                self.bump();
                Ok(self.mk(ExprKind::Bool(s == "true"), &start))
            }
            Tok::Ident(_) => {
                let (first, _) = self.parse_any_ident()?;
                let mut segs = vec![first];
                while self.is_p("::") {
                    self.bump();
                    let (s2, _) = self.parse_any_ident()?;
                    segs.push(s2);
                }
                let span = self.span_from(&start);
                Ok(self.mk(ExprKind::Path(Path { segs, span }), &start))
            }
            t => self.err("E0102", &format!("パターンが正しくありません（「{}」）", tok_text(&t)), start),
        }
    }

    pub fn parse_pattern(&mut self) -> PResult<Pattern> {
        let start = self.peek_span();
        let id = self.id();
        if let Tok::Ident(s) = self.peek() {
            if s == "_" {
                self.bump();
                return Ok(Pattern { id, kind: PatKind::Wild, span: start });
            }
            if s == "None" && !self.is_p_at(1, "::") {
                self.bump();
                return Ok(Pattern { id, kind: PatKind::None, span: start });
            }
            if s == "Some" && self.is_p_at(1, "(") {
                self.bump();
                self.bump();
                let inner = self.parse_pattern()?;
                self.expect_p(")")?;
                let span = self.span_from(&start);
                return Ok(Pattern { id, kind: PatKind::Some(Box::new(inner)), span });
            }
            if s != "true" && s != "false" && !self.is_p_at(1, "::") && !self.is_p_at(1, "..") && !self.is_p_at(1, "..=") {
                self.bump();
                return Ok(Pattern { id, kind: PatKind::Ident(s), span: start });
            }
        }
        let lo = self.parse_pat_value()?;
        if self.is_p("..") || self.is_p("..=") {
            let incl = self.is_p("..=");
            self.bump();
            let hi = self.parse_pat_value()?;
            let span = self.span_from(&start);
            return Ok(Pattern { id, kind: PatKind::Range(lo, hi, incl), span });
        }
        let span = self.span_from(&start);
        Ok(Pattern { id, kind: PatKind::Lit(lo), span })
    }

    fn parse_asm(&mut self, is_unsafe: bool) -> PResult<Expr> {
        let start = self.peek_span();
        self.expect_kw("asm")?;
        let mut binds = Vec::new();
        let mut props = Vec::new();
        if self.is_p("(") {
            self.bump();
            loop {
                if self.is_p(")") {
                    break;
                }
                if is_unsafe {
                    let (name, _) = self.parse_any_ident()?;
                    let mut text = String::new();
                    if self.eat_p("(") {
                        let mut depth = 1;
                        loop {
                            let t = self.peek();
                            if t == Tok::Eof {
                                let sp = self.peek_span();
                                return self.err("E0102", "「)」がありません", sp);
                            }
                            if t == Tok::Punct("(") {
                                depth += 1;
                            }
                            if t == Tok::Punct(")") {
                                depth -= 1;
                                if depth == 0 {
                                    self.bump();
                                    break;
                                }
                            }
                            self.bump();
                            text.push_str(&tok_text(&t));
                        }
                    }
                    props.push((name, text));
                } else {
                    let (dir, sp) = self.parse_any_ident()?;
                    let (reg, _) = self.parse_any_ident()?;
                    if dir == "in" {
                        self.expect_p("=")?;
                        let e = self.parse_range()?;
                        binds.push(AsmBind::In(reg, e));
                    } else if dir == "out" {
                        self.expect_p("->")?;
                        let (n, nsp) = self.parse_any_ident()?;
                        binds.push(AsmBind::Out(reg, n, nsp));
                    } else {
                        return self.err("E0102", "asm の受け渡しは in か out で書きます", sp);
                    }
                }
                if !self.eat_p(",") {
                    break;
                }
            }
            self.expect_p(")")?;
        }
        let text = self.raw_block()?;
        Ok(self.mk(ExprKind::Asm(AsmBlock { is_unsafe, binds, props, text }), &start))
    }

    // ---- ブロックと文 ----
    pub fn parse_block(&mut self) -> PResult<Block> {
        let start = self.expect_p("{")?;
        let old = self.no_struct;
        self.no_struct = false;
        let r = self.parse_block_inner(&start);
        self.no_struct = old;
        r
    }

    fn parse_block_inner(&mut self, start: &Span) -> PResult<Block> {
        let mut stmts = Vec::new();
        let mut tail: Option<Box<Expr>> = None;
        loop {
            if self.is_p("}") {
                break;
            }
            if self.at_eof() {
                let sp = self.peek_span();
                return self.err("E0102", "「}」がありません", sp);
            }
            if self.eat_p(";") {
                continue;
            }
            let sstart = self.peek_span();
            if self.is_kw("let") {
                let s = self.parse_let()?;
                stmts.push(s);
                continue;
            }
            let attrs = self.parse_attrs()?;
            if self.is_item_start() {
                let it = self.parse_item_rest(attrs)?;
                let id = self.id();
                let span = self.span_from(&sstart);
                stmts.push(Stmt { id, kind: StmtKind::Item(it), span });
                continue;
            }
            if self.is_kw("comptime") && self.is_kw_at(1, "assert") {
                self.bump();
                self.bump();
                let (e, m) = self.parse_cassert_body()?;
                let id = self.id();
                let span = self.span_from(&sstart);
                stmts.push(Stmt { id, kind: StmtKind::ComptimeAssert(e, m), span });
                continue;
            }
            let e = if self.starts_block_like() || !attrs.is_empty() {
                let e1 = self.parse_block_like(attrs)?;
                // ブロックの式の後ろに . や ? が続く場合は式として続ける
                if self.is_p(".") || self.is_p("?") {
                    self.continue_postfix(e1, &sstart)?
                } else {
                    e1
                }
            } else {
                self.parse_expr()?
            };
            if self.eat_p(";") {
                let id = self.id();
                let span = self.span_from(&sstart);
                stmts.push(Stmt { id, kind: StmtKind::Expr(e, true), span });
                continue;
            }
            if self.is_p("}") {
                tail = Some(Box::new(e));
                break;
            }
            if e.is_block_like() {
                let id = self.id();
                let span = self.span_from(&sstart);
                stmts.push(Stmt { id, kind: StmtKind::Expr(e, false), span });
                continue;
            }
            let t = self.peek();
            let sp = self.peek_span();
            return self.err("E0105", &format!("文の区切りの「;」がありません（「{}」があります）", tok_text(&t)), sp);
        }
        self.expect_p("}")?;
        let id = self.id();
        let span = self.span_from(start);
        Ok(Block { id, stmts, tail, span })
    }

    fn continue_postfix(&mut self, mut e: Expr, start: &Span) -> PResult<Expr> {
        loop {
            if self.is_p("?") {
                self.bump();
                e = self.mk(ExprKind::Try(Box::new(e)), start);
            } else if self.is_p(".") {
                self.bump();
                let (name, _) = self.parse_any_ident()?;
                if self.is_p("(") {
                    let args = self.parse_args()?;
                    e = self.mk(ExprKind::MethodCall(Box::new(e), name, args), start);
                } else {
                    e = self.mk(ExprKind::Field(Box::new(e), name), start);
                }
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_let(&mut self) -> PResult<Stmt> {
        let start = self.peek_span();
        self.expect_kw("let")?;
        let name = if self.is_kw("_") {
            self.bump();
            None
        } else {
            let (n, _) = self.parse_name()?;
            Some(n)
        };
        let mut ty = None;
        if self.eat_p(":") {
            ty = Some(self.parse_type()?);
        }
        let mut init = None;
        if self.eat_p("=") {
            init = Some(self.parse_expr()?);
        }
        self.expect_p(";")?;
        let id = self.id();
        let span = self.span_from(&start);
        Ok(Stmt { id, kind: StmtKind::Let(name, ty, init), span })
    }
}

/// 1 つのソースを解析する
pub fn parse_source(src: &str, file: usize, id_start: usize) -> (Vec<Item>, Vec<Diag>, usize) {
    let mut p = Parser::new(src, file, id_start);
    let items = p.parse_file();
    let errs = p.take_errors();
    (items, errs, p.next_id)
}

#[allow(dead_code)]
pub fn str_prefix_name(p: &StrPrefix) -> &'static str {
    match p {
        StrPrefix::Raw => "",
        StrPrefix::P => "p",
        StrPrefix::Z => "z",
        StrPrefix::Hi => "hi",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> Vec<Item> {
        let (items, errs, _) = parse_source(src, 0, 0);
        assert!(errs.is_empty(), "errors: {:?}", errs);
        items
    }

    #[test]
    fn items_basic() {
        let items = parse_ok(
            "use console::{putc, poll};\nconst W: u8 = 20;\nrange Mid = 0..8;\nenum Key { Left, Right, Quit }\n\
             flags Ifr { A, B }\nstruct M { x: u8, y: u8 }\nstatic S: [M; Mid] = [M { x: 0, y: 0 }; Mid];\n\
             shared(irq) T: u16 = 0;\nio(in) IFR: Ifr @ $600D;\nfn main() { let a = 1; a += 2; }\n",
        );
        assert_eq!(items.len(), 10);
    }

    #[test]
    fn precedence() {
        let items = parse_ok("const X: bool = A & $10 == 0;");
        if let ItemKind::Const(c) = &items[0].kind {
            match &c.value.as_ref().unwrap().kind {
                ExprKind::Binary(BinOp::Eq, l, _) => match &l.kind {
                    ExprKind::Binary(BinOp::And, _, _) => {}
                    _ => panic!("& should bind tighter"),
                },
                _ => panic!("expected =="),
            }
        } else {
            panic!();
        }
    }

    #[test]
    fn chained_compare_error() {
        let (_, errs, _) = parse_source("const X: bool = a < b < c;", 0, 0);
        assert!(errs.iter().any(|d| d.code == "E0104"));
    }

    #[test]
    fn mut_error() {
        let (_, errs, _) = parse_source("fn f() { let mut x = 1; }", 0, 0);
        assert!(errs.iter().any(|d| d.code == "E0103"));
    }

    #[test]
    fn asm_fn_and_block() {
        let items = parse_ok("asm fn r(x: u8) -> u8 {\n asl a ; c {x}\n rts\n}\nfn f() { asm { lda {X}\n } let y = 1; }");
        assert_eq!(items.len(), 2);
        if let ItemKind::Fn(f) = &items[0].kind {
            assert!(f.asm_body.as_ref().unwrap().text.contains("asl a"));
        }
    }

    #[test]
    fn control_flow() {
        parse_ok(
            "fn f() -> Option<u8> { 'game: loop { if let Some(k) = g() { break 'game; } }\n\
             match c { 'A' | 'a' => Some(1), 3..=9 => None, _ => None, }\n\
             #[reg(x)] for m in Mid { M[m].y = 0; }\n for x in (0..20).rev() { }\n None }",
        );
    }

    #[test]
    fn legacy_array_type() {
        let items = parse_ok("static MAP: u8[24][40] = [[0; 40]; 24];");
        if let ItemKind::Static(s) = &items[0].kind {
            match &s.ty.kind {
                TypeKind::Array(inner, n) => {
                    assert!(matches!(n.kind, ExprKind::Int(24, _)));
                    match &inner.kind {
                        TypeKind::Array(_, n2) => assert!(matches!(n2.kind, ExprKind::Int(40, _))),
                        _ => panic!(),
                    }
                }
                _ => panic!(),
            }
        }
    }

    #[test]
    fn comptime_items() {
        parse_ok("comptime if target.has(screen) { fn a() {} } else { fn a() {} }\ncomptime assert(1 == 1, \"x\");");
    }
}
