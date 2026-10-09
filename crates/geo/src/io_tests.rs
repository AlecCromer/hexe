use crate::*;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::var("HEXE_SCRATCH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/hexe-scratch"));
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name)
}
/// Unit cube: 8 points, 6 quads, 24 corners (outward, Houdini winding: clockwise seen from outside).
pub(crate) fn cube() -> Detail {
    let mut d = Detail::default();
    let r = d.add_points(8);
    let pts: Vec<[f32; 3]> = (0..8)
        .map(|i| [(i & 1) as f32 - 0.5, ((i >> 1) & 1) as f32 - 0.5, ((i >> 2) & 1) as f32 * 1.25 - 0.5])
        .collect();
    *d.p_mut() = pts;
    assert_eq!(r, 0..8);
    for q in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
        d.add_poly(&q, true);
    }
    d
}
fn counts(d: &Detail) -> (u32, u32, u32) {
    (d.point_count, d.vertex_count(), d.prim_count())
}
fn newell(d: &Detail) -> Vec<[f32; 3]> {
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
fn same_geometry(a: &Detail, b: &Detail) {
    assert_eq!(counts(a), counts(b));
    assert_eq!(a.p(), b.p(), "P byte-equal");
    assert_eq!(a.vertex_point, b.vertex_point);
    for (x, y) in newell(a).iter().zip(newell(b)) {
        assert!((0..3).all(|i| (x[i] - y[i]).abs() < 1e-6));
    }
}
#[test]
fn obj_roundtrip_cube() {
    let c = cube();
    let bytes = write_obj(&c).unwrap();
    let back = read_obj(&bytes).unwrap();
    assert_eq!(counts(&back), (8, 24, 6));
    same_geometry(&c, &back);
}
#[test]
fn ply_ascii_roundtrip_cube() {
    let c = cube();
    let back = read_ply(&write_ply(&c).unwrap()).unwrap();
    assert_eq!(counts(&back), (8, 24, 6));
    same_geometry(&c, &back);
}
/// Hand-built `binary_little_endian` twin of `cube()`.
fn cube_ply_binary(c: &Detail) -> Vec<u8> {
    let mut b = "ply\nformat binary_little_endian 1.0\nelement vertex 8\nproperty float x\nproperty float y\nproperty float z\nelement face 6\nproperty list uchar int vertex_indices\nend_header\n".to_string().into_bytes();
    for q in c.p() {
        for v in q {
            b.extend(v.to_le_bytes());
        }
    }
    for pr in &c.prims {
        b.push(pr.verts.len() as u8);
        for v in pr.verts.clone() {
            b.extend((c.vertex_point[v as usize] as i32).to_le_bytes());
        }
    }
    b
}
#[test]
fn ply_binary_roundtrip_cube_and_every_truncation_is_err() {
    let c = cube();
    let bin = cube_ply_binary(&c);
    let back = read_ply(&bin).unwrap();
    assert_eq!(counts(&back), (8, 24, 6));
    same_geometry(&c, &back);
    let body = bin.windows(10).position(|w| w == b"end_header").unwrap() + 11;
    for cut in body..bin.len() {
        assert!(read_ply(&bin[..cut]).is_err(), "cut {cut}");
    }
    for cut in 0..body {
        assert!(read_ply(&bin[..cut]).is_err(), "header cut {cut}");
    }
}
#[test]
fn ply_binary_write_formats_roundtrip() {
    use PlyFormat::*;
    let c = cube();
    for fmt in [BinaryLE, BinaryBE] {
        let bytes = write_ply_format(&c, fmt).unwrap();
        let back = read_ply(&bytes).unwrap();
        assert_eq!(counts(&back), (8, 24, 6));
        same_geometry(&c, &back);
    }
}
#[test]
fn obj_hand_fixture_quad_with_negative_indices() {
    let r = read_obj(b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf -4 -3 -2 -1\n").unwrap();
    assert_eq!(counts(&r), (4, 4, 1));
    assert_eq!(r.vertex_point, vec![0, 1, 2, 3]);
}
#[test]
fn obj_corner_forms_groups_materials_lines() {
    let t = "# c\nv 0 0 0 1\nv 1 0 0\nv 1 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvn 0 0 1\ng a b\nusemtl red\nf 1/1/1 2/2/1 3/3/1\ng c\nusemtl blue\nf 1//1 2//1 3//1\nl 1 2 3\n";
    let d = read_obj(t.as_bytes()).unwrap();
    assert_eq!(counts(&d), (3, 9, 3));
    assert_eq!(d.prims[2].kind, PrimKind::Polyline);
    let n = d.attr(AttrClass::Vertex, "N").unwrap();
    assert_eq!((n.type_info, n.data.len()), (TypeInfo::Normal, 9));
    let AttrData::V3(uv) = &d.attr(AttrClass::Vertex, "uv").unwrap().data else { panic!() };
    assert_eq!((uv[1], uv[4]), ([1.0, 0.0, 0.0], [0.0; 3]));
    assert_eq!(d.group(AttrClass::Prim, "a").unwrap().mask, vec![true, false, false]);
    assert_eq!(d.group(AttrClass::Prim, "b").unwrap().mask, vec![true, false, false]);
    assert_eq!(d.group(AttrClass::Prim, "c").unwrap().mask, vec![false, true, true]);
    let AttrData::Str { idx, table } = &d.attr(AttrClass::Prim, "shop_materialpath").unwrap().data else { panic!() };
    assert_eq!((idx.clone(), table.clone()), (vec![0, 1, 1], vec!["red".to_string(), "blue".to_string()]));
    // writer keeps normals, uvs, lines and materials
    let back = read_obj(&write_obj(&d).unwrap()).unwrap();
    assert_eq!(counts(&back), (3, 9, 3));
    assert_eq!(back.attr(AttrClass::Vertex, "N"), d.attr(AttrClass::Vertex, "N"));
    assert_eq!(back.attr(AttrClass::Vertex, "uv"), d.attr(AttrClass::Vertex, "uv"));
    assert_eq!(back.attr(AttrClass::Prim, "shop_materialpath"), d.attr(AttrClass::Prim, "shop_materialpath"));
    // groups are written and survive round-trip
    assert_eq!(back.group(AttrClass::Prim, "a"), d.group(AttrClass::Prim, "a"));
    assert_eq!(back.group(AttrClass::Prim, "b"), d.group(AttrClass::Prim, "b"));
    assert_eq!(back.group(AttrClass::Prim, "c"), d.group(AttrClass::Prim, "c"));
    assert_eq!(back.prims[2].kind, PrimKind::Polyline);
}
#[test]
fn ply_attributes_and_skips() {
    let t = "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty float nx\nproperty float ny\nproperty float nz\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nproperty float s\nproperty float t\nproperty float quality\nelement face 1\nproperty list uchar int vertex_indices\nelement edge 1\nproperty int a\nproperty int b\nend_header\n0 0 0 0 0 1 255 0 51 0 0 7\n1 0 0 0 0 1 0 255 0 1 0 7\n1 1 0 0 0 1 0 0 255 1 1 7\n3 0 1 2\n0 1\n";
    let d = read_ply(t.as_bytes()).unwrap();
    assert_eq!(counts(&d), (3, 3, 1));
    let AttrData::V3(cd) = &d.attr(AttrClass::Point, "Cd").unwrap().data else { panic!() };
    assert_eq!(cd[0], [1.0, 0.0, 0.2]);
    assert!(d.attr(AttrClass::Point, "N").is_some() && d.attr(AttrClass::Point, "uv").is_some());
    assert!(d.skipped_attribs.iter().any(|s| s.1 == "quality") && d.skipped_attribs.iter().any(|s| s.1 == "edge"));
    // written back, point attrs survive (Cd quantised to 8 bit)
    let back = read_ply(&write_ply(&d).unwrap()).unwrap();
    assert_eq!(back.attr(AttrClass::Point, "N"), d.attr(AttrClass::Point, "N"));
    assert_eq!(back.attr(AttrClass::Point, "uv"), d.attr(AttrClass::Point, "uv"));
    assert_eq!(back.attr(AttrClass::Point, "Cd"), d.attr(AttrClass::Point, "Cd"));
}
#[test]
fn hostile_ply() {
    let hdr = |fmt: &str, n: &str| {
        format!(
            "ply\nformat {fmt} 1.0\nelement vertex {n}\nproperty float x\nproperty float y\nproperty float z\nend_header\n"
        )
    };
    for fmt in ["ascii", "binary_little_endian", "binary_big_endian"] {
        let huge = hdr(fmt, "1000000000");
        assert!(matches!(read_ply(huge.as_bytes()), Err(GeoError::Count(_))), "{fmt} 1e9");
        let lie = hdr(fmt, "200000000"); // under MAX_ELEMS, far beyond the body
        assert!(matches!(read_ply(lie.as_bytes()), Err(GeoError::Count(_))), "{fmt} lie");
        assert!(read_ply(hdr(fmt, "-1").as_bytes()).is_err());
    }

    let face = |idx: &str| {
        format!(
            "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n0 1 0\n{idx}\n"
        )
    };
    assert!(matches!(read_ply(face("3 0 1 3").as_bytes()), Err(GeoError::Index(_))));
    assert!(matches!(read_ply(face("3 0 1 -1").as_bytes()), Err(GeoError::Index(_))));
    assert!(read_ply(face("2 0 1").as_bytes()).is_err());
    assert!(read_ply(face("3 0 1").as_bytes()).is_err());
    // binary list count past the data
    let mut b =
        "ply\nformat binary_little_endian 1.0\nelement face 1\nproperty list uint int vertex_indices\nend_header\n"
            .to_string()
            .into_bytes();
    b.extend(u32::MAX.to_le_bytes());
    assert!(read_ply(&b).is_err());
    for junk in [&b""[..], b"ply", b"ply\nend_header\n", b"plx\nformat ascii 1.0\nend_header\n"] {
        assert!(read_ply(junk).is_err());
    }
}
#[test]
fn hostile_obj() {
    for t in [
        "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 4\n",
        "v 0 0 0\nf 1 1 1\nf 0 1 2\n",
        "v 0 0 0\nf -9 -1 -1\n",
        "v 0 0 0\nv 1 1 1\nf 1 2\n",
        "v 0 0\n",
        "v nan 0 0\n",
        "v 0 0 0\nf 1/1/1/1 1 1\n",
        "v 0 0 0\nf 1/1 1 1\n",
        "v 0 0 0\nf a b c\n",
        "v 0 0 0\nl 1\n",
    ] {
        assert!(read_obj(t.as_bytes()).is_err(), "{t:?}");
    }
    assert!(read_obj(b"").is_ok());
    assert!(read_obj(&[0xff, 0xfe, b'v', 0, 0x80]).is_ok());
}
#[test]
fn write_refuses_non_poly_and_nonfinite() {
    let mut d = cube();
    d.p_mut()[0][0] = f32::NAN;
    assert!(write_obj(&d).is_err() && write_ply(&d).is_err());
    let mut d = cube();
    d.prims[0].kind = PrimKind::Sphere { xform: [0.0; 9] };
    assert!(matches!(write_obj(&d), Err(GeoError::Unsupported(_))));
    assert!(matches!(write_ply(&d), Err(GeoError::Unsupported(_))));
}
#[test]
fn read_write_any_dispatch() {
    let c = cube();
    for name in ["c.geo", "c.obj", "c.ply", "C.OBJ"] {
        let p = scratch(name);
        write_any(&p, &c).unwrap();
        assert_eq!(counts(&read_any(&p).unwrap()), (8, 24, 6), "{name}");
    }
    let b = scratch("c.bgeo");
    write_any(&b, &c).unwrap();
    assert!(std::fs::read(&b).unwrap().starts_with(&[0x7f, b'N', b'S', b'J', b'b']));
    assert_eq!(counts(&read_any(&b).unwrap()), (8, 24, 6));
    assert!(matches!(write_any(scratch("c.bgeo.sc"), &c), Err(GeoError::Unsupported(_))));
    assert!(matches!(write_any(scratch("c.stl"), &c), Err(GeoError::Unsupported(_))));
    assert!(matches!(read_any(scratch("c.stl")), Err(GeoError::Unsupported(_))));
    assert!(matches!(read_any(scratch("nope.obj")), Err(GeoError::Io(_))));
}
