//! hexe-cli: `--version`, `oracle <case> [input]`. Every command runs behind `catch_unwind` (AGENTS.md §4).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode;

/// Oracle entry point: `xtask oracle` runs `hexe-cli <case.toml cmd…> <input path>` and compares stdout
/// with `expected`. Add one arm per fixture family as ledger rows land.
fn oracle(case: &str, input: Option<&str>) -> Result<String, String> {
    match case {
        "parse-len" => {
            let text = std::fs::read_to_string(input.ok_or("oracle parse-len: needs an input path")?)
                .map_err(|e| e.to_string())?;
            hexe_core::parse_len(&text).map(|n| format!("{n}\n")).map_err(|e| e.to_string())
        }
        other => Err(format!("oracle: case `{other}` has no implementation yet")),
    }
}

fn dispatch(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => Ok(format!("hexe {}\n", env!("CARGO_PKG_VERSION"))),
        Some("oracle") => match args.get(1) {
            Some(case) => oracle(case, args.get(2).map(String::as_str)),
            None => Err("usage: hexe-cli oracle <case> [input]".into()),
        },
        _ => Err("usage: hexe-cli --version | oracle <case> [input]".into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match catch_unwind(AssertUnwindSafe(|| dispatch(&args))) {
        Ok(Ok(out)) => {
            print!("{out}");
            ExitCode::SUCCESS
        }
        Ok(Err(msg)) => {
            eprintln!("hexe-cli: {msg}");
            ExitCode::from(2)
        }
        Err(_) => {
            eprintln!("hexe-cli: internal error (panic caught)");
            ExitCode::from(70)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_flag_prints_name_and_version() {
        let out = dispatch(&["--version".into()]).unwrap();
        assert_eq!(out, format!("hexe {}\n", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn unknown_oracle_case_is_an_error_not_a_panic() {
        assert!(dispatch(&["oracle".into(), "nope".into()]).is_err());
    }
}
