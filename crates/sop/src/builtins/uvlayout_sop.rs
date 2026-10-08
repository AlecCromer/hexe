//! UVLayout SOP (token `uvlayout`, "lite"): packs the UV islands of the vertex attribute `uv` by bounding box, NO rotation.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; closed-form oracle only)
//! * Input needs a vertex `uv` (V2 or V3, e.g. from `uvproject` / `uvunwrap`), else `Failed`. Output `uv` is V3
//!   (type None); `uv.z` is kept from the input (V2 -> 0).
//! * Island = connected component of prims where two prims are joined when they share a point AND carry the bit-identical
//!   `uv` at that point (a point whose prims disagree on uv is a cut). Prims with no corners own no island.
//! * Packing (shelf, deterministic): islands sorted by bbox height descending (ties: lowest first-prim index). Target
//!   row width `W = max(widest island, sqrt(sum (w + padding) * (h + padding)))`. Islands fill a row left to right; an island that
//!   would end past `W` opens a new row above (row height = its first, tallest island). `padding` (>= 0) separates
//!   neighbours in both axes; there is NO margin around the whole layout. Each island is translated (never rotated or
//!   scaled individually) so its bbox min lands on its slot.
//! * `fit` (default on): the whole layout is then uniformly scaled by `1 / max(layout width, layout height)` so it
//!   fills `[0, 1]^2` along its longer side (aspect kept; `padding` scales with it). Off: slots stay in input units.
//! * Vertices outside every island (or with non-finite uv) keep their input value.
//!
//! Output = the input with vertex `uv` replaced; everything else untouched. Hostile: negative / non-finite `padding`,
//! non-toggle `fit` -> `BadParam`; island count above `MAX_ISLANDS` -> `Failed`.
use super::meshutil::{bad, failed, float, toggle};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const OP: &str = "uvlayout";
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
/// Cap on islands (AGENTS.md section 4); every prim is at most one island.
const MAX_ISLANDS: usize = 1 << 22;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("padding", ParamValue::F(0.01)).label("Padding").range(0.0, 1.0),
        ParamSpec::new("fit", ParamValue::B(true)).label("Fit To Unit Square"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(UvLayout { specs: spec() })
}
struct UvLayout {
    specs: Vec<ParamSpec>,
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while let Some(&p) = parent.get(i) {
        if p == i {
            break;
        }
        let gp = parent.get(p).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(i) {
            *slot = gp;
        }
        i = gp;
    }
    i
}
fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    // lower root wins -> island id order follows the first prim
    if let Some(slot) = parent.get_mut(ra.max(rb)) {
        *slot = ra.min(rb);
    }
}
/// Input uv as V3 (V2 widened), `None` when absent or another type.
fn input_uv(src: &Detail) -> Option<Vec<[f32; 3]>> {
    match src.attr(AttrClass::Vertex, "uv").map(|a| &a.data)? {
        AttrData::V3(v) => Some(v.clone()),
        AttrData::V2(v) => Some(v.iter().map(|c| [c[0], c[1], 0.0]).collect()),
        _ => None,
    }
}

struct Island {
    /// vertex slots
    verts: Vec<usize>,
    min: [f64; 2],
    w: f64,
    h: f64,
    first_prim: usize,
}

impl Node<Detail> for UvLayout {
    fn type_name(&self) -> &'static str {
        "uvlayout"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let padding = float(ctx.params, "padding")?;
        if padding < 0.0 {
            return Err(bad(format!("{OP}: `padding` must be >= 0")));
        }
        let fit = toggle(ctx.params, "fit")?;
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let uv = input_uv(src).ok_or_else(|| failed(OP, "input has no vertex `uv` (V2 or V3)"))?;
        if uv.len() != src.vertex_point.len() {
            return Err(failed(OP, "vertex `uv` length does not match the vertex count"));
        }
        // prim -> vertex slot range (valid only)
        let ranges: Vec<Option<(usize, usize)>> = src
            .prims
            .iter()
            .map(|pr| {
                let (lo, hi) = (usize::try_from(pr.verts.start).ok()?, usize::try_from(pr.verts.end).ok()?);
                (lo < hi && hi <= uv.len()).then_some((lo, hi))
            })
            .collect();
        // union prims through (point, uv-bits) corners
        let mut parent: Vec<usize> = (0..src.prims.len()).collect();
        let mut corners: Vec<(u32, [u32; 3], usize)> = Vec::new();
        for (i, r) in ranges.iter().enumerate() {
            let Some((lo, hi)) = *r else { continue };
            for v in lo..hi {
                if let (Some(&pt), Some(c)) = (src.vertex_point.get(v), uv.get(v)) {
                    corners.push((pt, c.map(f32::to_bits), i));
                }
            }
        }
        corners.sort_unstable();
        for w in corners.windows(2) {
            if let [a, b] = w {
                if a.0 == b.0 && a.1 == b.1 {
                    union(&mut parent, a.2, b.2);
                }
            }
        }
        // islands in order of first prim
        let mut slot_of_root: Vec<Option<usize>> = vec![None; src.prims.len()];
        let mut islands: Vec<Island> = Vec::new();
        for (i, r) in ranges.iter().enumerate() {
            let Some((lo, hi)) = *r else { continue };
            let root = find(&mut parent, i);
            let idx = match slot_of_root.get(root).copied().flatten() {
                Some(k) => k,
                None => {
                    if islands.len() >= MAX_ISLANDS {
                        return Err(failed(OP, "too many islands"));
                    }
                    islands.push(Island { verts: Vec::new(), min: [f64::INFINITY; 2], w: 0.0, h: 0.0, first_prim: i });
                    if let Some(s) = slot_of_root.get_mut(root) {
                        *s = Some(islands.len() - 1);
                    }
                    islands.len() - 1
                }
            };
            if let Some(isl) = islands.get_mut(idx) {
                isl.verts.extend(lo..hi);
            }
        }
        for isl in &mut islands {
            isl.verts.sort_unstable();
            isl.verts.dedup();
            let (mut mn, mut mx) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for c in isl.verts.iter().filter_map(|&v| uv.get(v)) {
                if c[0].is_finite() && c[1].is_finite() {
                    for j in 0..2 {
                        mn[j] = mn[j].min(f64::from(c[j]));
                        mx[j] = mx[j].max(f64::from(c[j]));
                    }
                }
            }
            if mn[0].is_finite() && mn[1].is_finite() {
                isl.min = mn;
                isl.w = mx[0] - mn[0];
                isl.h = mx[1] - mn[1];
            }
        }
        // shelf packing
        let mut order: Vec<usize> = (0..islands.len()).collect();
        order.sort_by(|&a, &b| {
            let (ia, ib) = (&islands[a], &islands[b]);
            ib.h.total_cmp(&ia.h).then(ia.first_prim.cmp(&ib.first_prim))
        });
        let area: f64 = islands.iter().map(|i| (i.w + padding) * (i.h + padding)).sum();
        let widest = islands.iter().map(|i| i.w).fold(0.0, f64::max);
        let row_w = widest.max(area.sqrt());
        let mut slots: Vec<[f64; 2]> = vec![[0.0; 2]; islands.len()];
        let (mut x, mut y, mut row_h) = (0.0f64, 0.0f64, 0.0f64);
        let (mut used_w, mut used_h) = (0.0f64, 0.0f64);
        for &k in &order {
            let isl = &islands[k];
            if x > 0.0 && x + isl.w > row_w + 1e-12 {
                y += row_h + padding;
                x = 0.0;
                row_h = 0.0;
            }
            if x == 0.0 {
                row_h = isl.h;
            }
            slots[k] = [x, y];
            used_w = used_w.max(x + isl.w);
            used_h = used_h.max(y + isl.h);
            x += isl.w + padding;
        }
        let span = used_w.max(used_h);
        let s = if fit && span > 0.0 { 1.0 / span } else { 1.0 };
        let mut out_uv = uv.clone();
        for (isl, slot) in islands.iter().zip(&slots) {
            if !isl.min[0].is_finite() {
                continue;
            }
            for &v in &isl.verts {
                let (Some(c), Some(o)) = (uv.get(v), out_uv.get_mut(v)) else { continue };
                if c[0].is_finite() && c[1].is_finite() {
                    let nx = (f64::from(c[0]) - isl.min[0] + slot[0]) * s;
                    let ny = (f64::from(c[1]) - isl.min[1] + slot[1]) * s;
                    let (nx, ny) = (nx as f32, ny as f32);
                    if nx.is_finite() && ny.is_finite() {
                        *o = [nx, ny, c[2]];
                    }
                }
            }
        }
        let mut out = (**src).clone();
        out.add_attr(Attribute {
            name: "uv".into(),
            class: AttrClass::Vertex,
            data: AttrData::V3(out_uv),
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
    fn params(padding: f32, fit: bool) -> ParamSet {
        defaults().with("padding", ParamValue::F(padding)).with("fit", ParamValue::B(fit))
    }
    fn uvs(d: &Detail) -> Vec<[f32; 3]> {
        match d.attr(AttrClass::Vertex, "uv").map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            other => panic!("no V3 vertex uv: {other:?}"),
        }
    }
    /// `n` disjoint quads of size `(w, h)`, every one overlaid on the same uv box `[0,w]x[0,h]`.
    fn islands(sizes: &[(f32, f32)]) -> Detail {
        let mut d = Detail::default();
        let mut uv = Vec::new();
        for (i, &(w, h)) in sizes.iter().enumerate() {
            let base = d.add_points(4).start;
            let f = i as f32;
            d.p_mut()[base as usize..base as usize + 4].copy_from_slice(&[
                [f, 0.0, 0.0],
                [f + 1.0, 0.0, 0.0],
                [f + 1.0, 1.0, 0.0],
                [f, 1.0, 0.0],
            ]);
            d.add_poly(&[base, base + 1, base + 2, base + 3], true);
            uv.extend([[0.0, 0.0, 0.0], [w, 0.0, 0.0], [w, h, 0.0], [0.0, h, 0.0]]);
        }
        d.add_attr(Attribute {
            name: "uv".into(),
            class: AttrClass::Vertex,
            data: AttrData::V3(uv),
            type_info: TypeInfo::None,
        });
        d
    }
    fn bbox(uv: &[[f32; 3]]) -> ([f32; 2], [f32; 2]) {
        let mut b = ([f32::MAX; 2], [f32::MIN; 2]);
        for c in uv {
            for j in 0..2 {
                b.0[j] = b.0[j].min(c[j]);
                b.1[j] = b.1[j].max(c[j]);
            }
        }
        b
    }
    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "uvlayout");
        assert_eq!(n.params().iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["padding", "fit"]);
    }
    #[test]
    fn four_unit_islands_pack_to_exact_quadrants() {
        // W = max(1, sqrt(4)) = 2: two rows of two, layout 2x2, fit scale .5
        let out = cook(islands(&[(1.0, 1.0); 4]), params(0.0, true)).expect("cooks");
        let uv = uvs(&out);
        let want_min = [[0.0, 0.0], [0.5, 0.0], [0.0, 0.5], [0.5, 0.5]];
        for (i, m) in want_min.iter().enumerate() {
            assert_eq!(
                uv[i * 4..i * 4 + 4],
                [[m[0], m[1], 0.0], [m[0] + 0.5, m[1], 0.0], [m[0] + 0.5, m[1] + 0.5, 0.0], [m[0], m[1] + 0.5, 0.0]],
                "island {i}"
            );
        }
    }
    #[test]
    fn padding_and_fit_off_keep_input_units() {
        // 2 unit islands, padding .5, no fit: W = max(1, sqrt(2*1.5*1.5)=2.12); 1.5 + 1 > W -> second opens row 2 at y = 1 + .5
        let out = cook(islands(&[(1.0, 1.0); 2]), params(0.5, false)).expect("cooks");
        let uv = uvs(&out);
        assert_eq!(bbox(&uv[0..4]), ([0.0, 0.0], [1.0, 1.0]));
        assert_eq!(bbox(&uv[4..8]), ([0.0, 1.5], [1.0, 2.5]));
        // fit on: layout 1 x 2.5 -> scale 1/2.5: y fills [0,1], x spans .4
        let fit = uvs(&cook(islands(&[(1.0, 1.0); 2]), params(0.5, true)).expect("cooks"));
        let (lo, hi) = bbox(&fit);
        assert_eq!((lo, hi[1]), ([0.0, 0.0], 1.0));
        assert!((hi[0] - 0.4).abs() < 1e-7);
    }
    #[test]
    fn mixed_sizes_no_overlap_no_rotation_all_inside_unit_square_shapes_preserved() {
        let sizes = [(3.0, 1.0), (1.0, 2.0), (2.0, 2.0), (0.5, 0.5), (1.0, 1.0), (4.0, 0.25)];
        let out = cook(islands(&sizes), params(0.1, true)).expect("cooks");
        let uv = uvs(&out);
        let (lo, hi) = bbox(&uv);
        assert!(lo[0] >= 0.0 && lo[1] >= 0.0 && hi[0] <= 1.0 + 1e-6 && hi[1] <= 1.0 + 1e-6, "{lo:?} {hi:?}");
        assert!((hi[0] - 1.0).abs() < 1e-6 || (hi[1] - 1.0).abs() < 1e-6, "longer side fills the square");
        let boxes: Vec<([f32; 2], [f32; 2])> = (0..sizes.len()).map(|i| bbox(&uv[i * 4..i * 4 + 4])).collect();
        // uniform scale: every island keeps its aspect and shares one scale factor
        let s = (boxes[0].1[0] - boxes[0].0[0]) / sizes[0].0;
        for (b, &(w, h)) in boxes.iter().zip(&sizes) {
            assert!(((b.1[0] - b.0[0]) - w * s).abs() < 1e-6 && ((b.1[1] - b.0[1]) - h * s).abs() < 1e-6);
        }
        // pairwise disjoint (interiors), padding apart on at least one axis
        for a in 0..boxes.len() {
            for b in a + 1..boxes.len() {
                let (p, q) = (boxes[a], boxes[b]);
                let apart_x = p.1[0] <= q.0[0] + 1e-6 || q.1[0] <= p.0[0] + 1e-6;
                let apart_y = p.1[1] <= q.0[1] + 1e-6 || q.1[1] <= p.0[1] + 1e-6;
                assert!(apart_x || apart_y, "islands {a} and {b} overlap: {p:?} {q:?}");
            }
        }
        // per-island corner order is untouched (no rotation / mirror): corner 1 is right of corner 0, corner 3 above
        for i in 0..sizes.len() {
            let c = &uv[i * 4..i * 4 + 4];
            assert!(c[1][0] > c[0][0] && c[1][1] == c[0][1] && c[3][1] > c[0][1] && c[3][0] == c[0][0], "island {i}");
        }
    }
    #[test]
    fn shared_points_with_equal_uv_form_one_island_and_cuts_split() {
        // two quads sharing an edge (points 1,2) -> one island when uv agrees, two when they disagree there
        let mut d = Detail::default();
        d.add_points(6);
        *d.p_mut() =
            vec![[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0]];
        d.add_poly(&[0, 1, 2, 3], true);
        d.add_poly(&[1, 4, 5, 2], true);
        let joined = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        let mk = |uv: Vec<[f32; 3]>| {
            let mut e = d.clone();
            e.add_attr(Attribute {
                name: "uv".into(),
                class: AttrClass::Vertex,
                data: AttrData::V3(uv),
                type_info: TypeInfo::None,
            });
            e
        };
        // one island 2x1, fit: scale .5, stays a connected strip: shared edge keeps equal uv
        let out = uvs(&cook(mk(joined.clone()), params(0.0, true)).expect("cooks"));
        assert_eq!(out[1], out[4]);
        assert_eq!(out[2], out[7]);
        assert_eq!(bbox(&out), ([0.0, 0.0], [1.0, 0.5]));
        // cut: second quad uv shifted -> two islands (1x1 each): 2x2 pack W=sqrt2->1.414? widest=1 -> row_w 1.414: one per row
        let mut cut = joined;
        for c in &mut cut[4..] {
            c[0] += 10.0;
        }
        let out = uvs(&cook(mk(cut), params(0.0, true)).expect("cooks"));
        assert_ne!(out[1], out[4], "cut edge stays cut");
        let (a, b) = (bbox(&out[0..4]), bbox(&out[4..8]));
        assert_eq!(a, ([0.0, 0.0], [0.5, 0.5]));
        assert_eq!(b, ([0.0, 0.5], [0.5, 1.0]));
    }
    #[test]
    fn accepts_v2_uv_and_requires_uv() {
        let mut d = islands(&[(1.0, 1.0)]);
        d.add_attr(Attribute {
            name: "uv".into(),
            class: AttrClass::Vertex,
            data: AttrData::V2(vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]),
            type_info: TypeInfo::None,
        });
        let uv = uvs(&cook(d, params(0.0, true)).expect("cooks"));
        assert_eq!(uv, [[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]);
        let mut none = islands(&[(1.0, 1.0)]);
        none.attrs.clear();
        assert!(matches!(cook(none, defaults()), Err(CookError::Failed(_))));
    }
    #[test]
    fn hostile_params_and_geometry() {
        for (k, v) in [
            ("padding", ParamValue::F(-0.1)),
            ("padding", ParamValue::F(f32::NAN)),
            ("padding", ParamValue::I(0)),
            ("fit", ParamValue::I(1)),
        ] {
            assert!(matches!(cook(islands(&[(1.0, 1.0)]), defaults().with(k, v)), Err(CookError::BadParam(_))), "{k}");
        }
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &defaults(), log: &|_| {} };
        assert_eq!(new().cook(&ctx, &[]).expect("cooks").point_count, 0);
        // empty detail with no uv attr -> Failed (no uv), not a panic
        assert!(cook(Detail::default(), defaults()).is_err());
        // degenerate / non-finite uv, zero-size islands, bad uv length
        let mut h = islands(&[(0.0, 0.0), (1.0, 1.0), (f32::NAN, 1.0)]);
        h.add_poly(&[0], false);
        h.add_poly(&[], false);
        if let Some(a) = h.attr(AttrClass::Vertex, "uv").cloned() {
            h.add_attr(a); // repad to the new vertex count
        }
        let out = cook(h.clone(), params(0.0, true)).expect("cooks");
        // non-finite input uv is left as is; every finite input stays finite
        let before = uvs(&h);
        for (b, a) in before.iter().zip(uvs(&out)) {
            if b.iter().all(|c| c.is_finite()) {
                assert!(a.iter().all(|c| c.is_finite()));
            } else {
                assert_eq!(b.map(f32::to_bits), a.map(f32::to_bits));
            }
        }
        let mut short = islands(&[(1.0, 1.0)]);
        short.add_attr(Attribute {
            name: "uv".into(),
            class: AttrClass::Vertex,
            data: AttrData::V3(vec![[0.0; 3]; 2]),
            type_info: TypeInfo::None,
        });
        // add_attr pads to class length, so a short attribute is repaired rather than rejected
        assert!(cook(short, defaults()).is_ok());
        let huge = islands(&[(f32::MAX, f32::MAX), (f32::MAX, 1.0)]);
        assert!(uvs(&cook(huge, params(1.0, true)).expect("cooks")).iter().flatten().all(|c| c.is_finite()));
    }
    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/uv.json"))
            .expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        let uv = uvs(&out);
        let (lo, hi) = bbox(&uv);
        assert!(lo[0] >= 0.0 && lo[1] >= 0.0 && hi[0] <= 1.0 + 1e-6 && hi[1] <= 1.0 + 1e-6);
        assert_eq!(crate::registry().create("uvlayout").map(|n| n.type_name()), Some("uvlayout"));
    }
}
