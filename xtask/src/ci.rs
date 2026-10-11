//! `cargo xtask ci`: the whole gate, stops at the first failure.
//! [leakscan if MODE=clean] → fmt → clippy -D warnings → test → layers → assets → deny (warn if absent) → parity → sops (docs/SOPS.md drift) → oracle → perf → size → render (headless app render proof).
use crate::{Res, root};
use std::process::Command;
use std::time::Instant;

fn cargo(args: &[&str]) -> Res {
    println!("$ cargo {}", args.join(" "));
    let st = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(args)
        .current_dir(root())
        .status()
        .map_err(|e| format!("spawn cargo: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("`cargo {}` exited with {st}", args.join(" "))) }
}

fn have_deny() -> bool {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["deny", "--version"])
        .current_dir(root())
        .output()
        .is_ok_and(|o| o.status.success())
}

pub fn deny(_: &[String]) -> Res {
    if !have_deny() {
        println!("deny: WARN cargo-deny not installed; skipped (cargo install cargo-deny --locked)");
        return Ok(());
    }
    cargo(&["deny", "check"])
}

pub fn run(_: &[String]) -> Res {
    let clean = crate::mode() == "clean";
    println!("ci: MODE={}", if clean { "clean" } else { "pirate" });
    type Step = (&'static str, fn() -> Res);
    let mut steps: Vec<Step> = Vec::new();
    if clean {
        steps.push(("leakscan", || crate::leakscan::run(&[])));
    }
    steps.extend([
        ("fmt", (|| cargo(&["fmt", "--all", "--", "--check"])) as fn() -> Res),
        ("clippy", || cargo(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])),
        ("test", || cargo(&["test", "--workspace"])),
        ("layers", || crate::layers::run(&[])),
        ("assets", || crate::assets::run(&[])),
        ("deny", || deny(&[])),
        ("parity", || crate::parity::run(&[])),
        ("sops", || crate::sops::run(&["--check".to_string()])),
        ("oracle", || crate::oracle::run(&[])),
        ("perf", || crate::perf::run(&[])),
        ("size", || crate::size::run(&[])),
        ("render", || crate::render::run(&[])),
    ]);
    let t0 = Instant::now();
    for (name, step) in &steps {
        let t = Instant::now();
        println!("\n== ci: {name} ==");
        step().map_err(|e| format!("step `{name}` failed: {e}"))?;
        println!("== ci: {name} ok ({:.1}s) ==", t.elapsed().as_secs_f64());
    }
    println!("\nci: GREEN ({} steps, {:.1}s)", steps.len(), t0.elapsed().as_secs_f64());
    Ok(())
}
