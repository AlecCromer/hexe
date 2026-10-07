//! H12 synthetic regressions (written BEFORE the fixes).
//! A: `run` primitives with `runtype Poly` carry a vertex LIST per item (pig.bgeo.sc shape), not one packed-prim vertex.
//! B: `dict` / `stringarray` point attributes have no `AttrData` form → recorded in `Detail::skipped_attribs`, never silent.
use crate::*;

fn geo(prims: &str, attrs: &str) -> String {
    format!(
        r#"{{"pointcount":8,"vertexcount":8,"primitivecount":2,
        "topology":{{"pointref":{{"indices":[0,1,2,3,4,5,6,7]}}}},
        "attributes":{{"pointattributes":[
          [{{"name":"P","type":"numeric"}},{{"values":{{"size":3,"storage":"fpreal32","tuples":[[0,0,0],[1,0,0],[1,1,0],[0,1,0],[0,0,1],[1,0,1],[1,1,1],[0,1,1]]}}}}]{attrs}]}},
        "primitives":{prims}}}"#
    )
}

#[test]
fn poly_run_with_vertex_lists_reads() {
    // exact header/item shape of pig.bgeo.sc: varyingfields ["vertex"], item = [[v0, v1, ...]]
    let prims = r#"[[["type","run","runtype","Poly","varyingfields",["vertex"],"uniformfields",{"closed":true}],
        [[[0,1,2,3]],[[4,5,6,7]]]]]"#;
    let d = read_geo_json(geo(prims, "").as_bytes()).expect("Poly run must parse");
    assert_eq!(d.prims.len(), 2);
    assert_eq!(d.prims[0].kind, PrimKind::Poly { closed: true });
    assert_eq!(d.prims[0].verts, 0..4);
    assert_eq!(d.prims[1].verts, 4..8);
}

#[test]
fn poly_run_open_and_triangles() {
    let prims = r#"[[["type","run","runtype","Poly","varyingfields",["vertex"],"uniformfields",{"closed":false}],
        [[[0,1,2]],[[3,4,5,6,7]]]]]"#;
    let d = read_geo_json(geo(prims, "").as_bytes()).unwrap();
    assert_eq!(d.prims[0].kind, PrimKind::Poly { closed: false });
    assert_eq!((d.prims[0].verts.clone(), d.prims[1].verts.clone()), (0..3, 3..8));
}

#[test]
fn poly_run_out_of_range_vertex_is_error() {
    let prims = r#"[[["type","run","runtype","Poly","varyingfields",["vertex"],"uniformfields",{"closed":true}],
        [[[0,1,2,3]],[[4,5,6,99]]]]]"#;
    assert!(matches!(read_geo_json(geo(prims, "").as_bytes()), Err(GeoError::Index(_))));
}

#[test]
fn packed_run_with_scalar_vertex_still_reads() {
    let prims = r#"[[["type","run","runtype","PackedGeometry","varyingfields",["vertex"],"uniformfields",{}],
        [[0],[1]]]]"#;
    let d = read_geo_json(geo(prims, "").as_bytes()).unwrap();
    assert_eq!(d.prims.len(), 2);
    assert_eq!(d.prims[1].verts, 1..2);
}

#[test]
fn run_item_without_vertex_is_error_not_minus_one() {
    let prims = r#"[[["type","run","runtype","PackedGeometry","varyingfields",["vertex"],"uniformfields",{}],
        [["x"],[1]]]]"#;
    let e = read_geo_json(geo(prims, "").as_bytes()).unwrap_err();
    assert!(matches!(e, GeoError::Syntax(_)), "{e:?}");
}

#[test]
fn dict_and_stringarray_attribs_are_recorded_as_skipped() {
    let attrs = r#",
      [{"name":"parms","type":"dict","options":{}},{"size":1,"storage":"int32","dicts":[],"indices":{"size":1,"storage":"int32","rawpagedata":[-1,-1,-1,-1,-1,-1,-1,-1]}}],
      [{"name":"tags","type":"stringarray","options":{}},{"storage":"int32","size":1,"strings":[],"indices":[[],[],[],[],[],[],[],[]]}],
      [{"name":"name","type":"string","options":{}},{"size":1,"storage":"int32","strings":["a","b"],"indices":{"size":1,"storage":"int32","rawpagedata":[0,1,0,1,0,1,0,1]}}]"#;
    let prims = r#"[[["type","run","runtype","Poly","varyingfields",["vertex"],"uniformfields",{"closed":true}],
        [[[0,1,2,3]],[[4,5,6,7]]]]]"#;
    let d = read_geo_json(geo(prims, attrs).as_bytes()).unwrap();
    let kept: Vec<&str> = d.attrs.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(kept, ["P", "name"]);
    assert_eq!(
        d.skipped_attribs,
        [
            (AttrClass::Point, "parms".to_string(), "dict".to_string()),
            (AttrClass::Point, "tags".to_string(), "stringarray".to_string()),
        ]
    );
}
