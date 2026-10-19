//! Enumerate SOP (token `enumerate`): writes the running index of every selected element into an int or string attribute
//! (Houdini 20.5 `enumerate`).
//!
//! # Semantics (hython 20.5 probes, `oracle/houdini/enumerate/`, cases pinned in `fixtures/sop-enumerate`)
//! `grouptype` (primitive | point | vertex) is the element class AND the class of the written attribute `attribname`.
//! Selected elements (`group`, empty = all) are walked in element order (vertices: prim order, then corner order):
//! * no piece attribute: element k of the selection gets k;
//! * `usepieceattrib`, `piecemode` = elements: the index of the element among the selected elements with the same piece
//!   value (a separate counter per value);
//! * `usepieceattrib`, `piecemode` = pieces: the rank of the element's piece value among the DISTINCT piece values of the
//!   selected elements, sorted ascending: ints numerically; strings by a natural order (digit runs compare by value,
//!   `x2 < x10`; equal values: the longer run first, `a001 < a01 < a1`; other bytes as SIGNED chars, so `B < a` and UTF-8
//!   lead bytes sort before ASCII; a prefix sorts first, `"" < "a"`).
//!
//! The piece attribute must exist on the `grouptype` class (else the cook fails); a numeric one is keyed by its FIRST
//! component, floats truncated toward zero (`-0.5` and `0.5` are one piece). `attribtype` int writes I32 (a new attribute's
//! unselected elements get -1; an existing numeric attribute is converted to int, values rounded half away from zero, tuple
//! size kept, component 0 written); string writes `prefix` + index (unselected: `""`; an existing string attribute keeps
//! its unselected values). Any other existing attribute of that name and class is replaced. An empty / invalid `attribname`
//! is a warning and the input passes through; `P` on points fails.
//!
//! Not modelled (-> `Unsupported`): vertex-group patterns with numeric tokens (Houdini reads `1` as "the vertices of prim 1"
//! and `1v2` as prim / corner pairs), `@attrib=value` group expressions (rejected by the shared pattern parser). The
//! "start value / increment / detail class" options of other Houdini versions do not exist in 20.5.
use super::group_sop::parse_pattern;
use super::meshutil::{bad, failed, menu, toggle};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;

const OP: &str = "enumerate";
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const PIECES: u32 = 1;
const STRING: u32 = 1;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("grouptype", ParamValue::Menu(0)).label("Group Type").menu(&[
            ("primitive", "Primitives"),
            ("point", "Points"),
            ("vertex", "Vertices"),
        ]),
        ParamSpec::new("usepieceattrib", ParamValue::B(false)).label("Piece Attribute"),
        ParamSpec::new("pieceattrib", ParamValue::S("name".into())).label("Piece Attribute Name"),
        ParamSpec::new("piecemode", ParamValue::Menu(0))
            .label("Piece Mode")
            .menu(&[("elements", "Enumerate Piece Elements"), ("pieces", "Enumerate Pieces")]),
        ParamSpec::new("attribname", ParamValue::S("index".into())).label("Attribute"),
        ParamSpec::new("attribtype", ParamValue::Menu(0))
            .label("Attribute Type")
            .menu(&[("int", "Integer"), ("string", "String")]),
        ParamSpec::new("prefix", ParamValue::S("piece".into())).label("Prefix"),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(Enumerate { specs: spec() })
}

struct Enumerate {
    specs: Vec<ParamSpec>,
}

/// A piece value: ints (and truncated floats) or strings.
#[derive(Clone, PartialEq, Eq)]
enum Key {
    Int(i64),
    Str(String),
}
impl Ord for Key {
    fn cmp(&self, o: &Self) -> Ordering {
        match (self, o) {
            (Key::Int(a), Key::Int(b)) => a.cmp(b),
            (Key::Str(a), Key::Str(b)) => natural_cmp(a.as_bytes(), b.as_bytes()),
            (Key::Int(_), Key::Str(_)) => Ordering::Less,
            (Key::Str(_), Key::Int(_)) => Ordering::Greater,
        }
    }
}
impl PartialOrd for Key {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Houdini's piece-name order (see module docs): natural digit runs, signed bytes elsewhere.
fn natural_cmp(a: &[u8], b: &[u8]) -> Ordering {
    let (mut i, mut j) = (0, 0);
    loop {
        match (a.get(i), b.get(j)) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let run = |s: &[u8], k: usize| s.get(k..).map_or(0, |r| r.iter().take_while(|c| c.is_ascii_digit()).count());
                let (ra, rb) = (run(a, i), run(b, j));
                let (da, db) = (a.get(i..i + ra).unwrap_or(&[]), b.get(j..j + rb).unwrap_or(&[]));
                let strip = |d: &[u8]| {
                    let z = d.iter().take_while(|&&c| c == b'0').count();
                    d.get(z..).unwrap_or(&[]).to_vec()
                };
                let (va, vb) = (strip(da), strip(db));
                let by_value = va.len().cmp(&vb.len()).then_with(|| va.cmp(&vb));
                if by_value != Ordering::Equal {
                    return by_value;
                }
                if ra != rb {
                    // equal values: more leading zeros sorts first
                    return rb.cmp(&ra);
                }
                i += ra;
                j += rb;
            }
            (Some(&x), Some(&y)) => {
                let o = (x as i8).cmp(&(y as i8));
                if o != Ordering::Equal {
                    return o;
                }
                i += 1;
                j += 1;
            }
        }
    }
}

fn valid_name(n: &str) -> bool {
    let mut c = n.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_') && c.all(|k| k.is_ascii_alphanumeric() || k == '_')
}

/// Element ids of `class` in Houdini walk order.
fn walk(d: &Detail, class: AttrClass) -> Vec<usize> {
    match class {
        AttrClass::Vertex => {
            let nv = d.vertex_point.len();
            let mut seen = vec![false; nv];
            let mut out = Vec::with_capacity(nv);
            for pr in &d.prims {
                for v in pr.verts.start as usize..(pr.verts.end as usize).min(nv) {
                    if let Some(s) = seen.get_mut(v).filter(|s| !**s) {
                        *s = true;
                        out.push(v);
                    }
                }
            }
            // vertices no prim owns (malformed input) keep their linear order after the owned ones
            out.extend((0..nv).filter(|&v| !seen.get(v).copied().unwrap_or(true)));
            out
        }
        c => (0..d.class_len(c)).collect(),
    }
}

/// Piece key per element of `class` from attribute `name`; `None` = no usable attribute.
fn keys(d: &Detail, class: AttrClass, name: &str) -> Option<Vec<Key>> {
    let a = d.attr(class, name)?;
    let n = d.class_len(class);
    let trunc = |x: f32| Key::Int(x.trunc() as i64);
    let first_f = |v: &[f32], s: usize| -> Vec<Key> { (0..n).map(|i| trunc(v.get(i * s).copied().unwrap_or(0.0))).collect() };
    Some(match &a.data {
        AttrData::I32(v) => (0..n).map(|i| Key::Int(i64::from(v.get(i).copied().unwrap_or(0)))).collect(),
        AttrData::IN { size, data } => {
            let s = (*size).max(1) as usize;
            (0..n).map(|i| Key::Int(i64::from(data.get(i * s).copied().unwrap_or(0)))).collect()
        }
        AttrData::F32(v) => first_f(v, 1),
        AttrData::V2(v) => (0..n).map(|i| trunc(v.get(i).map_or(0.0, |t| t[0]))).collect(),
        AttrData::V3(v) => (0..n).map(|i| trunc(v.get(i).map_or(0.0, |t| t[0]))).collect(),
        AttrData::V4(v) => (0..n).map(|i| trunc(v.get(i).map_or(0.0, |t| t[0]))).collect(),
        AttrData::FN { size, data } => first_f(data, (*size).max(1) as usize),
        AttrData::Str { idx, table } => (0..n)
            .map(|i| {
                let s = idx.get(i).and_then(|&k| usize::try_from(k).ok()).and_then(|k| table.get(k));
                Key::Str(s.cloned().unwrap_or_default())
            })
            .collect(),
    })
}

/// Half-away-from-zero rounding to i32 (saturating; NaN -> 0).
fn round_i(x: f32) -> i32 {
    x.round() as i32
}

/// The int attribute to write into: an existing numeric one converted (tuple size kept), else fresh -1s.
fn int_target(d: &Detail, class: AttrClass, name: &str, n: usize) -> (u32, Vec<i32>) {
    let conv = |v: &[f32]| v.iter().map(|&x| round_i(x)).collect::<Vec<_>>();
    match d.attr(class, name).map(|a| &a.data) {
        Some(AttrData::I32(v)) => (1, v.clone()),
        Some(AttrData::IN { size, data }) => (*size, data.clone()),
        Some(AttrData::F32(v)) => (1, conv(v)),
        Some(AttrData::V2(v)) => (2, conv(&v.iter().flatten().copied().collect::<Vec<_>>())),
        Some(AttrData::V3(v)) => (3, conv(&v.iter().flatten().copied().collect::<Vec<_>>())),
        Some(AttrData::V4(v)) => (4, conv(&v.iter().flatten().copied().collect::<Vec<_>>())),
        Some(AttrData::FN { size, data }) => (*size, conv(data)),
        _ => (1, vec![-1; n]),
    }
}

impl Node<Detail> for Enumerate {
    fn type_name(&self) -> &'static str {
        OP
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let class = match menu(p, "grouptype")? {
            0 => AttrClass::Prim,
            1 => AttrClass::Point,
            2 => AttrClass::Vertex,
            m => return Err(bad(format!("`grouptype`: menu index {m} out of range"))),
        };
        let use_piece = toggle(p, "usepieceattrib")?;
        let pieces = match menu(p, "piecemode")? {
            m @ (0 | PIECES) => m == PIECES,
            m => return Err(bad(format!("`piecemode`: menu index {m} out of range"))),
        };
        let as_string = match menu(p, "attribtype")? {
            m @ (0 | STRING) => m == STRING,
            m => return Err(bad(format!("`attribtype`: menu index {m} out of range"))),
        };
        let s = |n: &str| p.string(n).ok_or_else(|| bad(format!("`{n}`: missing or not a string")));
        let (pat, piece_name, name, prefix) = (s("group")?, s("pieceattrib")?, s("attribname")?, s("prefix")?);
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        if !valid_name(name) || name.len() > 256 {
            (ctx.log)(&format!("{OP}: invalid attribute name `{name}`; input passed through"));
            return Ok(Arc::clone(src));
        }
        if class == AttrClass::Point && name == "P" {
            return Err(failed(OP, "cannot write the enumeration into `P`"));
        }
        if prefix.len() > 4096 {
            return Err(bad("`prefix`: longer than 4096 bytes"));
        }
        if class == AttrClass::Vertex
            && pat.split_whitespace().any(|t| t.trim_start_matches('!').starts_with(|c: char| c.is_ascii_digit()))
        {
            return Err(CookError::Unsupported(
                "enumerate: numeric vertex-group patterns (prim / `NvM` vertex numbering) are not modelled".into(),
            ));
        }
        let n = src.class_len(class);
        let sel = parse_pattern(src, class, pat, OP)?;
        let order: Vec<usize> =
            walk(src, class).into_iter().filter(|&i| sel.as_ref().is_none_or(|m| m.get(i).copied().unwrap_or(false))).collect();
        let key = if use_piece {
            Some(keys(src, class, piece_name).ok_or_else(|| failed(OP, &format!("invalid piece attribute `{piece_name}`")))?)
        } else {
            None
        };
        // index per element (None = unselected)
        let mut index: Vec<Option<u64>> = vec![None; n];
        match &key {
            None => {
                for (k, &e) in order.iter().enumerate() {
                    if let Some(s) = index.get_mut(e) {
                        *s = Some(k as u64);
                    }
                }
            }
            Some(kv) if pieces => {
                let mut rank: BTreeMap<&Key, u64> = order.iter().filter_map(|&e| kv.get(e)).map(|k| (k, 0)).collect();
                for (r, v) in rank.values_mut().enumerate() {
                    *v = r as u64;
                }
                for &e in &order {
                    if let (Some(s), Some(r)) = (index.get_mut(e), kv.get(e).and_then(|k| rank.get(k))) {
                        *s = Some(*r);
                    }
                }
            }
            Some(kv) => {
                let mut count: BTreeMap<&Key, u64> = BTreeMap::new();
                for &e in &order {
                    let Some(k) = kv.get(e) else { continue };
                    let c = count.entry(k).or_insert(0);
                    if let Some(s) = index.get_mut(e) {
                        *s = Some(*c);
                    }
                    *c += 1;
                }
            }
        }
        let mut out = (**src).clone();
        if as_string {
            let (mut idx, mut table) = match src.attr(class, name).map(|a| &a.data) {
                Some(AttrData::Str { idx, table }) => (idx.clone(), table.clone()),
                _ => (vec![-1; n], Vec::new()),
            };
            idx.resize(n, -1);
            let mut lookup: std::collections::HashMap<String, i32> =
                table.iter().enumerate().filter_map(|(k, s)| Some((s.clone(), i32::try_from(k).ok()?))).collect();
            for (e, v) in index.iter().enumerate() {
                let Some(v) = v else { continue };
                let text = format!("{prefix}{v}");
                let k = match lookup.get(&text) {
                    Some(&k) => k,
                    None => {
                        let k = i32::try_from(table.len()).map_err(|_| failed(OP, "string table overflow"))?;
                        table.push(text.clone());
                        lookup.insert(text, k);
                        k
                    }
                };
                if let Some(s) = idx.get_mut(e) {
                    *s = k;
                }
            }
            out.add_attr(Attribute { name: name.into(), class, data: AttrData::Str { idx, table }, type_info: TypeInfo::None });
        } else {
            let (size, mut data) = int_target(src, class, name, n);
            let s = size.max(1) as usize;
            data.resize(n.checked_mul(s).ok_or_else(|| failed(OP, "attribute too large"))?, 0);
            for (e, v) in index.iter().enumerate() {
                let Some(v) = v else { continue };
                let v = i32::try_from(*v).map_err(|_| failed(OP, "index overflows int32"))?;
                if let Some(slot) = data.get_mut(e * s) {
                    *slot = v;
                }
            }
            let data = if size <= 1 { AttrData::I32(data) } else { AttrData::IN { size, data } };
            out.add_attr(Attribute { name: name.into(), class, data, type_info: TypeInfo::None });
        }
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, defaults};
    use super::*;
    use hexe_graph::ParamSet;

    fn run(d: Detail, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        let n = new();
        let p = ov.iter().fold(defaults(&*n), |p, (k, v)| p.with(k, v.clone()));
        cook(&*n, d, &p)
    }
    fn s(v: &str) -> ParamValue {
        ParamValue::S(v.into())
    }
    fn ints(d: &Detail, c: AttrClass, n: &str) -> Vec<i32> {
        match d.attr(c, n).map(|a| &a.data) {
            Some(AttrData::I32(v)) => v.clone(),
            Some(AttrData::IN { size, data }) => data.iter().step_by((*size).max(1) as usize).copied().collect(),
            _ => Vec::new(),
        }
    }
    fn strs(d: &Detail, c: AttrClass, n: &str) -> Vec<String> {
        match d.attr(c, n).map(|a| &a.data) {
            Some(AttrData::Str { idx, table }) => idx
                .iter()
                .map(|&k| usize::try_from(k).ok().and_then(|k| table.get(k)).cloned().unwrap_or_default())
                .collect(),
            _ => Vec::new(),
        }
    }
    fn str_attr(v: &[&str]) -> AttrData {
        let mut table: Vec<String> = Vec::new();
        let idx = v
            .iter()
            .map(|x| match table.iter().position(|t| t == x) {
                Some(k) => k as i32,
                None => {
                    table.push((*x).into());
                    (table.len() - 1) as i32
                }
            })
            .collect();
        AttrData::Str { idx, table }
    }
    /// 8 points on x, 5 open prims, prim `name` b a b c a, point `name`, point int `pid`, prim group `sel` = 1 3 4.
    fn probe_geo() -> Detail {
        let mut d = Detail::default();
        d.add_points(8);
        *d.p_mut() = (0..8).map(|i| [i as f32, 0.0, 0.0]).collect();
        for ids in [&[0, 1, 2][..], &[2, 3, 4], &[5, 6], &[6, 7, 0], &[1, 3]] {
            d.add_poly(ids, false);
        }
        let attr = |name: &str, class, data| Attribute { name: name.into(), class, data, type_info: TypeInfo::None };
        d.add_attr(attr("name", AttrClass::Prim, str_attr(&["b", "a", "b", "c", "a"])));
        d.add_attr(attr("name", AttrClass::Point, str_attr(&["x", "y", "x", "z", "y", "y", "x", "z"])));
        d.add_attr(attr("pid", AttrClass::Point, AttrData::I32(vec![5, 3, 5, -1, 3, 3, 7, 7])));
        d.groups.push(hexe_geo::Group {
            name: "sel".into(),
            class: AttrClass::Prim,
            mask: vec![false, true, false, true, true],
        });
        d
    }

    /// The hython probe table (`oracle/houdini/enumerate/enumerate_probe.py`) reproduced value for value.
    #[test]
    fn probe_table_matches_houdini() {
        let pt = ("grouptype", ParamValue::Menu(1));
        let on = ("usepieceattrib", ParamValue::B(true));
        let pcs = ("piecemode", ParamValue::Menu(1));
        let g = |ov: &[(&str, ParamValue)]| run(probe_geo(), ov).expect("cooks");
        assert_eq!(ints(&g(&[]), AttrClass::Prim, "index"), vec![0, 1, 2, 3, 4]);
        assert_eq!(ints(&g(&[pt.clone()]), AttrClass::Point, "index"), (0..8).collect::<Vec<_>>());
        assert_eq!(ints(&g(&[("grouptype", ParamValue::Menu(2))]), AttrClass::Vertex, "index"), (0..13).collect::<Vec<_>>());
        assert_eq!(ints(&g(&[("group", s("sel"))]), AttrClass::Prim, "index"), vec![-1, 0, -1, 1, 2]);
        assert_eq!(
            strs(&g(&[("attribtype", ParamValue::Menu(1))]), AttrClass::Prim, "index"),
            ["piece0", "piece1", "piece2", "piece3", "piece4"]
        );
        assert_eq!(
            strs(&g(&[("attribtype", ParamValue::Menu(1)), ("prefix", s("p_")), ("group", s("sel"))]), AttrClass::Prim, "index"),
            ["", "p_0", "", "p_1", "p_2"]
        );
        assert_eq!(ints(&g(&[on.clone()]), AttrClass::Prim, "index"), vec![0, 0, 1, 0, 1]);
        assert_eq!(ints(&g(&[on.clone(), pcs.clone()]), AttrClass::Prim, "index"), vec![1, 0, 1, 2, 0]);
        assert_eq!(ints(&g(&[on.clone(), pcs.clone(), ("group", s("sel"))]), AttrClass::Prim, "index"), vec![-1, 0, -1, 1, 0]);
        assert_eq!(ints(&g(&[on.clone(), ("group", s("sel"))]), AttrClass::Prim, "index"), vec![-1, 0, -1, 0, 1]);
        assert_eq!(ints(&g(&[on.clone(), pt.clone()]), AttrClass::Point, "index"), vec![0, 0, 1, 0, 1, 2, 2, 1]);
        assert_eq!(ints(&g(&[on.clone(), pt.clone(), pcs.clone()]), AttrClass::Point, "index"), vec![0, 1, 0, 2, 1, 1, 0, 2]);
        let pid = ("pieceattrib", s("pid"));
        assert_eq!(ints(&g(&[on.clone(), pt.clone(), pid.clone()]), AttrClass::Point, "index"), vec![0, 0, 1, 0, 1, 2, 0, 1]);
        assert_eq!(ints(&g(&[on.clone(), pt.clone(), pid.clone(), pcs.clone()]), AttrClass::Point, "index"), vec![
            2, 1, 2, 0, 1, 1, 3, 3
        ]);
        // string output of pieces; the output may overwrite the piece attribute itself
        assert_eq!(
            strs(&g(&[on.clone(), pcs.clone(), ("attribtype", ParamValue::Menu(1))]), AttrClass::Prim, "index"),
            ["piece1", "piece0", "piece1", "piece2", "piece0"]
        );
        let o = g(&[on.clone(), ("attribname", s("name"))]);
        assert_eq!(ints(&o, AttrClass::Prim, "name"), vec![0, 0, 1, 0, 1]);
        assert_eq!(strs(&o, AttrClass::Point, "name").len(), 8, "the point `name` is untouched");
        // a float piece attribute keys by its first component, truncated (P.x = 0..7: every point its own piece)
        assert_eq!(ints(&g(&[on.clone(), pt.clone(), ("pieceattrib", s("P"))]), AttrClass::Point, "index"), vec![0; 8]);
        // piece attribute on another class / missing: the cook fails
        assert!(matches!(run(probe_geo(), &[on.clone(), ("grouptype", ParamValue::Menu(2))]), Err(CookError::Failed(_))));
        assert!(matches!(run(probe_geo(), &[on, pt, ("pieceattrib", s("nope"))]), Err(CookError::Failed(_))));
    }

    #[test]
    fn natural_order_and_existing_attributes() {
        let words = ["b9", "B", "b10", "a", "", "b9", "\u{e9}"];
        let mut d = Detail::default();
        d.add_points(7);
        d.add_attr(Attribute { name: "s".into(), class: AttrClass::Point, data: str_attr(&words), type_info: TypeInfo::None });
        d.add_attr(Attribute {
            name: "f".into(),
            class: AttrClass::Point,
            data: AttrData::F32(vec![-0.5, 0.5, -1.5, 1.5, -0.0, 2.0, 0.25]),
            type_info: TypeInfo::None,
        });
        let base = [("grouptype", ParamValue::Menu(1)), ("usepieceattrib", ParamValue::B(true)), ("piecemode", ParamValue::Menu(1))];
        let o = run(d.clone(), &[base[0].clone(), base[1].clone(), base[2].clone(), ("pieceattrib", s("s"))]).expect("cooks");
        assert_eq!(ints(&o, AttrClass::Point, "index"), vec![4, 2, 5, 3, 0, 4, 1]);
        let o = run(d.clone(), &[base[0].clone(), base[1].clone(), base[2].clone(), ("pieceattrib", s("f"))]).expect("cooks");
        assert_eq!(ints(&o, AttrClass::Point, "index"), vec![1, 1, 0, 2, 1, 3, 1]);
        for (a, b, o) in [
            ("x2", "x10", Ordering::Less),
            ("a001", "a01", Ordering::Less),
            ("a01", "a1", Ordering::Less),
            ("ab", "b", Ordering::Less),
            ("A", "a", Ordering::Less),
            ("", "a", Ordering::Less),
            ("b9", "b9", Ordering::Equal),
        ] {
            assert_eq!(natural_cmp(a.as_bytes(), b.as_bytes()), o, "{a} vs {b}");
        }
        // existing float prim attribute -> int (rounded), existing string + int request -> fresh -1s
        let mut e = Detail::default();
        e.add_points(1);
        for _ in 0..3 {
            e.add_poly(&[0], false);
        }
        e.add_attr(Attribute { name: "fidx".into(), class: AttrClass::Prim, data: AttrData::F32(vec![2.5; 3]), type_info: TypeInfo::None });
        e.add_attr(Attribute { name: "sx".into(), class: AttrClass::Prim, data: str_attr(&["zz"; 3]), type_info: TypeInfo::None });
        e.add_attr(Attribute {
            name: "i3".into(),
            class: AttrClass::Prim,
            data: AttrData::IN { size: 3, data: vec![7; 9] },
            type_info: TypeInfo::None,
        });
        e.groups.push(hexe_geo::Group { name: "one".into(), class: AttrClass::Prim, mask: vec![false, true, false] });
        let one = ("group", s("one"));
        assert_eq!(ints(&run(e.clone(), &[one.clone(), ("attribname", s("fidx"))]).expect("c"), AttrClass::Prim, "fidx"), [3, 0, 3]);
        assert_eq!(ints(&run(e.clone(), &[one.clone(), ("attribname", s("sx"))]).expect("c"), AttrClass::Prim, "sx"), [-1, 0, -1]);
        let o = run(e.clone(), &[one.clone(), ("attribname", s("i3"))]).expect("c");
        assert!(matches!(&o.attr(AttrClass::Prim, "i3").map(|a| &a.data), Some(AttrData::IN { size: 3, data }) if data == &[7, 7, 7, 0, 7, 7, 7, 7, 7]));
        let o = run(e, &[one, ("attribname", s("sx")), ("attribtype", ParamValue::Menu(1))]).expect("c");
        assert_eq!(strs(&o, AttrClass::Prim, "sx"), ["zz", "piece0", "zz"]);
    }

    /// Synthetic hython cases (`oracle/houdini/enumerate/sop_enumerate_cases.py`): Houdini 20.5 in / out per case.
    #[test]
    fn recorded_cases_match_houdini_exactly() {
        let fx = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/sop-enumerate/");
        let cases: serde_json::Value =
            serde_json::from_slice(&std::fs::read(format!("{fx}cases.json")).expect("cases.json")).expect("json");
        let read = |f: &str| hexe_geo::read_geo(&std::fs::read(format!("{fx}{f}")).expect("fixture")).expect("geo");
        let input = read("in.geo");
        let obj = cases.as_object().expect("object");
        assert!(obj.len() >= 6);
        let n = new();
        for (name, parms) in obj {
            let mut p: ParamSet = defaults(&*n);
            for (k, v) in parms.as_object().expect("parms") {
                let spec = n.params().iter().find(|s| s.name == *k).expect("known parm");
                let val = match (&spec.default, v) {
                    (ParamValue::Menu(_), x) => ParamValue::Menu(x.as_u64().expect("menu") as u32),
                    (ParamValue::B(_), x) => ParamValue::B(x.as_i64().expect("toggle") != 0),
                    (ParamValue::S(_), x) => ParamValue::S(x.as_str().expect("string").into()),
                    other => panic!("unexpected parm {k}: {other:?}"),
                };
                p = p.with(k, val);
            }
            let h = read(&format!("{name}_houdini.geo"));
            let d = cook(&*n, input.clone(), &p).expect("cooks");
            let attr = parms.get("attribname").and_then(|v| v.as_str()).unwrap_or("index");
            let class = match parms.get("grouptype").and_then(|v| v.as_u64()).unwrap_or(0) {
                0 => AttrClass::Prim,
                1 => AttrClass::Point,
                _ => AttrClass::Vertex,
            };
            let (a, b) = (d.attr(class, attr).expect("hexe attr"), h.attr(class, attr).expect("houdini attr"));
            match (&a.data, &b.data) {
                (AttrData::Str { .. }, AttrData::Str { .. }) => assert_eq!(strs(&d, class, attr), strs(&h, class, attr), "{name}"),
                _ => assert_eq!(a.data, b.data, "{name}"),
            }
            let names = |x: &Detail| {
                let mut v: Vec<(String, u8)> =
                    x.attrs.iter().filter(|a| a.class != AttrClass::Detail).map(|a| (a.name.clone(), a.class as u8)).collect();
                v.sort();
                v
            };
            assert_eq!(names(&d), names(&h), "{name} attribute set");
        }
    }

    #[test]
    fn hostile_inputs_never_panic() {
        assert_eq!(run(Detail::default(), &[]).expect("empty").prims.len(), 0);
        assert_eq!(run(Detail::default(), &[("usepieceattrib", ParamValue::B(true))]).is_err(), true);
        // invalid / empty names pass the input through
        for bad_name in ["", "bad name", "9x", "\u{0}"] {
            let o = run(probe_geo(), &[("attribname", s(bad_name))]).expect("pass-through");
            assert!(o.attr(AttrClass::Prim, bad_name).is_none());
        }
        assert!(matches!(run(probe_geo(), &[("attribname", s("P")), ("grouptype", ParamValue::Menu(1))]), Err(CookError::Failed(_))));
        assert!(matches!(run(probe_geo(), &[("grouptype", ParamValue::Menu(7))]), Err(CookError::BadParam(_))));
        assert!(matches!(run(probe_geo(), &[("piecemode", ParamValue::Menu(9))]), Err(CookError::BadParam(_))));
        assert!(matches!(run(probe_geo(), &[("grouptype", ParamValue::F(f32::NAN))]), Err(CookError::BadParam(_))));
        assert!(matches!(
            run(probe_geo(), &[("grouptype", ParamValue::Menu(2)), ("group", s("1"))]),
            Err(CookError::Unsupported(_))
        ));
        assert!(run(probe_geo(), &[("group", s("nosuchgroup"))]).is_err());
        // NaN / huge float piece values, dangling vertex ranges
        let mut d = Detail::default();
        d.add_points(3);
        d.add_attr(Attribute {
            name: "f".into(),
            class: AttrClass::Point,
            data: AttrData::F32(vec![f32::NAN, f32::MAX, f32::NEG_INFINITY]),
            type_info: TypeInfo::None,
        });
        let o = run(d, &[("grouptype", ParamValue::Menu(1)), ("usepieceattrib", ParamValue::B(true)), ("pieceattrib", s("f")), ("piecemode", ParamValue::Menu(1))])
            .expect("cooks");
        assert_eq!(ints(&o, AttrClass::Point, "index"), vec![1, 2, 0]);
        let mut bad = probe_geo();
        bad.prims[0].verts = 40..2;
        assert!(run(bad, &[("grouptype", ParamValue::Menu(2))]).is_ok());
        let long = "x".repeat(10_000);
        assert!(run(probe_geo(), &[("prefix", s(&long)), ("attribtype", ParamValue::Menu(1))]).is_err());
    }
}
