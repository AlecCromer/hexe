//! Geometry spreadsheet (Houdini-style): one window of rows of the last cooked `Detail`, per attribute class.
//! Columns = element number + (class specifics) + every attribute of that class (`P` first; tuples split into
//! `name.x .y .z .w` / `name[i]`). Rows are windowed (`offset`, `limit`) so the UI can virtualise; at most
//! [`MAX_ROWS`] rows are ever addressable (`shown` of `total`).
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, PrimKind};
use serde::Serialize;
use serde_json::Value;

/// Rows the sheet exposes at most (the UI says `showing N of M` beyond this).
pub const MAX_ROWS: usize = 10_000;
/// Rows returned by one call at most.
pub const MAX_PAGE: usize = 1_000;

#[derive(Serialize, Debug, Default, PartialEq)]
pub struct Counts {
    pub point: usize,
    pub vertex: usize,
    pub prim: usize,
    pub detail: usize,
}

#[derive(Serialize, Debug, Default, PartialEq)]
pub struct Sheet {
    pub class: String,
    pub columns: Vec<String>,
    /// Rows `offset..offset+rows.len()` of the class; cell 0 is the element number.
    pub rows: Vec<Vec<Value>>,
    pub offset: usize,
    /// Elements of the class in the geometry.
    pub total: usize,
    /// Elements addressable (`min(total, MAX_ROWS)`).
    pub shown: usize,
    pub counts: Counts,
}

pub fn parse_class(s: &str) -> Result<AttrClass, String> {
    match s {
        "point" => Ok(AttrClass::Point),
        "vertex" => Ok(AttrClass::Vertex),
        "prim" => Ok(AttrClass::Prim),
        "detail" => Ok(AttrClass::Detail),
        other => Err(format!("unknown attribute class `{other}` (point | vertex | prim | detail)")),
    }
}

fn class_name(c: AttrClass) -> &'static str {
    match c {
        AttrClass::Point => "point",
        AttrClass::Vertex => "vertex",
        AttrClass::Prim => "prim",
        AttrClass::Detail => "detail",
    }
}

fn f(x: f32) -> Value {
    if !x.is_finite() {
        return Value::String(
            if x.is_nan() {
                "nan"
            } else if x > 0.0 {
                "inf"
            } else {
                "-inf"
            }
            .into(),
        );
    }
    format!("{x}").parse::<f64>().ok().and_then(serde_json::Number::from_f64).map_or(Value::Null, Value::Number)
}

fn tuple_labels(name: &str, n: u32) -> Vec<String> {
    match n {
        0 | 1 => vec![name.to_string()],
        2..=4 => ["x", "y", "z", "w"].iter().take(n as usize).map(|c| format!("{name}.{c}")).collect(),
        _ => (0..n).map(|i| format!("{name}[{i}]")).collect(),
    }
}

/// The cells of element `i` of `a` (one per tuple component; missing data -> empty strings, never a panic).
fn cells(a: &Attribute, i: usize) -> Vec<Value> {
    let n = a.data.tuple_size().max(1) as usize;
    let miss = || vec![Value::String(String::new()); n];
    match &a.data {
        AttrData::F32(v) => v.get(i).map_or_else(miss, |x| vec![f(*x)]),
        AttrData::I32(v) => v.get(i).map_or_else(miss, |x| vec![Value::from(*x)]),
        AttrData::V2(v) => v.get(i).map_or_else(miss, |x| x.iter().map(|c| f(*c)).collect()),
        AttrData::V3(v) => v.get(i).map_or_else(miss, |x| x.iter().map(|c| f(*c)).collect()),
        AttrData::V4(v) => v.get(i).map_or_else(miss, |x| x.iter().map(|c| f(*c)).collect()),
        AttrData::Str { idx, table } => idx.get(i).map_or_else(miss, |k| {
            let s = usize::try_from(*k).ok().and_then(|k| table.get(k)).cloned().unwrap_or_default();
            vec![Value::String(s)]
        }),
        AttrData::FN { data, .. } => {
            data.get(i * n..(i + 1) * n).map_or_else(miss, |x| x.iter().map(|c| f(*c)).collect())
        }
        AttrData::IN { data, .. } => {
            data.get(i * n..(i + 1) * n).map_or_else(miss, |x| x.iter().map(|c| Value::from(*c)).collect())
        }
    }
}

fn prim_type(k: &PrimKind) -> String {
    match k {
        PrimKind::Poly { closed: true } => "poly".into(),
        PrimKind::Poly { closed: false } => "poly (open)".into(),
        PrimKind::Polyline => "polyline".into(),
        PrimKind::Sphere { .. } => "sphere".into(),
        PrimKind::Tube { .. } => "tube".into(),
        PrimKind::Volume => "volume".into(),
        PrimKind::PackedRef => "packed".into(),
        PrimKind::Unknown { ty, .. } => ty.clone(),
    }
}

pub fn counts(d: &Detail) -> Counts {
    Counts { point: d.point_count as usize, vertex: d.vertex_point.len(), prim: d.prims.len(), detail: 1 }
}

/// Rows `offset..offset+limit` of `class` (limit capped at [`MAX_PAGE`], rows at [`MAX_ROWS`]).
pub fn sheet(d: &Detail, class: AttrClass, offset: usize, limit: usize) -> Sheet {
    let total = d.class_len(class);
    let shown = total.min(MAX_ROWS);
    let mut attrs: Vec<&Attribute> = d.attrs.iter().filter(|a| a.class == class).collect();
    attrs.sort_by_key(|a| a.name != "P"); // stable: P first, the rest in storage order
    let mut columns: Vec<String> = match class {
        AttrClass::Point => vec!["ptnum".into()],
        AttrClass::Vertex => vec!["vtxnum".into(), "point".into()],
        AttrClass::Prim => vec!["primnum".into(), "type".into(), "verts".into()],
        AttrClass::Detail => vec!["detail".into()],
    };
    for a in &attrs {
        columns.extend(tuple_labels(&a.name, a.data.tuple_size()));
    }
    let start = offset.min(shown);
    let end = start.saturating_add(limit.min(MAX_PAGE)).min(shown);
    let rows = (start..end)
        .map(|i| {
            let mut row: Vec<Value> = vec![Value::from(i)];
            match class {
                AttrClass::Vertex => row.push(d.vertex_point.get(i).map_or(Value::Null, |p| Value::from(*p))),
                AttrClass::Prim => {
                    let p = d.prims.get(i);
                    row.push(p.map_or(Value::Null, |p| Value::String(prim_type(&p.kind))));
                    row.push(p.map_or(Value::Null, |p| Value::from(p.verts.len())));
                }
                _ => {}
            }
            for a in &attrs {
                row.extend(cells(a, i));
            }
            row
        })
        .collect();
    Sheet { class: class_name(class).into(), columns, rows, offset: start, total, shown, counts: counts(d) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hexe_geo::TypeInfo;
    use serde_json::json;

    fn tri() -> Detail {
        let mut d = Detail::default();
        let ids = d.add_points(3);
        let pts: Vec<u32> = ids.collect();
        d.p_mut().copy_from_slice(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.5]]);
        d.add_poly(&pts, true);
        d.add_attr(Attribute {
            name: "name".into(),
            class: AttrClass::Point,
            data: AttrData::Str { idx: vec![0, 1, -1], table: vec!["a".into(), "b".into()] },
            type_info: TypeInfo::None,
        });
        d.add_attr(Attribute {
            name: "Cd".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[1.0, 0.0, 0.0]; 3]),
            type_info: TypeInfo::Color,
        });
        d
    }

    #[test]
    fn point_sheet_has_ptnum_p_and_every_attr() {
        let s = sheet(&tri(), AttrClass::Point, 0, 100);
        assert_eq!(s.columns, ["ptnum", "P.x", "P.y", "P.z", "name", "Cd.x", "Cd.y", "Cd.z"]);
        assert_eq!((s.total, s.shown, s.rows.len()), (3, 3, 3));
        assert_eq!(
            s.rows[2],
            vec![json!(2), json!(0.0), json!(1.0), json!(0.5), json!(""), json!(1.0), json!(0.0), json!(0.0)]
        );
        assert_eq!(s.rows[1][4], json!("b"));
        assert!(s.rows.iter().all(|r| r.len() == s.columns.len()));
    }

    #[test]
    fn prim_vertex_detail_classes() {
        let d = tri();
        let p = sheet(&d, AttrClass::Prim, 0, 10);
        assert_eq!(p.columns, ["primnum", "type", "verts"]);
        assert_eq!(p.rows, vec![vec![json!(0), json!("poly"), json!(3)]]);
        let v = sheet(&d, AttrClass::Vertex, 1, 1);
        assert_eq!(
            (v.columns.clone(), v.rows.clone()),
            (vec!["vtxnum".to_string(), "point".to_string()], vec![vec![json!(1), json!(1)]])
        );
        let g = sheet(&d, AttrClass::Detail, 0, 10);
        assert_eq!((g.total, g.rows.len(), g.counts), (1, 1, Counts { point: 3, vertex: 3, prim: 1, detail: 1 }));
    }

    #[test]
    fn windowing_and_cap() {
        let mut d = Detail::default();
        d.add_points(12_345);
        let s = sheet(&d, AttrClass::Point, 9_990, 50);
        assert_eq!((s.total, s.shown, s.offset, s.rows.len()), (12_345, 10_000, 9_990, 10), "rows stop at the 10k cap");
        assert_eq!(s.rows.first().map(|r| r[0].clone()), Some(json!(9_990)));
        let past = sheet(&d, AttrClass::Point, 99_999, 50);
        assert!(past.rows.is_empty() && past.offset == 10_000);
        assert_eq!(sheet(&d, AttrClass::Point, 0, usize::MAX).rows.len(), MAX_PAGE, "page capped");
    }

    #[test]
    fn hostile_cells_do_not_panic() {
        let mut d = tri();
        d.attrs.push(Attribute {
            name: "bad".into(),
            class: AttrClass::Point,
            data: AttrData::F32(vec![f32::NAN]), // shorter than the class
            type_info: TypeInfo::None,
        });
        let s = sheet(&d, AttrClass::Point, 0, 10);
        assert_eq!(s.rows[0].last(), Some(&json!("nan")));
        assert_eq!(s.rows[2].last(), Some(&json!("")));
        assert!(parse_class("edge").is_err());
    }
}
