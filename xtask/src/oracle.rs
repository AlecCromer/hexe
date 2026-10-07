//! `cargo xtask oracle [--raise]`: run every fixtures/<case>/case.toml against the release CLI and score it.
//! case.toml: `cmd = ["oracle","parse-len"]` (args for `target/release/hexe-cli`), `input`, `expected` (files in the case
//! dir), `compare = bytes|json|text-f1|png-psnr|geo`, optional `feature` (ledger id) and `threshold`. `cmd` args may use
//! `{input}` `{expected}` (absolute paths in the case dir) and `{out}` (a fresh file under `<target>/oracle/<case>/`; when
//! present the CLI must write its result there and THAT file is compared, else stdout is). No `{input}` anywhere → the
//! absolute input path is appended (legacy form). `geo` = both parsed with `hexe_geo::read_geo`: point/vertex/prim counts
//! equal, `P` within 1e-5, same (class, name) attribute set; score 1.0 or 0.0. Fails on: a case of a `shipped` ledger row under its threshold; any score under its floor in
//! xtask/baselines/oracle.json. `--raise` ratchets floors up (never down).
use crate::{Res, baseline, parity, root};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub cmd: Vec<String>,
    pub input: String,
    pub expected: String,
    pub compare: String,
    pub feature: Option<String>,
    pub threshold: Option<f64>,
}

impl Case {
    fn threshold(&self) -> f64 {
        self.threshold.unwrap_or(match self.compare.as_str() {
            "text-f1" => 0.97,
            "png-psnr" => 40.0,
            _ => 1.0,
        })
    }
}

pub fn word_f1(a: &str, b: &str) -> f64 {
    let (wa, wb): (Vec<&str>, Vec<&str>) = (a.split_whitespace().collect(), b.split_whitespace().collect());
    if wa.is_empty() && wb.is_empty() {
        return 1.0;
    }
    let mut pool = wb.clone();
    let mut hit = 0usize;
    for w in &wa {
        if let Some(i) = pool.iter().position(|p| p == w) {
            pool.swap_remove(i);
            hit += 1;
        }
    }
    if hit == 0 {
        return 0.0;
    }
    let (p, r) = (hit as f64 / wa.len() as f64, hit as f64 / wb.len() as f64);
    2.0 * p * r / (p + r)
}

fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().map_err(|e| e.to_string())?;
    let (w, h) = (reader.info().width, reader.info().height);
    if u64::from(w) * u64::from(h) > 64_000_000 {
        return Err("png too large".into());
    }
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or("png buffer size overflow")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    Ok((w, h, buf))
}

/// PSNR in dB between two PNGs (capped at 100 when identical; 0 when the shapes differ).
pub fn png_psnr(a: &[u8], b: &[u8]) -> Result<f64, String> {
    let ((wa, ha, da), (wb, hb, db)) = (decode_png(a)?, decode_png(b)?);
    if (wa, ha, da.len()) != (wb, hb, db.len()) || da.is_empty() {
        return Ok(0.0);
    }
    let mse = da.iter().zip(&db).map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2)).sum::<f64>() / da.len() as f64;
    Ok(if mse == 0.0 { 100.0 } else { (10.0 * (255.0f64 * 255.0 / mse).log10()).min(100.0) })
}

/// Max |actual - expected| per `P` component.
pub const GEO_P_TOL: f32 = 1e-5;
/// `None` = same geometry per the `geo` compare mode, `Some(first difference)` otherwise; `Err` = a side does not parse.
pub fn geo_mismatch(actual: &[u8], expected: &[u8]) -> Result<Option<String>, String> {
    let parse = |who: &str, b: &[u8]| hexe_geo::read_geo(b).map_err(|e| format!("{who}: {e}"));
    let (a, e) = (parse("actual", actual)?, parse("expected", expected)?);
    if a.point_count != e.point_count {
        return Ok(Some(format!("point count {} != {}", a.point_count, e.point_count)));
    }
    if a.vertex_count() != e.vertex_count() {
        return Ok(Some(format!("vertex count {} != {}", a.vertex_count(), e.vertex_count())));
    }
    if a.prim_count() != e.prim_count() {
        return Ok(Some(format!("prim count {} != {}", a.prim_count(), e.prim_count())));
    }
    let names = |d: &hexe_geo::Detail| {
        let mut v: Vec<String> = d.attrs.iter().map(|x| format!("{:?}:{}", x.class, x.name)).collect();
        v.sort();
        v
    };
    if names(&a) != names(&e) {
        return Ok(Some(format!("attribute sets differ: {:?} vs {:?}", names(&a), names(&e))));
    }
    for (i, (pa, pe)) in a.p().iter().zip(e.p()).enumerate() {
        if pa.iter().zip(pe).any(|(x, y)| x != y && ((x - y).abs() > GEO_P_TOL || (x - y).is_nan())) {
            return Ok(Some(format!("P[{i}] {pa:?} != {pe:?} (tol {GEO_P_TOL})")));
        }
    }
    Ok(None)
}
pub fn score(compare: &str, actual: &[u8], expected: &[u8]) -> Result<f64, String> {
    match compare {
        "bytes" => Ok(f64::from(u8::from(actual == expected))),
        "json" => {
            let parse = |b: &[u8]| serde_json::from_slice::<serde_json::Value>(b).map_err(|e| e.to_string());
            Ok(f64::from(u8::from(parse(actual)? == parse(expected)?)))
        }
        "text-f1" => Ok(word_f1(&String::from_utf8_lossy(actual), &String::from_utf8_lossy(expected))),
        "png-psnr" => png_psnr(actual, expected),
        "geo" => match geo_mismatch(actual, expected)? {
            None => Ok(1.0),
            Some(why) => {
                eprintln!("oracle geo: {why}");
                Ok(0.0)
            }
        },
        other => Err(format!("unknown compare `{other}` (bytes|json|text-f1|png-psnr|geo)")),
    }
}

/// `cmd` with `{input}` `{expected}` `{out}` substituted; no `{input}` anywhere → input path appended (legacy form).
pub fn expand_cmd(cmd: &[String], input: &Path, expected: &Path, out: &Path) -> Vec<String> {
    let uses_input = cmd.iter().any(|a| a.contains("{input}"));
    let sub = |a: &String| {
        a.replace("{input}", &input.to_string_lossy())
            .replace("{expected}", &expected.to_string_lossy())
            .replace("{out}", &out.to_string_lossy())
    };
    let mut v: Vec<String> = cmd.iter().map(sub).collect();
    if !uses_input {
        v.push(input.to_string_lossy().into_owned());
    }
    v
}
fn run_case(bin: &Path, dir: &Path, c: &Case) -> Result<f64, String> {
    let id = dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let out_dir = crate::target_dir().join("oracle").join(&id);
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("mkdir {}: {e}", out_dir.display()))?;
    let out_path = out_dir.join("out");
    let _ = std::fs::remove_file(&out_path);
    let args = expand_cmd(&c.cmd, &dir.join(&c.input), &dir.join(&c.expected), &out_path);
    let out = Command::new(bin).args(&args).output().map_err(|e| format!("spawn {}: {e}", bin.display()))?;
    let expected = std::fs::read(dir.join(&c.expected)).map_err(|e| format!("read expected: {e}"))?;
    if !out.status.success() {
        eprintln!("oracle {id}: {}", String::from_utf8_lossy(&out.stderr).trim());
        return Ok(0.0);
    }
    let actual = if c.cmd.iter().any(|a| a.contains("{out}")) {
        std::fs::read(&out_path).map_err(|e| format!("cmd did not write {{out}} ({}): {e}", out_path.display()))?
    } else {
        out.stdout
    };
    score(&c.compare, &actual, &expected)
}
pub fn run(args: &[String]) -> Res {
    let root = root();
    let ledger = parity::load(&root)?;
    let shipped: BTreeSet<&str> =
        ledger.feature.iter().filter(|f| f.status == "shipped").map(|f| f.id.as_str()).collect();
    let mut dirs: Vec<_> = std::fs::read_dir(root.join("fixtures"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("case.toml").is_file())
        .collect();
    dirs.sort();
    if dirs.is_empty() {
        println!("oracle: 0 cases (fixtures/<case>/case.toml); nothing to compare yet");
        return Ok(());
    }
    crate::build_release("hexe-cli")?;
    let bin = crate::release_bin("hexe-cli");
    let floor_path = root.join("xtask/baselines/oracle.json");
    let floors = baseline::load(&floor_path);
    let (mut errs, mut scores) = (Vec::new(), baseline::Map::new());
    println!("{:<28} {:<9} {:>9} {:>9} {:>8}  status", "case", "compare", "score", "thresh", "floor");
    for dir in &dirs {
        let id = dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let c: Case = match std::fs::read_to_string(dir.join("case.toml"))
            .map_err(|e| e.to_string())
            .and_then(|t| toml::from_str(&t).map_err(|e| e.to_string()))
        {
            Ok(c) => c,
            Err(e) => {
                errs.push(format!("{id}: case.toml: {e}"));
                continue;
            }
        };
        let s = run_case(&bin, dir, &c).unwrap_or_else(|e| {
            errs.push(format!("{id}: {e}"));
            0.0
        });
        let is_shipped = c.feature.as_deref().is_some_and(|f| shipped.contains(f));
        let floor = floors.get(&id).copied().unwrap_or(0.0);
        let status = if s < floor - 1e-9 {
            errs.push(format!("{id}: score {s:.3} fell below floor {floor:.3}"));
            "FLOOR"
        } else if s >= c.threshold() {
            "pass"
        } else if is_shipped {
            errs.push(format!(
                "{id}: shipped feature `{}` scores {s:.3} < {:.3}",
                c.feature.as_deref().unwrap_or(""),
                c.threshold()
            ));
            "FAIL"
        } else {
            "xfail"
        };
        println!("{id:<28} {:<9} {s:>9.3} {:>9.3} {floor:>8.3}  {status}", c.compare, c.threshold());
        scores.insert(id, s);
    }
    if args.iter().any(|a| a == "--raise") {
        baseline::save(&floor_path, &baseline::ratchet(&floors, &scores, true))?;
        println!("oracle: floors raised in xtask/baselines/oracle.json");
    }
    for e in &errs {
        println!("  - {e}");
    }
    if errs.is_empty() { Ok(()) } else { Err(format!("{} oracle failure(s)", errs.len())) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f1_and_exact_compares() {
        assert_eq!(word_f1("a b c", "a b c"), 1.0);
        assert_eq!(word_f1("a", "b"), 0.0);
        assert!((word_f1("a b", "a c") - 0.5).abs() < 1e-9);
        assert_eq!(score("bytes", b"x", b"x").unwrap(), 1.0);
        assert_eq!(score("json", b"{\"a\":1, \"b\":2}", b"{\"b\":2,\"a\":1}").unwrap(), 1.0);
        assert!(score("nope", b"", b"").is_err());
    }

    #[test]
    fn psnr_identical_vs_different() {
        let enc = |v: u8| {
            let mut out = Vec::new();
            let mut e = png::Encoder::new(&mut out, 2, 2);
            e.set_color(png::ColorType::Grayscale);
            e.set_depth(png::BitDepth::Eight);
            let mut w = e.write_header().unwrap();
            w.write_image_data(&[v; 4]).unwrap();
            drop(w);
            out
        };
        assert_eq!(png_psnr(&enc(10), &enc(10)).unwrap(), 100.0);
        let d = png_psnr(&enc(10), &enc(20)).unwrap();
        assert!(d > 20.0 && d < 40.0, "{d}");
        assert!(png_psnr(b"junk", &enc(1)).is_err());
    }
    #[test]
    fn expand_cmd_substitutes_and_appends() {
        let (i, e, o) = (Path::new("/c/in"), Path::new("/c/exp"), Path::new("/t/out"));
        let c = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(expand_cmd(&c(&["oracle", "x"]), i, e, o), c(&["oracle", "x", "/c/in"]));
        assert_eq!(
            expand_cmd(&c(&["cook", "{input}", "--out", "{out}"]), i, e, o),
            c(&["cook", "/c/in", "--out", "/t/out"])
        );
        assert_eq!(expand_cmd(&c(&["--ref={expected}", "{input}"]), i, e, o), c(&["--ref=/c/exp", "/c/in"]));
    }
    #[test]
    fn geo_compare_counts_p_and_attr_names() {
        let mk = |pts: &[[f32; 3]], extra: Option<&str>| {
            let mut d = hexe_geo::Detail::default();
            d.add_points(pts.len() as u32);
            *d.p_mut() = pts.to_vec();
            if let Some(n) = extra {
                d.add_attr(hexe_geo::Attribute {
                    name: n.into(),
                    class: hexe_geo::AttrClass::Point,
                    data: hexe_geo::AttrData::F32(vec![0.0; pts.len()]),
                    type_info: hexe_geo::TypeInfo::None,
                });
            }
            hexe_geo::write_geo_json(&d)
        };
        let base = mk(&[[0.0, 0.0, 0.0], [1.0, 2.0, 3.0]], None);
        assert_eq!(score("geo", &base, &base).unwrap(), 1.0);
        let near = mk(&[[0.0, 0.0, 0.0], [1.000_001, 2.0, 3.0]], None);
        assert_eq!(score("geo", &near, &base).unwrap(), 1.0);
        let far = mk(&[[0.0, 0.0, 0.0], [1.001, 2.0, 3.0]], None);
        assert!(geo_mismatch(&far, &base).unwrap().unwrap().contains("P[1]"));
        let fewer = mk(&[[0.0, 0.0, 0.0]], None);
        assert!(geo_mismatch(&fewer, &base).unwrap().unwrap().contains("point count"));
        let attr = mk(&[[0.0, 0.0, 0.0], [1.0, 2.0, 3.0]], Some("foo"));
        assert!(geo_mismatch(&attr, &base).unwrap().unwrap().contains("attribute sets"));
        assert!(score("geo", b"junk", &base).is_err());
    }
}
