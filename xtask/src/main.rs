//! Project automation for hexe, run via `cargo xtask <command>`.
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod assets;
mod baseline;
mod ci;
mod layers;
mod leakscan;
mod oracle;
mod parity;
mod perf;
mod size;
mod version;

pub type Res = Result<(), String>;
type Command = fn(&[String]) -> Res;

/// Every subcommand: name, one-line summary, entry point.
const COMMANDS: &[(&str, &str, Command)] = &[
    (
        "ci",
        "fmt, clippy -D warnings, test, [leakscan if MODE=clean], layers, assets, deny, parity, oracle (stops at first failure)",
        ci::run,
    ),
    ("layers", "Enforce the crate dependency layering (xtask/src/layers.rs TABLE)", layers::run),
    ("assets", "Asset policy vs ATTRIBUTION.toml; --write regenerates ATTRIBUTION.md", assets::run),
    ("deny", "Dependency licences/bans/sources (deny.toml; needs cargo-deny)", ci::deny),
    ("parity", "Validate parity/features.toml; per-tier progress", parity::run),
    (
        "oracle",
        "Run fixtures/<case>/case.toml against the release CLI; floors in xtask/baselines/oracle.json (--raise to ratchet up); --houdini = real-Houdini compare + report",
        oracle::run,
    ),
    ("perf", "Release CLI vs perf/budgets.toml + xtask/baselines/perf.json via /usr/bin/time (--tighten)", perf::run),
    ("size", "Release binary size vs [binary] max_mb and original_mb (ratio)", size::run),
    ("leakscan", "Scan spec/ (or a dir) for decompiler residue; ci runs it only when MODE=clean", leakscan::run),
    ("version", "Print the workspace version, or `version set X.Y.Z[-pre]`", version::run),
];

/// Repository root (parent of `xtask/`).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Cargo's target dir: `CARGO_TARGET_DIR` (each lane has its own) else `target/`.
pub fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root().join("target"), |d| root().join(d))
}

/// `cargo build --release -p <pkg>`.
pub fn build_release(pkg: &str) -> Res {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let st = std::process::Command::new(cargo)
        .args(["build", "--release", "-p", pkg])
        .current_dir(root())
        .status()
        .map_err(|e| e.to_string())?;
    if st.success() { Ok(()) } else { Err(format!("release build of {pkg} failed")) }
}

/// Path of the release binary for package `pkg`.
pub fn release_bin(pkg: &str) -> PathBuf {
    target_dir().join("release").join(format!("{pkg}{}", std::env::consts::EXE_SUFFIX))
}

/// `MODE:` from the first lines of AGENTS.md: `pirate` (default) or `clean`.
pub fn mode() -> String {
    let text = std::fs::read_to_string(root().join("AGENTS.md")).unwrap_or_default();
    let first = text.lines().take(5).find_map(|l| l.trim().strip_prefix("MODE:")).unwrap_or("pirate");
    if first.trim().starts_with("clean") { "clean".into() } else { "pirate".into() }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(name) = args.first() else {
        print_help();
        return ExitCode::FAILURE;
    };
    if matches!(name.as_str(), "help" | "-h" | "--help") {
        print_help();
        return ExitCode::SUCCESS;
    }
    let Some((_, _, command)) = COMMANDS.iter().find(|(n, _, _)| n == name) else {
        eprintln!("xtask: unknown command `{name}`\n");
        print_help();
        return ExitCode::FAILURE;
    };
    match command(&args[1..]) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask {name}: error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    eprintln!("Usage: cargo xtask <command> [args]\n\nCommands:");
    for (name, about, _) in COMMANDS {
        eprintln!("  {name:<10} {about}");
    }
}
