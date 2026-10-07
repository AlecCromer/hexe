//! `cargo xtask version [set X.Y.Z[-pre]]`: `[workspace.package] version` in the root Cargo.toml is the single source.
use crate::Res;

/// The `version = "…"` value inside `[workspace.package]`.
pub fn read(manifest: &str) -> Result<String, String> {
    let (_, line) = find_line(manifest)?;
    let v = line.split_once('=').map(|(_, v)| v.trim().trim_matches('"').to_string()).unwrap_or_default();
    if v.is_empty() { Err("empty [workspace.package] version".into()) } else { Ok(v) }
}

/// `manifest` with the workspace version replaced; everything else byte-for-byte unchanged.
pub fn replace(manifest: &str, new: &str) -> Result<String, String> {
    validate(new)?;
    let (idx, _) = find_line(manifest)?;
    let mut out = String::with_capacity(manifest.len() + 8);
    for (i, line) in manifest.split_inclusive('\n').enumerate() {
        if i == idx {
            let eol = if line.ends_with("\r\n") {
                "\r\n"
            } else if line.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            out.push_str(&format!("version = \"{new}\"{eol}"));
        } else {
            out.push_str(line);
        }
    }
    Ok(out)
}

/// Semver without build metadata: `MAJOR.MINOR.PATCH` plus an optional `-pre.release` tag.
pub fn validate(v: &str) -> Result<(), String> {
    let bad = || Err(format!("`{v}` is not a version like 1.2.3 or 1.2.3-rc.1"));
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let nums: Vec<&str> = core.split('.').collect();
    let num_ok =
        |n: &&str| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && !(n.len() > 1 && n.starts_with('0'));
    if nums.len() != 3 || !nums.iter().all(num_ok) {
        return bad();
    }
    if let Some(p) = pre
        && (p.is_empty()
            || p.split('.').any(|id| id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')))
    {
        return bad();
    }
    Ok(())
}

fn find_line(manifest: &str) -> Result<(usize, &str), String> {
    let mut in_section = false;
    for (i, line) in manifest.lines().enumerate() {
        let t = line.trim();
        if t.starts_with('[') {
            in_section = t == "[workspace.package]";
            continue;
        }
        if in_section && t.strip_prefix("version").is_some_and(|r| r.trim_start().starts_with('=')) {
            return Ok((i, line));
        }
    }
    Err("no `version = \"…\"` in [workspace.package] of the root Cargo.toml".into())
}

pub fn run(args: &[String]) -> Res {
    let path = crate::root().join("Cargo.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => {
            println!("{}", read(&text)?);
            Ok(())
        }
        ["set", new] => {
            let old = read(&text)?;
            std::fs::write(&path, replace(&text, new)?).map_err(|e| format!("write {}: {e}", path.display()))?;
            println!(
                "version {old} -> {new} (internal `version = \"{old}\"` pins in [workspace.dependencies] need a manual bump)"
            );
            Ok(())
        }
        _ => Err("usage: cargo xtask version [set X.Y.Z[-pre]]".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const M: &str = "[workspace]\nversion = \"9.9.9\"\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n";

    #[test]
    fn reads_only_the_workspace_package_version() {
        assert_eq!(read(M).unwrap(), "0.1.0");
    }

    #[test]
    fn replace_touches_one_line() {
        let out = replace(M, "0.2.0-rc.1").unwrap();
        assert_eq!(out, M.replace("\"0.1.0\"", "\"0.2.0-rc.1\""));
    }

    #[test]
    fn validate_rejects_junk() {
        for bad in ["1", "1.2", "01.2.3", "1.2.3-", "1.2.3-a..b", "v1.2.3"] {
            assert!(validate(bad).is_err(), "{bad}");
        }
        assert!(validate("1.2.3-rc.1").is_ok());
    }
}
