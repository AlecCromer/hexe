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
//! * QB additions: `fillmode` 3 = `ears` (concave-safe ear clipping in the loop's own plane: n-2 triangles, winding of the
//!   loop, a degenerate loop falls back to a fan); `ptgroup` S (POINT group, space-separated union; empty = all): a hole is
//!   filled only when EVERY loop point is in the group (unknown group = `Failed`); `smooth` B off + `smoothscale` F 1: fan /
//!   quads centre = `mean_rim + smoothscale * (mean_rim - mean_ring)` where `mean_ring` is the mean of the points adjacent to
//!   the loop but not on it (linear tangent-continuing extrapolation of the surrounding surface; no neighbours = flat mean).
//!   NOT modelled (DEFERRED): inner rings of one prim (hexe-geo has no polygon-with-holes data model).
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
const MODE_EARS: u32 = 3;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("fillmode", ParamValue::Menu(MODE_SINGLE)).label("Fill Mode").menu(&[
            ("single", "Single Polygon"),
            ("fan", "Triangle Fan"),
            ("quads", "Quads"),
            ("ears", "Concave Triangulation"),
        ]),
        ParamSpec::new("ptgroup", ParamValue::S(String::new())).label("Point Group"),
        ParamSpec::new("smooth", ParamValue::B(false)).label("Smooth Centre"),
        ParamSpec::new("smoothscale", ParamValue::F(1.0)).label("Smooth Scale").range(0.0, 4.0),
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
    boundary_loops(d, None)
}

/// `holes` restricted to the prims flagged in `mask` (None = all): the open boundary of exactly that prim subset, in the
/// winding that closes it outward (used by `clip`'s `capclip`).
pub(super) fn boundary_loops(d: &Detail, mask: Option<&[bool]>) -> Vec<Vec<u32>> {
    let mut count: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for (qi, pr) in d.prims.iter().enumerate() {
        if mask.is_some_and(|m| !m.get(qi).copied().unwrap_or(false)) {
            continue;
        }
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

/// Ear clipping of the loop `h` in its own plane: triangles in loop order (n-2 of them for a simple loop).
fn ear_clip(h: &[u32], pos: &[V3]) -> Vec<[u32; 3]> {
    let sub = |a: V3, b: V3| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: V3, b: V3| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let dot = |a: V3, b: V3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let q = |i: u32| pos[i as usize];
    // polygon area vector (right-hand sum of cross products, origin-independent via the first corner)
    let mut nrm = [0.0; 3];
    for k in 1..h.len().saturating_sub(1) {
        let c = cross(sub(q(h[k]), q(h[0])), sub(q(h[k + 1]), q(h[0])));
        nrm = [nrm[0] + c[0], nrm[1] + c[1], nrm[2] + c[2]];
    }
    let mut rest: Vec<u32> = h.to_vec();
    let mut out = Vec::with_capacity(h.len().saturating_sub(2));
    while rest.len() > 3 {
        let m = rest.len();
        let ear = (0..m).find(|&i| {
            let (a, b, c) = (rest[(i + m - 1) % m], rest[i], rest[(i + 1) % m]);
            if dot(cross(sub(q(b), q(a)), sub(q(c), q(b))), nrm) <= 0.0 {
                return false; // reflex or flat corner
            }
            !rest.iter().any(|&o| {
                if o == a || o == b || o == c {
                    return false;
                }
                let p = q(o);
                dot(cross(sub(q(b), q(a)), sub(p, q(a))), nrm) >= 0.0
                    && dot(cross(sub(q(c), q(b)), sub(p, q(b))), nrm) >= 0.0
                    && dot(cross(sub(q(a), q(c)), sub(p, q(c))), nrm) >= 0.0
            })
        });
        let i = ear.unwrap_or(0);
        out.push([rest[(i + m - 1) % m], rest[i], rest[(i + 1) % m]]);
        rest.remove(i);
    }
    out.push([rest[0], rest[1], rest[2]]);
    out
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
            Some(v @ 0..=3) => v,
            Some(v) => return Err(bad(format!("polyfill: `fillmode`: menu index {v} out of range 0..=3"))),
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
        let smooth = p.toggle("smooth").ok_or_else(|| bad("polyfill: `smooth`: missing or not a toggle"))?;
        let smoothscale = match p.float("smoothscale") {
            Some(v) if v.is_finite() && v >= 0.0 => f64::from(v),
            _ => return Err(bad("polyfill: `smoothscale`: missing, non-finite or negative")),
        };
        let ptgroup =
            p.string("ptgroup").ok_or_else(|| bad("polyfill: `ptgroup`: missing or not a string"))?.to_string();
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let in_group: Option<Vec<bool>> = if ptgroup.split_whitespace().next().is_none() {
            None
        } else {
            let mut sel = vec![false; src.point_count as usize];
            for n in ptgroup.split_whitespace() {
                let g = src
                    .group(AttrClass::Point, n)
                    .ok_or_else(|| CookError::Failed(format!("polyfill: unknown point group `{n}`")))?;
                for (s, m) in sel.iter_mut().zip(&g.mask) {
                    *s |= *m;
                }
            }
            Some(sel)
        };
        let todo: Vec<Vec<u32>> = holes(src)
            .into_iter()
            .filter(|h| maxedges == 0 || h.len() <= maxedges)
            .filter(|h| {
                in_group.as_ref().is_none_or(|s| h.iter().all(|&i| s.get(i as usize).copied().unwrap_or(false)))
            })
            .collect();
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
            if mode == MODE_EARS {
                for t in ear_clip(h, &pos) {
                    g.add_poly(&t, true);
                }
                continue;
            }
            let mean: V3 = h.iter().fold([0.0; 3], |a, &i| {
                let q = pos[i as usize];
                [a[0] + q[0] / n as f64, a[1] + q[1] / n as f64, a[2] + q[2] / n as f64]
            });
            let mut centre = mean;
            if smooth {
                let on_loop: BTreeSet<u32> = h.iter().copied().collect();
                let mut ring: BTreeSet<u32> = BTreeSet::new();
                for pr in &src.prims {
                    let Some(pts) = closed_poly_pts(src, pr) else { continue };
                    for k in 0..pts.len() {
                        let (a, b) = (pts[k] as u32, pts[(k + 1) % pts.len()] as u32);
                        if on_loop.contains(&a) && !on_loop.contains(&b) {
                            ring.insert(b);
                        }
                        if on_loop.contains(&b) && !on_loop.contains(&a) {
                            ring.insert(a);
                        }
                    }
                }
                if !ring.is_empty() {
                    let rm: V3 = ring.iter().fold([0.0; 3], |a, &i| {
                        let q = pos[i as usize];
                        let w = 1.0 / ring.len() as f64;
                        [a[0] + q[0] * w, a[1] + q[1] * w, a[2] + q[2] * w]
                    });
                    for k in 0..3 {
                        centre[k] = mean[k] + smoothscale * (mean[k] - rm[k]);
                    }
                }
            }
            let c = g.add_points(1).start;
            let at = g.p_mut();
            let slot = at.len() - 1;
            at[slot] = [centre[0] as f32, centre[1] as f32, centre[2] as f32];
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

    /// Side quads only of the prism over the footprint `pts` (x, y in the plane), height 1, same winding as `prism`.
    fn footprint_prism(pts: &[[f32; 2]]) -> Detail {
        let n = pts.len() as u32;
        let mut d = Detail::default();
        d.add_points(2 * n);
        *d.p_mut() = (0..2 * n)
            .map(|i| {
                let q = pts[(i % n) as usize];
                [q[0], if i < n { 0.0 } else { 1.0 }, -q[1]]
            })
            .collect();
        for i in 0..n {
            let j = (i + 1) % n;
            d.add_poly(&[i, n + i, n + j, j], true);
        }
        d
    }
    fn tri_area(d: &Detail, prim: usize) -> f64 {
        let pos = super::super::meshutil::positions(d);
        let pr = &d.prims[prim];
        let ids: Vec<usize> =
            d.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&i| i as usize).collect();
        let (a, b, c) = (pos[ids[0]], pos[ids[1]], pos[ids[2]]);
        let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let x = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        0.5 * (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt()
    }

    #[test]
    fn ears_fill_concave_l_holes_with_exact_area_and_a_watertight_outward_solid() {
        // L footprint (area 3), both caps open: ears -> 4 + 4 triangles, volume 3 * height 1
        let l = footprint_prism(&[[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]]);
        let d = run(l.clone(), defaults().with("fillmode", ParamValue::Menu(MODE_EARS))).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (12, 6 + 8));
        let (mut top, mut bot) = (0.0, 0.0);
        for i in 6..14 {
            assert_eq!(d.prims[i].verts.len(), 3);
            let y = d.p()[d.vertex_point[d.prims[i].verts.start as usize] as usize][1];
            if y > 0.5 { top += tri_area(&d, i) } else { bot += tri_area(&d, i) }
        }
        assert!((top - 3.0).abs() < 1e-5 && (bot - 3.0).abs() < 1e-5, "{top} {bot}");
        let v = signed_volume(&d);
        assert!((v - 3.0).abs() < 1e-5, "volume {v}");
        // a convex hole -> still n-2 triangles; 3-hole stays one triangle
        let h = run(prism(6), defaults().with("fillmode", ParamValue::Menu(MODE_EARS))).expect("cooks");
        assert_eq!(h.prim_count(), 6 + 2 * 4);
        assert!((signed_volume(&h) - 1.5 * 3.0_f64.sqrt()).abs() < 1e-5);
        let t = run(prism(3), defaults().with("fillmode", ParamValue::Menu(MODE_EARS))).expect("cooks");
        assert_eq!(t.prim_count(), 3 + 2);
        // single mode keeps the L as ONE polygon
        let s = run(l, defaults()).expect("cooks");
        assert_eq!(s.prim_count(), 6 + 2);
    }

    #[test]
    fn ptgroup_restricts_filling_to_holes_whose_points_are_all_grouped() {
        let mut d = prism(4);
        d.groups.push(Group { name: "top".into(), class: AttrClass::Point, mask: (0..8).map(|i| i >= 4).collect() });
        let top = run(d.clone(), defaults().with("ptgroup", ParamValue::S("top".into()))).expect("cooks");
        assert_eq!(top.prim_count(), 4 + 1);
        let all = run(d.clone(), defaults()).expect("cooks");
        assert_eq!(all.prim_count(), 4 + 2);
        // a group covering only part of a loop fills nothing -> input shared
        d.groups.push(Group { name: "part".into(), class: AttrClass::Point, mask: (0..8).map(|i| i == 5).collect() });
        let none = run(d.clone(), defaults().with("ptgroup", ParamValue::S("part".into()))).expect("cooks");
        assert_eq!(none.prim_count(), 4);
        assert!(matches!(run(d, defaults().with("ptgroup", ParamValue::S("nope".into()))), Err(CookError::Failed(_))));
    }

    #[test]
    fn smooth_centre_extrapolates_the_surrounding_surface() {
        // unit hex prism: top rim y=1, neighbours y=0 -> top centre y=2; bottom rim y=0, neighbours y=1 -> bottom centre y=-1
        let fan = defaults().with("fillmode", ParamValue::Menu(MODE_FAN));
        let flat = run(prism(6), fan.clone()).expect("cooks");
        let sm = run(prism(6), fan.clone().with("smooth", ParamValue::B(true))).expect("cooks");
        let ys = |d: &Detail| {
            let mut v: Vec<f32> = d.p()[12..].iter().map(|q| q[1]).collect();
            v.sort_by(f32::total_cmp);
            v
        };
        assert_eq!(ys(&flat), vec![0.0, 1.0]);
        assert_eq!(ys(&sm), vec![-1.0, 2.0]);
        assert!(sm.p()[12..].iter().all(|q| q[0].abs() < 1e-6 && q[2].abs() < 1e-6));
        let half = run(prism(6), fan.with("smooth", ParamValue::B(true)).with("smoothscale", ParamValue::F(0.5)))
            .expect("cooks");
        assert_eq!(ys(&half), vec![-0.5, 1.5]);
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
            ("fillmode", ParamValue::Menu(4)),
            ("smoothscale", ParamValue::F(-1.0)),
            ("smoothscale", ParamValue::F(f32::NAN)),
            ("smooth", ParamValue::I(1)),
            ("ptgroup", ParamValue::I(1)),
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
