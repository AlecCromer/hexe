//! `cargo xtask size`: release binary size vs perf/budgets.toml `[binary] max_mb` and the original (`original_mb`).
use crate::{Res, perf, release_bin};

/// `ours / original` when the original size is known.
pub fn ratio(ours_mb: f64, original_mb: f64) -> Option<f64> {
    (original_mb > 0.0).then(|| ours_mb / original_mb)
}

pub fn run(_: &[String]) -> Res {
    let b = perf::load()?.binary;
    crate::build_release(&b.bin)?;
    let path = release_bin(&b.bin);
    let bytes = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?.len();
    let mb = bytes as f64 / 1_048_576.0;
    let r = ratio(mb, b.original_mb).map_or_else(|| "n/a (original_mb unset)".to_string(), |r| format!("{r:.2}x"));
    println!(
        "size: {} = {bytes} bytes ({mb:.2} MB) | max {:.1} MB | original {:.1} MB | ratio {r}",
        b.bin, b.max_mb, b.original_mb
    );
    if mb > b.max_mb { Err(format!("{mb:.2} MB exceeds max_mb {:.1}", b.max_mb)) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_needs_a_known_original() {
        assert_eq!(ratio(5.0, 0.0), None);
        assert_eq!(ratio(5.0, 20.0), Some(0.25));
    }
}
