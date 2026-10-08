//! `cargo xtask render`: headless-render proof of the desktop app. Builds the UI (`bun run build`), then runs
//! `hexe-tauri examples/polyextrude.json --render target/ci/polyextrude.png --size WxH` and asserts the PNG exists, is non-empty,
//! has the requested dimensions and is not a flat single-colour image. Needs a macOS GUI session (WKWebView + WebGL);
//! set `HEXE_CI_NO_GUI=1` to skip with a warning on a machine without one.
use crate::{Res, root};
use std::process::Command;

/// Render size asserted by the step.
const SIZE: (u32, u32) = (640, 400);
const SCENE: &str = "examples/polyextrude.json";
const OUT: &str = "target/ci/polyextrude.png";

/// Width and height from a PNG's IHDR chunk; `None` when `b` is not a PNG.
pub fn png_dims(b: &[u8]) -> Option<(u32, u32)> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let h = b.get(..24)?;
    if h[..8] != SIG || &h[12..16] != b"IHDR" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
    Some((be(16), be(20)))
}

fn run_in(dir: &std::path::Path, prog: &str, args: &[&str]) -> Res {
    println!("$ {prog} {}", args.join(" "));
    let st = Command::new(prog)
        .args(args)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("spawn {prog}: {e} (is it installed?)"))?;
    if st.success() { Ok(()) } else { Err(format!("`{prog} {}` exited with {st}", args.join(" "))) }
}

pub fn run(_: &[String]) -> Res {
    if std::env::var_os("HEXE_CI_NO_GUI").is_some_and(|v| v == "1") {
        println!("render: WARN HEXE_CI_NO_GUI=1, headless render skipped");
        return Ok(());
    }
    let app = root().join("apps/hexe-tauri");
    if !app.join("node_modules").exists() {
        run_in(&app, "bun", &["install", "--frozen-lockfile"])?;
    }
    run_in(&app, "bun", &["run", "build"])?;
    let out = root().join(OUT);
    let _ = std::fs::remove_file(&out);
    let size = format!("{}x{}", SIZE.0, SIZE.1);
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    run_in(
        &root(),
        &cargo,
        &[
            "run",
            "-q",
            "-p",
            "hexe-tauri",
            "--features",
            "tauri/custom-protocol",
            "--",
            SCENE,
            "--render",
            OUT,
            "--size",
            &size,
        ],
    )?;
    let bytes = std::fs::read(&out).map_err(|e| format!("render: {}: {e}", out.display()))?;
    if bytes.is_empty() {
        return Err(format!("render: {} is empty", out.display()));
    }
    let dims = png_dims(&bytes).ok_or_else(|| format!("render: {} is not a PNG", out.display()))?;
    if dims != SIZE {
        return Err(format!("render: {} is {}x{}, wanted {}x{}", out.display(), dims.0, dims.1, SIZE.0, SIZE.1));
    }
    // a failed GL draw yields one flat colour: PNG of a 640x400 flat image compresses to well under 2 KB, a real scene does not
    if bytes.len() < 4096 {
        return Err(format!("render: {} is only {} bytes: the canvas looks blank", out.display(), bytes.len()));
    }
    println!("render: {} = {}x{}, {} bytes", OUT, dims.0, dims.1, bytes.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dims_come_from_ihdr() {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&640u32.to_be_bytes());
        b.extend_from_slice(&400u32.to_be_bytes());
        assert_eq!(png_dims(&b), Some((640, 400)));
        assert_eq!(png_dims(&b[..20]), None);
        assert_eq!(png_dims(b"GIF89a......................"), None);
    }
}
