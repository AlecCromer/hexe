//! H8 oracle: Houdini-shipped geometry (fixtures/houdini-ship). Expected values come from fixtures/houdini-ship/MANIFEST.toml,
//! derived without this reader. `gate = true` files must parse and match; `gate = false` files are findings (FAILURES.md).
use crate::*;
use std::path::PathBuf;
use toml::{Table, Value};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/houdini-ship")
}
fn manifest() -> Vec<Table> {
    let s = std::fs::read_to_string(dir().join("MANIFEST.toml")).expect("houdini-ship/MANIFEST.toml");
    let t: Table = s.parse().expect("valid toml");
    let Some(Value::Array(a)) = t.get("file") else { panic!("no [[file]] rows") };
    a.iter().filter_map(|v| v.as_table().cloned()).collect()
}
fn strs(r: &Table, k: &str) -> Vec<String> {
    let mut v: Vec<String> = r
        .get(k)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("missing {k}"))
        .iter()
        .map(|x| x.as_str().expect("string").to_string())
        .collect();
    v.sort();
    v
}
fn int(r: &Table, k: &str) -> i64 {
    r.get(k).and_then(Value::as_integer).unwrap_or_else(|| panic!("missing {k}"))
}
fn names(d: &Detail, c: AttrClass) -> Vec<String> {
    let mut v: Vec<String> = d.attrs.iter().filter(|a| a.class == c).map(|a| a.name.clone()).collect();
    v.sort();
    v
}
fn cls(c: AttrClass) -> &'static str {
    match c {
        AttrClass::Point => "point",
        AttrClass::Vertex => "vertex",
        AttrClass::Prim => "prim",
        AttrClass::Detail => "detail",
    }
}
fn all_names(d: &Detail, c: AttrClass) -> Vec<String> {
    // `Pw` (NURBS weights taken from the point w) is not one of the oracle's listed attributes
    let mut v: Vec<String> = names(d, c).into_iter().filter(|n| n != "Pw").collect();
    v.extend(d.skipped_attribs.iter().filter(|s| s.0 == c).map(|s| s.1.clone()));
    v.sort();
    v
}
fn gnames(d: &Detail, c: AttrClass) -> Vec<String> {
    let mut v: Vec<String> = d.groups.iter().filter(|g| g.class == c).map(|g| g.name.clone()).collect();
    v.sort();
    v
}

#[test]
fn houdini_ship_gated_files_match_oracle() {
    let rows = manifest();
    assert!(rows.iter().filter(|r| r.get("gate") == Some(&Value::Boolean(true))).count() >= 3);
    for r in rows.iter().filter(|r| r.get("gate") == Some(&Value::Boolean(true))) {
        let name = r["name"].as_str().expect("name");
        let bytes = std::fs::read(dir().join(r["path"].as_str().expect("path"))).expect("fixture file");
        assert_eq!(bytes.len() as i64, int(r, "bytes"), "{name}: byte size");
        let d = read_geo(&bytes).unwrap_or_else(|e| panic!("{name}: gated file failed to parse: {e}"));
        assert_eq!(i64::from(d.point_count), int(r, "expected_points"), "{name}: points");
        assert_eq!(i64::from(d.vertex_count()), int(r, "expected_vertices"), "{name}: vertices");
        assert_eq!(i64::from(d.prim_count()), int(r, "expected_prims"), "{name}: prims");
        if let Some(Value::Array(want)) = r.get("expected_prim_types") {
            let got: Vec<String> = d
                .prims
                .iter()
                .map(|p| match &p.kind {
                    PrimKind::Poly { .. } => "Poly".to_string(),
                    PrimKind::Sphere { .. } => "Sphere".to_string(),
                    PrimKind::Tube { .. } => "Tube".to_string(),
                    PrimKind::Unknown { ty, .. } => ty.clone(),
                    PrimKind::BezierCurve { .. } => "BezierCurve".to_string(),
                    PrimKind::NurbsCurve { .. } => "NURBCurve".to_string(),
                    PrimKind::BezierSurf { .. } => "BezierMesh".to_string(),
                    PrimKind::NurbsSurf { .. } => "NURBMesh".to_string(),
                    other => format!("{other:?}"),
                })
                .collect();
            let want: Vec<String> = want.iter().map(|x| x.as_str().expect("string").to_string()).collect();
            assert_eq!(got, want, "{name}: prim types");
        }
        assert_eq!(all_names(&d, AttrClass::Point), strs(r, "expected_point_attrs"), "{name}: point attrs");
        assert_eq!(all_names(&d, AttrClass::Vertex), strs(r, "expected_vertex_attrs"), "{name}: vertex attrs");
        assert_eq!(all_names(&d, AttrClass::Prim), strs(r, "expected_prim_attrs"), "{name}: prim attrs");
        if r.contains_key("expected_detail_attrs") {
            assert_eq!(all_names(&d, AttrClass::Detail), strs(r, "expected_detail_attrs"), "{name}: detail attrs");
        }
        let mut skipped: Vec<String> =
            d.skipped_attribs.iter().map(|(c, n, t)| format!("{}:{n}:{t}", cls(*c))).collect();
        skipped.sort();
        let want_skipped = if r.contains_key("expected_skipped") { strs(r, "expected_skipped") } else { Vec::new() };
        assert_eq!(skipped, want_skipped, "{name}: skipped_attribs");
        assert_eq!(gnames(&d, AttrClass::Point), strs(r, "expected_point_groups"), "{name}: point groups");
        assert_eq!(gnames(&d, AttrClass::Prim), strs(r, "expected_prim_groups"), "{name}: prim groups");
        let want: Vec<f64> = r
            .get("expected_first_p")
            .and_then(Value::as_array)
            .expect("expected_first_p")
            .iter()
            .map(|x| x.as_float().expect("float"))
            .collect();
        let got = d.p().first().copied().unwrap_or_else(|| panic!("{name}: no P"));
        for i in 0..3 {
            assert!((f64::from(got[i]) - want[i]).abs() < 1e-6, "{name}: first P[{i}] {} vs {}", got[i], want[i]);
        }
    }
}

/// Ungated files are findings: each must be documented in FAILURES.md; a file that starts parsing is reported, not failed.
#[test]
fn houdini_ship_ungated_files_are_documented() {
    let fails = std::fs::read_to_string(dir().join("FAILURES.md")).expect("houdini-ship/FAILURES.md");
    for r in manifest().iter().filter(|r| r.get("gate") == Some(&Value::Boolean(false))) {
        let name = r["name"].as_str().expect("name");
        assert!(fails.contains(name), "{name}: ungated but not recorded in FAILURES.md");
        let bytes = std::fs::read(dir().join(r["path"].as_str().expect("path"))).expect("fixture file");
        match read_geo(&bytes) {
            Err(e) => eprintln!("houdini-ship {name}: still fails ({e})"),
            Ok(_) => eprintln!("houdini-ship {name}: NOW PARSES; promote to gate=true with oracle values"),
        }
    }
}
