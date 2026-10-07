//! hexe core (L0): pure logic, no I/O, no UI. Hostile input is the default (AGENTS.md §4).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use std::fmt;

/// Largest input any `hexe-core` parser accepts (capped allocations / work).
pub const MAX_INPUT: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    Empty,
    TooLarge { len: usize, max: usize },
    NotANumber(String),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::Empty => write!(f, "empty input"),
            CoreError::TooLarge { len, max } => write!(f, "input of {len} bytes exceeds the {max}-byte cap"),
            CoreError::NotANumber(s) => write!(f, "`{s}` is not a non-negative number"),
        }
    }
}

impl std::error::Error for CoreError {}

/// Example fallible operation: parse a decimal length (checked, capped). Replace with the first real atom.
pub fn parse_len(input: &str) -> Result<usize, CoreError> {
    let t = input.trim();
    if t.is_empty() {
        return Err(CoreError::Empty);
    }
    if t.len() > MAX_INPUT {
        return Err(CoreError::TooLarge { len: t.len(), max: MAX_INPUT });
    }
    t.parse::<usize>().map_err(|_| CoreError::NotANumber(t.chars().take(32).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_len_accepts_numbers() {
        assert_eq!(parse_len(" 42\n").unwrap(), 42);
    }

    #[test]
    fn parse_len_rejects_hostile_input() {
        assert_eq!(parse_len(""), Err(CoreError::Empty));
        assert!(matches!(parse_len("-1"), Err(CoreError::NotANumber(_))));
        assert!(matches!(parse_len("99999999999999999999999999"), Err(CoreError::NotANumber(_))));
    }
}
