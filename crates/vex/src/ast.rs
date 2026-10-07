//! Syntax tree.
use crate::{Ty, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub k: ExprKind,
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Lit(Value),
    Var(String),
    Attr(Option<Ty>, String),
    /// Component read/write: `.x/.y/.z/.w` (= `.r/.g/.b/.a`) → index.
    Member(Box<Expr>, usize),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `target op= rhs` (`None` = plain `=`).
    Assign(Option<BinOp>, Box<Expr>, Box<Expr>),
    IncDec {
        target: Box<Expr>,
        delta: i64,
        post: bool,
    },
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    VecLit(Vec<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Declarator {
    pub name: String,
    pub init: Option<Expr>,
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Decl(Ty, Vec<Declarator>),
    Expr(Expr),
    Block(Vec<Stmt>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    For(Option<Box<Stmt>>, Option<Expr>, Option<Expr>, Box<Stmt>),
    While(Expr, Box<Stmt>),
    Break,
    Continue,
    Return,
    Empty,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Program {
    pub stmts: Vec<Stmt>,
}
