//! rop_geometry: DOC-VERIFIED against Houdini 20.5 help `out/geometry.txt` (rop_geometry = Geometry output node; text only).
//! Doc facts: saves the input geometry to disk; Valid Frame Range (current frame | range | strict range); Start/End/Inc
//! inclusive; `$F` frame variables in the path. Oracle: file written, reload == input, input passes through.
mod common;
use common::*;
use hexe_geo::{read_any, AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookError, ParamValue as PV};
use std::sync::Arc;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("hexe_rop_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn path(p: &std::path::Path) -> PV {
    s(p.to_str().unwrap())
}
fn rich() -> Arc<Detail> {
    let mut d = (*grid3()).clone();
    d.add_attr(Attribute {
        name: "Cd".into(),
        class: AttrClass::Point,
        data: AttrData::V3((0..9).map(|i| [i as f32 / 8.0, 0.5, 0.25]).collect()),
        type_info: TypeInfo::Color,
    });
    Arc::new(d)
}

#[test]
fn writes_geo_and_reload_equals_input_and_passes_through() {
    let dir = tmp("geo");
    let f = dir.join("out.geo");
    let input = rich();
    let o = cook("rop_geometry", &[("sopoutput", path(&f))], Some(&input)).unwrap();
    assert_eq!(*o, *input, "input passes through");
    assert!(f.exists(), "file written");
    let back = read_any(&f).unwrap();
    assert_eq!(back.point_count, input.point_count);
    assert_eq!(back.p(), input.p());
    assert_eq!(back.prims.len(), input.prims.len());
    assert_eq!(back.vertex_point, input.vertex_point);
    assert_eq!(back.attr(AttrClass::Point, "Cd"), input.attr(AttrClass::Point, "Cd"));
}

#[test]
fn writes_bgeo_and_obj_by_extension() {
    let dir = tmp("fmt");
    let input = rich();
    for ext in ["bgeo", "obj", "ply"] {
        let f = dir.join(format!("out.{ext}"));
        cook("rop_geometry", &[("sopoutput", path(&f))], Some(&input)).unwrap();
        let back = read_any(&f).unwrap_or_else(|e| panic!("{ext}: {e}"));
        assert_eq!(back.point_count, 9, "{ext}");
        assert_eq!(back.prims.len(), 4, "{ext}");
        for (a, b) in back.p().iter().zip(input.p()) {
            assert!((0..3).all(|k| close(a[k], b[k])), "{ext}: {a:?} vs {b:?}");
        }
    }
    // .bgeo is lossless for attributes
    assert_eq!(read_any(dir.join("out.bgeo")).unwrap().attr(AttrClass::Point, "Cd"), input.attr(AttrClass::Point, "Cd"));
}

#[test]
fn frame_variable_expands_per_frame_and_range_gates_writes() {
    let dir = tmp("frames");
    let tpl = dir.join("f.$F4.geo");
    let input = rich();
    // frame 1 at t=0, frame 3 at t=2/24 (houdini $F)
    cook_at("rop_geometry", &[("sopoutput", path(&tpl))], Some(&input), 0.0).unwrap();
    cook_at("rop_geometry", &[("sopoutput", path(&tpl))], Some(&input), 2.0 / 24.0).unwrap();
    assert!(dir.join("f.0001.geo").exists() && dir.join("f.0003.geo").exists());
    assert!(!dir.join("f.0002.geo").exists());
    // Valid Frame Range = range 1..5 step 2: frame 2 is off the grid -> nothing written, frame 3 writes
    let rng = [("sopoutput", path(&tpl)), ("trange", PV::Menu(1)), ("f", PV::V3([1.0, 5.0, 2.0]))];
    let dir2 = tmp("frames2");
    let tpl2 = dir2.join("g.$F.geo");
    let r = [("sopoutput", path(&tpl2)), ("trange", PV::Menu(1)), ("f", PV::V3([1.0, 5.0, 2.0]))];
    let _ = rng;
    cook_at("rop_geometry", &r, Some(&input), 1.0 / 24.0).unwrap();
    assert!(!dir2.join("g.2.geo").exists(), "frame 2 off the grid");
    cook_at("rop_geometry", &r, Some(&input), 2.0 / 24.0).unwrap();
    assert!(dir2.join("g.3.geo").exists());
    // strict: off-grid frame is an error
    let strict = [("sopoutput", path(&tpl2)), ("trange", PV::Menu(2)), ("f", PV::V3([1.0, 5.0, 2.0]))];
    assert!(matches!(cook_at("rop_geometry", &strict, Some(&input), 1.0 / 24.0), Err(CookError::Failed(_))));
}

#[test]
fn edge_empty_input_empty_path_and_missing_dir() {
    let dir = tmp("edge");
    let f = dir.join("empty.geo");
    let e = Arc::new(Detail::default());
    cook("rop_geometry", &[("sopoutput", path(&f))], Some(&e)).unwrap();
    let back = read_any(&f).unwrap();
    assert_eq!((back.point_count, back.prims.len()), (0, 0), "empty in -> empty out");
    assert!(matches!(cook("rop_geometry", &[], Some(&e)), Err(CookError::BadParam(_))), "empty path");
    let deep = dir.join("a/b/x.geo");
    assert!(cook("rop_geometry", &[("sopoutput", path(&deep))], Some(&e)).is_err(), "missing parent dir, mkpath off");
    cook("rop_geometry", &[("sopoutput", path(&deep)), ("mkpath", PV::B(true))], Some(&e)).unwrap();
    assert!(deep.exists());
}
