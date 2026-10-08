//! Circle SOP (token `circle`): polygon / polyline circle or arc. Generator (no inputs).
//!
//! `type`: polygon | polyline implemented; nurbs | bezier → `CookError::Unsupported`. `orient`: xy yz zx. `radius` is declared
//! `V3` (no V2 variant; `radius[2]` ignored): `radius[0]` along the plane's first axis, `radius[1]` along its second.
//! `arc`: `closed` (full circle; `beginangle`/`endangle` ignored) | `open_arc` (`beginangle..endangle` degrees).
//!
//! # Point order (verified against real Houdini 20.5, `fixtures/sop-circle/houdini.geo` + orient/reverse cases in docs/HOUDINI-ORACLE.md)
//! `reverse` B (default ON, Houdini's default): with `u = r0 cos(a_i)`, `v = r1 sin(a_i)` and `sg = -1` when reverse else `+1`:
//! xy `(u, sg v, 0)`; yz `(0, v, -sg u)` (first radius along Z, second along Y); zx `(u, 0, -sg v)`; all + `center`. Point 0 sits on
//! the first radius axis (+X for xy / zx, +Z for yz with reverse on). Default (reverse on, xy) runs CLOCKWISE seen from +Z: pt1 =
//! `(r0 cos(2pi/n), -r1 sin(2pi/n))`. Reverse off = counter-clockwise (xy).
//! * closed: `divisions` points, `a_i = 2*pi*i/divisions`; the closing point is NOT repeated.
//! * open_arc: `divisions + 1` points, `a_i = begin + (end - begin)*i/divisions` (degrees → radians).
//!
//! Prim: one `Poly`. Closed iff `type == polygon` and `arc == closed`; every `polyline` and every `open_arc` is open.
//! `divisions` < 3 for a closed circle, < 1 for an open arc, or above the allocation cap → `BadParam` / `Failed`.
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const MAX_POINTS: u32 = 1 << 20;
/// `type` menu (polygon polyline nurbs bezier).
const TYPE_POLYGON: u32 = 0;
const TYPE_POLYLINE: u32 = 1;
const TYPE_BEZIER: u32 = 3;
/// `orient` menu (xy yz zx).
const ORIENT_XY: u32 = 0;
const ORIENT_YZ: u32 = 1;
const ORIENT_ZX: u32 = 2;
/// `arc` menu (closed open_arc).
const ARC_CLOSED: u32 = 0;
const ARC_OPEN: u32 = 1;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("type", ParamValue::Menu(TYPE_POLYGON)).label("Primitive Type"),
        ParamSpec::new("orient", ParamValue::Menu(ORIENT_XY)).label("Orientation"),
        ParamSpec::new("radius", ParamValue::V3([1.0, 1.0, 0.0])).label("Radius").range(0.0, 10.0),
        ParamSpec::new("center", ParamValue::V3([0.0, 0.0, 0.0])).label("Center").range(-1.0, 1.0),
        ParamSpec::new("divisions", ParamValue::I(12)).label("Divisions").range(3.0, 50.0),
        ParamSpec::new("arc", ParamValue::Menu(ARC_CLOSED)).label("Arc Type"),
        // Houdini default is ON (verified): the circle runs clockwise about the plane normal.
        ParamSpec::new("reverse", ParamValue::B(true)).label("Reverse"),
        ParamSpec::new("beginangle", ParamValue::F(0.0)).label("Start Angle").range(0.0, 360.0),
        ParamSpec::new("endangle", ParamValue::F(360.0)).label("End Angle").range(0.0, 360.0),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(Circle { specs: spec() })
}

struct Circle {
    specs: Vec<ParamSpec>,
}

fn bad(name: &str, what: &str) -> CookError {
    CookError::BadParam(format!("circle.{name}: {what}"))
}

impl Node<Detail> for Circle {
    fn type_name(&self) -> &'static str {
        "circle"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, _: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let ty = p.menu("type").ok_or_else(|| bad("type", "missing or not a menu"))?;
        let orient = p.menu("orient").ok_or_else(|| bad("orient", "missing or not a menu"))?;
        let radius = p.vec3("radius").ok_or_else(|| bad("radius", "missing or not a vector"))?;
        let center = p.vec3("center").ok_or_else(|| bad("center", "missing or not a vector"))?;
        let div = p.int("divisions").ok_or_else(|| bad("divisions", "missing or not an integer"))?;
        let arc = p.menu("arc").ok_or_else(|| bad("arc", "missing or not a menu"))?;
        let reverse = p.toggle("reverse").ok_or_else(|| bad("reverse", "missing or not a toggle"))?;
        // `reverse` mirrors the curve (point 0 stays put): Houdini flips the second in-plane component (see module docs).
        let sgn = if reverse { -1.0 } else { 1.0 };
        let begin = p.float("beginangle").ok_or_else(|| bad("beginangle", "missing or not a float"))?;
        let end = p.float("endangle").ok_or_else(|| bad("endangle", "missing or not a float"))?;
        if ty > TYPE_BEZIER {
            return Err(bad("type", "menu index out of range 0..=3"));
        }
        if orient > ORIENT_ZX {
            return Err(bad("orient", "menu index out of range 0..=2"));
        }
        if arc > ARC_OPEN {
            return Err(bad("arc", "menu index out of range 0..=1"));
        }
        if ty > TYPE_POLYLINE {
            let name = if ty == 2 { "nurbs" } else { "bezier" };
            return Err(CookError::Unsupported(format!("circle type `{name}` is not implemented")));
        }
        if !radius.iter().chain(center.iter()).all(|c| c.is_finite()) || !begin.is_finite() || !end.is_finite() {
            return Err(bad("radius/center/angles", "non-finite value"));
        }
        let open_arc = arc == ARC_OPEN;
        let min_div = if open_arc { 1 } else { 3 };
        let div = u32::try_from(div).ok().filter(|&n| n >= min_div).ok_or_else(|| {
            bad("divisions", if open_arc { "need at least 1 division" } else { "need at least 3 divisions" })
        })?;
        let n = if open_arc { div.checked_add(1) } else { Some(div) };
        let n = n
            .filter(|&n| n <= MAX_POINTS)
            .ok_or_else(|| CookError::Failed(format!("circle: {div} divisions exceeds the {MAX_POINTS} point cap")))?;
        let (r0, r1) = (f64::from(radius[0]), f64::from(radius[1]));
        let c = center.map(f64::from);
        let (b, e) = (f64::from(begin).to_radians(), f64::from(end).to_radians());
        let mut g = Detail::default();
        let ids = g.add_points(n);
        let pos = g.p_mut();
        pos.clear();
        for i in 0..n {
            let t = f64::from(i) / f64::from(div);
            let a = if open_arc { b + (e - b) * t } else { std::f64::consts::TAU * t };
            let (u, v) = (r0 * a.cos(), r1 * a.sin());
            let local = match orient {
                ORIENT_XY => [u, sgn * v, 0.0],
                ORIENT_YZ => [0.0, v, -sgn * u],
                _ => [u, 0.0, -sgn * v],
            };
            pos.push([0, 1, 2].map(|k| (c[k] + local[k]) as f32));
        }
        let ring: Vec<u32> = ids.collect();
        g.add_poly(&ring, ty == TYPE_POLYGON && !open_arc);
        Ok(Arc::new(g))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_geo::PrimKind;

    fn cook(params: &str) -> Result<Arc<Detail>, String> {
        let s = format!(r#"{{"nodes":[{{"name":"c","type":"circle","params":{params}}}],"display":"c"}}"#);
        let (mut g, id) = load_scene(&s).map_err(|e| e.to_string())?;
        g.cook(id, 0.0).map_err(|e| e.to_string())
    }
    fn near(a: &[f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    #[test]
    fn four_divisions_radius_1_xy_is_the_four_axis_points_one_closed_prim() {
        let d = cook(r#"{"divisions":4}"#).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (4, 1));
        // Houdini default reverse=1: clockwise seen from +Z
        let want = [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        for (p, w) in d.p().iter().zip(want) {
            assert!(near(p, w), "{p:?} vs {w:?}");
        }
        assert!(matches!(d.prims[0].kind, PrimKind::Poly { closed: true }));
        assert_eq!(d.vertex_point, vec![0, 1, 2, 3]);
    }

    #[test]
    fn polyline_type_is_open_same_points() {
        let d = cook(r#"{"divisions":4,"type":1}"#).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (4, 1));
        assert!(matches!(d.prims[0].kind, PrimKind::Poly { closed: false }));
    }

    #[test]
    fn arc_0_to_90_four_divisions_is_five_points_open() {
        let d = cook(r#"{"divisions":4,"arc":1,"beginangle":0,"endangle":90}"#).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (5, 1));
        assert!(matches!(d.prims[0].kind, PrimKind::Poly { closed: false }));
        assert!(near(&d.p()[0], [1.0, 0.0, 0.0]) && near(&d.p()[4], [0.0, -1.0, 0.0]));
        let h = (45.0f32).to_radians();
        let a2 = [h.cos(), -h.sin(), 0.0];
        assert!(near(&d.p()[2], a2));
    }

    #[test]
    fn orient_yz_zx_follow_houdini_axes_with_radius_and_center() {
        let yz = cook(r#"{"divisions":4,"orient":1,"radius":[2,3,0],"center":[1,1,1]}"#).expect("cooks");
        // Houdini yz: first radius along Z, second along Y; reverse on
        assert!(near(&yz.p()[0], [1.0, 1.0, 3.0]) && near(&yz.p()[1], [1.0, 4.0, 1.0]));
        let zx = cook(r#"{"divisions":4,"orient":2}"#).expect("cooks");
        assert!(near(&zx.p()[0], [1.0, 0.0, 0.0]) && near(&zx.p()[1], [0.0, 0.0, 1.0]));
        // reverse on = clockwise about +Z (Newell -Z); reverse off = counter-clockwise (+Z).
        let xy = cook(r#"{"divisions":4}"#).expect("cooks");
        let q = xy.p();
        let n: f32 = (0..4).map(|i| q[i][0] * q[(i + 1) % 4][1] - q[(i + 1) % 4][0] * q[i][1]).sum();
        assert!(n < 0.0);
        let ccw = cook(r#"{"divisions":4,"reverse":false}"#).expect("cooks");
        let q = ccw.p();
        let n: f32 = (0..4).map(|i| q[i][0] * q[(i + 1) % 4][1] - q[(i + 1) % 4][0] * q[i][1]).sum();
        assert!(n > 0.0 && near(&q[1], [0.0, 1.0, 0.0]));
    }

    #[test]
    fn nurbs_bezier_unsupported_and_hostile_divisions_rejected() {
        for t in [2, 3] {
            let s = format!(r#"{{"nodes":[{{"name":"c","type":"circle","params":{{"type":{t}}}}}],"display":"c"}}"#);
            let (mut g, id) = load_scene(&s).expect("loads");
            assert!(matches!(
                g.cook(id, 0.0),
                Err(hexe_graph::GraphError::Node { source: CookError::Unsupported(_), .. })
            ));
        }
        assert!(cook(r#"{"divisions":2}"#).is_err());
        assert!(cook(r#"{"divisions":0,"arc":1}"#).is_err());
        assert!(cook(r#"{"divisions":-1}"#).is_err());
        assert!(cook(r#"{"divisions":2000000}"#).is_err());
        assert!(cook(r#"{"divisions":2,"arc":1}"#).is_ok());
    }
}
