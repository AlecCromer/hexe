//! Add SOP (token `add`): append points (and optionally a polyline through them) to the input.
//!
//! Params: `points` (S, whitespace-separated `x,y,z` triples, same grammar as the Curve SOP; empty = none) · `prims` (B, off)
//! · `connect` (menu `all` | `none`, default all) · `delete_geometry` (B, off: drop the input, keep only what is added).
//!
//! # Point / prim order (UNVERIFIED vs Houdini, hython blocked)
//! Output points = input points (ids unchanged) then the added points in text order (ids `n_in..`). Added points get
//! zero / default for every other input attribute. Output prims = input prims unchanged, then (when `prims` is on and
//! `connect` = `all` and the result has ≥ 2 points) ONE open `Poly` over ALL output points in id order (input + added).
//! `delete_geometry` first empties the input, so the polyline then runs through the added points only.
//! `connect` = `none` adds no prim even with `prims` on. No change requested → the input is passed through untouched.
use super::curve_sop::parse_coords;
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

/// Output point cap (AGENTS.md §4).
const MAX_POINTS: u64 = 1 << 22;
/// `connect` menu (all none).
const CONNECT_ALL: u32 = 0;
const CONNECT_NONE: u32 = 1;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("points", ParamValue::S(String::new())).label("Points"),
        ParamSpec::new("prims", ParamValue::B(false)).label("Add Polyline"),
        ParamSpec::new("connect", ParamValue::Menu(CONNECT_ALL))
            .label("Connect")
            .menu(&[("all", "All Points"), ("none", "None")]),
        ParamSpec::new("delete_geometry", ParamValue::B(false)).label("Delete Geometry But Keep the Points"),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(Add { specs: spec() })
}

struct Add {
    specs: Vec<ParamSpec>,
}

fn bad(name: &str, what: &str) -> CookError {
    CookError::BadParam(format!("add.{name}: {what}"))
}

impl Node<Detail> for Add {
    fn type_name(&self) -> &'static str {
        "add"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    /// Houdini: the input is optional (no input = only the added points).
    fn min_inputs(&self) -> usize {
        0
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let text = p.string("points").ok_or_else(|| bad("points", "missing or not a string"))?;
        let prims = p.toggle("prims").ok_or_else(|| bad("prims", "missing or not a toggle"))?;
        let connect = p.menu("connect").ok_or_else(|| bad("connect", "missing or not a menu"))?;
        let delete = p.toggle("delete_geometry").ok_or_else(|| bad("delete_geometry", "missing or not a toggle"))?;
        if connect > CONNECT_NONE {
            return Err(bad("connect", "menu index out of range 0..=1"));
        }
        let added = parse_coords("add.points", text)?;
        let input = inputs.first();
        let join = prims && connect == CONNECT_ALL;
        if added.is_empty() && !join && !delete {
            return Ok(input.cloned().unwrap_or_default());
        }
        let mut g = match input {
            Some(d) if !delete => (**d).clone(),
            _ => Detail::default(),
        };
        let total = u64::from(g.point_count) + added.len() as u64;
        if total > MAX_POINTS {
            return Err(CookError::Failed(format!("add: {total} points exceeds the {MAX_POINTS} point cap")));
        }
        if !added.is_empty() {
            let n = u32::try_from(added.len()).map_err(|_| CookError::Failed("add: too many points".into()))?;
            let start = g.add_points(n).start as usize;
            let pos = g.p_mut();
            for (slot, q) in pos.iter_mut().skip(start).zip(added) {
                *slot = q;
            }
        }
        if join && g.point_count >= 2 {
            let all: Vec<u32> = (0..g.point_count).collect();
            g.add_poly(&all, false);
        }
        Ok(Arc::new(g))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_geo::PrimKind;

    fn scene(params: &str, grid: bool) -> String {
        let g = r#"{"name":"g","type":"grid","params":{"rows":3,"cols":3}},"#;
        let (nodes, edges) = if grid { (g, r#"[["g",0,"a",0]]"#) } else { ("", "[]") };
        format!(r#"{{"nodes":[{nodes}{{"name":"a","type":"add","params":{params}}}],"edges":{edges},"display":"a"}}"#)
    }
    fn cook(params: &str, grid: bool) -> Result<Arc<Detail>, String> {
        let (mut g, id) = load_scene(&scene(params, grid)).map_err(|e| e.to_string())?;
        g.cook(id, 0.0).map_err(|e| e.to_string())
    }
    const THREE: &str = "0,0,0 1,0,0 1,1,0";

    #[test]
    fn three_coords_no_prims_is_three_points_zero_prims() {
        let d = cook(&format!(r#"{{"points":"{THREE}"}}"#), false).expect("cooks");
        assert_eq!((d.point_count, d.prim_count(), d.vertex_count()), (3, 0, 0));
        assert_eq!(d.p(), &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]]);
    }

    #[test]
    fn prims_connect_all_is_one_open_polyline_of_three_verts() {
        let d = cook(&format!(r#"{{"points":"{THREE}","prims":true}}"#), false).expect("cooks");
        assert_eq!((d.point_count, d.prim_count(), d.vertex_count()), (3, 1, 3));
        assert!(matches!(d.prims[0].kind, PrimKind::Poly { closed: false }));
        assert_eq!(d.vertex_point, vec![0, 1, 2]);
        let none = cook(&format!(r#"{{"points":"{THREE}","prims":true,"connect":1}}"#), false).expect("cooks");
        assert_eq!((none.point_count, none.prim_count()), (3, 0));
    }

    #[test]
    fn grid_3x3_connect_all_is_nine_vert_polyline_plus_four_quads() {
        let d = cook(r#"{"prims":true}"#, true).expect("cooks");
        assert_eq!((d.point_count, d.prim_count(), d.vertex_count()), (9, 5, 4 * 4 + 9));
        assert!(d.prims[..4].iter().all(|p| matches!(p.kind, PrimKind::Poly { closed: true }) && p.verts.len() == 4));
        assert!(matches!(d.prims[4].kind, PrimKind::Poly { closed: false }) && d.prims[4].verts.len() == 9);
        let line = &d.vertex_point[d.prims[4].verts.start as usize..d.prims[4].verts.end as usize];
        assert_eq!(line, (0..9).collect::<Vec<u32>>().as_slice());
    }

    #[test]
    fn added_points_follow_input_points_and_delete_geometry_keeps_only_added() {
        let d = cook(&format!(r#"{{"points":"{THREE}","prims":true}}"#), true).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (12, 5));
        assert_eq!(&d.p()[9..], &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]]);
        assert_eq!(d.prims[4].verts.len(), 12);
        let only =
            cook(&format!(r#"{{"points":"{THREE}","prims":true,"delete_geometry":true}}"#), true).expect("cooks");
        assert_eq!((only.point_count, only.prim_count(), only.vertex_count()), (3, 1, 3));
        assert_eq!(only.p(), d.p().get(9..).unwrap_or(&[]));
    }

    #[test]
    fn no_change_passes_input_through_and_bad_coords_error() {
        let d = cook("{}", true).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (9, 4));
        assert!(cook(r#"{"points":"1,2"}"#, false).is_err());
        assert!(cook(r#"{"points":"1,2,3","prims":true}"#, false).map(|d| d.prim_count()) == Ok(0));
        assert_eq!(cook("{}", false).map(|d| d.point_count), Ok(0));
    }
}
