//! UVUnwrap SOP (token `uvunwrap`, "simple" unwrap): every primitive becomes its own island, projected along its
//! best-fit axis, normalised and placed in a `tile x tile` cell grid. Writes VERTEX `uv` (V3, type None).
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; closed-form oracle only)
//! * Best-fit normal of a prim = Newell vector of its corner points (any corner count; closed or open). Dominant axis =
//!   the largest `|component|` (ties: x before y before z); a zero / non-finite normal -> axis z, positive.
//! * Projection (right-handed, same pairs as `uvproject`): axis x -> `(u, v) = (Y, Z)`, y -> `(Z, X)`, z -> `(X, Y)`;
//!   when the dominant component is NEGATIVE, `u` is mirrored (`-a`) so the island reads the same from the front.
//! * Island local coords = `(raw - bbox_min) / max(width, height)` of the prim's own corners: aspect preserved, longer
//!   side spans exactly `[0, 1]`; a point-like prim (extent 0) -> `(0, 0)`.
//! * `tile` (int `1..=1024`, default 1): prim `i` sits in cell `(i mod tile, (i / tile) mod tile)`;
//!   `uv = (cell + local) / tile` (so `tile = 1` overlays every island on `[0,1]^2`; `tile >= ceil(sqrt(prims))`
//!   gives each prim its own cell). Prims beyond `tile^2` wrap onto earlier cells.
//! * `uv.z = 0`. A vertex shared by several prims is one vertex PER prim (vertex-class), so islands never share a value.
//!   Vertices owned by no prim / dangling or non-finite points -> `(0, 0, 0)`.
//!
//! Output = the input with vertex `uv` replaced/added; everything else untouched. Hostile: `tile` outside `1..=1024`
//! or not an int -> `BadParam`.
use super::meshutil::{bad, int, newell_corners};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const MAX_TILE: i32 = 1024;
/// Newell magnitudes at or below this have no direction.
const MIN_NEWELL: f64 = 1e-12;

pub fn spec() -> Vec<ParamSpec> {
    vec![ParamSpec::new("tile", ParamValue::I(1)).label("Tiles per Side").range(1.0, MAX_TILE as f32)]
}
pub fn new() -> Box<SopNode> {
    Box::new(UvUnwrap { specs: spec() })
}
struct UvUnwrap {
    specs: Vec<ParamSpec>,
}

/// `(axis, negative)` of the best-fit normal of `pts`.
fn dominant(pts: &[[f64; 3]]) -> (usize, bool) {
    let n = newell_corners(pts); // Houdini sign: outward-facing
    let mag = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if !mag.is_finite() || mag <= MIN_NEWELL {
        return (2, false);
    }
    let mut ax = 0;
    for i in 1..3 {
        if n[i].abs() > n[ax].abs() {
            ax = i;
        }
    }
    (ax, n[ax] > 0.0) // negative = faces -axis; Houdini normal is the negated right-handed Newell
}
/// Projection component indices `(a, b)` for the dominant axis.
fn pair(axis: usize) -> (usize, usize) {
    match axis {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    }
}

impl Node<Detail> for UvUnwrap {
    fn type_name(&self) -> &'static str {
        "uvunwrap"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let tile = int(ctx.params, "tile")?;
        if !(1..=MAX_TILE).contains(&tile) {
            return Err(bad(format!("uvunwrap: `tile`: {tile} outside 1..={MAX_TILE}")));
        }
        let tile = tile as usize;
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let pos = src.p();
        let mut uv = vec![[0.0f32; 3]; src.vertex_point.len()];
        for (i, pr) in src.prims.iter().enumerate() {
            let (Ok(lo), Ok(hi)) = (usize::try_from(pr.verts.start), usize::try_from(pr.verts.end)) else { continue };
            let Some(vp) = src.vertex_point.get(lo..hi) else { continue };
            // usable corners: (vertex slot, position)
            let corners: Vec<(usize, [f64; 3])> = vp
                .iter()
                .enumerate()
                .filter_map(|(k, &pt)| {
                    let q = pos.get(usize::try_from(pt).ok()?)?;
                    let q = q.map(f64::from);
                    q.iter().all(|c| c.is_finite()).then_some((lo + k, q))
                })
                .collect();
            if corners.is_empty() {
                continue;
            }
            let pts: Vec<[f64; 3]> = corners.iter().map(|c| c.1).collect();
            let (axis, neg) = dominant(&pts);
            let (ia, ib) = pair(axis);
            let flat: Vec<[f64; 2]> = pts.iter().map(|q| [if neg { -q[ia] } else { q[ia] }, q[ib]]).collect();
            let (mut mn, mut mx) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for f in &flat {
                for j in 0..2 {
                    mn[j] = mn[j].min(f[j]);
                    mx[j] = mx[j].max(f[j]);
                }
            }
            let ext = (mx[0] - mn[0]).max(mx[1] - mn[1]);
            let cell = [(i % tile) as f64, ((i / tile) % tile) as f64];
            let t = tile as f64;
            for (c, f) in corners.iter().zip(&flat) {
                let local = |j: usize| if ext > 0.0 { (f[j] - mn[j]) / ext } else { 0.0 };
                let o = [(cell[0] + local(0)) / t, (cell[1] + local(1)) / t, 0.0].map(|x| x as f32);
                if let Some(slot) = uv.get_mut(c.0) {
                    *slot = if o.iter().all(|x| x.is_finite()) { o } else { [0.0; 3] };
                }
            }
        }
        let mut out = (**src).clone();
        out.add_attr(Attribute {
            name: "uv".into(),
            class: AttrClass::Vertex,
            data: AttrData::V3(uv),
            type_info: TypeInfo::None,
        });
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_graph::ParamSet;
    fn cook(d: Detail, p: ParamSet) -> Result<Arc<Detail>, CookError> {
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        new().cook(&ctx, &[Arc::new(d)])
    }
    fn defaults() -> ParamSet {
        new().params().iter().fold(ParamSet::new(), |p, s| p.with(&s.name, s.default.clone()))
    }
    fn tile(n: i32) -> ParamSet {
        defaults().with("tile", ParamValue::I(n))
    }
    fn uvs(d: &Detail) -> Vec<[f32; 3]> {
        match d.attr(AttrClass::Vertex, "uv").map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            other => panic!("no V3 vertex uv: {other:?}"),
        }
    }
    /// Unit box (corners +-0.5, 8 shared points), 6 outward-wound quads.
    fn cube() -> Detail {
        let mut d = Detail::default();
        d.add_points(8);
        let c = |b: u32, i: u32| if (i >> b) & 1 == 1 { 0.5 } else { -0.5 };
        *d.p_mut() = (0..8).map(|i| [c(2, i), c(1, i), c(0, i)]).collect();
        for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
            d.add_poly(&f, true);
        }
        d
    }
    fn quad(pts: [[f32; 3]; 4]) -> Detail {
        let mut d = Detail::default();
        d.add_points(4);
        *d.p_mut() = pts.to_vec();
        d.add_poly(&[0, 1, 2, 3], true);
        d
    }
    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "uvunwrap");
        assert_eq!(n.params().iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["tile"]);
        assert_eq!(defaults().int("tile"), Some(1));
    }
    #[test]
    fn z_facing_rect_projects_xy_normalised_and_keeps_aspect() {
        // 4x2 rect in z=7 facing +Z: u = x-min over longer side (4), v = y-min over 4 -> v in [0, .5]
        let q = quad([[0.0, 0.0, 7.0], [4.0, 0.0, 7.0], [4.0, 2.0, 7.0], [0.0, 2.0, 7.0]]);
        let uv = uvs(&cook(q, defaults()).expect("cooks"));
        assert_eq!(uv, [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.5, 0.0], [0.0, 0.5, 0.0]]);
    }
    #[test]
    fn back_facing_prim_mirrors_u_and_each_axis_uses_its_pair() {
        // same rect wound the other way faces -Z: u mirrored -> corner (4,0) gets u=0, (0,0) gets u=1
        let q = quad([[0.0, 2.0, 7.0], [4.0, 2.0, 7.0], [4.0, 0.0, 7.0], [0.0, 0.0, 7.0]]);
        let uv = uvs(&cook(q, defaults()).expect("cooks"));
        assert_eq!(uv, [[1.0, 0.5, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]);
        // +X-facing quad in x=3 (ccw seen from +X: Y then Z): (u,v)=(y,z)
        let x = quad([[3.0, 0.0, 0.0], [3.0, 1.0, 0.0], [3.0, 1.0, 1.0], [3.0, 0.0, 1.0]]);
        assert_eq!(
            uvs(&cook(x, defaults()).expect("cooks")),
            [[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]
        );
        // +Y-facing quad: (u,v)=(z,x); ccw seen from +Y is z then x
        let y = quad([[0.0, 5.0, 0.0], [0.0, 5.0, 1.0], [1.0, 5.0, 1.0], [1.0, 5.0, 0.0]]);
        assert_eq!(
            uvs(&cook(y, defaults()).expect("cooks")),
            [[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]
        );
    }
    #[test]
    fn box_each_face_is_a_full_unit_island_and_tiles_place_them_in_cells() {
        let out = cook(cube(), tile(1)).expect("cooks");
        let uv = uvs(&out);
        assert_eq!(uv.len(), 24);
        for pr in &out.prims {
            let f: Vec<[f32; 3]> = pr.verts.clone().map(|v| uv[v as usize]).collect();
            let mut corners: Vec<[i32; 2]> = f.iter().map(|c| [c[0] as i32, c[1] as i32]).collect();
            assert!(f.iter().all(|c| (c[0] == 0.0 || c[0] == 1.0) && (c[1] == 0.0 || c[1] == 1.0) && c[2] == 0.0));
            corners.sort_unstable();
            assert_eq!(corners, [[0, 0], [0, 1], [1, 0], [1, 1]], "each face covers the unit square exactly");
        }
        // tile 3: prim i in cell (i%3, i/3); uv = (cell + local)/3
        let t3 = uvs(&cook(cube(), tile(3)).expect("cooks"));
        for (i, pr) in out.prims.iter().enumerate() {
            let (cx, cy) = ((i % 3) as f32, (i / 3) as f32);
            for v in pr.verts.clone() {
                let l = uv[v as usize];
                assert_eq!(t3[v as usize], [(cx + l[0]) / 3.0, (cy + l[1]) / 3.0, 0.0], "prim {i}");
            }
        }
        // distinct cells -> islands' bboxes are disjoint (interiors)
        assert!(t3.iter().all(|c| c[0] >= 0.0 && c[0] <= 1.0 && c[1] >= 0.0 && c[1] <= 1.0));
        // wrap: tile 1 puts prim 5 in cell (0,0) again; tile 2 puts prim 4 back on cell (0,0)
        let t2 = uvs(&cook(cube(), tile(2)).expect("cooks"));
        let v4 = out.prims[4].verts.start as usize;
        assert_eq!(t2[v4], [uv[v4][0] / 2.0, uv[v4][1] / 2.0, 0.0]);
    }
    #[test]
    fn slanted_prim_picks_dominant_axis_and_degenerate_falls_back_to_z() {
        // normal (1,1.5,0)-ish: dominant y -> (z, x). triangle in plane y = 1.5*(-x)... use simple tilt: plane with n ~ (0.3,1,0)
        let tri = {
            let mut d = Detail::default();
            d.add_points(3);
            *d.p_mut() = vec![[0.0, 0.0, 0.0], [0.0, 0.0, 2.0], [2.0, -0.6, 0.0]];
            d.add_poly(&[0, 1, 2], true);
            d
        };
        let uv = uvs(&cook(tri, defaults()).expect("cooks"));
        // n = (b-a)x(c-a) = (0,0,2)x(2,-.6,0) = (1.2, 4, 0): +Y dominant; (u,v) = (z,x) normalised by 2
        assert_eq!(uv, [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        // zero-area prim (collinear) -> z axis, u=x, v=y
        let line = quad([[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [2.0, 2.0, 0.0], [3.0, 3.0, 0.0]]);
        let uv = uvs(&cook(line, defaults()).expect("cooks"));
        assert!((uv[3][0] - 1.0).abs() < 1e-7 && (uv[3][1] - 1.0).abs() < 1e-7 && uv[0] == [0.0; 3]);
    }
    #[test]
    fn uv_is_vertex_class_and_leaves_everything_else_alone() {
        let c = cube();
        let out = cook(c.clone(), defaults()).expect("cooks");
        let a = out.attr(AttrClass::Vertex, "uv").expect("vertex uv");
        assert_eq!((a.class, a.data.len(), a.type_info), (AttrClass::Vertex, 24, TypeInfo::None));
        let mut back = (*out).clone();
        back.attrs.retain(|x| x.name != "uv");
        assert_eq!(back, c);
    }
    #[test]
    fn hostile_params_and_geometry() {
        for v in [0, -1, MAX_TILE + 1] {
            assert!(matches!(cook(cube(), tile(v)), Err(CookError::BadParam(_))), "{v}");
        }
        assert!(matches!(cook(cube(), defaults().with("tile", ParamValue::F(1.0))), Err(CookError::BadParam(_))));
        assert!(cook(cube(), tile(MAX_TILE)).is_ok());
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &defaults(), log: &|_| {} };
        assert_eq!(new().cook(&ctx, &[]).expect("cooks").point_count, 0);
        assert_eq!(cook(Detail::default(), defaults()).expect("cooks").vertex_point.len(), 0);
        let mut h = cube();
        h.add_poly(&[0, 0, 0, 0], true); // zero area
        h.add_poly(&[0, 1], false); // polyline
        h.add_poly(&[3], false); // single corner
        h.vertex_point[0] = 99; // dangling
        h.p_mut()[7] = [f32::NAN, 0.0, f32::INFINITY];
        h.p_mut()[6] = [f32::MAX, -f32::MAX, f32::MAX];
        let out = cook(h, tile(2)).expect("cooks");
        let uv = uvs(&out);
        assert_eq!(uv.len(), out.vertex_point.len());
        assert!(uv.iter().flatten().all(|c| c.is_finite()));
        assert_eq!(uv[0], [0.0; 3]);
    }
    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/uv.json"))
            .expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert_eq!(uvs(&out).len(), 24);
        assert_eq!(crate::registry().create("uvunwrap").map(|n| n.type_name()), Some("uvunwrap"));
    }
}
