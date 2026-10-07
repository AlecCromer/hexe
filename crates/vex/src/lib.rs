//! hexe vex (L0, standalone, no deps): clean-room VEX subset v0 = lexer + parser + tree-walking interpreter.
//! Geometry access goes through the [`Host`] trait; this crate never sees `Detail`. Hostile input is the default
//! (AGENTS.md §4): source size, nesting depth and executed steps are all capped; every error is a positioned `Err`.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
mod ast;
mod builtins;
mod error;
mod interp;
mod lexer;
mod parser;
mod value;
pub use ast::{BinOp, Declarator, Expr, ExprKind, Program, Stmt, UnOp};
pub use error::{ErrorKind, HostError, VexError};
pub use interp::{Host, run};
#[cfg(test)]
mod tests;
pub use lexer::{MAX_SRC, Tok, Token, lex};
pub use parser::{MAX_DEPTH, parse};
pub use value::{Ty, Value, builtin_ty, is_readonly};
