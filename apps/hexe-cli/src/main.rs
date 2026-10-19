//! hexe-cli: `--version`, `cook <scene.json> --out <file.geo> [--time T | --frame N]`, `nodes`, `info <geo>`, `oracle <case> [input]`.
//! Every command runs behind `catch_unwind` (AGENTS.md §4); every failure is one stderr line + exit 1.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
mod endo;
mod open;
use hexe_graph::{ParamSpec, ParamValue};
use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode;

/// Largest scene file `cook` reads (hostile input is the default).
const MAX_SCENE_BYTES: u64 = 64 << 20;
const USAGE: &str = "usage: hexe-cli --version | cook <scene.json> --out <file.geo> [--time T | --frame N] | nodes | convert <in> <out> | info <geo-file> | open <file.hip|.ens|.nmc|.nmb|.nma> [format flags] (open --help) | endo-survey <Resources-dir> [--out md] | oracle <case> [input]";

/// `open --help` text: per-format flags.
const OPEN_HELP: &str = "usage: hexe-cli open <file.hip|.ens|.nmc|.nmb|.nma> [format flags]
  Houdini (.hip .hipnc .hiplc .hip.gz): [--geo N] [--scene-out F] [--cook-out F.geo] [--all-out DIR]
  Endorphin (.ens .nmc .nmb .nma):      [--out scene.json] [--geo out.geo]";

/// `open`: dispatch on the file extension (case-insensitive) to the Houdini or Endorphin reader.
fn open_dispatch(args: &[String]) -> Result<String, String> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return Ok(OPEN_HELP.to_string());
    }
    let is_hip = |a: &str| {
        let l = a.to_ascii_lowercase();
        [".hip", ".hipnc", ".hiplc", ".hip.gz"].iter().any(|e| l.ends_with(e))
    };
    // The file is the argument with a known extension (flag values like `--geo 3` never match).
    if args.iter().any(|a| is_hip(a)) {
        return open::open_cmd(args);
    }
    if args.iter().any(|a| endo::is_endo(a)) {
        return endo::open_cmd(args);
    }
    let file = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or("usage: hexe-cli open <file.hip|.ens|.nmc|.nmb|.nma> [format flags] (open --help)")?;
    Err(format!(
        "open: unknown extension on `{file}` (houdini: .hip .hipnc .hiplc .hip.gz [--geo --scene-out --cook-out --all-out]; endorphin: .ens .nmc .nmb .nma [--out --geo])"
    ))
}

/// Largest geometry file `info` reads (hostile input is the default).
const MAX_GEO_BYTES: u64 = 1 << 30;

fn class_name(c: hexe_geo::AttrClass) -> &'static str {
    match c {
        hexe_geo::AttrClass::Point => "point",
        hexe_geo::AttrClass::Vertex => "vertex",
        hexe_geo::AttrClass::Prim => "prim",
        hexe_geo::AttrClass::Detail => "detail",
    }
}

/// Geometry bytes (any container `read_geo` sniffs) -> counts, attribute names, and every attribute the reader could
/// not represent (`skipped class:name:type`; never silent).
fn info_text(bytes: &[u8]) -> Result<String, String> {
    Ok(info_detail(&hexe_geo::read_geo(bytes).map_err(|e| e.to_string())?))
}

fn info_detail(d: &hexe_geo::Detail) -> String {
    let mut out = format!("points {}\nvertices {}\nprims {}\n", d.point_count, d.vertex_count(), d.prim_count());
    for a in &d.attrs {
        let _ = writeln!(out, "attr {}:{}", class_name(a.class), a.name);
    }
    for (c, n, t) in &d.skipped_attribs {
        let _ = writeln!(out, "skipped {}:{n}:{t}", class_name(*c));
    }
    out
}

fn info(args: &[String]) -> Result<String, String> {
    let [path] = args else { return Err("usage: hexe-cli info <geo-file>".into()) };
    let meta = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
    if meta.len() > MAX_GEO_BYTES {
        return Err(format!("{path}: larger than {MAX_GEO_BYTES} bytes"));
    }
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".obj") || lower.ends_with(".ply") {
        return Ok(info_detail(&hexe_geo::read_any(path).map_err(|e| format!("{path}: {e}"))?));
    }
    info_text(&std::fs::read(path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| format!("{path}: {e}"))
}

/// Oracle entry point: `xtask oracle` runs `hexe-cli <case.toml cmd…>` (see fixtures/README.md) and compares the
/// result with `expected`. Add one arm per fixture family as ledger rows land.
fn oracle(case: &str, input: Option<&str>) -> Result<String, String> {
    match case {
        "parse-len" => {
            let text = std::fs::read_to_string(input.ok_or("oracle parse-len: needs an input path")?)
                .map_err(|e| e.to_string())?;
            hexe_core::parse_len(&text).map(|n| format!("{n}\n")).map_err(|e| e.to_string())
        }
        // Houdini-free twin of `oracle/houdini/_common.py summarize()` run on a hexe `.geo` (no hython load-back).
        "summary" => oracle_summary(input.ok_or("oracle summary: needs a .geo path")?),
        other => Err(format!("oracle: case `{other}` has no implementation yet")),
    }
}

/// Same JSON keys/rounding (6 decimals) as `_common.summarize()`; prim type names are hou.primType names.
fn oracle_summary(path: &str) -> Result<String, String> {
    use hexe_geo::{AttrClass, PrimKind};
    let meta = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
    if meta.len() > MAX_GEO_BYTES {
        return Err(format!("{path}: larger than {MAX_GEO_BYTES} bytes"));
    }
    let d = hexe_geo::read_geo(&std::fs::read(path).map_err(|e| format!("{path}: {e}"))?)
        .map_err(|e| format!("{path}: {e}"))?;
    const CAP: usize = 5000;
    let r6 = |x: f32| (f64::from(x) * 1e6).round() / 1e6;
    let p_all: Vec<[f64; 3]> = d.p().iter().take(CAP).map(|p| [r6(p[0]), r6(p[1]), r6(p[2])]).collect();
    let names = |c: AttrClass| {
        let mut v: Vec<&str> = d.attrs.iter().filter(|a| a.class == c).map(|a| a.name.as_str()).collect();
        v.sort_unstable();
        v
    };
    let groups = |c: AttrClass| {
        let mut v: Vec<&str> = d.groups.iter().filter(|g| g.class == c).map(|g| g.name.as_str()).collect();
        v.sort_unstable();
        v
    };
    let mut prim_types = std::collections::BTreeSet::new();
    let (mut prim_pts, mut closed) = (Vec::new(), Vec::new());
    for pr in d.prims.iter().take(CAP) {
        let (ty, c) = match &pr.kind {
            PrimKind::Poly { closed } => ("Polygon", *closed),
            PrimKind::Polyline => ("Polygon", false),
            PrimKind::Sphere { .. } => ("Sphere", true),
            PrimKind::Tube { .. } => ("Tube", true),
            PrimKind::Volume => ("Volume", true),
            PrimKind::PackedRef => ("PackedPrim", true),
            PrimKind::BezierCurve { closed, .. } => ("BezierCurve", *closed),
            PrimKind::NurbsCurve { closed, .. } => ("NURBSCurve", *closed),
            PrimKind::BezierSurf { .. } => ("BezierSurface", true),
            PrimKind::NurbsSurf { .. } => ("NURBSSurface", true),
            PrimKind::Unknown { ty, .. } => (ty.as_str(), true),
        };
        prim_types.insert(ty.to_string());
        closed.push(c);
        prim_pts.push(pr.verts.clone().filter_map(|v| d.vertex_point.get(v as usize).copied()).collect::<Vec<u32>>());
    }
    let (lo, hi) = d.bbox();
    let bb = |v: [f32; 3]| [r6(v[0]), r6(v[1]), r6(v[2])];
    let empty = d.p().is_empty();
    let v = serde_json::json!({
        "points": d.point_count, "prims": d.prim_count(), "vertices": d.vertex_count(),
        "attribs": {"point": names(AttrClass::Point), "prim": names(AttrClass::Prim), "vertex": names(AttrClass::Vertex), "detail": names(AttrClass::Detail)},
        "prim_types": prim_types, "point_groups": groups(AttrClass::Point), "prim_groups": groups(AttrClass::Prim),
        "bbox_min": if empty { serde_json::Value::Null } else { serde_json::json!(bb(lo)) },
        "bbox_max": if empty { serde_json::Value::Null } else { serde_json::json!(bb(hi)) },
        "P": p_all.iter().take(8).collect::<Vec<_>>(), "P_all": p_all, "prim_pts": prim_pts, "prim_closed": closed,
    });
    Ok(format!(
        "{v}
"
    ))
}

/// Built-in SOPs + the hexe-native `motion` node (same set as the desktop app's palette, `hexe-tauri` `app_registry`).
fn registry() -> hexe_sop::Registry {
    let mut r = hexe_sop::registry();
    r.register(sop_motion::TYPE_NAME, sop_motion::spec, sop_motion::new);
    r
}

/// Scene JSON text → cooked display node as `.geo` JSON bytes + `(points, prims)`.
#[cfg(test)]
fn cook_scene(json: &str, time: f32) -> Result<(Vec<u8>, u32, u32), String> {
    cook_scene_in(json, time, None)
}

/// Like `cook_scene`; relative `file` SOP paths also resolve against `base` (the scene file's directory).
#[cfg(test)]
fn cook_scene_in(json: &str, time: f32, base: Option<&std::path::Path>) -> Result<(Vec<u8>, u32, u32), String> {
    cook_scene_range(json, time, base, None)
}

/// `cook_scene_in`, first cooking the display node at every frame of `frames` = `(start, end, step)` (frame 1 = time 0)
/// so a `rop_geometry` renders its range; the output is then the display cook at `time`.
fn cook_scene_range(
    json: &str,
    time: f32,
    base: Option<&std::path::Path>,
    frames: Option<(i64, i64, i64)>,
) -> Result<(Vec<u8>, u32, u32), String> {
    let hexe_sop::Loaded { mut graph, display, mut ui, .. } =
        hexe_sop::load_scene_with_base(json, &registry(), base).map_err(|e| e.to_string())?;
    if let Some((a, b, s)) = frames {
        hexe_sop::render_frames(&mut graph, display, a, b, s)?;
    }
    // channel references (`ch("../null1/p")`) evaluated at the cook frame (24 fps, as `graph.cook` times it)
    let fps = 24.0;
    for (node, chan, e) in hexe_sop::chans::apply_channels(&mut graph, &mut ui, f64::from(time) * fps + 1.0, fps) {
        eprintln!("hexe-cli: channel {node}.{chan}: {e}");
    }
    let detail = graph.cook(display, time).map_err(|e| e.to_string())?;
    Ok((hexe_geo::write_geo_json(&detail), detail.point_count, detail.prim_count()))
}

struct CookArgs {
    scene: String,
    out: String,
    time: f32,
    frames: Option<(i64, i64, i64)>,
}

/// `A`, `A-B` or `A-BxS` (frames, step >= 1, start <= end).
fn parse_frames(v: &str) -> Result<(i64, i64, i64), String> {
    let bad = || format!("cook: bad --frames `{v}` (want A, A-B or A-BxS)");
    let (range, step) = match v.split_once('x') {
        Some((r, s)) => (r, s.parse::<i64>().map_err(|_| bad())?),
        None => (v, 1),
    };
    let (a, b) = match range.split_once('-').filter(|(a, _)| !a.is_empty()) {
        Some((a, b)) => (a.parse::<i64>().map_err(|_| bad())?, b.parse::<i64>().map_err(|_| bad())?),
        None => {
            let a = range.parse::<i64>().map_err(|_| bad())?;
            (a, a)
        }
    };
    if step < 1 || b < a {
        return Err(bad());
    }
    Ok((a, b, step))
}

fn parse_cook(args: &[String]) -> Result<CookArgs, String> {
    let (mut scene, mut out, mut time, mut frames) = (None, None, 0.0f32, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" | "-o" => out = Some(it.next().ok_or("cook: --out needs a path")?.clone()),
            "--frames" => frames = Some(parse_frames(it.next().ok_or("cook: --frames needs A-B[xS]")?)?),
            // `--frame N`: timeline $F (1-based) -> time = (N-1)/fps, the same mapping the app timeline uses.
            "--frame" => {
                let v = it.next().ok_or("cook: --frame needs a number")?;
                let n = v.parse::<f32>().ok().filter(|t| t.is_finite()).ok_or(format!("cook: bad --frame `{v}`"))?;
                time = (n - 1.0) / hexe_graph::DEFAULT_FPS;
            }
            "--time" => {
                let v = it.next().ok_or("cook: --time needs a number")?;
                time = v.parse::<f32>().ok().filter(|t| t.is_finite()).ok_or(format!("cook: bad --time `{v}`"))?;
            }
            flag if flag.starts_with('-') => return Err(format!("cook: unknown option `{flag}`")),
            path if scene.is_none() => scene = Some(path.to_string()),
            extra => return Err(format!("cook: unexpected argument `{extra}`")),
        }
    }
    Ok(CookArgs {
        scene: scene.ok_or("usage: hexe-cli cook <scene.json> --out <file.geo> [--time T | --frame N]")?,
        out: out.ok_or("cook: --out <file.geo> is required")?,
        time,
        frames,
    })
}

fn cook(args: &[String]) -> Result<String, String> {
    let a = parse_cook(args)?;
    let meta = std::fs::metadata(&a.scene).map_err(|e| format!("{}: {e}", a.scene))?;
    if meta.len() > MAX_SCENE_BYTES {
        return Err(format!("{}: scene larger than {MAX_SCENE_BYTES} bytes", a.scene));
    }
    // Endorphin files cook directly: a one-node `file` graph over the opened rest-pose geometry.
    let json = if endo::is_endo(&a.scene) {
        endo::scene_json(&a.scene)?
    } else {
        std::fs::read_to_string(&a.scene).map_err(|e| format!("{}: {e}", a.scene))?
    };
    let (bytes, points, prims) = cook_scene_range(&json, a.time, std::path::Path::new(&a.scene).parent(), a.frames)?;
    std::fs::write(&a.out, bytes).map_err(|e| format!("{}: {e}", a.out))?;
    Ok(format!("cooked {points} points, {prims} prims -> {}\n", a.out))
}

/// `convert <in> <out>`: `read_any` -> `write_any` (in: .geo .bgeo .bgeo.sc .obj .ply; out: .geo .obj .ply).
fn convert(args: &[String]) -> Result<String, String> {
    let [input, output] = args else { return Err("usage: hexe-cli convert <in> <out>".into()) };
    let d = hexe_geo::read_any(input).map_err(|e| format!("{input}: {e}"))?;
    hexe_geo::write_any(output, &d).map_err(|e| format!("{output}: {e}"))?;
    Ok(format!(
        "converted {} points, {} vertices, {} prims: {input} -> {output}\n",
        d.point_count,
        d.vertex_count(),
        d.prim_count()
    ))
}

fn show(v: &ParamValue) -> String {
    match v {
        ParamValue::F(x) => format!("F {x}"),
        ParamValue::I(x) => format!("I {x}"),
        ParamValue::B(x) => format!("B {x}"),
        ParamValue::V3([x, y, z]) => format!("V3 [{x}, {y}, {z}]"),
        ParamValue::S(x) => format!("S {x:?}"),
        ParamValue::Menu(x) => format!("Menu {x}"),
    }
}

fn show_param(p: &ParamSpec) -> String {
    let range = p.range.map_or(String::new(), |(lo, hi)| format!("  range {lo}..{hi}"));
    format!("  {:<16} {}{range}\n", p.name, show(&p.default))
}

/// `nodes`: every registered SOP type with its inputs and `token  Type default` param table.
fn nodes() -> String {
    let reg = registry();
    let mut out = String::new();
    for name in reg.type_names() {
        let inputs = reg
            .create(name)
            .map_or_else(String::new, |n| n.inputs().iter().map(|i| i.name).collect::<Vec<_>>().join(", "));
        let _ = writeln!(out, "{name}  (inputs: {})", if inputs.is_empty() { "none" } else { &inputs });
        for p in reg.specs(name).unwrap_or_default() {
            out.push_str(&show_param(&p));
        }
    }
    out
}

fn dispatch(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => Ok(format!("hexe {}\n", env!("CARGO_PKG_VERSION"))),
        Some("cook") => cook(&args[1..]),
        Some("nodes") => Ok(nodes()),
        Some("convert") => convert(&args[1..]),
        Some("info") => info(&args[1..]),
        Some("open") => open_dispatch(&args[1..]),
        Some("endo-survey") => endo::survey_cmd(&args[1..]),
        Some("oracle") => match args.get(1) {
            Some(case) => oracle(case, args.get(2).map(String::as_str)),
            None => Err("usage: hexe-cli oracle <case> [input]".into()),
        },
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match catch_unwind(AssertUnwindSafe(|| dispatch(&args))) {
        Ok(Ok(out)) => {
            print!("{out}");
            ExitCode::SUCCESS
        }
        Ok(Err(msg)) => {
            eprintln!("hexe-cli: {msg}");
            ExitCode::FAILURE
        }
        Err(_) => {
            eprintln!("hexe-cli: internal error (panic caught)");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn version_flag_prints_name_and_version() {
        let out = dispatch(&["--version".into()]).unwrap();
        assert_eq!(out, format!("hexe {}\n", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn unknown_oracle_case_is_an_error_not_a_panic() {
        assert!(dispatch(&["oracle".into(), "nope".into()]).is_err());
    }

    #[test]
    fn cook_args_parse_and_reject() {
        let a = parse_cook(&s(&["a.json", "--out", "o.geo", "--time", "1.5"])).unwrap();
        assert_eq!((a.scene.as_str(), a.out.as_str(), a.time), ("a.json", "o.geo", 1.5));
        for bad in [
            s(&[]),
            s(&["a.json"]),
            s(&["--out", "o.geo"]),
            s(&["a.json", "--out"]),
            s(&["a.json", "--out", "o", "--time", "nan"]),
            s(&["a.json", "--out", "o", "--time", "x"]),
            s(&["a.json", "--out", "o", "--bogus"]),
            s(&["a.json", "b.json", "--out", "o"]),
        ] {
            assert!(parse_cook(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn cook_scene_null_is_empty_geo_roundtrip() {
        let (bytes, p, pr) = cook_scene(r#"{"nodes":[{"name":"n","type":"null"}],"display":"n"}"#, 0.0).unwrap();
        assert_eq!((p, pr), (0, 0));
        let d = hexe_geo::read_geo(&bytes).unwrap();
        assert_eq!((d.point_count, d.prim_count(), d.p().len()), (0, 0, 0));
    }

    #[test]
    fn cook_scene_errors_are_messages() {
        assert!(cook_scene("{", 0.0).unwrap_err().contains("invalid scene JSON"));
        assert!(cook_scene(r#"{"nodes":[],"display":"q"}"#, 0.0).is_err());
        assert!(dispatch(&s(&["cook", "/nonexistent/scene.json", "--out", "x.geo"])).is_err());
        assert!(dispatch(&s(&["bogus"])).is_err());
    }

    #[test]
    fn oracle_summary_matches_houdini_summary_shape() {
        let dir = std::env::temp_dir().join("hexe_oracle_summary_test");
        std::fs::create_dir_all(&dir).unwrap();
        let geo = dir.join("box.geo");
        let mut d = hexe_geo::Detail::default();
        d.add_points(3);
        *d.p_mut() = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        d.add_poly(&[0, 1, 2], true);
        std::fs::write(&geo, hexe_geo::write_geo_json(&d)).unwrap();
        let out = dispatch(&s(&["oracle", "summary", geo.to_str().unwrap()])).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!((v["points"].as_u64(), v["prims"].as_u64(), v["vertices"].as_u64()), (Some(3), Some(1), Some(3)));
        assert_eq!(v["prim_pts"], serde_json::json!([[0, 1, 2]]));
        assert_eq!(v["prim_types"], serde_json::json!(["Polygon"]));
        assert_eq!(v["prim_closed"], serde_json::json!([true]));
        assert_eq!(v["attribs"]["point"], serde_json::json!(["P"]));
        assert!(dispatch(&s(&["oracle", "summary"])).is_err());
    }

    #[test]
    fn info_prints_skipped_attribs() {
        let g = r#"{"pointcount":1,"vertexcount":0,"primitivecount":0,"attributes":{"pointattributes":[
          [{"name":"P","type":"numeric"},{"values":{"size":3,"storage":"fpreal32","tuples":[[0,0,0]]}}],
          [{"name":"tags","type":"stringarray"},{"strings":[],"indices":[[]]}]]}}"#;
        let out = info_text(g.as_bytes()).unwrap();
        assert!(out.contains("attr point:P\n") && out.contains("skipped point:tags:stringarray\n"), "{out}");
        assert!(info(&[]).is_err() && info(&s(&["/nonexistent/x.bgeo"])).is_err());
    }

    #[test]
    fn convert_roundtrips_formats_and_rejects_bad_args() {
        let dir = std::path::PathBuf::from(
            std::env::var("HEXE_SCRATCH")
                .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/hexe-scratch").into()),
        );
        std::fs::create_dir_all(&dir).unwrap();
        let p = |n: &str| dir.join(n).to_str().unwrap().to_string();
        let scene = r#"{"nodes":[{"name":"b","type":"box"}],"display":"b"}"#;
        let (geo, _, _) = cook_scene(scene, 0.0).unwrap();
        std::fs::write(p("cli_box.geo"), geo).unwrap();
        let out = dispatch(&s(&["convert", &p("cli_box.geo"), &p("cli_box.obj")])).unwrap();
        assert!(out.starts_with("converted 8 points, 24 vertices, 6 prims"), "{out}");
        let out = dispatch(&s(&["convert", &p("cli_box.obj"), &p("cli_box.ply")])).unwrap();
        assert!(out.contains("8 points, 24 vertices, 6 prims"), "{out}");
        assert!(dispatch(&s(&["info", &p("cli_box.ply")])).unwrap().starts_with("points 8\nvertices 24\nprims 6"));
        let bad_in = s(&["convert", "/nonexistent/a.obj", "b.ply"]);
        for bad in [s(&["convert"]), s(&["convert", "a.obj"]), bad_in, s(&["convert", &p("cli_box.obj"), "x.stl"])] {
            assert!(dispatch(&bad).is_err(), "{bad:?}");
        }
    }
    #[test]
    fn cook_with_rop_writes_the_file_and_file_sop_is_scene_relative() {
        let dir = std::path::PathBuf::from(
            std::env::var("HEXE_SCRATCH")
                .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/hexe-scratch").into()),
        );
        std::fs::create_dir_all(&dir).unwrap();
        let (out, geo) = (dir.join("cli_rop.ply"), dir.join("cli_rop_cook.geo"));
        let _ = std::fs::remove_file(&out);
        let scene = format!(
            r#"{{"nodes":[{{"name":"b","type":"box"}},{{"name":"r","type":"rop_geometry","params":{{"sopoutput":{:?}}}}}],"edges":[["b",0,"r",0]],"display":"r"}}"#,
            out.to_str().unwrap()
        );
        std::fs::write(dir.join("cli_rop_scene.json"), scene).unwrap();
        let args = s(&["cook", dir.join("cli_rop_scene.json").to_str().unwrap(), "--out", geo.to_str().unwrap()]);
        assert!(dispatch(&args).unwrap().starts_with("cooked 8 points, 6 prims"));
        let back = hexe_geo::read_any(&out).unwrap();
        assert_eq!((back.point_count, back.vertex_count(), back.prim_count()), (8, 24, 6));
    }
    #[test]
    fn cook_frames_renders_the_rop_range_and_parses_strictly() {
        assert_eq!(parse_frames("3").unwrap(), (3, 3, 1));
        assert_eq!(parse_frames("1-24").unwrap(), (1, 24, 1));
        assert_eq!(parse_frames("1-10x3").unwrap(), (1, 10, 3));
        for bad in ["", "x", "5-1", "1-5x0", "a-b", "1-"] {
            assert!(parse_frames(bad).is_err(), "{bad}");
        }
        let dir = std::path::PathBuf::from(
            std::env::var("HEXE_SCRATCH")
                .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/hexe-scratch").into()),
        );
        std::fs::create_dir_all(&dir).unwrap();
        let name = |f: u32| dir.join(format!("cli_fr_{f}.obj"));
        for f in 1..=4 {
            let _ = std::fs::remove_file(name(f));
        }
        let scene = format!(
            r#"{{"nodes":[{{"name":"b","type":"box"}},{{"name":"r","type":"rop_geometry","params":{{"sopoutput":{:?},"trange":1,"f":[1,4,1]}}}}],"edges":[["b",0,"r",0]],"display":"r"}}"#,
            dir.join("cli_fr_$F.obj").to_str().unwrap()
        );
        std::fs::write(dir.join("cli_fr_scene.json"), scene).unwrap();
        let geo = dir.join("cli_fr.geo");
        let args = s(&[
            "cook",
            dir.join("cli_fr_scene.json").to_str().unwrap(),
            "--out",
            geo.to_str().unwrap(),
            "--frames",
            "1-3",
        ]);
        assert!(dispatch(&args).unwrap().starts_with("cooked 8 points, 6 prims"));
        assert_eq!((1..=4).map(|f| name(f).exists()).collect::<Vec<_>>(), [true, true, true, false]);
        assert!(dispatch(&s(&["cook", "x.json", "--out", "o.geo", "--frames", "9-1"])).is_err());
    }
    #[test]
    fn example_file_rop_scene_cooks_from_any_cwd() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let json = std::fs::read_to_string(root.join("examples/file-rop.json")).unwrap();
        let dir = std::path::PathBuf::from(
            std::env::var("HEXE_SCRATCH")
                .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/hexe-scratch").into()),
        );
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("file-rop-out.obj");
        let _ = std::fs::remove_file(&out);
        // forward slashes: a Windows path pasted raw into the JSON string would be read as `\U...` escapes
        let json = json.replace("file-rop-out.obj", &out.to_str().unwrap().replace('\\', "/"));
        let (_, p, pr) = cook_scene_in(&json, 0.0, Some(&root.join("examples"))).unwrap();
        assert_eq!((p, pr), (6, 8));
        let back = hexe_geo::read_any(&out).unwrap();
        assert_eq!((back.point_count, back.vertex_count(), back.prim_count()), (6, 24, 8));
    }

    #[test]
    fn nodes_lists_null() {
        let out = nodes();
        assert!(out.contains("null  (inputs: input)"), "{out}");
    }
}
