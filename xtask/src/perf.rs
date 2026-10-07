//! `cargo xtask perf [--tighten]`: run the release CLI on every budgeted fixture under `/usr/bin/time` and enforce
//! perf/budgets.toml (`[[fixture]] case, startup_ms, run_ms, peak_rss_mb`) plus no regression vs
//! xtask/baselines/perf.json (ceilings; `--tighten` only ever lowers them). Needs a release build of `[binary] bin`.
use crate::{Res, baseline, oracle, release_bin, root};
use serde::Deserialize;
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    pub binary: BinaryBudget,
    #[serde(default)]
    pub fixture: Vec<FixtureBudget>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BinaryBudget {
    /// Cargo package whose release binary is measured (`hexe-cli` until the app is the product).
    pub bin: String,
    pub max_mb: f64,
    /// Size of the original program in MB; 0 = unknown.
    #[serde(default)]
    pub original_mb: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureBudget {
    pub case: String,
    pub startup_ms: f64,
    pub run_ms: f64,
    pub peak_rss_mb: f64,
}

pub fn load() -> Result<Budgets, String> {
    let p = root().join("perf/budgets.toml");
    toml::from_str(&std::fs::read_to_string(&p).map_err(|e| format!("read {}: {e}", p.display()))?)
        .map_err(|e| format!("{}: {e}", p.display()))
}

/// Peak RSS in MB from `/usr/bin/time -l` (macOS, bytes) or `-v` (GNU, kbytes) stderr.
pub fn parse_rss_mb(stderr: &str) -> Option<f64> {
    for line in stderr.lines() {
        let t = line.trim();
        if t.ends_with("maximum resident set size") {
            return t.split_whitespace().next()?.parse::<f64>().ok().map(|b| b / 1_048_576.0);
        }
        if let Some(v) = t.strip_prefix("Maximum resident set size (kbytes):") {
            return v.trim().parse::<f64>().ok().map(|k| k / 1024.0);
        }
    }
    None
}

/// Wall ms and peak RSS MB of one run of `bin args` under /usr/bin/time.
fn timed(bin: &std::path::Path, args: &[String]) -> Result<(f64, f64), String> {
    let flag = if cfg!(target_os = "macos") { "-l" } else { "-v" };
    let t0 = Instant::now();
    let out = Command::new("/usr/bin/time")
        .arg(flag)
        .arg(bin)
        .args(args)
        .output()
        .map_err(|e| format!("/usr/bin/time: {e}"))?;
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    let rss = parse_rss_mb(&String::from_utf8_lossy(&out.stderr)).ok_or("no peak RSS in /usr/bin/time output")?;
    Ok((ms, rss))
}

fn best_of(n: usize, bin: &std::path::Path, args: &[String]) -> Result<(f64, f64), String> {
    let mut best = (f64::MAX, 0.0f64);
    for _ in 0..n {
        let (ms, rss) = timed(bin, args)?;
        best = (best.0.min(ms), best.1.max(rss));
    }
    Ok(best)
}

/// Regression test against a ceiling with noise slack (ms: +25% +5; MB: +10% +1).
pub fn regressed(key: &str, value: f64, ceiling: f64) -> bool {
    let limit = if key.ends_with("_ms") { ceiling * 1.25 + 5.0 } else { ceiling * 1.10 + 1.0 };
    value > limit
}

pub fn run(args: &[String]) -> Res {
    let root = root();
    let b = load()?;
    if b.fixture.is_empty() {
        println!("perf: 0 fixture budgets in perf/budgets.toml; nothing to measure yet");
        return Ok(());
    }
    crate::build_release(&b.binary.bin)?;
    let bin = release_bin(&b.binary.bin);
    let path = root.join("xtask/baselines/perf.json");
    let ceilings = baseline::load(&path);
    let (mut errs, mut seen) = (Vec::new(), baseline::Map::new());
    println!("{:<24} {:>12} {:>10} {:>12}", "case", "startup_ms", "run_ms", "peak_rss_mb");
    for f in &b.fixture {
        let dir = root.join("fixtures").join(&f.case);
        let case: oracle::Case =
            toml::from_str(&std::fs::read_to_string(dir.join("case.toml")).map_err(|e| format!("{}: {e}", f.case))?)
                .map_err(|e| format!("{}: {e}", f.case))?;
        let (startup, rss0) = best_of(3, &bin, &["--version".to_string()])?;
        let mut argv = case.cmd.clone();
        argv.push(dir.join(&case.input).to_string_lossy().into_owned());
        let (run_ms, rss) = best_of(3, &bin, &argv)?;
        let rss = rss.max(rss0);
        println!("{:<24} {startup:>12.1} {run_ms:>10.1} {rss:>12.1}", f.case);
        for (k, v, budget) in
            [("startup_ms", startup, f.startup_ms), ("run_ms", run_ms, f.run_ms), ("peak_rss_mb", rss, f.peak_rss_mb)]
        {
            let key = format!("{}.{k}", f.case);
            if v > budget {
                errs.push(format!("{key}: {v:.1} over budget {budget}"));
            }
            if let Some(c) = ceilings.get(&key).filter(|c| regressed(k, v, **c)) {
                errs.push(format!("{key}: {v:.1} regressed vs baseline {c:.1}"));
            }
            seen.insert(key, v);
        }
    }
    if args.iter().any(|a| a == "--tighten") {
        baseline::save(&path, &baseline::ratchet(&ceilings, &seen, false))?;
        println!("perf: baselines tightened");
    }
    for e in &errs {
        println!("  - {e}");
    }
    if errs.is_empty() {
        println!("perf: OK");
        Ok(())
    } else {
        Err(format!("{} perf failure(s)", errs.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_macos_and_gnu_time_output() {
        assert_eq!(parse_rss_mb("  0.01 real\n  2097152  maximum resident set size\n"), Some(2.0));
        assert_eq!(parse_rss_mb("\tMaximum resident set size (kbytes): 4096\n"), Some(4.0));
        assert_eq!(parse_rss_mb("nothing"), None);
    }

    #[test]
    fn regression_slack() {
        assert!(!regressed("a.run_ms", 12.0, 10.0));
        assert!(regressed("a.run_ms", 40.0, 10.0));
        assert!(regressed("a.peak_rss_mb", 20.0, 10.0));
    }
}
