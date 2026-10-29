// 抽象構文木（付録A）
use crate::diag::Span;
use crate::lexer::{CharUnit, StrPrefix};

pub type NodeId = usize;

#[derive(Clone, Debug)]
pub enum AttrArg {
    Ident(String),
    Int(i128),
}

#[derive(Clone, Debug)]
pub struct Attr {
    pub name: String,
    pub args: Vec<AttrArg>,
    pub span: Span,
}

pub fn find_attr<'a>(attrs: &'a [Attr], name: &str) -> Option<&'a Attr> {
    attrs.iter().find(|a| a.name == name)
}

pub fn attr_ident_arg(a: &Attr) -> Option<String> {
    match a.args.first() {
        Some(AttrArg::Ident(s)) => Some(s.clone()),
        _ => None,
    }
}

pub fn attr_int_arg(a: &Attr) -> Option<i128> {
    match a.args.first() {
        Some(AttrArg::Int(v)) => Some(*v),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct Path {
    pub segs: Vec<String>,
    pub span: Span,
}

impl Path {
    pub fn text(&self) -> String {
        self.segs.join("::")
    }
}

#[derive(Clone, Debug)]
pub enum TypeKind {
    Path(Path),
    Array(Box<Type>, Box<Expr>),
    Ptr(Box<Type>),
    Option(Box<Type>),
    StrBuf(Box<Expr>),
    Enc(Box<Type>, String),
    Unit,
}

#[derive(Clone, Debug)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum IoDir {
    In,
    Out,
    Rw,
    Rmw,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FnKind {
    Normal,
    Const,
    Comptime,
    Asm,
    Extern,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub reg: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct AsmText {
    pub text: String,
    pub line0: u32,
    pub file: usize,
}

#[derive(Clone, Debug)]
pub struct FnDecl {
    pub name: String,
    pub kind: FnKind,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub ret_reg: Option<String>,
    pub body: Option<Block>,
    pub asm_body: Option<AsmText>,
    pub extern_addr: Option<Expr>,
    pub clobbers: Vec<String>,
    pub stack: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct UseDecl {
    pub path: Path,
    pub names: Option<Vec<(String, Span)>>,
    pub alias: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ConstDecl {
    pub name: String,
    pub ty: Option<Type>,
    pub value: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct StaticDecl {
    pub name: String,
    pub ty: Type,
    pub value: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct SharedDecl {
    pub name: String,
    pub writer: Option<String>,
    pub ty: Type,
    pub value: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct IoDecl {
    pub name: String,
    pub dir: IoDir,
    pub ty: Type,
    pub addr: Expr,
}

#[derive(Clone, Debug)]
pub struct StructDecl {
    pub name: String,
    pub fields: Vec<(String, Type, Span)>,
}

#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub name: String,
    pub variants: Vec<(String, Option<Expr>, Span)>,
}

#[derive(Clone, Debug)]
pub struct RangeDecl {
    pub name: String,
    pub lo: Expr,
    pub hi: Expr,
    pub inclusive: bool,
}

#[derive(Clone, Debug)]
pub struct FlagsDecl {
    pub name: String,
    pub names: Vec<(String, Span)>,
}

#[derive(Clone, Debug)]
pub struct FnSetDecl {
    pub name: String,
    pub fns: Vec<(String, Span)>,
}

#[derive(Clone, Debug)]
pub struct TypeAliasDecl {
    pub name: String,
    pub ty: Type,
}

#[derive(Clone, Debug)]
pub enum IfaceMember {
    Fn(FnDecl),
    Const(String, Type),
}

#[derive(Clone, Debug)]
pub struct InterfaceDecl {
    pub name: String,
    pub members: Vec<IfaceMember>,
}

#[derive(Clone, Debug)]
pub struct ImplDecl {
    pub name: String,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct CIfItems {
    pub cond: Expr,
    pub then_items: Vec<Item>,
    pub else_items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct CMatchItems {
    pub scrut: Expr,
    pub arms: Vec<(Vec<Pattern>, Vec<Item>)>,
}

#[derive(Clone, Debug)]
pub enum ItemKind {
    Use(UseDecl),
    Const(ConstDecl),
    Static(StaticDecl),
    Shared(SharedDecl),
    Io(IoDecl),
    Fn(FnDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Range(RangeDecl),
    Flags(FlagsDecl),
    FnSet(FnSetDecl),
    TypeAlias(TypeAliasDecl),
    Interface(InterfaceDecl),
    Impl(ImplDecl),
    Init(Block),
    ComptimeIf(CIfItems),
    ComptimeMatch(CMatchItems),
    ComptimeAssert(Expr, String),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub attrs: Vec<Attr>,
    pub is_pub: bool,
    pub kind: ItemKind,
    pub span: Span,
}

impl Item {
    pub fn name(&self) -> Option<String> {
        match &self.kind {
            ItemKind::Const(d) => Some(d.name.clone()),
            ItemKind::Static(d) => Some(d.name.clone()),
            ItemKind::Shared(d) => Some(d.name.clone()),
            ItemKind::Io(d) => Some(d.name.clone()),
            ItemKind::Fn(d) => Some(d.name.clone()),
            ItemKind::Struct(d) => Some(d.name.clone()),
            ItemKind::Enum(d) => Some(d.name.clone()),
            ItemKind::Range(d) => Some(d.name.clone()),
            ItemKind::Flags(d) => Some(d.name.clone()),
            ItemKind::FnSet(d) => Some(d.name.clone()),
            ItemKind::TypeAlias(d) => Some(d.name.clone()),
            ItemKind::Interface(d) => Some(d.name.clone()),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    LAnd,
    LOr,
}

impl BinOp {
    pub fn is_cmp(&self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge)
    }
    pub fn text(&self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::And => "&",
            BinOp::Or => "|",
            BinOp::Xor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::Le => "<=",
            BinOp::Ge => ">=",
            BinOp::LAnd => "&&",
            BinOp::LOr => "||",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
    Addr,
}

#[derive(Clone, Debug)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TimedKind {
    Cycles,
    IoSpan,
    MaxCycles,
}

#[derive(Clone, Debug)]
pub enum AsmBind {
    In(String, Expr),
    Out(String, String, Span),
}

#[derive(Clone, Debug)]
pub struct AsmBlock {
    pub is_unsafe: bool,
    pub binds: Vec<AsmBind>,
    pub props: Vec<(String, String)>,
    pub text: AsmText,
}

#[derive(Clone, Debug)]
pub struct Arm {
    pub pats: Vec<Pattern>,
    pub body: Expr,
}

#[derive(Clone, Debug)]
pub enum PatKind {
    Wild,
    Ident(String),
    Lit(Expr),
    Range(Expr, Expr, bool),
    Some(Box<Pattern>),
    None,
}

#[derive(Clone, Debug)]
pub struct Pattern {
    pub id: NodeId,
    pub kind: PatKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i128, bool),
    Real(f64),
    Bool(bool),
    Char(CharUnit, Option<String>),
    Str(StrPrefix, Vec<CharUnit>, Option<String>),
    Path(Path),
    Unit,
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Assign(Option<BinOp>, Box<Expr>, Box<Expr>),
    Cast(Box<Expr>, Type),
    Try(Box<Expr>),
    Field(Box<Expr>, String),
    MethodCall(Box<Expr>, String, Vec<Arg>),
    Call(Box<Expr>, Vec<Arg>),
    Index(Box<Expr>, Box<Expr>),
    Range(Option<Box<Expr>>, Option<Box<Expr>>, bool),
    StructLit(Path, Vec<(String, Expr, Span)>, Option<Box<Expr>>),
    ArrayList(Vec<Expr>),
    ArrayRepeat(Box<Expr>, Box<Expr>),
    ArrayFor(String, Box<Expr>, Box<Expr>),
    Block(Block),
    If(Box<Expr>, Block, Option<Box<Expr>>),
    // Pattern は Expr を含む（PatKind::Lit・Range）ので、Box で包んで型の大きさを有限にする（E0072）
    IfLet(Box<Pattern>, Box<Expr>, Block, Option<Box<Expr>>),
    Match(Box<Expr>, Vec<Arm>, Vec<Attr>),
    Loop(Option<String>, Block),
    While(Option<String>, Box<Expr>, Block, Vec<Attr>),
    For(Option<String>, String, Box<Expr>, Block, Vec<Attr>),
    Atomic(Block),
    Timed(TimedKind, Box<Expr>, Block),
    Asm(AsmBlock),
    Break(Option<String>, Option<Box<Expr>>),
    Continue(Option<String>),
    Return(Option<Box<Expr>>),
    ComptimeIf(Box<Expr>, Block, Option<Box<Expr>>),
    ComptimeMatch(Box<Expr>, Vec<Arm>),
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub id: NodeId,
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    /// ブロックで終わる式（後ろの ; を省略できる）
    pub fn is_block_like(&self) -> bool {
        matches!(
            self.kind,
            ExprKind::Block(_)
                | ExprKind::If(..)
                | ExprKind::IfLet(..)
                | ExprKind::Match(..)
                | ExprKind::Loop(..)
                | ExprKind::While(..)
                | ExprKind::For(..)
                | ExprKind::Atomic(_)
                | ExprKind::Timed(..)
                | ExprKind::Asm(_)
                | ExprKind::ComptimeIf(..)
                | ExprKind::ComptimeMatch(..)
        )
    }
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    Let(Option<String>, Option<Type>, Option<Expr>),
    Expr(Expr, bool),
    Item(Item),
    ComptimeAssert(Expr, String),
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub id: NodeId,
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub id: NodeId,
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

pub const KEYWORDS: [&str; 38] = [
    "as", "break", "const", "continue", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "pub", "return", "static", "struct", "true", "type", "unsafe", "use", "while",
    "range", "flags", "fnset", "interface", "io", "shared", "init", "comptime", "asm", "atomic", "cycles",
    "io_span", "max_cycles",
];

pub fn is_keyword(s: &str) -> bool {
    KEYWORDS.iter().any(|k| *k == s)
}
