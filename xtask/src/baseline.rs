//! Ratcheted baselines: flat `{ "key": number }` JSON files under xtask/baselines/. Floors only rise, ceilings only fall.
use std::collections::BTreeMap;
use std::path::Path;

pub type Map = BTreeMap<String, f64>;

pub fn load(path: &Path) -> Map {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(path: &Path, m: &Map) -> Result<(), String> {
    let text = serde_json::to_string_pretty(m).map_err(|e| e.to_string())?;
    std::fs::write(path, text + "\n").map_err(|e| format!("write {}: {e}", path.display()))
}

/// Merge `new` into `old`: keep the better value per key (`higher_is_better` picks max, else min).
pub fn ratchet(old: &Map, new: &Map, higher_is_better: bool) -> Map {
    let mut out = old.clone();
    for (k, v) in new {
        let e = out.entry(k.clone()).or_insert(*v);
        *e = if higher_is_better { e.max(*v) } else { e.min(*v) };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratchet_never_loosens() {
        let old: Map = [("a".into(), 0.9), ("b".into(), 10.0)].into();
        let new: Map = [("a".into(), 0.5), ("b".into(), 4.0), ("c".into(), 1.0)].into();
        let up = ratchet(&old, &new, true);
        assert_eq!((up["a"], up["b"], up["c"]), (0.9, 10.0, 1.0));
        let down = ratchet(&old, &new, false);
        assert_eq!((down["a"], down["b"]), (0.5, 4.0));
    }
}
