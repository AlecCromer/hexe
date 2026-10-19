use crate::*;
use std::path::PathBuf;

fn all_geo() -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/gpd-all/expected.geo");
    std::fs::read(p).expect("fixture fixtures/gpd-all/expected.geo")
}

fn attr_sig(d: &Detail) -> Vec<(String, AttrClass, u32, usize)> {
    d.attrs.iter().map(|a| (a.name.clone(), a.class, a.data.tuple_size(), a.data.len())).collect()
}

#[test]
fn gpd_all_counts_match_header() {
    let d = read_geo(&all_geo()).unwrap();
    // header of Houdini's all.geo: NPoints 367 NPrims 13 NPointGroups 1 NPrimGroups 1 NPointAttrib 4
    assert_eq!(d.point_count, 367);
    assert_eq!(d.prims.len(), 13);
    assert_eq!(d.p().len(), 367);
    assert_eq!(d.groups.iter().filter(|g| g.class == AttrClass::Point).count(), 1);
    assert_eq!(d.groups.iter().filter(|g| g.class == AttrClass::Prim).count(), 1);
    let names: Vec<&str> = d.attrs.iter().map(|a| a.name.as_str()).collect();
    // `Pw` = the file's per-point homogeneous w (real rational weights used by its NURBS prims), exposed as an attribute
    assert_eq!(names, ["P", "Cd", "Alpha", "uv", "N", "Pw"]);
    assert!(d.attrs.iter().all(|a| a.data.len() == 367));
    let pg = d.group(AttrClass::Point, "pointGroup").unwrap();
    assert_eq!(pg.mask.len(), 367);
    assert_eq!(pg.mask.iter().filter(|&&b| b).count(), 13);
    let qg = d.group(AttrClass::Prim, "primGroup").unwrap();
    assert_eq!(qg.mask.iter().filter(|&&b| b).count(), 3);
    // spot value: point 5 = (1 0 0), Cd = (0.833333 0 0)
    assert_eq!(d.p()[5], [1.0, 0.0, 0.0]);
    assert!(
        matches!(d.attr(AttrClass::Point, "Cd").unwrap().data, AttrData::V3(ref v) if (v[5][0] - 0.833333).abs() < 1e-6)
    );
    // prim types seen: Circle Sphere Tube MetaBall MetaSQuad Poly Mesh NURBMesh BezierMesh NURBMesh 3xBezierCurve (Run)
    let kinds: Vec<String> = d
        .prims
        .iter()
        .map(|p| match &p.kind {
            PrimKind::Poly { .. } => "Poly".into(),
            PrimKind::Sphere { .. } => "Sphere".into(),
            PrimKind::Tube { .. } => "Tube".into(),
            PrimKind::Unknown { ty, .. } => ty.clone(),
            PrimKind::BezierCurve { .. } => "BezierCurve".into(),
            PrimKind::NurbsCurve { .. } => "NURBCurve".into(),
            PrimKind::BezierSurf { .. } => "BezierMesh".into(),
            PrimKind::NurbsSurf { .. } => "NURBMesh".into(),
            other => format!("{other:?}"),
        })
        .collect();
    eprintln!("ALLGEO pts={} verts={} prims={} kinds={kinds:?}", d.point_count, d.vertex_point.len(), d.prims.len());
    assert_eq!(
        kinds,
        [
            "Circle",
            "Sphere",
            "Tube",
            "MetaBall",
            "MetaSQuad",
            "Poly",
            "Mesh",
            "NURBMesh",
            "BezierMesh",
            "BezierCurve",
            "BezierCurve",
            "BezierCurve",
            "NURBMesh"
        ]
    );
    // vertex_point total = 1+1+1+1+1+10+100+64+100+57+15+84+20
    assert_eq!(d.vertex_point.len(), 1 + 1 + 1 + 1 + 1 + 10 + 100 + 64 + 100 + 57 + 15 + 84 + 20);
}

#[test]
fn json_roundtrip_gpd_all() {
    let d = read_geo(&all_geo()).unwrap();
    let bytes = write_geo_json(&d);
    let back = read_geo_json(&bytes).unwrap();
    // counts, attr names/classes/sizes, prim verts
    assert_eq!(back.point_count, d.point_count);
    assert_eq!(attr_sig(&back), attr_sig(&d));
    assert_eq!(back.vertex_point, d.vertex_point);
    assert_eq!(
        back.prims.iter().map(|p| p.verts.clone()).collect::<Vec<_>>(),
        d.prims.iter().map(|p| p.verts.clone()).collect::<Vec<_>>()
    );
    for (a, b) in back.p().iter().zip(d.p()) {
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-6);
        }
    }
    // and full structural equality (f32 shortest-decimal round trip is exact)
    assert_eq!(back, d);
    // second generation is byte-stable
    assert_eq!(write_geo_json(&back), bytes);
}

fn synthetic_a() -> Detail {
    let mut d = Detail::default();
    d.add_points(5);
    d.p_mut().copy_from_slice(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.5, 0.5, 1.0]]);
    d.add_poly(&[0, 1, 2], true);
    d.add_poly(&[0, 2, 3], true);
    d.add_poly(&[0, 1, 2, 3], true);
    d.add_poly(&[3, 4], false);
    d.add_attr(Attribute {
        name: "N".into(),
        class: AttrClass::Point,
        data: AttrData::V3(vec![[0.0, 0.0, 1.0]; 5]),
        type_info: TypeInfo::Normal,
    });
    d.add_attr(Attribute {
        name: "id".into(),
        class: AttrClass::Point,
        data: AttrData::I32(vec![10, 11, 12, 13, 14]),
        type_info: TypeInfo::None,
    });
    d.add_attr(Attribute {
        name: "name".into(),
        class: AttrClass::Prim,
        data: AttrData::Str { idx: vec![0, 1, 0, -1], table: vec!["a".into(), "b".into()] },
        type_info: TypeInfo::None,
    });
    d.add_attr(Attribute {
        name: "uv".into(),
        class: AttrClass::Vertex,
        data: AttrData::V2((0..12).map(|i| [i as f32 * 0.1, 0.3]).collect()),
        type_info: TypeInfo::None,
    });
    d.add_attr(Attribute {
        name: "scale".into(),
        class: AttrClass::Detail,
        data: AttrData::F32(vec![2.5]),
        type_info: TypeInfo::None,
    });
    d.add_attr(Attribute {
        name: "m".into(),
        class: AttrClass::Point,
        data: AttrData::FN { size: 6, data: (0..30).map(|i| i as f32).collect() },
        type_info: TypeInfo::None,
    });
    let mut m = vec![false; 5];
    m[1] = true;
    m[2] = true;
    d.groups.push(Group { name: "top".into(), class: AttrClass::Point, mask: m });
    d.groups.push(Group { name: "tris".into(), class: AttrClass::Prim, mask: vec![true, true, false, false] });
    d
}

fn synthetic_b() -> Detail {
    let mut d = Detail::default();
    d.add_points(4);
    d.p_mut().copy_from_slice(&[[0.0; 3], [1.0, 2.0, 3.0], [-1.0, 0.5, 1e-7], [3.4e38, -1.0e-30, 7.0]]);
    d.vertex_point = vec![0, 1, 2, 3];
    d.prims.push(Prim { kind: PrimKind::Sphere { xform: [1.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0] }, verts: 0..1 });
    d.prims.push(Prim { kind: PrimKind::Tube { xform: [1.0; 9], taper: 0.25, closed: true }, verts: 1..2 });
    d.prims.push(Prim { kind: PrimKind::Poly { closed: false }, verts: 2..4 });
    d.prims.push(Prim {
        kind: PrimKind::Unknown {
            ty: "Volume".into(),
            json: r#"[["type","Volume"],{"res":[2,2,2],"vertex":[]}]"#.into(),
        },
        verts: 0..0,
    });
    d
}

fn synthetic_c() -> Detail {
    // empty + merge of two details
    let mut d = synthetic_a();
    d.merge(&synthetic_a());
    d
}

#[test]
fn synthetic_details_roundtrip() {
    for (i, d) in [synthetic_a(), synthetic_b(), synthetic_c(), Detail::default()].iter_mut().enumerate() {
        d.canonicalize();
        let back = read_geo_json(&write_geo_json(d)).unwrap_or_else(|e| panic!("detail {i}: {e}"));
        // Detail::default has no P; reader adds an empty one
        if d.attrs.is_empty() {
            assert_eq!(back.point_count, 0);
            continue;
        }
        assert_eq!(&back, &*d, "detail {i}");
    }
}

#[test]
fn merge_offsets_everything() {
    let c = synthetic_c();
    assert_eq!(c.point_count, 10);
    assert_eq!(c.prims.len(), 8);
    assert_eq!(c.vertex_point.len(), 24);
    assert_eq!(c.prims[4].verts, 12..15);
    assert_eq!(c.vertex_point[12], 5);
    for a in &c.attrs {
        assert_eq!(
            a.data.len(),
            c.class_len(a.class).max(if a.class == AttrClass::Detail { 1 } else { 0 }),
            "{}",
            a.name
        );
    }
    let ((lo, hi), n) = (c.bbox(), c.group(AttrClass::Point, "top").unwrap().mask.len());
    assert_eq!((lo, hi), ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]));
    assert_eq!(n, 10);
}

fn houdini_shape() -> &'static str {
    // hand-authored in Houdini's alternating key/value layout; P size 4; rawpagedata with a constant page; boolRLE group
    r#"["fileversion","20.5.487","hasindex",false,"pointcount",4,"vertexcount",6,"primitivecount",2,
    "info",{"software":"x"},
    "topology",["pointref",["indices",[0,1,2,0,2,3]]],
    "attributes",["pointattributes",[
      [["scope","public","type","numeric","name","P","options",{"type":{"type":"string","value":"point"}}],
       ["size",4,"storage","fpreal32","values",["size",4,"storage","fpreal32","tuples",[[0,0,0,1],[1,0,0,1],[1,1,0,1],[0,1,0,1]]]]],
      [["scope","public","type","numeric","name","Cd","options",{"type":{"type":"string","value":"color"}}],
       ["size",3,"storage","fpreal32","values",["size",3,"storage","fpreal32","packing",[3],"pagesize",2,"constantpageflags",[[2]],"rawpagedata",[1,2,3, 9,9,9,  7,7,7,8,8,8]]]],
      [["scope","public","type","string","name","tag","options",{}],
       ["size",1,"storage","int32","strings",["x","y"],"indices",["size",1,"storage","int32","arrays",[[0,1,1,0]]]]]
    ],"primitiveattributes",[
      [["scope","public","type","numeric","name","cls","options",{}],
       ["size",1,"storage","int64","values",["size",1,"storage","int64","tuples",[[5],[6]]]]]
    ]],
    "primitives",[[["type","Polygon_run"],["startvertex",0,"nprimitives",2,"nvertices_rle",[3,2]]]],
    "pointgroups",[[["name","g"],["selection",["unordered",["boolRLE",[0,1,1,2,0,1]]]]]],
    "primitivegroups",[[["name","p"],["selection",["unordered",["boolRLE",[1,2]]]]]]
    ]"#
}

#[test]
fn json_reads_houdini_shape() {
    let d = read_geo_json(houdini_shape().as_bytes()).unwrap();
    assert_eq!((d.point_count, d.vertex_point.len(), d.prims.len()), (4, 6, 2));
    assert_eq!(d.p()[2], [1.0, 1.0, 0.0]);
    assert_eq!(d.prims[1].verts, 3..6);
    // rawpagedata: pagesize 2, page0 constant? flags word 2 = 0b10 → page 1 constant. page0 = 2 elems of 3 (non-const) → needs 6 values
    // data: [1,2,3, 9,9,9] = page0 (2 tuples), then [7,7,7] const page1 → replicated; leftover values must not be required
    let cd = d.attr(AttrClass::Point, "Cd").unwrap();
    assert_eq!(cd.type_info, TypeInfo::Color);
    assert_eq!(cd.data, AttrData::V3(vec![[1.0, 2.0, 3.0], [9.0, 9.0, 9.0], [7.0, 7.0, 7.0], [7.0, 7.0, 7.0]]));
    let tag = d.attr(AttrClass::Point, "tag").unwrap();
    assert_eq!(tag.data, AttrData::Str { idx: vec![0, 1, 1, 0], table: vec!["x".into(), "y".into()] });
    assert_eq!(d.attr(AttrClass::Prim, "cls").unwrap().data, AttrData::I32(vec![5, 6]));
    // value,count pairs [0,1,1,2,0,1] → F T T F
    assert_eq!(d.group(AttrClass::Point, "g").unwrap().mask, [false, true, true, false]);
    assert_eq!(d.group(AttrClass::Prim, "p").unwrap().mask, [true, true]);
}

#[test]
fn groups_roundtrip_and_boolrle() {
    // alternating run-length interpretation: [1,2,1] = F T T F
    let alt = houdini_shape().replace("[0,1,1,2,0,1]", "[1,2,1]");
    let d = read_geo_json(alt.as_bytes()).unwrap();
    assert_eq!(d.group(AttrClass::Point, "g").unwrap().mask, [false, true, true, false]);
    // RLE that does not cover the element count → Err
    let bad = houdini_shape().replace("[0,1,1,2,0,1]", "[0,1,1,9]");
    assert!(read_geo_json(bad.as_bytes()).is_err());
    // write→read keeps masks incl. alternating/edge shapes
    let mut d = synthetic_a();
    d.groups.push(Group { name: "none".into(), class: AttrClass::Point, mask: vec![false; 5] });
    d.groups.push(Group { name: "all".into(), class: AttrClass::Point, mask: vec![true; 5] });
    d.groups.push(Group { name: "alt".into(), class: AttrClass::Prim, mask: vec![true, false, true, false] });
    d.groups.push(Group { name: "v".into(), class: AttrClass::Vertex, mask: (0..12).map(|i| i % 3 == 0).collect() });
    d.canonicalize();
    let back = read_geo_json(&write_geo_json(&d)).unwrap();
    assert_eq!(back.groups, d.groups);
    // legacy GPD ordered/unordered groups (all.geo)
    let g = read_geo(&all_geo()).unwrap();
    assert_eq!(g.groups.len(), 2);
}

/// Houdini 20.5 ASCII writes small groups as one flag per element (`["i8", [1, 1, 0, ...]]`, seen in hython
/// `saveToFile` output); a flag list that does not match the element count is an error, never a panic.
#[test]
fn per_element_flag_groups() {
    let i8s = houdini_shape().replace(r#"["boolRLE",[0,1,1,2,0,1]]"#, r#"["i8",[0,1,1,0]]"#);
    let d = read_geo_json(i8s.as_bytes()).unwrap();
    assert_eq!(d.group(AttrClass::Point, "g").unwrap().mask, [false, true, true, false]);
    let i32s = houdini_shape().replace(r#"["boolRLE",[1,2]]"#, r#"["i32",[0,7]]"#);
    assert_eq!(read_geo_json(i32s.as_bytes()).unwrap().group(AttrClass::Prim, "p").unwrap().mask, [false, true]);
    for bad in [r#"["i8",[0,1,1]]"#, r#"["i8",[0,1,1,0,1]]"#, r#"["i8","x"]"#, r#"["i8",[0,1,1,1e999]]"#] {
        let s = houdini_shape().replace(r#"["boolRLE",[0,1,1,2,0,1]]"#, bad);
        assert!(read_geo_json(s.as_bytes()).is_err(), "{bad}");
    }
}

#[test]
fn hostile_input_is_err_never_panic() {
    let good = write_geo_json(&synthetic_a());
    // every truncation of a valid file: no panic; a strict prefix must never decode to the full thing
    for cut in (0..good.len()).step_by(7) {
        assert!(read_geo_json(&good[..cut]).is_err(), "prefix {cut} accepted");
    }
    let gpd = all_geo();
    for cut in (0..gpd.len()).step_by(97) {
        let _ = read_geo(&gpd[..cut]); // must not panic; truncation inside trailing groups may still decode
    }
    assert!(read_geo(&gpd[..gpd.len() / 2]).is_err());
    let s = std::str::from_utf8(&good).unwrap();
    // pointref index >= pointcount
    let bad = s.replacen("\"indices\",[0,1,2", "\"indices\",[0,1,99", 1);
    assert_ne!(bad, s);
    assert!(matches!(read_geo_json(bad.as_bytes()), Err(GeoError::Index(_))));
    // negative counts
    for key in ["pointcount", "vertexcount", "primitivecount"] {
        let re = format!("\"{key}\",");
        let i = s.find(&re).unwrap() + re.len();
        let j = i + s[i..].find(',').unwrap();
        let neg = format!("{}-5{}", &s[..i], &s[j..]);
        assert!(matches!(read_geo_json(neg.as_bytes()), Err(GeoError::Count(_))), "{key}");
    }
    // absurd counts / mismatching counts
    let huge = s.replacen("\"pointcount\",5", "\"pointcount\",4000000000", 1);
    assert!(read_geo_json(huge.as_bytes()).is_err());
    let mism = s.replacen("\"primitivecount\",4", "\"primitivecount\",5", 1);
    assert!(read_geo_json(mism.as_bytes()).is_err());
    // string index outside table, rawpagedata amplification, deep nesting, garbage
    let sidx = houdini_shape().replace("[[0,1,1,0]]", "[[0,1,7,0]]");
    assert!(matches!(read_geo_json(sidx.as_bytes()), Err(GeoError::Index(_))));
    let amp = houdini_shape().replace("\"pointcount\",4", "\"pointcount\",134217728");
    assert!(read_geo_json(amp.as_bytes()).is_err());
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(read_geo_json(deep.as_bytes()).is_err());
    for junk in [
        &b""[..],
        b"null",
        b"[1,2,3]",
        b"{}",
        b"PGEOMETRY",
        b"PGEOMETRY V2 NPoints 99999999 NPrims 0",
        b"\xff\xfe",
        b"BgeoV",
    ] {
        assert!(read_geo(junk).is_err());
    }
    // GPD: negative counts, vertex referencing a missing point, bogus group bits
    let g = String::from_utf8(gpd.clone()).unwrap();
    assert!(read_geo(g.replacen("NPoints 367", "NPoints -3", 1).as_bytes()).is_err());
    assert!(read_geo(g.replacen("Poly 10 < 5 6", "Poly 10 < 5 9999", 1).as_bytes()).is_err());
    assert!(read_geo(g.replacen("367 0000", "367 0x00", 1).as_bytes()).is_err());
}

#[test]
fn unknown_prim_types_pass_through() {
    let s = houdini_shape().replace(
        r#"[["type","Polygon_run"],["startvertex",0,"nprimitives",2,"nvertices_rle",[3,2]]]"#,
        r#"[["type","Polygon_run"],["startvertex",0,"nprimitives",1,"nvertices_rle",[3,1]]],[["type","PackedGeometry"],["vertex",[3],"funky",{"a":[1,2]}]]"#,
    );
    let d = read_geo_json(s.as_bytes()).unwrap();
    assert!(matches!(&d.prims[1].kind, PrimKind::Unknown { ty, .. } if ty == "PackedGeometry"));
    assert_eq!(d.prims[1].verts, 3..4);
    assert_eq!(read_geo_json(&write_geo_json(&d)).unwrap(), d);
}

#[test]
fn bgeo_binary_reads() {
    // legacy BgeoV (H13 reader): repo byte copy of Houdini GPD/all.bgeo
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/houdini-ship/gpd-all-legacy/all.bgeo");
    let d = read_geo(&std::fs::read(p).unwrap()).unwrap();
    assert_eq!((d.point_count, d.vertex_count(), d.prim_count()), (367, 455, 13));
    // binary-JSON (bjson): shipped axis.bgeo
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/bgeo-bjson/axis.bgeo");
    assert_eq!(read_geo(&std::fs::read(p).unwrap()).unwrap().point_count, 7);
}

/// promote-1: Houdini 20.5 ASCII writes a plain per-prim `nvertices` list (not `nvertices_rle`) when counts do not run-length
/// encode (real hython clip output, mixed 3/5-gons).
#[test]
fn polygon_run_accepts_plain_nvertices_list() {
    let d = read_geo_json(include_bytes!("../../../fixtures/sop-clip-tilt/houdini.geo")).unwrap();
    let counts: Vec<usize> = d.prims.iter().map(|p| p.verts.len()).collect();
    assert_eq!((d.point_count, counts), (10, vec![3, 5, 5, 3, 3, 5]));
}

#[test]
fn edge_groups_read_write_merge() {
    // Houdini 20.5's own `edgegroups` layout (saved by hython: groupcreate edge group `p2-3 p5-6` on a sphere)
    let text = houdini_shape().replace(
        r#""primitivegroups",[[["name","p"],["selection",["unordered",["boolRLE",[1,2]]]]]]"#,
        r#""primitivegroups",[[["name","p"],["selection",["unordered",["boolRLE",[1,2]]]]]],
    "edgegroups",[[["name","eg"],["points",[0,1,2,3,1,0]]]]"#,
    );
    let d = read_geo_json(text.as_bytes()).unwrap();
    assert_eq!(d.edge_group("eg").unwrap().edges, vec![[0, 1], [2, 3]], "pairs; the reversed duplicate 1-0 kept once");
    // round trip through the writer
    let back = read_geo_json(&write_geo_json(&d)).unwrap();
    assert_eq!(back.edge_groups, d.edge_groups);
    // merge: same-named groups join, the second input's points shift
    let mut m = d.clone();
    m.merge(&d);
    assert_eq!(m.edge_group("eg").unwrap().edges, vec![[0, 1], [2, 3], [4, 5], [6, 7]]);
    // hostile: odd point list / point out of range / no name
    for bad in
        [r#"[[["name","e"],["points",[0,1,2]]]]"#, r#"[[["name","e"],["points",[0,99]]]]"#, r#"[[["points",[0,1]]]]"#]
    {
        let t = houdini_shape().replace(
            r#""primitivegroups",[[["name","p"],["selection",["unordered",["boolRLE",[1,2]]]]]]"#,
            &format!(
                r#""primitivegroups",[[["name","p"],["selection",["unordered",["boolRLE",[1,2]]]]]],"edgegroups",{bad}"#
            ),
        );
        assert!(read_geo_json(t.as_bytes()).is_err(), "{bad}");
    }
    // set_edge_group drops self-edges and out-of-range points
    let mut s = d.clone();
    s.set_edge_group("x", [[0, 0], [0, 9], [3, 2], [2, 3]]);
    assert_eq!(s.edge_group("x").unwrap().edges, vec![[3, 2]]);
}
