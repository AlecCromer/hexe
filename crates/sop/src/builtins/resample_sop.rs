//! Resample SOP (token `resample`): re-sample polylines / polygons at even arc-length steps.
//!
//! Params: `method` (menu `length` | `segments`) · `length` (F, 0.1) · `segments` (I, 10) · `lastsegment` (B, on) ·
//! `resample_closed` (B, on: closed polygons are re-sampled as loops; off: they pass through untouched).
//!
//! # Semantics (UNVERIFIED vs Houdini, hython blocked)
//! Curves = `Poly` prims (closed or open) and `Polyline` prims with ≥ 2 vertices and total length > 0. Each becomes ONE new
//! prim of the same closedness over NEW points, curve by curve in prim order, samples ascending along the curve. Let `L` = arc
//! length (a closed curve includes its closing edge) and `n_full = floor(L / length + 1e-6)`, `rem = L/length - n_full`.
//! * `segments`: `n = segments` even segments; open → `n + 1` points (both ends kept), closed → `n` points (sample 0 = vertex 0).
//! * `length`, `rem <= 1e-6` (fits exactly): `n = n_full` even segments (spacing exactly `length` up to f32).
//! * `length`, `lastsegment` on ("edge_length_exact: last segment shorter"): samples at `k * length`, `k = 0..=n_full`,
//!   open curves then get a final sample at the end vertex (the last segment is the shorter one); a closed curve's last
//!   segment runs back to sample 0 and is the shorter one.
//! * `length`, `lastsegment` off (stretch): `n = n_full + 1` even segments, every segment ≤ `length`.
//!
//! Even samples sit at `L * k / n`. Point attributes are re-sampled: float-like (`F32` `V2` `V3` `V4` `FN`) lerp between the
//! two bracketing source points; int / string take the nearer source point. Prim attributes copy from the source prim, detail
//! attributes pass through. Vertex attributes and groups of the re-sampled curves are dropped.
//! Non-curve prims (other kinds, < 2 vertices) and closed polygons when `resample_closed` is off PASS THROUGH untouched
//! (points, vertices, attributes, groups copied via `compact`), appended after the re-sampled curves. Zero-length curves are
//! dropped.
//! Hostile input: non-finite `P`, a vertex without a point, or > `MAX_POINTS` output points → `Failed`;
//! `length` ≤ 0 / non-finite, `segments` < 1 → `BadParam`.
use super::group_sop::compact;
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

/// Output point cap (AGENTS.md §4).
const MAX_POINTS: usize = 1 << 22;
/// Float tolerance on `L / length` and friends.
const EPS: f64 = 1e-6;
/// `method` menu (length segments).
const METHOD_LENGTH: u32 = 0;
const METHOD_SEGMENTS: u32 = 1;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("method", ParamValue::Menu(METHOD_LENGTH)).label("Method"),
        ParamSpec::new("length", ParamValue::F(0.1)).label("Length").range(0.001, 10.0),
        ParamSpec::new("segments", ParamValue::I(10)).label("Segments").range(1.0, 100.0),
        ParamSpec::new("lastsegment", ParamValue::B(true)).label("Keep Shorter Last Segment"),
        ParamSpec::new("resample_closed", ParamValue::B(true)).label("Resample Closed Curves"),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(Resample { specs: spec() })
}

struct Resample {
    specs: Vec<ParamSpec>,
}

fn bad(name: &str, what: &str) -> CookError {
    CookError::BadParam(format!("resample.{name}: {what}"))
}

fn failed(what: &str) -> CookError {
    CookError::Failed(format!("resample: {what}"))
}

/// One output point: source indices `a`, `b` and blend `t` (`a` when `t == 0`).
type Sample = (usize, usize, f64);

/// Arc-length positions (`0..=L`, ascending) of the output samples. Open: includes both ends. Closed: excludes `L`.
fn positions(l: f64, closed: bool, method: u32, length: f64, segments: u32, last: bool) -> Result<Vec<f64>, CookError> {
    let even = |n: usize| -> Result<Vec<f64>, CookError> {
        let count = if closed { n } else { n.saturating_add(1) };
        if count > MAX_POINTS {
            return Err(failed("output exceeds the point cap"));
        }
        Ok((0..count).map(|k| l * k as f64 / n as f64).collect())
    };
    if method == METHOD_SEGMENTS {
        return even(segments as usize);
    }
    let ratio = l / length;
    if !ratio.is_finite() || ratio > MAX_POINTS as f64 {
        return Err(failed("output exceeds the point cap"));
    }
    let full = (ratio + EPS).floor();
    let n_full = full as usize;
    let rem = ratio - full;
    if rem <= EPS {
        return even(n_full.max(1));
    }
    if last {
        let mut v: Vec<f64> = (0..=n_full).map(|k| length * k as f64).collect();
        if !closed {
            v.push(l);
        }
        Ok(v)
    } else {
        even(n_full + 1)
    }
}

fn lerp_arr<const N: usize>(v: &[[f32; N]], s: &[Sample]) -> Vec<[f32; N]> {
    s.iter()
        .map(|&(a, b, t)| {
            let (x, y) = (v.get(a).copied().unwrap_or([0.0; N]), v.get(b).copied().unwrap_or([0.0; N]));
            if t == 0.0 {
                return x;
            }
            let mut o = [0.0f32; N];
            for k in 0..N {
                let (p, q) = (f64::from(x[k]), f64::from(y[k]));
                o[k] = (p + (q - p) * t) as f32;
            }
            o
        })
        .collect()
}

fn lerp_flat(v: &[f32], size: usize, s: &[Sample]) -> Vec<f32> {
    let at = |i: usize, k: usize| v.get(i * size + k).copied().map_or(0.0, f64::from);
    s.iter()
        .flat_map(|&(a, b, t)| (0..size).map(move |k| (a, b, t, k)))
        .map(|(a, b, t, k)| (at(a, k) + (at(b, k) - at(a, k)) * t) as f32)
        .collect()
}

fn near_idx(s: &[Sample]) -> Vec<usize> {
    s.iter().map(|&(a, b, t)| if t < 0.5 { a } else { b }).collect()
}

/// Re-sample one attribute over `s`: float-like blend, int / string pick the nearer source.
fn gather(data: &AttrData, s: &[Sample]) -> AttrData {
    let pick = near_idx(s);
    match data {
        AttrData::F32(v) => AttrData::F32(lerp_flat(v, 1, s)),
        AttrData::V2(v) => AttrData::V2(lerp_arr(v, s)),
        AttrData::V3(v) => AttrData::V3(lerp_arr(v, s)),
        AttrData::V4(v) => AttrData::V4(lerp_arr(v, s)),
        AttrData::FN { size, data } => AttrData::FN { size: *size, data: lerp_flat(data, *size as usize, s) },
        AttrData::I32(v) => AttrData::I32(pick.iter().map(|&i| v.get(i).copied().unwrap_or(0)).collect()),
        AttrData::Str { idx, table } => AttrData::Str {
            idx: pick.iter().map(|&i| idx.get(i).copied().unwrap_or(-1)).collect(),
            table: table.clone(),
        },
        AttrData::IN { size, data } => {
            let n = *size as usize;
            AttrData::IN {
                size: *size,
                data: pick
                    .iter()
                    .flat_map(|&i| (0..n).map(move |k| (i, k)))
                    .map(|(i, k)| data.get(i * n + k).copied().unwrap_or(0))
                    .collect(),
            }
        }
    }
}

impl Node<Detail> for Resample {
    fn type_name(&self) -> &'static str {
        "resample"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let method = p.menu("method").ok_or_else(|| bad("method", "missing or not a menu"))?;
        let length = p.float("length").ok_or_else(|| bad("length", "missing or not a float"))?;
        let segs = p.int("segments").ok_or_else(|| bad("segments", "missing or not an integer"))?;
        let last = p.toggle("lastsegment").ok_or_else(|| bad("lastsegment", "missing or not a toggle"))?;
        let rclosed = p.toggle("resample_closed").ok_or_else(|| bad("resample_closed", "missing or not a toggle"))?;
        if method > METHOD_SEGMENTS {
            return Err(bad("method", "menu index out of range 0..=1"));
        }
        if method == METHOD_LENGTH && !(length.is_finite() && length > 0.0) {
            return Err(bad("length", "must be finite and > 0"));
        }
        let segs = u32::try_from(segs).ok().filter(|&n| n >= 1);
        if method == METHOD_SEGMENTS && segs.is_none() {
            return Err(bad("segments", "need at least 1 segment"));
        }
        let segs = segs.unwrap_or(1);
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let pos = src.p();
        let mut samples: Vec<Sample> = Vec::new();
        let mut shapes: Vec<(usize, usize, bool)> = Vec::new(); // (source prim, sample count, closed)
        let mut keep_prim = vec![false; src.prims.len()]; // pass-through prims
        for (pi, prim) in src.prims.iter().enumerate() {
            let closed = match prim.kind {
                PrimKind::Poly { closed } => closed,
                PrimKind::Polyline => false,
                _ => {
                    keep_prim[pi] = true;
                    continue;
                }
            };
            let verts = src.vertex_point.get(prim.verts.start as usize..prim.verts.end as usize).unwrap_or(&[]);
            if (closed && !rclosed) || verts.len() < 2 {
                keep_prim[pi] = true;
                continue;
            }
            let pts: Vec<usize> = verts.iter().map(|&v| v as usize).collect();
            let mut xyz: Vec<[f64; 3]> = Vec::with_capacity(pts.len() + 1);
            for &i in &pts {
                let q = pos.get(i).ok_or_else(|| failed("vertex references a missing point"))?;
                if !q.iter().all(|c| c.is_finite()) {
                    return Err(failed("non-finite P"));
                }
                xyz.push(q.map(f64::from));
            }
            // edge e runs vertex e -> vertex (e+1) % m (the wrap edge only exists when closed)
            let edges = if closed { pts.len() } else { pts.len() - 1 };
            let mut cum: Vec<f64> = Vec::with_capacity(edges + 1);
            cum.push(0.0);
            for e in 0..edges {
                let (a, b) = (xyz[e], xyz[(e + 1) % pts.len()]);
                let d = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
                cum.push(cum[e] + d);
            }
            let total = cum[edges];
            if !(total.is_finite() && total > 0.0) {
                continue;
            }
            let at = positions(total, closed, method, f64::from(length), segs, last)?;
            if samples.len().saturating_add(at.len()) > MAX_POINTS {
                return Err(failed("output exceeds the point cap"));
            }
            let mut e = 0usize;
            for &s in &at {
                // last edge whose start is <= s (skips zero-length edges); s == total lands on the final edge with t = 1
                while e + 1 < edges && cum[e + 1] <= s {
                    e += 1;
                }
                let span = cum[e + 1] - cum[e];
                let t = if span > 0.0 { ((s - cum[e]) / span).clamp(0.0, 1.0) } else { 0.0 };
                samples.push((pts[e], pts[(e + 1) % pts.len()], t));
            }
            shapes.push((pi, at.len(), closed));
        }
        let n = u32::try_from(samples.len()).map_err(|_| failed("too many points"))?;
        let mut out = Detail::default();
        out.add_points(n);
        let mut base = 0u32;
        for &(_, count, closed) in &shapes {
            let c = u32::try_from(count).map_err(|_| failed("too many points"))?;
            let ids: Vec<u32> = (base..base + c).collect();
            out.add_poly(&ids, closed);
            base += c;
        }
        let prim_samples: Vec<Sample> = shapes.iter().map(|&(pi, _, _)| (pi, pi, 0.0)).collect();
        for a in &src.attrs {
            let data = match a.class {
                AttrClass::Point => gather(&a.data, &samples),
                AttrClass::Prim => gather(&a.data, &prim_samples),
                AttrClass::Detail => a.data.clone(),
                AttrClass::Vertex => continue,
            };
            out.add_attr(Attribute { name: a.name.clone(), class: a.class, data, type_info: a.type_info });
        }
        out.canonicalize();
        if keep_prim.iter().any(|&k| k) {
            let mut keep_pt = vec![false; src.point_count as usize];
            for (pr, _) in src.prims.iter().zip(&keep_prim).filter(|(_, k)| **k) {
                for &q in src.vertex_point.get(pr.verts.start as usize..pr.verts.end as usize).unwrap_or(&[]) {
                    if let Some(k) = keep_pt.get_mut(q as usize) {
                        *k = true;
                    }
                }
            }
            out.merge(&compact(src, &keep_pt, &keep_prim, "resample")?);
            out.canonicalize();
        }
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_geo::TypeInfo;
    use hexe_graph::ParamSet;

    /// `nodes` = JSON node list text, wired in a chain to `r` (the resample, last).
    fn cook(head: &str, params: &str) -> Result<Arc<Detail>, String> {
        let s = format!(
            r#"{{"nodes":[{head},{{"name":"r","type":"resample","params":{params}}}],"edges":[["h",0,"r",0]],"display":"r"}}"#
        );
        let (mut g, id) = load_scene(&s).map_err(|e| e.to_string())?;
        g.cook(id, 0.0).map_err(|e| e.to_string())
    }
    fn line(extra: &str) -> String {
        format!(r#"{{"name":"h","type":"line","params":{{{extra}}}}}"#)
    }
    fn circle(extra: &str) -> String {
        format!(r#"{{"name":"h","type":"circle","params":{{{extra}}}}}"#)
    }
    fn gaps(d: &Detail) -> Vec<f32> {
        d.p().windows(2).map(|w| (0..3).map(|k| (w[1][k] - w[0][k]).powi(2)).sum::<f32>().sqrt()).collect()
    }

    #[test]
    fn line_dist_1_two_points_length_quarter_is_five_points_spacing_exact() {
        let d = cook(&line(""), r#"{"length":0.25}"#).expect("cooks");
        assert_eq!((d.point_count, d.prim_count(), d.vertex_count()), (5, 1, 5));
        for (i, p) in d.p().iter().enumerate() {
            assert_eq!(*p, [0.0, 0.25 * i as f32, 0.0]);
        }
        assert!(matches!(d.prims[0].kind, PrimKind::Poly { closed: false }));
    }

    #[test]
    fn segments_10_is_eleven_points_ends_kept() {
        let d = cook(&line(""), r#"{"method":1,"segments":10}"#).expect("cooks");
        assert_eq!(d.point_count, 11);
        assert_eq!((d.p()[0], d.p()[10]), ([0.0, 0.0, 0.0], [0.0, 1.0, 0.0]));
        assert!(gaps(&d).iter().all(|g| (g - 0.1).abs() < 1e-6));
    }

    #[test]
    fn circle_div4_segments_8_is_eight_points_with_every_edge_midpoint() {
        let d = cook(&circle(r#""divisions":4"#), r#"{"method":1,"segments":8}"#).expect("cooks");
        assert_eq!((d.point_count, d.prim_count(), d.vertex_count()), (8, 1, 8));
        assert!(matches!(d.prims[0].kind, PrimKind::Poly { closed: true }));
        let mids = [[0.5, 0.5, 0.0], [-0.5, 0.5, 0.0], [-0.5, -0.5, 0.0], [0.5, -0.5, 0.0]];
        for m in mids {
            assert!(d.p().iter().any(|p| p.iter().zip(m).all(|(a, b)| (a - b).abs() < 1e-6)), "missing {m:?}");
        }
        // every output point lies on the diamond |x|+|y| = 1 (on the edges, not the circle)
        assert!(d.p().iter().all(|p| ((p[0].abs() + p[1].abs()) - 1.0).abs() < 1e-6 && p[2] == 0.0));
        // exactly 4 original corners + 4 midpoints, in loop order (corner, midpoint, corner, ...)
        assert!((d.p()[0][0] - 1.0).abs() < 1e-6 && (d.p()[1][0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn lastsegment_keeps_shorter_last_or_stretches() {
        // L = 1, length 0.3: n_full = 3, rem = 1/3
        let keep = cook(&line(""), r#"{"length":0.3}"#).expect("cooks");
        assert_eq!(keep.point_count, 5);
        let g = gaps(&keep);
        assert!(g[..3].iter().all(|x| (x - 0.3).abs() < 1e-6) && (g[3] - 0.1).abs() < 1e-6);
        assert_eq!(keep.p()[4], [0.0, 1.0, 0.0]);
        let stretch = cook(&line(""), r#"{"length":0.3,"lastsegment":false}"#).expect("cooks");
        assert_eq!(stretch.point_count, 5);
        assert!(gaps(&stretch).iter().all(|x| (x - 0.25).abs() < 1e-6 && *x <= 0.3));
        // length longer than the curve: just the two ends
        let long = cook(&line(""), r#"{"length":5}"#).expect("cooks");
        assert_eq!(long.point_count, 2);
        // closed + keep: loop of 4 edges of length sqrt2 (L = 5.657) at length 2: samples 0,2,4 then back to start
        let c = cook(&circle(r#""divisions":4"#), r#"{"length":0.5}"#).expect("cooks");
        let l = 4.0 * 2f32.sqrt();
        assert_eq!(c.point_count as f32, (l / 0.5).floor() + 1.0);
    }

    #[test]
    fn closed_curve_pass_through_toggle_and_non_curves_dropped() {
        let off = cook(&circle(r#""divisions":4"#), r#"{"resample_closed":false}"#).expect("cooks");
        assert_eq!((off.point_count, off.prim_count()), (4, 1)); // passed through untouched
        let grid = r#"{"name":"h","type":"grid","params":{"rows":3,"cols":3,"surftype":0}}"#;
        let rows = cook(grid, r#"{"method":1,"segments":4}"#).expect("cooks");
        assert_eq!((rows.point_count, rows.prim_count()), (15, 3));
        let quads = r#"{"name":"h","type":"grid","params":{"rows":3,"cols":3}}"#;
        assert_eq!(cook(quads, r#"{"method":1,"segments":4}"#).expect("cooks").point_count, 4 * 4); // 4 closed quads x 4 loop samples
    }

    #[test]
    fn merged_grid_and_line_non_curves_pass_through_untouched() {
        // line (open, resampled to 5 pts) + 2x2-quad grid; resample_closed off => the 4 closed quads pass through as-is
        let s = r#"{"nodes":[
            {"name":"l","type":"line","params":{}},
            {"name":"g","type":"grid","params":{"rows":3,"cols":3}},
            {"name":"m","type":"merge"},
            {"name":"r","type":"resample","params":{"length":0.25,"resample_closed":false}}],
            "edges":[["l",0,"m",0],["g",0,"m",1],["m",0,"r",0]],"display":"r"}"#;
        let (mut g, id) = load_scene(s).expect("loads");
        let out = g.cook(id, 0.0).expect("cooks");
        let (mut g2, id2) = load_scene(
            &s.replace(r#""name":"r""#, r#""name":"rr""#)
                .replace(r#""r",0]"#, r#""rr",0]"#)
                .replace(r#""display":"r""#, r#""display":"m""#),
        )
        .expect("loads");
        let merged = g2.cook(id2, 0.0).expect("cooks");
        // 5 new line points + all 9 grid points; 1 resampled line + 4 untouched quads
        assert_eq!((out.point_count, out.prim_count(), out.vertex_count()), (5 + merged.point_count - 2, 5, 5 + 16));
        let quads: Vec<_> = out.prims.iter().filter(|p| matches!(p.kind, PrimKind::Poly { closed: true })).collect();
        assert_eq!(quads.len(), 4);
        // the grid's 9 points survive bit-exact (same P multiset as the merged source's grid points)
        let grid_p: Vec<[f32; 3]> = merged.p()[2..].to_vec();
        assert_eq!(&out.p()[5..], &grid_p[..]);
        // each pass-through quad has the same 4 positions as in the source
        let src_q = |d: &Detail| -> Vec<Vec<[f32; 3]>> {
            d.prims
                .iter()
                .filter(|p| matches!(p.kind, PrimKind::Poly { closed: true }))
                .map(|p| {
                    d.vertex_point[p.verts.start as usize..p.verts.end as usize]
                        .iter()
                        .map(|&v| d.p()[v as usize])
                        .collect()
                })
                .collect()
        };
        assert_eq!(src_q(&out), src_q(&merged));
    }
    #[test]
    fn attributes_lerp_ints_pick_nearest_prim_and_detail_pass() {
        let mut src = Detail::default();
        src.add_points(2);
        *src.p_mut() = vec![[0.0, 0.0, 0.0], [4.0, 0.0, 0.0]];
        src.add_poly(&[0, 1], false);
        src.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Point,
            data: AttrData::F32(vec![0.0, 8.0]),
            type_info: TypeInfo::None,
        });
        src.add_attr(Attribute {
            name: "c".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[0.0; 3], [1.0, 2.0, 4.0]]),
            type_info: TypeInfo::Color,
        });
        src.add_attr(Attribute {
            name: "i".into(),
            class: AttrClass::Point,
            data: AttrData::I32(vec![10, 20]),
            type_info: TypeInfo::None,
        });
        src.add_attr(Attribute {
            name: "pr".into(),
            class: AttrClass::Prim,
            data: AttrData::I32(vec![7]),
            type_info: TypeInfo::None,
        });
        src.add_attr(Attribute {
            name: "dt".into(),
            class: AttrClass::Detail,
            data: AttrData::F32(vec![3.0]),
            type_info: TypeInfo::None,
        });
        let full = spec().into_iter().fold(ParamSet::new(), |acc, s| acc.with(&s.name, s.default.clone()));
        let full = full.with("method", ParamValue::Menu(1)).with("segments", ParamValue::I(4));
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &full, log: &|_| {} };
        let out = new().cook(&ctx, &[Arc::new(src)]).expect("cooks");
        assert_eq!(out.point_count, 5);
        let f = |n: &str| match out.attr(AttrClass::Point, n).map(|a| &a.data) {
            Some(AttrData::F32(v)) => v.clone(),
            _ => vec![],
        };
        assert_eq!(f("w"), vec![0.0, 2.0, 4.0, 6.0, 8.0]);
        assert_eq!(out.p()[2], [2.0, 0.0, 0.0]);
        match out.attr(AttrClass::Point, "c").map(|a| (&a.data, a.type_info)) {
            Some((AttrData::V3(v), TypeInfo::Color)) => assert_eq!(v[2], [0.5, 1.0, 2.0]),
            o => panic!("c: {o:?}"),
        }
        match out.attr(AttrClass::Point, "i").map(|a| &a.data) {
            Some(AttrData::I32(v)) => assert_eq!(v, &vec![10, 10, 20, 20, 20]),
            o => panic!("i: {o:?}"),
        }
        assert_eq!(out.attr(AttrClass::Prim, "pr").map(|a| a.data.clone()), Some(AttrData::I32(vec![7])));
        assert!(out.attr(AttrClass::Detail, "dt").is_some());
    }

    #[test]
    fn hostile_params_and_geometry_are_errors() {
        assert!(cook(&line(""), r#"{"length":0}"#).is_err());
        assert!(cook(&line(""), r#"{"length":-1}"#).is_err());
        assert!(cook(&line(""), r#"{"method":1,"segments":0}"#).is_err());
        assert!(cook(&line(""), r#"{"length":0.000001}"#).is_ok());
        assert!(cook(&line(""), r#"{"method":1,"segments":100000000}"#).is_err());
        assert!(cook(&line(""), r#"{"length":1e-30}"#).is_err());
        // zero-length curve and empty input drop quietly
        let zero = r#"{"name":"h","type":"curve","params":{"coords":"1,1,1 1,1,1"}}"#;
        assert_eq!(cook(zero, "{}").expect("cooks").point_count, 0);
        let none = r#"{"name":"h","type":"null"}"#;
        assert_eq!(cook(none, "{}").expect("cooks").point_count, 0);
    }

    #[test]
    fn example_scene_cooks() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/curves.json"))
            .expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert!(out.point_count > 0 && out.prim_count() > 0);
        for t in ["line", "circle", "curve", "resample", "add"] {
            assert_eq!(crate::registry().create(t).map(|n| n.type_name()), Some(t));
        }
    }
}
