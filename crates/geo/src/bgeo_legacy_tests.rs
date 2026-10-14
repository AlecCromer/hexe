//! H13: legacy `BgeoV` reader. Oracle = the text twin `fixtures/gpd-all/expected.geo` (byte copy of Houdini's `all.geo`),
//! parsed by the GPD reader. Plus hostile-input sweeps (every truncation, lying counts, byte flips).
use crate::*;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}
fn bin() -> Vec<u8> {
    std::fs::read(root().join("houdini-ship/gpd-all-legacy/all.bgeo")).expect("all.bgeo")
}
fn twin() -> Detail {
    read_geo_gpd(&std::fs::read(root().join("gpd-all/expected.geo")).expect("twin")).expect("twin parses")
}
fn floats(a: &AttrData) -> Vec<f32> {
    match a {
        AttrData::F32(v) => v.clone(),
        AttrData::V2(v) => v.iter().flatten().copied().collect(),
        AttrData::V3(v) => v.iter().flatten().copied().collect(),
        AttrData::V4(v) => v.iter().flatten().copied().collect(),
        AttrData::FN { data, .. } => data.clone(),
        _ => Vec::new(),
    }
}
fn close(a: &[f32], b: &[f32], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: len");
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        // the twin prints 6 significant digits
        assert!((x - y).abs() <= 1e-5 * x.abs().max(1.0), "{what}[{i}]: {x} vs {y}");
    }
}

#[test]
fn binary_equals_text_twin() {
    let (b, t) = (read_geo(&bin()).expect("binary parses"), twin());
    assert_eq!((b.point_count, b.vertex_count(), b.prim_count()), (367, 455, 13));
    assert_eq!((b.point_count, b.vertex_count(), b.prim_count()), (t.point_count, t.vertex_count(), t.prim_count()));
    assert_eq!(b.vertex_point, t.vertex_point, "vertex->point map");
    for (i, (x, y)) in b.prims.iter().zip(&t.prims).enumerate() {
        assert_eq!(x.verts, y.verts, "prim {i} vertex range");
        match (&x.kind, &y.kind) {
            (PrimKind::Unknown { ty: a, .. }, PrimKind::Unknown { ty: c, .. }) => assert_eq!(a, c, "prim {i} type"),
            (PrimKind::Sphere { xform: a }, PrimKind::Sphere { xform: c }) => close(a, c, "sphere xform"),
            (PrimKind::Tube { xform: a, taper: p, closed: q }, PrimKind::Tube { xform: c, taper: r, closed: s }) => {
                close(a, c, "tube xform");
                assert_eq!((p, q), (r, s));
            }
            (a, c) => assert_eq!(a, c, "prim {i} kind"),
        }
    }
    let (mut an, mut tn): (Vec<_>, Vec<_>) = (b.attrs.iter().map(|a| (&a.name, a.class)).collect(), Vec::new());
    tn.extend(t.attrs.iter().map(|a| (&a.name, a.class)));
    an.sort_by_key(|x| x.0);
    tn.sort_by_key(|x| x.0);
    assert_eq!(an, tn, "attribute names/classes");
    for a in &b.attrs {
        let o = t.attrs.iter().find(|o| o.name == a.name).expect("twin attr");
        assert_eq!(a.type_info, o.type_info, "{} type_info", a.name);
        close(&floats(&a.data), &floats(&o.data), &a.name);
    }
    assert_eq!(b.groups, t.groups, "groups (names, class, masks)");
    assert_eq!(b.groups[0].mask.iter().filter(|x| **x).count(), 13);
}

#[test]
fn unmodelled_type_is_unsupported_with_hex_name() {
    let mut b = bin();
    let o = b.windows(4).position(|w| w == [0, 0, 0x10, 0]).expect("Circle token");
    b[o..o + 4].copy_from_slice(&0x0008u32.to_be_bytes());
    match read_geo(&b) {
        Err(GeoError::Unsupported(s)) => assert!(s.contains("0x8"), "{s}"),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn other_versions_are_unsupported() {
    let mut b = bin();
    b[8] = 9;
    assert!(matches!(read_geo(&b), Err(GeoError::Unsupported(_))));
}

#[test]
fn every_truncation_is_a_clean_error() {
    let b = bin();
    let end = read_geo(&b).map(|_| ()).is_ok();
    assert!(end);
    for n in 0..b.len() - 1 {
        assert!(read_geo(&b[..n]).is_err() || n >= b.len() - 1, "prefix {n} must not parse");
    }
}

#[test]
fn lying_header_counts_never_panic_or_overallocate() {
    let b = bin();
    // header ints at 9..41 (8 x i32) + version at 5
    for field in 0..9usize {
        for lie in [0u32, 1, 2, 367, 368, 0x7fff_ffff, 0x1000_0000, 0xffff_ffff, 0x8000_0000, 70_000] {
            let mut m = b.clone();
            let o = 5 + field * 4;
            m[o..o + 4].copy_from_slice(&lie.to_be_bytes());
            let _ = read_geo(&m);
        }
    }
}

#[test]
fn byte_flips_never_panic() {
    let b = bin();
    let mut s = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..30_000 {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let mut m = b.clone();
        let k = 1 + (s >> 60) as usize % 3;
        for j in 0..k {
            let at = ((s >> (8 + j * 11)) as usize) % m.len();
            m[at] = (s >> (j * 7)) as u8;
        }
        let _ = read_geo(&m);
    }
}

#[test]
fn wide_indices_for_65536_points_parse() {
    // synthetic: 65536 points, one 2-vertex Poly with u32 indices (npoints >= 65536 => i32 indices)
    let mut b = b"BgeoV".to_vec();
    for v in [2i32, 65536, 1, 0, 0, 0, 0, 0, 0] {
        b.extend(v.to_be_bytes());
    }
    b.extend(std::iter::repeat_n(0u8, 65536 * 16));
    b.extend(1u32.to_be_bytes());
    b.extend(2i32.to_be_bytes());
    b.push(b':');
    b.extend(70_000u32.wrapping_sub(5000).to_be_bytes());
    b.extend(65535u32.to_be_bytes());
    b.extend([0u8, 0xff]);
    let d = read_geo(&b).expect("parse");
    assert_eq!((d.point_count, d.vertex_point.clone()), (65536, vec![65000, 65535]));
}

/// Houdini install files (no twin): record the outcome, never panic. Skipped when Houdini is not installed.
#[test]
fn installed_legacy_files_do_not_panic() {
    let base = std::path::Path::new("/Applications/Houdini");
    if !base.exists() {
        return;
    }
    let mut stack = vec![base.to_path_buf()];
    let mut seen = std::collections::BTreeMap::new();
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "bgeo") && e.metadata().is_ok_and(|m| m.len() < 2_000_000) {
                let Ok(bytes) = std::fs::read(&p) else { continue };
                if bytes.starts_with(b"BgeoV") {
                    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    let r = read_geo(&bytes).map(|d| (d.point_count, d.vertex_count(), d.prim_count()));
                    seen.insert(name, format!("{r:?}"));
                }
            }
        }
    }
    for (n, r) in &seen {
        eprintln!("legacy {n}: {r}");
    }
}

/// H39: Houdini itself (hython `oracle/houdini/bgeo-legacy-counts.py`) loaded every legacy install file -> `fixtures/bgeo-legacy/counts.json`.
/// Reader counts + P bbox must equal Houdini's. No-op per file when the Houdini install is absent.
#[test]
fn installed_legacy_files_match_houdini_counts() {
    let want: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("bgeo-legacy/counts.json")).expect("counts.json"))
            .expect("json");
    let base = std::path::Path::new("/Applications/Houdini/Houdini20.5.487");
    let mut stack = vec![base.to_path_buf()];
    let (mut checked, mut failed) = (0, Vec::new());
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_symlink() {
            } else if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "bgeo") {
                let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let Some(w) = want.get(&name) else { continue };
                let Ok(bytes) = std::fs::read(&p) else { continue };
                if !bytes.starts_with(b"BgeoV") {
                    continue;
                }
                let d = match read_geo(&bytes) {
                    Ok(d) => d,
                    Err(e) => {
                        failed.push(format!("{name}: {e}"));
                        continue;
                    }
                };
                let n = |k: &str| w[k].as_u64().unwrap_or(u64::MAX);
                let got = (u64::from(d.point_count), u64::from(d.prim_count()), u64::from(d.vertex_count()));
                assert_eq!(got, (n("points"), n("prims"), n("vertices")), "{name}: points/prims/vertices vs Houdini");
                let (lo, hi) = d.bbox();
                let bb = w["bbox"].as_array().expect("bbox");
                // Houdini's bbox includes quadric extents (sphere/tube/circle/metaball); the point bbox only matches without them
                let quadric = w["prim_types"].as_array().is_some_and(|t| {
                    t.iter().any(|x| matches!(x.as_str(), Some("Sphere" | "Tube" | "Circle" | "Metaball")))
                });
                for (i, v) in lo.iter().chain(hi.iter()).enumerate().filter(|_| !quadric) {
                    let x = bb[i].as_f64().expect("f");
                    assert!((f64::from(*v) - x).abs() < 1e-3, "{name}: bbox[{i}] {v} vs {x}");
                }
                checked += 1;
            }
        }
    }
    eprintln!("legacy files == Houdini counts: {checked}; failures {failed:?}");
    assert!(failed.is_empty(), "{failed:?}");
}
