//! PolyFill SOP (token `polyfill`): close the open boundary loops (holes) of a polygon mesh.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! * Only CLOSED polys with >= 3 in-range corners count. A directed edge `u -> v` is a boundary edge when no poly has `v -> u`;
//!   an edge used twice in the same direction (non-manifold) is ignored. Boundary edges are reversed and chained into loops
//!   (at a pinch point the first unused edge is taken); a chain that does not close, repeats a point or has < 3 points is
//!   left open. Each loop is a hole, filled with the winding that is consistent with its neighbours (the filled mesh keeps
//!   one orientation, a watertight outward shell stays outward).
//! * `fillmode` menu `single` (0, default: one n-gon per hole) | `fan` (1: new centre point = corner mean, triangles
//!   `[c, v_i, v_i+1]`) | `quads` (2: new centre, quads `[c, v_2k, v_2k+1, v_2k+2]`, odd tail = one triangle). A 3-corner hole is
//!   always one triangle; a 4-corner hole is one quad in `single` / `quads`.
//! * `maxedges` I 0: holes with more boundary edges than this are skipped (0 = no limit). `filledgroup` S `filled`: PRIM group of
//!   the new polys (empty = none; no whitespace). Nothing to fill = the input `Arc` is shared.
//! * Everything else untouched; new points and vertices default-fill their attributes (the centre point gets only `P`).
//!   NOT modelled (DEFERRED): `holes`-in-polygon (inner rings of one prim), planar-projection ear clipping for concave holes
//!   (the single n-gon is simply the loop), smooth-curvature fill, point-group restriction.
use super::meshutil::{V3, bad, closed_poly_pts};
use crate::SopNode;
use hexe_geo::{AttrClass, Detail, Group};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const MODE_SINGLE: u32 = 0;
const MODE_FAN: u32 = 1;
const MODE_QUADS: u32 = 2;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("fillmode", ParamValue::Menu(MODE_SINGLE)).label("Fill Mode").menu(&[
            ("single", "Single Polygon"),
            ("fan", "Triangle Fan"),
            ("quads", "Quads"),
        ]),
        ParamSpec::new("maxedges", ParamValue::I(0)).label("Max Edges (0 = no limit)").range(0.0, 100.0),
        ParamSpec::new("filledgroup", ParamValue::S("filled".into())).label("Filled Prims Group"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(PolyFill { specs: spec() })
}
struct PolyFill {
    specs: Vec<ParamSpec>,
}

/// Hole loops (point-id cycles in the winding that matches their neighbours), deterministic order.
fn holes(d: &Detail) -> Vec<Vec<u32>> {
    let mut count: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for pr in &d.prims {
        let Some(pts) = closed_poly_pts(d, pr) else { continue };
        for k in 0..pts.len() {
            *count.entry((pts[k] as u32, pts[(k + 1) % pts.len()] as u32)).or_insert(0) += 1;
        }
    }
    // reversed boundary edges v -> u, grouped by start point
    let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (&(u, v), &c) in &count {
        if c == 1 && u != v && !count.contains_key(&(v, u)) {
            out.entry(v).or_default().push(u);
        }
    }
    let mut used: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut loops = Vec::new();
    let starts: Vec<(u32, u32)> = out.iter().flat_map(|(&a, bs)| bs.iter().map(move |&b| (a, b))).collect();
    for (a, b) in starts {
        if used.contains(&(a, b)) {
            continue;
        }
        used.insert((a, b));
        let mut lp = vec![a];
        let mut cur = b;
        let mut closed = true;
        while cur != a {
            lp.push(cur);
            let Some(nx) = out.get(&cur).and_then(|v| v.iter().copied().find(|&n| !used.contains(&(cur, n)))) else {
                closed = false;
                break;
            };
            used.insert((cur, nx));
            cur = nx;
            if lp.len() > count.len() + 1 {
                closed = false;
                break;
            }
        }
        let distinct: BTreeSet<u32> = lp.iter().copied().collect();
        if closed && lp.len() >= 3 && distinct.len() == lp.len() {
            loops.push(lp);
        }
    }
    loops
}

impl Node<Detail> for PolyFill {
    fn type_name(&self) -> &'static str {
        "polyfill"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let mode = match p.menu("fillmode") {
            Some(v @ 0..=2) => v,
            Some(v) => return Err(bad(format!("polyfill: `fillmode`: menu index {v} out of range 0..=2"))),
            None => return Err(bad("polyfill: `fillmode`: missing or not a menu")),
        };
        let maxedges = match p.int("maxedges") {
            Some(v) if v >= 0 => v as usize,
            _ => return Err(bad("polyfill: `maxedges`: missing, not an int or negative")),
        };
        let gname = match p.string("filledgroup") {
            Some(s) if !s.chars().any(char::is_whitespace) => s.to_string(),
            _ => return Err(bad("polyfill: `filledgroup`: missing or contains whitespace")),
        };
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let todo: Vec<Vec<u32>> = holes(src).into_iter().filter(|h| maxedges == 0 || h.len() <= maxedges).collect();
        if todo.is_empty() {
            return Ok(Arc::clone(src));
        }
        let mut g = (**src).clone();
        let first_new = g.prim_count();
        let pos = super::meshutil::positions(&g);
        for h in &todo {
            let n = h.len();
            let single = n == 3 || mode == MODE_SINGLE || (n == 4 && mode == MODE_QUADS);
            if single {
                g.add_poly(h, true);
                continue;
            }
            let mean: V3 = h.iter().fold([0.0; 3], |a, &i| {
                let q = pos[i as usize];
                [a[0] + q[0] / n as f64, a[1] + q[1] / n as f64, a[2] + q[2] / n as f64]
            });
            let c = g.add_points(1).start;
            let at = g.p_mut();
            let slot = at.len() - 1;
            at[slot] = [mean[0] as f32, mean[1] as f32, mean[2] as f32];
            if mode == MODE_FAN {
                for k in 0..n {
                    g.add_poly(&[c, h[k], h[(k + 1) % n]], true);
                }
            } else {
                let mut k = 0;
                while k < n {
                    if k + 2 <= n {
                        g.add_poly(&[c, h[k], h[(k + 1) % n], h[(k + 2) % n]], true);
                    } else {
                        g.add_poly(&[c, h[k], h[(k + 1) % n]], true);
                    }
                    k += 2;
                }
            }
        }
        if !gname.is_empty() {
            let total = g.prim_count() as usize;
            let mask: Vec<bool> = (0..total).map(|i| i >= first_new as usize).collect();
            g.groups.retain(|x| !(x.class == AttrClass::Prim && x.name == gname));
            g.groups.push(Group { name: gname, class: AttrClass::Prim, mask });
        }
        Ok(Arc::new(g))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit;
    use super::super::surfutil::testkit::{curve, signed_volume};
    use super::*;
    use crate::load_scene;
    use hexe_graph::ParamSet;

    fn run(d: Detail, p: ParamSet) -> Result<Arc<Detail>, CookError> {
        let n = new();
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        n.cook(&ctx, &[Arc::new(d)])
    }
    fn defaults() -> ParamSet {
        testkit::defaults(&*new())
    }
    fn open_box() -> Detail {
        // cube without its +Y face (faces 3 = [2,6,7,3] in testkit order)
        let c = testkit::cube();
        let keep: Vec<_> = c.prims.iter().enumerate().filter(|(i, _)| *i != 3).map(|(_, p)| p.clone()).collect();
        let mut d = Detail::default();
        d.add_points(8);
        *d.p_mut() = c.p().to_vec();
        for p in keep {
            let ids: Vec<u32> = c.vertex_point[p.verts.start as usize..p.verts.end as usize].to_vec();
            d.add_poly(&ids, true);
        }
        d
    }
    /// n-gon prism, side quads only (open top and bottom), outward Houdini winding.
    fn prism(n: u32) -> Detail {
        let mut d = Detail::default();
        d.add_points(2 * n);
        *d.p_mut() = (0..2 * n)
            .map(|i| {
                let a = std::f32::consts::TAU * (i % n) as f32 / n as f32;
                [a.cos(), if i < n { 0.0 } else { 1.0 }, -a.sin()]
            })
            .collect();
        for i in 0..n {
            let j = (i + 1) % n;
            d.add_poly(&[i, n + i, n + j, j], true);
        }
        d
    }

    #[test]
    fn box_missing_its_top_is_closed_by_one_outward_quad() {
        let d = run(open_box(), defaults()).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (8, 6));
        assert!((signed_volume(&d) - 1.0).abs() < 1e-9, "{}", signed_volume(&d));
        let g = d.group(AttrClass::Prim, "filled").expect("group");
        assert_eq!(g.mask.iter().filter(|&&b| b).count(), 1);
        assert!(g.mask[5]);
    }

    #[test]
    fn fan_and_quads_add_a_centre_point_and_keep_the_volume() {
        let f = run(open_box(), defaults().with("fillmode", ParamValue::Menu(MODE_FAN))).expect("cooks");
        assert_eq!((f.point_count, f.prim_count()), (9, 5 + 4));
        assert!(f.prims[5..].iter().all(|p| p.verts.len() == 3));
        assert_eq!(f.p()[8], [0.0, 0.5, 0.0]);
        assert!((signed_volume(&f) - 1.0).abs() < 1e-9);
        // a 4-hole is one quad in quads mode
        let q = run(open_box(), defaults().with("fillmode", ParamValue::Menu(MODE_QUADS))).expect("cooks");
        assert_eq!((q.point_count, q.prim_count()), (8, 6));
        // hexagon: centre + 3 quads; pentagon: centre + 2 quads + 1 tri
        let hex = run(prism(6), defaults().with("fillmode", ParamValue::Menu(MODE_QUADS))).expect("cooks");
        assert_eq!((hex.point_count, hex.prim_count()), (12 + 2, 6 + 2 * 3));
        assert!(hex.prims[6..].iter().all(|p| p.verts.len() == 4));
        let pent = run(prism(5), defaults().with("fillmode", ParamValue::Menu(MODE_QUADS))).expect("cooks");
        let sizes: Vec<usize> = pent.prims[5..].iter().map(|p| p.verts.len()).collect();
        assert_eq!(sizes, vec![4, 4, 3, 4, 4, 3]);
        assert!(signed_volume(&pent) > 0.0);
    }

    #[test]
    fn prism_with_two_holes_single_mode_is_a_watertight_outward_solid() {
        let d = run(prism(6), defaults()).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (12, 8));
        let want = 1.5 * 3.0f64.sqrt(); // regular hexagon area (R=1) x height 1
        assert!((signed_volume(&d) - want).abs() < 1e-5, "{} vs {want}", signed_volume(&d));
        // maxedges below the hole size skips both
        let s = run(prism(6), defaults().with("maxedges", ParamValue::I(5))).expect("cooks");
        assert_eq!(s.prim_count(), 6);
        assert!(s.group(AttrClass::Prim, "filled").is_none());
    }

    #[test]
    fn closed_meshes_open_polys_and_non_manifold_edges_pass_through_shared() {
        let cube = testkit::cube();
        let a = Arc::new(cube.clone());
        let n = new();
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &defaults(), log: &|_| {} };
        assert!(Arc::ptr_eq(&n.cook(&ctx, std::slice::from_ref(&a)).expect("cooks"), &a));
        let o = Arc::new(curve(&[[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]], false));
        assert!(Arc::ptr_eq(&n.cook(&ctx, std::slice::from_ref(&o)).expect("cooks"), &o));
        // three triangles sharing edge 0->1 (same direction twice): not a hole
        let mut nm = Detail::default();
        nm.add_points(5);
        nm.add_poly(&[0, 1, 2], true);
        nm.add_poly(&[0, 1, 3], true);
        nm.add_poly(&[0, 1, 4], true);
        let out = run(nm, defaults()).expect("cooks");
        assert!(out.prim_count() >= 3);
    }

    #[test]
    fn attributes_and_existing_groups_survive_and_the_group_is_replaceable() {
        let mut d = open_box();
        d.groups.push(Group { name: "side".into(), class: AttrClass::Prim, mask: vec![true; 5] });
        let o = run(d, defaults().with("filledgroup", ParamValue::S("cap".into()))).expect("cooks");
        assert_eq!(
            o.group(AttrClass::Prim, "side").map(|g| g.mask.clone()),
            Some(vec![true, true, true, true, true, false])
        );
        assert_eq!(o.group(AttrClass::Prim, "cap").map(|g| g.mask.iter().filter(|&&b| b).count()), Some(1));
        let none = run(open_box(), defaults().with("filledgroup", ParamValue::S(String::new()))).expect("cooks");
        assert!(none.groups.is_empty());
    }

    #[test]
    fn hostile_params_and_geometry() {
        for (k, v) in [
            ("fillmode", ParamValue::Menu(3)),
            ("maxedges", ParamValue::I(-1)),
            ("maxedges", ParamValue::F(1.0)),
            ("filledgroup", ParamValue::S("a b".into())),
            ("filledgroup", ParamValue::I(1)),
        ] {
            assert!(matches!(run(open_box(), defaults().with(k, v)), Err(CookError::BadParam(_))), "{k}");
        }
        assert_eq!(run(Detail::default(), defaults()).expect("cooks").prim_count(), 0);
        let mut d = open_box();
        d.vertex_point[0] = 99; // dangling corner: that prim is ignored, never panics
        assert!(run(d, defaults()).is_ok());
        let mut deg = Detail::default();
        deg.add_points(3);
        deg.add_poly(&[0, 0, 0], true);
        assert!(run(deg, defaults()).is_ok());
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/polyfill.json"))
            .expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert!(out.group(AttrClass::Prim, "filled").is_some());
        assert_eq!(crate::registry().create("polyfill").map(|n| n.type_name()), Some("polyfill"));
    }
}
