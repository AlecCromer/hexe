//! ROP Geometry Output (`rop_geometry`, `rop-geometry`): cooking writes input 0 to `sopoutput` (format by extension via
//! `hexe_geo::write_any`: `.geo` `.obj` `.ply`) and passes the input through. `trange` Menu 0 = off (current frame) is the
//! only mode; other values -> `Unsupported` (no frame ranges yet). Relative `sopoutput` is relative to the cwd.
//! The graph cooks (so writes) once per dirty state, not per `cook` call.
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("sopoutput", ParamValue::S(String::new())).label("Output File"),
        ParamSpec::new("trange", ParamValue::Menu(0))
            .label("Valid Frame Range")
            .menu(&[("off", "Render Current Frame")]),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(RopGeometry { specs: spec() })
}

struct RopGeometry {
    specs: Vec<ParamSpec>,
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
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        if ctx.params.menu("trange").unwrap_or(0) != 0 {
            return Err(CookError::Unsupported("rop_geometry: only trange = off (single frame) is implemented".into()));
        }
        let path = ctx.params.string("sopoutput").unwrap_or_default();
        if path.is_empty() {
            return Err(CookError::BadParam("`sopoutput` is empty".into()));
        }
        let input = inputs.first().cloned().unwrap_or_default();
        hexe_geo::write_any(path, &input).map_err(|e| CookError::Failed(e.to_string()))?;
        (ctx.log)(&format!("rop_geometry: wrote {path}"));
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::super::file_sop::tests::scratch;
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
    #[test]
    fn rop_rejects_bad_params_and_unwritable_paths() {
        let cook = |json: String| {
            load_scene(&json)
                .map_err(|e| e.to_string())
                .and_then(|(mut g, id)| g.cook(id, 0.0).map(|_| ()).map_err(|e| e.to_string()))
        };
        assert!(cook(box_rop("", "")).expect_err("empty").contains("sopoutput"));
        let p = scratch("rop_never.obj");
        assert!(cook(box_rop(p.to_str().expect("utf8"), ",\"trange\":1")).expect_err("trange").contains("trange"));
        assert!(!p.exists());
        assert!(cook(box_rop("/nonexistent-dir-hexe/x.obj", "")).is_err());
        assert!(cook(box_rop("x.stl", "")).expect_err("ext").contains("extension"));
    }
}
