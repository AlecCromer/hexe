//! H39: golden compare of the 8 Houdini-shipped binary-JSON fixtures against their ASCII `.geo` twins
//! (`oracle/houdini/bgeo-twins.py`: Houdini 20.5.487 loads the .bgeo, saves the .geo). Counts + P bit-exact.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use crate::*;
use std::path::PathBuf;

const NAMES: [&str; 8] =
    ["axis", "cross", "ctrl-box", "ctrl-disc-xy", "ctrl-planes-xy", "sg_geolight", "simple_sphere", "tree0"];
/// Houdini-side counts (printed by bgeo-twins.py): points, prims, vertices.
const HOU: [(&str, u32, u32, u32); 8] = [
    ("axis", 7, 3, 6),
    ("cross", 6, 3, 6),
    ("ctrl-box", 24, 6, 24),
    ("ctrl-disc-xy", 40, 1, 40),
    ("ctrl-planes-xy", 4, 1, 4),
    ("sg_geolight", 48, 9, 64),
    ("simple_sphere", 240, 80, 240),
    ("tree0", 1084, 636, 2356),
];

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/bgeo-bjson")
}
pub(crate) fn load(n: &str) -> (Vec<u8>, Detail, Detail) {
    let b = std::fs::read(dir().join(format!("{n}.bgeo"))).expect("bgeo");
    let t = std::fs::read(dir().join(format!("{n}.geo"))).expect("twin");
    let (db, dt) = (read_geo(&b).expect("bgeo reads"), read_geo(&t).expect("twin reads"));
    (b, db, dt)
}

#[test]
fn bgeo_counts_and_p_equal_geo_twin() {
    assert_eq!(NAMES.len(), HOU.len());
    for (n, pts, prims, verts) in HOU {
        let (_, b, t) = load(n);
        assert_eq!(
            (b.point_count, b.prim_count(), b.vertex_count()),
            (pts, prims, verts),
            "{n}: bgeo vs Houdini counts"
        );
        assert_eq!(
            (b.point_count, b.prim_count(), b.vertex_count()),
            (t.point_count, t.prim_count(), t.vertex_count()),
            "{n}: bgeo vs twin counts"
        );
        assert_eq!(b.p().len(), t.p().len(), "{n}: P len");
        let mut max = 0.0_f32;
        for (x, y) in b.p().iter().zip(t.p()) {
            for k in 0..3 {
                max = max.max((x[k] - y[k]).abs());
            }
        }
        assert_eq!(max, 0.0, "{n}: max|dP|");
        assert_eq!(b.vertex_point, t.vertex_point, "{n}: vertex->point");
        assert_eq!(
            b.prims.iter().map(|p| p.verts.clone()).collect::<Vec<_>>(),
            t.prims.iter().map(|p| p.verts.clone()).collect::<Vec<_>>(),
            "{n}: prim vertex ranges"
        );
    }
}

#[test]
fn bgeo_attribute_names_equal_geo_twin() {
    for n in NAMES {
        let (_, b, t) = load(n);
        let k = |d: &Detail| {
            let mut v: Vec<_> = d.attrs.iter().map(|a| (a.class as u8, a.name.clone())).collect();
            v.sort();
            v
        };
        assert_eq!(k(&b), k(&t), "{n}: attribute set");
    }
}

/// H39: writer round trip on the 8 shipped fixtures: bgeo -> Detail -> write_geo_bjson -> Detail identical; value tree identical;
/// re-write is byte-stable; counts + P still equal the Houdini twin.
#[test]
fn writer_round_trips_all_shipped_fixtures() {
    for n in NAMES {
        let (orig, d, t) = load(n);
        let out = write_geo_bjson(&d).unwrap_or_else(|e| panic!("{n}: write: {e}"));
        assert!(out.starts_with(&[0x7f, b'N', b'S', b'J', b'b']), "{n}: magic");
        let back = read_geo(&out).unwrap_or_else(|e| panic!("{n}: reread: {e}"));
        assert_eq!(back, d, "{n}: Detail round trip");
        assert_eq!(write_geo_bjson(&back).unwrap(), out, "{n}: second write byte-stable");
        assert_eq!(
            bjson_to_value(&out).unwrap(),
            serde_json::from_slice::<serde_json::Value>(&write_geo_json(&d)).unwrap(),
            "{n}: value tree vs ASCII writer"
        );
        assert_eq!(
            (back.point_count, back.prim_count(), back.vertex_count()),
            (t.point_count, t.prim_count(), t.vertex_count()),
            "{n}: twin counts"
        );
        assert_eq!(back.p(), t.p(), "{n}: P == twin");
        // Houdini's own value tree re-encodes losslessly through our value encoder
        let tree = bjson_to_value(&orig).unwrap();
        assert_eq!(bjson_to_value(&value_to_bjson(&tree).unwrap()).unwrap(), tree, "{n}: tree re-encode");
    }
}
