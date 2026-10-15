//! DOC-VERIFIED vs Houdini 20.5 help `out/geometry.txt`, tests/rop_geometry_semantics.rs (write + reload == input; not a recorded row).
//! ROP Geometry Output (`rop_geometry`, `rop-geometry`): cooking writes input 0 to `sopoutput` (format by extension via
//! `hexe_geo::write_any`: `.geo` JSON, `.bgeo` binary JSON, `.obj`, `.ply`) and passes the input through.
//!
//! * Frame = `round(time * fps) + 1` (Houdini `$F`: frame 1 at time 0). `$F`, `$F<n>` (zero-padded to `n` digits), `${F}` and
//!   `${F<n>}` in `sopoutput` expand to it. The node is time dependent (re-cooks, so re-writes, per frame).
//! * `trange` menu `off`(0: write the current frame) | `range`(1) | `strict`(2). `f` V3 = (start, end, step) in frames
//!   (start <= end, step >= 1). `range`: a cook at a frame ON the range grid (`start + k*step <= end`) writes, any other
//!   frame passes the input through without writing; `strict`: a frame off the grid is `Failed`. `render_frames` (crate
//!   fn) cooks a graph node at every frame of a range.
//! * `mkpath` B off: create missing parent directories of the output.
//! * Scripts `prerender` / `postrender` / `preframe` / `postframe` S (shell commands run by `sh -c`, env `HEXE_FRAME` and
//!   `HEXE_OUTPUT`) are REFUSED (`Unsupported`) unless `allowscripts` B is on (scene files are untrusted input). Order per
//!   cook: prerender (first frame of the range, or always when `trange` is off), preframe, write, postframe, postrender
//!   (last grid frame, or always when off). A non-zero exit is `Failed`.
//! * Relative `sopoutput`: the scene file's directory when loaded via `load_scene_in` (like the `file` SOP), else the cwd.
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

const SCRIPTS: [&str; 4] = ["prerender", "preframe", "postframe", "postrender"];

pub fn spec() -> Vec<ParamSpec> {
    let mut v = vec![
        ParamSpec::new("sopoutput", ParamValue::S(String::new())).label("Output File"),
        ParamSpec::new("trange", ParamValue::Menu(0)).label("Valid Frame Range").menu(&[
            ("off", "Render Current Frame"),
            ("range", "Render Frame Range"),
            ("strict", "Render Frame Range Only (Strict)"),
        ]),
        ParamSpec::new("f", ParamValue::V3([1.0, 1.0, 1.0])).label("Start/End/Inc"),
        ParamSpec::new("mkpath", ParamValue::B(false)).label("Create Intermediate Directories"),
        ParamSpec::new("allowscripts", ParamValue::B(false)).label("Allow Shell Scripts"),
    ];
    for s in SCRIPTS {
        v.push(ParamSpec::new(s, ParamValue::S(String::new())).label(s));
    }
    v
}

pub fn new() -> Box<SopNode> {
    Box::new(RopGeometry { specs: spec() })
}

struct RopGeometry {
    specs: Vec<ParamSpec>,
}

/// Expand `$F`, `$F<n>`, `${F}`, `${F<n>}` (zero-padded) with `frame`; every other `$` stays literal.
fn expand_frame(path: &str, frame: i64) -> String {
    let b: Vec<char> = path.chars().collect();
    let mut out = String::with_capacity(path.len() + 8);
    let mut i = 0;
    while i < b.len() {
        if b[i] == '$' {
            let braced = b.get(i + 1) == Some(&'{');
            let mut j = i + 1 + usize::from(braced);
            if b.get(j) == Some(&'F') {
                j += 1;
                let ds = j;
                while b.get(j).is_some_and(char::is_ascii_digit) {
                    j += 1;
                }
                let width: usize = b[ds..j].iter().collect::<String>().parse().unwrap_or(0);
                let closed = !braced || b.get(j) == Some(&'}');
                let word_end = braced || !b.get(j).is_some_and(|c| c.is_ascii_alphabetic());
                if closed && word_end {
                    out.push_str(&format!("{frame:0width$}", width = width.min(32)));
                    i = j + usize::from(braced);
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// `(start, end, step)` frames from `f`; hostile values are `BadParam`.
fn frame_range(p: &ParamSet) -> Result<(i64, i64, i64), CookError> {
    let f = p.vec3("f").filter(|v| v.iter().all(|c| c.is_finite() && c.abs() < 1e9));
    let Some(f) = f else { return Err(CookError::BadParam("`f`: missing or not a finite vector".into())) };
    let (a, b, s) = (f[0].round() as i64, f[1].round() as i64, f[2].round() as i64);
    if b < a || s < 1 {
        return Err(CookError::BadParam(format!("`f` must satisfy start <= end and step >= 1, got {a} {b} {s}")));
    }
    Ok((a, b, s))
}

fn run_script(cmd: &str, frame: i64, out: &str) -> Result<(), CookError> {
    let st = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .env("HEXE_FRAME", frame.to_string())
        .env("HEXE_OUTPUT", out)
        .status()
        .map_err(|e| CookError::Failed(format!("rop_geometry: script `{cmd}`: {e}")))?;
    if st.success() { Ok(()) } else { Err(CookError::Failed(format!("rop_geometry: script `{cmd}` exited with {st}"))) }
}

impl Node<Detail> for RopGeometry {
    fn type_name(&self) -> &'static str {
        "rop_geometry"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn time_dependent(&self) -> bool {
        true
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let trange = p.menu("trange").unwrap_or(0);
        if trange > 2 {
            return Err(CookError::BadParam(format!("`trange` menu index {trange} out of range 0..=2")));
        }
        let template = p.string("sopoutput").unwrap_or_default();
        if template.is_empty() {
            return Err(CookError::BadParam("`sopoutput` is empty".into()));
        }
        let allow =
            p.toggle("allowscripts").ok_or_else(|| CookError::BadParam("`allowscripts`: not a toggle".into()))?;
        let mkpath = p.toggle("mkpath").ok_or_else(|| CookError::BadParam("`mkpath`: not a toggle".into()))?;
        let script = |n: &str| -> Result<&str, CookError> {
            let s = p.string(n).ok_or_else(|| CookError::BadParam(format!("`{n}`: missing or not a string")))?;
            if !s.trim().is_empty() && !allow {
                return Err(CookError::Unsupported(format!("rop_geometry: `{n}` needs `allowscripts` on")));
            }
            Ok(s.trim())
        };
        let [pre_r, pre_f, post_f, post_r] =
            [script(SCRIPTS[0])?, script(SCRIPTS[1])?, script(SCRIPTS[2])?, script(SCRIPTS[3])?];
        let frame = (f64::from(ctx.time) * f64::from(ctx.fps)).round() as i64 + 1;
        let input = inputs.first().cloned().unwrap_or_default();
        let (mut first, mut last) = (true, true);
        if trange != 0 {
            let (a, b, s) = frame_range(p)?;
            let on_grid = frame >= a && frame <= b && (frame - a) % s == 0;
            if !on_grid {
                if trange == 2 {
                    return Err(CookError::Failed(format!(
                        "rop_geometry: frame {frame} is outside the strict range {a}-{b}x{s}"
                    )));
                }
                (ctx.log)(&format!("rop_geometry: frame {frame} is outside {a}-{b}x{s}: nothing written"));
                return Ok(input);
            }
            first = frame == a;
            last = frame == a + (b - a) / s * s;
        }
        let path = expand_frame(template, frame);
        if first && !pre_r.is_empty() {
            run_script(pre_r, frame, &path)?;
        }
        if !pre_f.is_empty() {
            run_script(pre_f, frame, &path)?;
        }
        if mkpath && let Some(dir) = std::path::Path::new(&path).parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| CookError::Failed(format!("{}: {e}", dir.display())))?;
        }
        hexe_geo::write_any(&path, &input).map_err(|e| CookError::Failed(e.to_string()))?;
        (ctx.log)(&format!("rop_geometry: wrote {path}"));
        if !post_f.is_empty() {
            run_script(post_f, frame, &path)?;
        }
        if last && !post_r.is_empty() {
            run_script(post_r, frame, &path)?;
        }
        Ok(input)
    }
}

/// Cook `id` at every frame of `start..=end` step `step` (frame 1 = time 0) so a `rop_geometry` there renders the range.
/// Returns the number of frames cooked; the first failing frame aborts with its error.
pub fn render_frames(
    graph: &mut hexe_graph::Graph<Detail>,
    id: hexe_graph::NodeId,
    start: i64,
    end: i64,
    step: i64,
) -> Result<u32, String> {
    if step < 1 || end < start || end - start > 1_000_000 {
        return Err(format!("bad frame range {start}-{end}x{step}"));
    }
    let fps = f64::from(graph.fps());
    let mut n = 0;
    let mut f = start;
    while f <= end {
        graph.cook(id, ((f - 1) as f64 / fps) as f32).map_err(|e| format!("frame {f}: {e}"))?;
        n += 1;
        f += step;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::super::file_sop::tests::scratch;
    use super::expand_frame;
    use crate::load_scene;
    use hexe_geo::{AttrClass, Detail};

    fn box_rop(out: &str, extra: &str) -> String {
        format!(
            r#"{{"nodes":[{{"name":"b","type":"box"}},{{"name":"r","type":"rop_geometry","params":{{"sopoutput":{out:?}{extra}}}}}],"edges":[["b",0,"r",0]],"display":"r"}}"#
        )
    }
    fn prim_normals(d: &Detail) -> Vec<[f32; 3]> {
        d.prims
            .iter()
            .map(|pr| {
                let ps: Vec<[f32; 3]> = pr.verts.clone().map(|v| d.p()[d.vertex_point[v as usize] as usize]).collect();
                let mut n = [0.0f32; 3];
                for i in 0..ps.len() {
                    let (a, b) = (ps[i], ps[(i + 1) % ps.len()]);
                    n[0] += (a[1] - b[1]) * (a[2] + b[2]);
                    n[1] += (a[2] - b[2]) * (a[0] + b[0]);
                    n[2] += (a[0] - b[0]) * (a[1] + b[1]);
                }
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                [n[0] / l, n[1] / l, n[2] / l]
            })
            .collect()
    }

    /// Oracle: box -> rop writes the file; read back = 8 points / 24 vertices / 6 prims, P byte-equal, prim N equal.
    #[test]
    fn box_rop_writes_each_format_and_reads_back() {
        for ext in ["obj", "ply", "geo"] {
            let p = scratch(&format!("rop_box.{ext}"));
            let _ = std::fs::remove_file(&p);
            let (mut g, id) = load_scene(&box_rop(p.to_str().expect("utf8"), "")).expect("loads");
            let cooked = g.cook(id, 0.0).expect("cooks");
            let back = hexe_geo::read_any(&p).expect("written file reads");
            assert_eq!((back.point_count, back.vertex_count(), back.prim_count()), (8, 24, 6), "{ext}");
            assert_eq!(back.p(), cooked.p(), "{ext}: P byte-equal");
            for (a, b) in prim_normals(&back).iter().zip(prim_normals(&cooked)) {
                assert!((0..3).all(|i| (a[i] - b[i]).abs() < 1e-6), "{ext}");
            }
            assert!(back.attr(AttrClass::Point, "P").is_some());
        }
    }
    fn at(g: &mut hexe_graph::Graph<Detail>, id: hexe_graph::NodeId, frame: i64) -> Result<(), String> {
        g.cook(id, (frame - 1) as f32 / 24.0).map(|_| ()).map_err(|e| e.to_string())
    }
    fn scene(path: &std::path::Path, params: serde_json::Value) -> String {
        let mut p = serde_json::json!({ "sopoutput": path.to_str().expect("utf8") });
        for (k, v) in params.as_object().expect("object") {
            p[k] = v.clone();
        }
        serde_json::json!({"nodes":[{"name":"b","type":"box"},{"name":"r","type":"rop_geometry","params":p}],"edges":[["b",0,"r",0]],"display":"r"})
            .to_string()
    }

    #[test]
    fn frame_variable_expansion() {
        assert_eq!(expand_frame("a/b$F4.obj", 7), "a/b0007.obj");
        assert_eq!(expand_frame("x${F3}y", 7), "x007y");
        assert_eq!(expand_frame("f$F.obj", 12), "f12.obj");
        assert_eq!(expand_frame("${F}", 5), "5");
        assert_eq!(expand_frame("$FPS $F2 $$F1 ${F", 3), "$FPS 03 $3 ${F");
        assert_eq!(expand_frame("plain.obj", 9), "plain.obj");
        assert_eq!(expand_frame("n$F4", -3), "n-003");
    }

    #[test]
    fn bgeo_output_reads_back_equal() {
        let p = scratch("rop_box.bgeo");
        let _ = std::fs::remove_file(&p);
        let (mut g, id) = load_scene(&box_rop(p.to_str().expect("utf8"), "")).expect("loads");
        let cooked = g.cook(id, 0.0).expect("cooks");
        let bytes = std::fs::read(&p).expect("written");
        assert!(bytes.starts_with(&[0x7f, b'N', b'S', b'J', b'b']));
        assert_eq!(hexe_geo::read_any(&p).expect("reads"), *cooked);
    }

    #[test]
    fn frame_range_writes_only_grid_frames_and_strict_rejects_the_rest() {
        let base = scratch("rop_rng_$F3.obj");
        let name = |f: i64| base.with_file_name(format!("rop_rng_{f:03}.obj"));
        for f in 1..=7 {
            let _ = std::fs::remove_file(name(f));
        }
        let json = scene(&base, serde_json::json!({"trange": 1, "f": [1.0, 5.0, 2.0]}));
        let (mut g, id) = load_scene(&json).expect("loads");
        assert_eq!(crate::render_frames(&mut g, id, 1, 7, 1), Ok(7));
        let wrote: Vec<i64> = (1..=7).filter(|&f| name(f).exists()).collect();
        assert_eq!(wrote, vec![1, 3, 5]);
        assert_eq!(hexe_geo::read_any(name(3)).expect("reads").prim_count(), 6);
        // strict: off-grid frame fails, on-grid works
        let strict = scene(&base, serde_json::json!({"trange": 2, "f": [1.0, 5.0, 2.0]}));
        let (mut g, id) = load_scene(&strict).expect("loads");
        assert!(at(&mut g, id, 2).expect_err("off grid").contains("outside the strict range"));
        assert!(at(&mut g, id, 5).is_ok() && at(&mut g, id, 6).is_err());
        // bad range params
        for f in [[5.0, 1.0, 1.0], [1.0, 5.0, 0.0], [f32::NAN, 1.0, 1.0]] {
            let bad = scene(&base, serde_json::json!({"trange": 1, "f": f}));
            let r = load_scene(&bad).map_err(|e| e.to_string()).and_then(|(mut g, id)| at(&mut g, id, 1));
            assert!(r.is_err(), "{f:?}");
        }
        assert!(
            crate::render_frames(&mut g, id, 5, 1, 1).is_err() && crate::render_frames(&mut g, id, 1, 5, 0).is_err()
        );
    }

    #[test]
    fn scripts_run_in_order_only_when_allowed_and_failures_propagate() {
        let out = scratch("rop_scr_$F.obj");
        let log = scratch("rop_scr.log");
        let _ = std::fs::remove_file(&log);
        let l = log.to_str().expect("utf8");
        let scripts = |allow: bool| {
            serde_json::json!({"trange": 1, "f": [1.0, 3.0, 1.0], "allowscripts": allow,
                "prerender": format!("echo pre >> {l}"), "preframe": format!("echo a$HEXE_FRAME >> {l}"),
                "postframe": format!("echo b$HEXE_FRAME >> {l}"), "postrender": format!("echo post >> {l}")})
        };
        let (mut g, id) = load_scene(&scene(&out, scripts(false))).expect("loads");
        assert!(at(&mut g, id, 1).expect_err("refused").contains("allowscripts"));
        assert!(!log.exists());
        let (mut g, id) = load_scene(&scene(&out, scripts(true))).expect("loads");
        assert_eq!(crate::render_frames(&mut g, id, 1, 3, 1), Ok(3));
        let lines: Vec<String> = std::fs::read_to_string(&log).expect("log").lines().map(str::to_string).collect();
        assert_eq!(lines, ["pre", "a1", "b1", "a2", "b2", "a3", "b3", "post"]);
        // trange off: pre + post around every single cook
        let _ = std::fs::remove_file(&log);
        let mut one = scripts(true);
        one["trange"] = serde_json::json!(0);
        let (mut g, id) = load_scene(&scene(&out, one)).expect("loads");
        at(&mut g, id, 4).expect("cooks");
        assert_eq!(
            std::fs::read_to_string(&log).expect("log").lines().collect::<Vec<_>>(),
            ["pre", "a4", "b4", "post"]
        );
        // a failing script aborts the cook
        let fail = serde_json::json!({"allowscripts": true, "preframe": "exit 3"});
        let (mut g, id) = load_scene(&scene(&out, fail)).expect("loads");
        assert!(at(&mut g, id, 1).expect_err("script failed").contains("exited"));
    }

    #[test]
    fn mkpath_and_scene_relative_output() {
        let dir = scratch("rop_mk");
        let _ = std::fs::remove_dir_all(&dir);
        let deep = dir.join("a/b/out.obj");
        let (mut g, id) = load_scene(&scene(&deep, serde_json::json!({}))).expect("loads");
        assert!(at(&mut g, id, 1).is_err() && !deep.exists());
        let (mut g, id) = load_scene(&scene(&deep, serde_json::json!({"mkpath": true}))).expect("loads");
        at(&mut g, id, 1).expect("creates dirs");
        assert!(deep.exists());
        // relative sopoutput is anchored at the scene directory when loaded with a base
        let rel = serde_json::json!({"nodes":[{"name":"b","type":"box"},{"name":"r","type":"rop_geometry","params":{"sopoutput":"rel_rop.obj"}}],"edges":[["b",0,"r",0]],"display":"r"}).to_string();
        let _ = std::fs::remove_file(dir.join("rel_rop.obj"));
        let (mut g, id) = crate::load_scene_in(&rel, Some(&dir)).expect("loads");
        at(&mut g, id, 1).expect("cooks");
        assert!(dir.join("rel_rop.obj").exists());
    }

    #[test]
    fn rop_rejects_bad_params_and_unwritable_paths() {
        let cook = |json: String| {
            load_scene(&json)
                .map_err(|e| e.to_string())
                .and_then(|(mut g, id)| g.cook(id, 0.0).map(|_| ()).map_err(|e| e.to_string()))
        };
        assert!(cook(box_rop("", "")).expect_err("empty").contains("sopoutput"));
        let p = scratch("rop_never.obj");
        assert!(cook(box_rop(p.to_str().expect("utf8"), ",\"trange\":3")).expect_err("trange").contains("trange"));
        assert!(!p.exists());
        assert!(cook(box_rop("/nonexistent-dir-hexe/x.obj", "")).is_err());
        assert!(cook(box_rop("x.stl", "")).expect_err("ext").contains("extension"));
    }
}
