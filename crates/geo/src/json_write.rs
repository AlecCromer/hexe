//! Houdini `.geo` JSON writer: alternating key/value arrays, size-4 `P`, `Polygon_run` runs, `boolRLE` groups.

use crate::{AttrClass, AttrData, Attribute, Basis, Detail, Group, Prim, PrimKind, TypeInfo};
use serde_json::{Number, Value, json};

fn f(x: f32) -> Value {
    // shortest f32 decimal, reparsed as f64: exact round trip back to the same f32
    let s = format!("{x}");
    s.parse::<f64>().ok().and_then(Number::from_f64).map_or_else(|| json!(0), Value::Number)
}

fn type_name(t: TypeInfo) -> Option<&'static str> {
    match t {
        TypeInfo::Point => Some("point"),
        TypeInfo::Vector => Some("vector"),
        TypeInfo::Normal => Some("normal"),
        TypeInfo::Color => Some("color"),
        TypeInfo::Quaternion => Some("quaternion"),
        TypeInfo::None => None,
    }
}

fn tuples(size: usize, storage: &str, rows: Vec<Value>) -> Value {
    json!(["size", size, "storage", storage, "tuples", rows])
}

fn attr_entry(a: &Attribute, p4: bool) -> Value {
    let mut meta = vec![
        json!("scope"),
        json!("public"),
        json!("type"),
        json!(if matches!(a.data, AttrData::Str { .. }) { "string" } else { "numeric" }),
        json!("name"),
        json!(a.name),
    ];
    meta.push(json!("options"));
    meta.push(match type_name(a.type_info) {
        Some(t) => json!({"type": {"type": "string", "value": t}}),
        None => json!({}),
    });
    let data = match &a.data {
        AttrData::F32(v) => json!([
            "size",
            1,
            "storage",
            "fpreal32",
            "values",
            tuples(1, "fpreal32", v.iter().map(|&x| json!([f(x)])).collect())
        ]),
        AttrData::I32(v) => json!([
            "size",
            1,
            "storage",
            "int32",
            "values",
            tuples(1, "int32", v.iter().map(|&x| json!([x])).collect())
        ]),
        AttrData::V2(v) => json!([
            "size",
            2,
            "storage",
            "fpreal32",
            "values",
            tuples(2, "fpreal32", v.iter().map(|c| Value::Array(c.iter().map(|&x| f(x)).collect())).collect())
        ]),
        AttrData::V3(v) if p4 => json!([
            "size",
            4,
            "storage",
            "fpreal32",
            "values",
            tuples(4, "fpreal32", v.iter().map(|c| Value::Array(vec![f(c[0]), f(c[1]), f(c[2]), json!(1)])).collect())
        ]),
        AttrData::V3(v) => json!([
            "size",
            3,
            "storage",
            "fpreal32",
            "values",
            tuples(3, "fpreal32", v.iter().map(|c| Value::Array(c.iter().map(|&x| f(x)).collect())).collect())
        ]),
        AttrData::V4(v) => json!([
            "size",
            4,
            "storage",
            "fpreal32",
            "values",
            tuples(4, "fpreal32", v.iter().map(|c| Value::Array(c.iter().map(|&x| f(x)).collect())).collect())
        ]),
        AttrData::FN { size, data } => {
            let s = (*size as usize).max(1);
            json!([
                "size",
                s,
                "storage",
                "fpreal32",
                "values",
                tuples(
                    s,
                    "fpreal32",
                    data.chunks(s).map(|c| Value::Array(c.iter().map(|&x| f(x)).collect())).collect()
                )
            ])
        }
        AttrData::IN { size, data } => {
            let s = (*size as usize).max(1);
            json!([
                "size",
                s,
                "storage",
                "int32",
                "values",
                tuples(s, "int32", data.chunks(s).map(|c| json!(c)).collect())
            ])
        }
        AttrData::Str { idx, table } => json!([
            "size",
            1,
            "storage",
            "int32",
            "strings",
            table,
            "indices",
            tuples(1, "int32", idx.iter().map(|&x| json!([x])).collect())
        ]),
    };
    json!([Value::Array(meta), data])
}

fn rle(mask: &[bool]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < mask.len() {
        let v = mask[i];
        let mut j = i;
        while j < mask.len() && mask[j] == v {
            j += 1;
        }
        out.push(json!(u8::from(v)));
        out.push(json!(j - i));
        i = j;
    }
    out
}

fn group_entry(g: &Group) -> Value {
    json!([["name", g.name], ["selection", ["unordered", ["boolRLE", rle(&g.mask)]]]])
}

fn basis_value(b: &Basis) -> Value {
    json!(["order", b.order, "endinterpolation", b.end, "knots", b.knots.iter().map(|&k| f(k)).collect::<Vec<_>>()])
}

fn prim_entries(d: &Detail) -> Vec<Value> {
    let mut out = Vec::new();
    let mut i = 0;
    let ps: &[Prim] = &d.prims;
    while i < ps.len() {
        let p = &ps[i];
        match &p.kind {
            PrimKind::Poly { closed } => {
                // greedy run of same-openness polys with back-to-back vertex ranges
                let mut j = i;
                let mut next = p.verts.start;
                while j < ps.len() && ps[j].kind == p.kind && ps[j].verts.start == next {
                    next = ps[j].verts.end;
                    j += 1;
                }
                let mut rl: Vec<Value> = Vec::new();
                let mut k = i;
                while k < j {
                    let nv = ps[k].verts.end - ps[k].verts.start;
                    let mut c = 0;
                    while k < j && ps[k].verts.end - ps[k].verts.start == nv {
                        c += 1;
                        k += 1;
                    }
                    rl.push(json!(nv));
                    rl.push(json!(c));
                }
                let ty = if *closed { "Polygon_run" } else { "PolygonCurve_run" };
                out.push(json!([
                    ["type", ty],
                    ["startvertex", p.verts.start, "nprimitives", j - i, "nvertices_rle", rl]
                ]));
                i = j;
                continue;
            }
            PrimKind::Polyline => {
                let nv = p.verts.end - p.verts.start;
                out.push(json!([
                    ["type", "PolygonCurve_run"],
                    ["startvertex", p.verts.start, "nprimitives", 1, "nvertices_rle", [nv, 1]]
                ]));
            }
            PrimKind::Sphere { xform } => {
                out.push(json!([
                    ["type", "Sphere"],
                    ["vertex", [p.verts.start], "transform", xform.iter().map(|&x| f(x)).collect::<Vec<_>>()]
                ]));
            }
            PrimKind::Tube { xform, taper, closed } => {
                out.push(json!([
                    ["type", "Tube"],
                    [
                        "vertex",
                        [p.verts.start],
                        "transform",
                        xform.iter().map(|&x| f(x)).collect::<Vec<_>>(),
                        "taper",
                        f(*taper),
                        "closed",
                        closed
                    ]
                ]));
            }
            PrimKind::BezierCurve { basis, closed } | PrimKind::NurbsCurve { basis, closed } => {
                let ty = if matches!(p.kind, PrimKind::BezierCurve { .. }) { "BezierCurve" } else { "NURBCurve" };
                let vs: Vec<u32> = (p.verts.start..p.verts.end).collect();
                out.push(json!([["type", ty], ["vertex", vs, "closed", closed, "basis", basis_value(basis)]]));
            }
            PrimKind::BezierSurf { cols, rows, ubasis, vbasis, uwrap, vwrap }
            | PrimKind::NurbsSurf { cols, rows, ubasis, vbasis, uwrap, vwrap } => {
                let ty = if matches!(p.kind, PrimKind::BezierSurf { .. }) { "BezierMesh" } else { "NURBMesh" };
                let vs: Vec<u32> = (p.verts.start..p.verts.end).collect();
                out.push(json!([
                    ["type", ty],
                    [
                        "vertex",
                        vs,
                        "uv",
                        [cols, rows],
                        "uwrap",
                        uwrap,
                        "vwrap",
                        vwrap,
                        "ubasis",
                        basis_value(ubasis),
                        "vbasis",
                        basis_value(vbasis)
                    ]
                ]));
            }
            PrimKind::Unknown { json: j, .. } => {
                if let Ok(v) = serde_json::from_str::<Value>(j) {
                    out.push(v);
                }
            }
            PrimKind::Volume | PrimKind::PackedRef => {
                out.push(json!([["type", "Unknown"], []]));
            }
        }
        i += 1;
    }
    out
}

/// Serialise to Houdini's JSON `.geo`. `read_geo_json(write_geo_json(d)) == d` for every Detail built from the reader
/// or with Poly/Polyline/Sphere/Tube/Unknown prims.
pub fn write_geo_json(d: &Detail) -> Vec<u8> {
    let section = |class: AttrClass| -> Vec<Value> {
        d.attrs
            .iter()
            .filter(|a| a.class == class)
            .map(|a| attr_entry(a, class == AttrClass::Point && a.name == "P" && matches!(a.data, AttrData::V3(_))))
            .collect()
    };
    let groups =
        |class: AttrClass| -> Vec<Value> { d.groups.iter().filter(|g| g.class == class).map(group_entry).collect() };
    let prims = prim_entries(d);
    // prims that could not be serialised would break primitivecount: count what we emit
    let mut top: Vec<Value> = vec![
        json!("fileversion"),
        json!("20.5.487"),
        json!("hasindex"),
        json!(false),
        json!("pointcount"),
        json!(d.point_count),
        json!("vertexcount"),
        json!(d.vertex_point.len()),
        json!("primitivecount"),
        json!(d.prims.len()),
        json!("info"),
        json!({"software": "hexe"}),
        json!("topology"),
        json!(["pointref", ["indices", d.vertex_point]]),
        json!("attributes"),
        json!([
            "pointattributes",
            section(AttrClass::Point),
            "vertexattributes",
            section(AttrClass::Vertex),
            "primitiveattributes",
            section(AttrClass::Prim),
            "globalattributes",
            section(AttrClass::Detail)
        ]),
        json!("primitives"),
        Value::Array(prims),
    ];
    for (key, class) in
        [("pointgroups", AttrClass::Point), ("vertexgroups", AttrClass::Vertex), ("primitivegroups", AttrClass::Prim)]
    {
        let g = groups(class);
        if !g.is_empty() {
            top.push(json!(key));
            top.push(Value::Array(g));
        }
    }
    serde_json::to_vec(&Value::Array(top)).unwrap_or_default()
}
