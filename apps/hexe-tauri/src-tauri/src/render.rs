//! Headless render (`hexe scene.json --render out.png [--size WxH] [--wire]`): the window cooks the scene, draws the three.js
//! canvas at exactly WxH, and sends `canvas.toDataURL("image/png")` back; this module decodes + validates it and writes the
//! PNG. Pure helpers only (the window plumbing lives in `lib.rs`), so every step is unit-tested without a webview.
use serde::Serialize;

/// Default render size when `--size` is absent.
pub const DEFAULT_SIZE: (u32, u32) = (1280, 800);
/// Per-side bounds of `--size` (egui parity: 8192 px target cap).
pub const MIN_SIDE: u32 = 16;
pub const MAX_SIDE: u32 = 8192;

/// What the window must render (answer of the `render_request` command).
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct RenderReq {
    pub out: String,
    pub width: u32,
    pub height: u32,
    pub wire: bool,
}

/// `WxH` (pixels) -> size; both sides within `MIN_SIDE..=MAX_SIDE`.
pub fn parse_size(v: &str) -> Result<(u32, u32), String> {
    let bad = || format!("--size {v}: want WxH pixels, each {MIN_SIDE}..={MAX_SIDE} (e.g. 1280x800)");
    let (w, h) = v.split_once(['x', 'X']).ok_or_else(bad)?;
    let (w, h) = (w.trim().parse::<u32>().map_err(|_| bad())?, h.trim().parse::<u32>().map_err(|_| bad())?);
    if (MIN_SIDE..=MAX_SIDE).contains(&w) && (MIN_SIDE..=MAX_SIDE).contains(&h) { Ok((w, h)) } else { Err(bad()) }
}

fn b64(c: u8) -> Option<u32> {
    Some(u32::from(match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    }))
}

/// Standard base64 (padding optional, ASCII whitespace ignored).
pub fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        acc = (acc << 6) | b64(c).ok_or_else(|| format!("base64: bad byte 0x{c:02x}"))?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// `data:image/png;base64,....` -> the PNG bytes.
pub fn decode_data_url(url: &str) -> Result<Vec<u8>, String> {
    let body = url.strip_prefix("data:image/png;base64,").ok_or("render: canvas did not produce a PNG data URL")?;
    base64_decode(body)
}

const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Pixel size from the IHDR chunk; `None` when `bytes` is not a PNG.
pub fn png_dims(bytes: &[u8]) -> Option<(u32, u32)> {
    let head = bytes.get(..24)?;
    if head[..8] != PNG_SIG || &head[12..16] != b"IHDR" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
    Some((be(16), be(20)))
}

/// Validate (PNG, exact `want` dims) and write `png` to `path`, creating parent dirs.
pub fn write_png(path: &str, png: &[u8], want: (u32, u32)) -> Result<(), String> {
    let got = png_dims(png).ok_or("render: the canvas data is not a PNG")?;
    if got != want {
        return Err(format!("render: canvas is {}x{}, wanted {}x{}", got.0, got.1, want.0, want.1));
    }
    if let Some(dir) = std::path::Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("render: {}: {e}", dir.display()))?;
    }
    std::fs::write(path, png).map_err(|e| format!("render: {path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_parses_and_rejects() {
        assert_eq!(parse_size("800x600"), Ok((800, 600)));
        assert_eq!(parse_size(" 1280 X 800 "), Ok((1280, 800)));
        for bad in ["", "800", "x", "8x8", "9000x100", "0x0", "ax b", "-1x100", "100x100x1"] {
            assert!(parse_size(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn base64_rfc4648_vectors() {
        for (plain, enc) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_decode(enc).expect("decodes"), plain.as_bytes(), "{enc}");
        }
        assert_eq!(base64_decode("Zm9v\nYmFy").expect("whitespace ok"), b"foobar");
        assert!(base64_decode("Zm9v!").is_err());
    }

    /// 1x1 PNG (the canonical smallest one).
    const TINY: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

    #[test]
    fn data_url_to_png_with_dims() {
        let png = decode_data_url(TINY).expect("png");
        assert_eq!(png_dims(&png), Some((1, 1)));
        assert!(decode_data_url("data:image/jpeg;base64,AAAA").is_err());
        assert_eq!(png_dims(b"not a png at all, but long enough to index"), None);
    }

    #[test]
    fn write_png_checks_dims_and_creates_dirs() {
        let png = decode_data_url(TINY).expect("png");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/t4-render-test");
        let file = dir.join("nested/out.png");
        let path = file.to_string_lossy().to_string();
        assert!(write_png(&path, &png, (2, 2)).expect_err("wrong dims").contains("1x1, wanted 2x2"));
        write_png(&path, &png, (1, 1)).expect("written");
        assert_eq!(std::fs::read(&file).expect("read back"), png);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
