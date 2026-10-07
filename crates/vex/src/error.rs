//! Errors: every failure carries a 1-based `line:col` (AGENTS.md §4: never panic, always a positioned `Err`).
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Parse,
    Runtime,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VexError {
    pub kind: ErrorKind,
    pub line: u32,
    pub col: u32,
    pub msg: String,
}

impl VexError {
    pub fn new(kind: ErrorKind, line: u32, col: u32, msg: impl Into<String>) -> Self {
        VexError { kind, line, col, msg: msg.into() }
    }
    pub fn parse(line: u32, col: u32, msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Parse, line, col, msg)
    }
}

impl fmt::Display for VexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.msg)
    }
}
impl std::error::Error for VexError {}

/// Failure reported by the geometry host; the interpreter adds the position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    Failed(String),
    Unsupported(String),
}
