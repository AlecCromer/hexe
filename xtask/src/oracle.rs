//! `cargo xtask oracle [--raise]`: run every fixtures/<case>/case.toml against the release CLI and score it.
//! case.toml: `cmd = ["oracle","parse-len"]` (args for `target/release/hexe-cli`), `input`, `expected` (files in the case
//! dir), `compare = bytes|json|text-f1|png-psnr|geo`, optional `feature` (ledger id) and `threshold`. `cmd` args may use
//! `{input}` `{expected}` (absolute paths in the case dir) and `{out}` (a fresh file under `<target>/oracle/<case>/`; when
//! present the CLI must write its result there and THAT file is compared, else stdout is). No `{input}` anywhere → the
//! absolute input path is appended (legacy form). `geo` = both parsed with `hexe_geo::read_geo`: point/vertex/prim counts
//! equal, `P` within 1e-5, same (class, name) attribute set; score 1.0 or 0.0. Fails on: a case of a `shipped` ledger row under its threshold; any score under its floor in
//! xtask/baselines/oracle.json. `--raise` ratchets floors up (never down).
//! `stat` / `stat-disp` = STATISTICAL oracle (parity/README.md) for generators whose RNG / noise stream cannot be reproduced; score 1.0 or 0.0:
//! `stat`: point count exact, bbox min/max per axis within 2 % of the expected extent, mean nearest-neighbour distance +-5 %, `P` std per axis +-5 %,
//! (class, name) attribute set exact. `stat-disp` (needs `base = "<file>"`, the cook's input): point count exact, mean and std of |P - base| +-5 %,
//! attribute set exact. Mirror of `oracle/houdini/statoracle.py`; keep both tolerance sets in sync.
//! `stat-uv` (UV layout, parity/README.md): island count exact, per-island uv bbox size and total uv area (coverage) +-5 %, layout extent +-5 %; packing order not compared.
//! `cargo xtask oracle --houdini [compare.py args]`: the REAL-Houdini comparison instead (AGENTS section 2 python exception):
//! builds the debug `hexe-cli`, runs `oracle/houdini/compare.py --hexe-cli <it> [args]` (hexe cook of every
//! `fixtures/sop-*/scene.json` vs the committed `houdini.json`; hython only loads hexe's `.geo` back) then
//! `oracle/houdini/report.py` -> `docs/HOUDINI-ORACLE.md`. Re-record the Houdini side with `hython oracle/houdini/run_all.py`.
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
    /// `stat-disp` only: file in the case dir holding the cook's input geometry
    pub base: Option<String>,
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
/// Statistical oracle tolerances (relative; `STAT_ABS` is the absolute floor for degenerate axes). Keep in sync with oracle/houdini/statoracle.py.
pub const STAT_BBOX_TOL: f64 = 0.02;
pub const STAT_NN_TOL: f64 = 0.05;
pub const STAT_STD_TOL: f64 = 0.05;
pub const STAT_ABS: f64 = 1e-4;
const STAT_MAX_POINTS: usize = 200_000;
fn mean_std(v: &[f64]) -> (f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0);
    }
    let n = v.len() as f64;
    let m = v.iter().sum::<f64>() / n;
    (m, (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / n).sqrt())
}
/// Mean nearest-neighbour distance (exact): x-sorted sweep, each side stops once the x gap alone exceeds the best distance.
pub fn mean_nn(p: &[[f32; 3]]) -> f64 {
    if p.len() < 2 {
        return 0.0;
    }
    let mut s: Vec<[f64; 3]> = p.iter().map(|q| q.map(f64::from)).collect();
    s.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut tot = 0.0;
    for (i, a) in s.iter().enumerate() {
        let mut best = f64::INFINITY;
        let (above, below) = (s.get(i + 1..).unwrap_or(&[]), s.get(..i).unwrap_or(&[]));
        for side in [above.iter().collect::<Vec<_>>(), below.iter().rev().collect::<Vec<_>>()] {
            for b in side {
                let dx = b[0] - a[0];
                if dx * dx >= best {
                    break;
                }
                best = best.min(dx * dx + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2));
            }
        }
        tot += best.sqrt();
    }
    tot / s.len() as f64
}
fn rel_ok(h: f64, x: f64, tol: f64) -> bool {
    (x - h).abs() <= tol * h.abs() + STAT_ABS
}
fn attr_names(d: &hexe_geo::Detail) -> Vec<String> {
    let mut v: Vec<String> = d.attrs.iter().map(|x| format!("{:?}:{}", x.class, x.name)).collect();
    v.sort();
    v
}
/// `None` = statistically the same per the `stat` (base `None`) / `stat-disp` (base = the cook input) mode, `Some(failures)` otherwise.
pub fn stat_mismatch(actual: &[u8], expected: &[u8], base: Option<&[u8]>) -> Result<Option<String>, String> {
    let parse = |who: &str, b: &[u8]| hexe_geo::read_geo(b).map_err(|e| format!("{who}: {e}"));
    let (a, e) = (parse("actual", actual)?, parse("expected", expected)?);
    if a.p().len() > STAT_MAX_POINTS || e.p().len() > STAT_MAX_POINTS {
        return Err(format!("stat: more than {STAT_MAX_POINTS} points"));
    }
    let mut bad: Vec<String> = Vec::new();
    if a.point_count != e.point_count {
        bad.push(format!("point count {} != {}", a.point_count, e.point_count));
    }
    if attr_names(&a) != attr_names(&e) {
        bad.push(format!("attribute sets differ: {:?} vs {:?}", attr_names(&a), attr_names(&e)));
    }
    let (pa, pe) = (a.p(), e.p());
    if pa.len() != pe.len() || pe.is_empty() {
        bad.push("P length".into());
        return Ok(Some(bad.join("; ")));
    }
    if let Some(bb) = base {
        let b = parse("base", bb)?;
        if b.p().len() != pe.len() {
            return Err(format!("base has {} points, expected {}", b.p().len(), pe.len()));
        }
        let mag = |p: &[[f32; 3]]| -> Vec<f64> {
            p.iter()
                .zip(b.p())
                .map(|(q, r)| (0..3).map(|k| f64::from(q[k] - r[k]).powi(2)).sum::<f64>().sqrt())
                .collect()
        };
        let ((mh, sh), (mx, sx)) = (mean_std(&mag(pe)), mean_std(&mag(pa)));
        if !rel_ok(mh, mx, STAT_STD_TOL) {
            bad.push(format!("disp mean {mx:.5} vs {mh:.5}"));
        }
        if !rel_ok(sh, sx, STAT_STD_TOL) {
            bad.push(format!("disp std {sx:.5} vs {sh:.5}"));
        }
        return Ok((!bad.is_empty()).then(|| bad.join("; ")));
    }
    for ax in 0..3 {
        let ch: Vec<f64> = pe.iter().map(|p| f64::from(p[ax])).collect();
        let cx: Vec<f64> = pa.iter().map(|p| f64::from(p[ax])).collect();
        let range = |v: &[f64]| v.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), &x| (l.min(x), h.max(x)));
        let ((lo_h, hi_h), (lo_x, hi_x)) = (range(&ch), range(&cx));
        let tol = STAT_BBOX_TOL * (hi_h - lo_h) + STAT_ABS;
        if (lo_x - lo_h).abs() > tol || (hi_x - hi_h).abs() > tol {
            bad.push(format!("bbox[{ax}] [{lo_x:.4},{hi_x:.4}] vs [{lo_h:.4},{hi_h:.4}]"));
        }
        let (sh, sx) = (mean_std(&ch).1, mean_std(&cx).1);
        if !rel_ok(sh, sx, STAT_STD_TOL) {
            bad.push(format!("std[{ax}] {sx:.4} vs {sh:.4}"));
        }
    }
    let (nh, nx) = (mean_nn(pe), mean_nn(pa));
    if !rel_ok(nh, nx, STAT_NN_TOL) {
        bad.push(format!("mean NN {nx:.5} vs {nh:.5}"));
    }
    Ok((!bad.is_empty()).then(|| bad.join("; ")))
}
/// `stat-uv` tolerance (relative, per island size / coverage). Keep in sync with parity/README.md.
pub const STAT_UV_TOL: f64 = 0.05;
fn uv_islands(d: &hexe_geo::Detail) -> Result<(Vec<[f64; 4]>, f64), String> {
    use hexe_geo::{AttrClass, AttrData};
    let uv: Vec<[f32; 2]> = match d.attr(AttrClass::Vertex, "uv").map(|a| &a.data) {
        Some(AttrData::V3(v)) => v.iter().map(|q| [q[0], q[1]]).collect(),
        Some(AttrData::V2(v)) => v.clone(),
        _ => return Err("stat-uv: no vertex uv".into()),
    };
    let n = d.prims.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let mut seen: std::collections::HashMap<(u32, [u32; 2]), usize> = std::collections::HashMap::new();
    let mut corners: Vec<Vec<[f64; 2]>> = Vec::with_capacity(n);
    for (qi, pr) in d.prims.iter().enumerate() {
        let mut c = Vec::new();
        for v in pr.verts.start..pr.verts.end {
            let (Some(&pt), Some(u)) = (d.vertex_point.get(v as usize), uv.get(v as usize)) else { continue };
            c.push([f64::from(u[0]), f64::from(u[1])]);
            let key = (pt, [u[0].to_bits(), u[1].to_bits()]);
            match seen.get(&key) {
                Some(&o) => {
                    let (a, b) = (find(&mut parent, o), find(&mut parent, qi));
                    parent[a.max(b)] = a.min(b);
                }
                None => {
                    seen.insert(key, qi);
                }
            }
        }
        corners.push(c);
    }
    let mut boxes: std::collections::BTreeMap<usize, [f64; 4]> = std::collections::BTreeMap::new();
    let mut area = 0.0;
    for (qi, c) in corners.iter().enumerate() {
        if c.is_empty() {
            continue;
        }
        let r = find(&mut parent, qi);
        let b = boxes.entry(r).or_insert([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY]);
        for q in c {
            *b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
        }
        let m = c.len();
        area += (0..m).map(|k| c[k][0] * c[(k + 1) % m][1] - c[(k + 1) % m][0] * c[k][1]).sum::<f64>().abs() / 2.0;
    }
    Ok((boxes.into_values().collect(), area))
}
/// `None` = same UV layout statistics: island count exact, every island's bbox width and height (islands in first-prim order) within `STAT_UV_TOL`,
/// layout extent likewise, total uv area (coverage) within `STAT_UV_TOL`. Placement itself (packing order) is NOT compared.
pub fn stat_uv_mismatch(actual: &[u8], expected: &[u8]) -> Result<Option<String>, String> {
    let parse = |who: &str, b: &[u8]| hexe_geo::read_geo(b).map_err(|e| format!("{who}: {e}"));
    let ((ba, ca), (be, ce)) = (uv_islands(&parse("actual", actual)?)?, uv_islands(&parse("expected", expected)?)?);
    let mut bad: Vec<String> = Vec::new();
    if ba.len() != be.len() {
        bad.push(format!("island count {} != {}", ba.len(), be.len()));
        return Ok(Some(bad.join("; ")));
    }
    for (i, (a, e)) in ba.iter().zip(&be).enumerate() {
        let (wa, ha, we, he) = (a[2] - a[0], a[3] - a[1], e[2] - e[0], e[3] - e[1]);
        if !rel_ok(we, wa, STAT_UV_TOL) || !rel_ok(he, ha, STAT_UV_TOL) {
            bad.push(format!("island {i} size {wa:.4}x{ha:.4} vs {we:.4}x{he:.4}"));
        }
    }
    let ext = |b: &[[f64; 4]]| {
        b.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |m, q| {
            [m[0].min(q[0]), m[1].min(q[1]), m[2].max(q[2]), m[3].max(q[3])]
        })
    };
    let (xa, xe) = (ext(&ba), ext(&be));
    if !rel_ok(xe[2] - xe[0], xa[2] - xa[0], STAT_UV_TOL) || !rel_ok(xe[3] - xe[1], xa[3] - xa[1], STAT_UV_TOL) {
        bad.push(format!(
            "layout extent {:.4}x{:.4} vs {:.4}x{:.4}",
            xa[2] - xa[0],
            xa[3] - xa[1],
            xe[2] - xe[0],
            xe[3] - xe[1]
        ));
    }
    if !rel_ok(ce, ca, STAT_UV_TOL) {
        bad.push(format!("coverage {ca:.4} vs {ce:.4}"));
    }
    Ok((!bad.is_empty()).then(|| bad.join("; ")))
}
#[cfg(test)]
pub fn score(compare: &str, actual: &[u8], expected: &[u8]) -> Result<f64, String> {
    score_base(compare, actual, expected, None)
}
pub fn score_base(compare: &str, actual: &[u8], expected: &[u8], base: Option<&[u8]>) -> Result<f64, String> {
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
        "stat" | "stat-disp" => {
            if (compare == "stat-disp") != base.is_some() {
                return Err(format!(
                    "`{compare}` {} a `base` file",
                    if compare == "stat" { "takes no" } else { "needs" }
                ));
            }
            match stat_mismatch(actual, expected, base)? {
                None => Ok(1.0),
                Some(why) => {
                    eprintln!("oracle {compare}: {why}");
                    Ok(0.0)
                }
            }
        }
        "stat-uv" => match stat_uv_mismatch(actual, expected)? {
            None => Ok(1.0),
            Some(why) => {
                eprintln!("oracle stat-uv: {why}");
                Ok(0.0)
            }
        },
        other => Err(format!("unknown compare `{other}` (bytes|json|text-f1|png-psnr|geo|stat|stat-disp|stat-uv)")),
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
    let base =
        c.base.as_ref().map(|f| std::fs::read(dir.join(f)).map_err(|e| format!("read base: {e}"))).transpose()?;
    score_base(&c.compare, &actual, &expected, base.as_deref())
}
/// `cargo xtask oracle --houdini [compare.py args...]`: compare.py then report.py (see module docs).
fn run_houdini(args: &[String]) -> Res {
    let root = root();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let st = Command::new(cargo)
        .args(["build", "-p", "hexe-cli"])
        .current_dir(&root)
        .status()
        .map_err(|e| format!("cargo build: {e}"))?;
    if !st.success() {
        return Err("debug build of hexe-cli failed".into());
    }
    let bin = crate::target_dir().join("debug").join(format!("hexe-cli{}", std::env::consts::EXE_SUFFIX));
    let dir = root.join("oracle").join("houdini");
    let rest: Vec<&String> = args.iter().filter(|a| a.as_str() != "--houdini").collect();
    let st = Command::new("python3")
        .arg(dir.join("compare.py"))
        .arg("--hexe-cli")
        .arg(&bin)
        .args(rest)
        .current_dir(&root)
        .status()
        .map_err(|e| format!("spawn python3 compare.py: {e}"))?;
    if !st.success() {
        return Err(format!("compare.py failed ({st})"));
    }
    let st = Command::new("python3")
        .arg(dir.join("report.py"))
        .current_dir(&root)
        .status()
        .map_err(|e| format!("spawn python3 report.py: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("report.py failed ({st})")) }
}
pub fn run(args: &[String]) -> Res {
    if args.iter().any(|a| a == "--houdini") {
        return run_houdini(args);
    }
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
    fn stat_uv_compares_layout_statistics_not_placement() {
        // two unit-square polygons on 4 shared-nothing points each = two islands; uv = square scaled by `k`, island 2 placed at x offset `off`
        let mk = |k: f32, off: f32| {
            let mut d = hexe_geo::Detail::default();
            d.add_points(8);
            *d.p_mut() = (0..8).map(|i| [i as f32, 0.0, 0.0]).collect();
            d.add_poly(&[0, 1, 2, 3], true);
            d.add_poly(&[4, 5, 6, 7], true);
            let uv: Vec<[f32; 3]> = (0..8)
                .map(|i| {
                    let (x, y) = ([0.0, 1.0, 1.0, 0.0][i % 4], [0.0, 0.0, 1.0, 1.0][i % 4]);
                    [x * k + if i >= 4 { off } else { 0.0 }, y * k, 0.0]
                })
                .collect();
            d.add_attr(hexe_geo::Attribute {
                name: "uv".into(),
                class: hexe_geo::AttrClass::Vertex,
                data: hexe_geo::AttrData::V3(uv),
                type_info: hexe_geo::TypeInfo::None,
            });
            hexe_geo::write_geo_json(&d)
        };
        let base = mk(0.4, 0.5);
        assert_eq!(score("stat-uv", &mk(0.4, 0.51), &base).unwrap(), 1.0, "small placement drift is free");
        assert_eq!(score("stat-uv", &mk(0.405, 0.5), &base).unwrap(), 1.0, "1.25 % size is inside the band");
        assert_eq!(score("stat-uv", &mk(0.5, 0.5), &base).unwrap(), 0.0, "25 % size fails");
    }
    #[test]
    fn stat_compare_distributions_not_streams() {
        let grid = |off: f32, n: usize, extra: Option<&str>| {
            let mut d = hexe_geo::Detail::default();
            let pts: Vec<[f32; 3]> = (0..n * n).map(|i| [(i / n) as f32 + off, 0.0, (i % n) as f32 - off]).collect();
            d.add_points(pts.len() as u32);
            *d.p_mut() = pts;
            if let Some(x) = extra {
                d.add_attr(hexe_geo::Attribute {
                    name: x.into(),
                    class: hexe_geo::AttrClass::Point,
                    data: hexe_geo::AttrData::F32(vec![0.0; n * n]),
                    type_info: hexe_geo::TypeInfo::None,
                });
            }
            hexe_geo::write_geo_json(&d)
        };
        let base = grid(0.0, 20, None);
        // same lattice nudged by 0.01: P differs everywhere (geo would fail) but every statistic is inside tolerance
        let near = grid(0.01, 20, None);
        assert!(geo_mismatch(&near, &base).unwrap().is_some());
        assert_eq!(score("stat", &near, &base).unwrap(), 1.0);
        assert_eq!(score("stat", &base, &base).unwrap(), 1.0);
        assert_eq!(score("stat", &grid(0.0, 19, None), &base).unwrap(), 0.0, "count is exact");
        assert_eq!(score("stat", &grid(0.0, 20, Some("foo")), &base).unwrap(), 0.0, "attribute names are exact");
        let wide = {
            let mut d = hexe_geo::read_geo(&base).unwrap();
            for p in d.p_mut() {
                p[0] *= 1.1;
            }
            hexe_geo::write_geo_json(&d)
        };
        let why = stat_mismatch(&wide, &base, None).unwrap().unwrap();
        assert!(why.contains("bbox[0]") && why.contains("std[0]"), "{why}");
        assert!(score("stat", b"junk", &base).is_err());
        assert!(score("stat-disp", &base, &base).is_err(), "stat-disp needs a base");
        // stat-disp: |P - base| mean/std
        let moved = |k: f32| {
            let mut d = hexe_geo::read_geo(&base).unwrap();
            for (i, p) in d.p_mut().iter_mut().enumerate() {
                p[1] = k * ((i % 7) as f32 - 3.0);
            }
            hexe_geo::write_geo_json(&d)
        };
        assert_eq!(score_base("stat-disp", &moved(1.02), &moved(1.0), Some(&base)).unwrap(), 1.0);
        assert_eq!(score_base("stat-disp", &moved(1.2), &moved(1.0), Some(&base)).unwrap(), 0.0);
    }
    #[test]
    fn mean_nn_of_a_unit_lattice_is_one() {
        let p: Vec<[f32; 3]> = (0..25).map(|i| [(i / 5) as f32, (i % 5) as f32, 0.0]).collect();
        assert!((mean_nn(&p) - 1.0).abs() < 1e-9);
        assert_eq!(mean_nn(&p[..1]), 0.0);
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
