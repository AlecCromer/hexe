//! Smooth SOP (token `smooth`): implicit Laplacian smoothing of point positions.
//!
//! # Semantics (UNVERIFIED vs Houdini: analytical oracle only)
//! Solves (I + λL)P' = P for each coordinate independently using conjugate gradient,
//! where L = weighted Laplacian matrix (CSR), λ = strength, iterations = filter quality.
//! Methods: uniform (equal weights), scale-dominant (edge-length inverse), curvature-dominant
//! (cotangent weights approximation). `constrainboundary`: boundary points (edges used by
//! exactly one closed poly) kept at their original positions (rows zeroed, diagonal = 1).
//!
//! Params: `method` M 0 (uniform=0, scale=1, curv=2), `strength` F 0.5, `filterquality` I 10
//! (CG iterations, 0 = input Arc shared, max 1000), `constrainboundary` B false.
//! Work bound `filterquality * (edges + points) <= 2^31`, else `Failed`.
use super::meshutil::{V3, bad, failed, float, int, menu, point_normals, positions, sub, to_f32, toggle};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, Prim, PrimKind, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const OP: &str = "smooth";
const MAX_QUALITY: i32 = 1000;
const MAX_WORK: u64 = 1 << 31;
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("method", ParamValue::Menu(0)).label("Weighting Method").menu(&[
            ("uniform", "Uniform"),
            ("scale", "Scale-Dominant"),
            ("curvature", "Curvature-Dominant"),
        ]),
        ParamSpec::new("strength", ParamValue::F(0.5)).label("Strength").range(0.0, 2.0),
        ParamSpec::new("filterquality", ParamValue::I(10)).label("Filter Quality").range(0.0, 100.0),
        ParamSpec::new("constrainboundary", ParamValue::B(false)).label("Constrain Boundary Points"),
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("attribs", ParamValue::S(String::new())).label("Extra Attributes"),
        ParamSpec::new("updatenormals", ParamValue::B(false)).label("Update Normals"),
    ]
}

/// Point-group selection (union of space-separated names); `None` = every point.
fn point_selection(src: &Detail, names: &str) -> Result<Option<Vec<bool>>, CookError> {
    let names: Vec<&str> = names.split_whitespace().collect();
    if names.is_empty() {
        return Ok(None);
    }
    let mut sel = vec![false; src.point_count as usize];
    for n in names {
        let g = src.group(AttrClass::Point, n).ok_or_else(|| failed(OP, &format!("unknown point group `{n}`")))?;
        for (s, m) in sel.iter_mut().zip(&g.mask) {
            *s |= *m;
        }
    }
    Ok(Some(sel))
}

pub fn new() -> Box<SopNode> {
    Box::new(Smooth { specs: spec() })
}

struct Smooth {
    specs: Vec<ParamSpec>,
}

/// Ids of a prim's vertex points that are in range, in order.
fn prim_points(d: &Detail, pr: &Prim) -> Vec<usize> {
    let (lo, hi) = (usize::try_from(pr.verts.start).unwrap_or(0), usize::try_from(pr.verts.end).unwrap_or(0));
    let np = d.point_count as usize;
    d.vertex_point.get(lo..hi).unwrap_or(&[]).iter().map(|&p| p as usize).filter(|&p| p < np).collect()
}

/// Edge list with weights. method: 0=uniform, 1=scale, 2=curvature.
fn build_edges(d: &Detail, pts: &[V3], method: u32) -> Vec<(usize, usize, f64)> {
    let np = d.point_count as usize;
    let mut edges: Vec<(usize, usize, f64)> = Vec::new();

    for pr in &d.prims {
        let (closed, linear) = match pr.kind {
            PrimKind::Poly { closed } => (closed, true),
            PrimKind::Polyline => (false, true),
            _ => (false, false),
        };
        if !linear {
            continue;
        }
        let plist = prim_points(d, pr);
        let n = plist.len();
        let m = if closed && n >= 3 { n } else { n.saturating_sub(1) };

        for k in 0..m {
            let (Some(&a), Some(&b)) = (plist.get(k), plist.get((k + 1) % n)) else { continue };
            if a == b || a >= np || b >= np {
                continue;
            }

            let weight = match method {
                0 => 1.0, // uniform
                1 => {
                    // scale-dominant: inverse edge length
                    let pa = pts.get(a).copied().unwrap_or([0.0; 3]);
                    let pb = pts.get(b).copied().unwrap_or([0.0; 3]);
                    let len = sub(pb, pa).iter().map(|x| x * x).sum::<f64>().sqrt();
                    if len > 1e-12 { 1.0 / len } else { 1.0 }
                }
                _ => {
                    // curvature-dominant: cotangent-like approximation
                    // For simplicity, use inverse squared edge length as approximation
                    let pa = pts.get(a).copied().unwrap_or([0.0; 3]);
                    let pb = pts.get(b).copied().unwrap_or([0.0; 3]);
                    let len_sq = sub(pb, pa).iter().map(|x| x * x).sum::<f64>();
                    if len_sq > 1e-12 { 1.0 / len_sq } else { 1.0 }
                }
            };
            edges.push((a, b, weight));
            edges.push((b, a, weight));
        }
    }
    edges.sort_unstable_by_key(|e| (e.0, e.1));
    // Deduplicate and sum weights
    let mut dedup = Vec::new();
    let mut i = 0;
    while i < edges.len() {
        let (a, b, mut w) = edges[i];
        let mut j = i + 1;
        while j < edges.len() && edges[j].0 == a && edges[j].1 == b {
            w += edges[j].2;
            j += 1;
        }
        dedup.push((a, b, w));
        i = j;
    }
    dedup
}

/// Find boundary points (edges used by exactly one closed poly).
fn find_boundary(d: &Detail) -> Vec<bool> {
    let np = d.point_count as usize;
    let mut closed_edges: Vec<(usize, usize)> = Vec::new();

    for pr in &d.prims {
        let (closed, linear) = match pr.kind {
            PrimKind::Poly { closed } => (closed, true),
            PrimKind::Polyline => (false, true),
            _ => (false, false),
        };
        if !linear || !closed {
            continue;
        }
        let plist = prim_points(d, pr);
        let n = plist.len();
        if n < 3 {
            continue;
        }
        for k in 0..n {
            let (Some(&a), Some(&b)) = (plist.get(k), plist.get((k + 1) % n)) else { continue };
            if a != b {
                closed_edges.push((a.min(b), a.max(b)));
            }
        }
    }
    closed_edges.sort_unstable();
    let mut boundary = vec![false; np];
    let mut i = 0;
    while i < closed_edges.len() {
        let j = closed_edges[i..].iter().take_while(|e| **e == closed_edges[i]).count();
        if j == 1 {
            let (a, b) = closed_edges[i];
            boundary[a] = true;
            boundary[b] = true;
        }
        i += j;
    }
    boundary
}

/// CSR representation: (row_ptr, col_ind, values) for matrix L.
/// L_ii = -sum(weights of edges from i), L_ij = weight(i,j).
fn build_laplacian_csr(np: usize, edges: &[(usize, usize, f64)]) -> (Vec<usize>, Vec<usize>, Vec<f64>) {
    let mut row_ptr = vec![0; np + 1];
    let mut col_ind = Vec::new();
    let mut values = Vec::new();

    // Count entries per row
    for &(a, _, _) in edges {
        if a < np {
            row_ptr[a + 1] += 1;
        }
    }
    // Each row also has a diagonal entry
    for i in 0..np {
        row_ptr[i + 1] += 1;
    }

    // Prefix sum
    for i in 0..np {
        row_ptr[i + 1] += row_ptr[i];
    }

    col_ind.resize(row_ptr[np], 0);
    values.resize(row_ptr[np], 0.0);

    let mut temp_ptr = row_ptr[..np].to_vec();

    // Fill off-diagonal entries
    for &(a, b, w) in edges {
        if a < np && b < np {
            let idx = temp_ptr[a];
            col_ind[idx] = b;
            values[idx] = w;
            temp_ptr[a] += 1;
        }
    }

    // Sort each row and compute diagonal
    for i in 0..np {
        let lo = row_ptr[i];
        let hi = temp_ptr[i];

        // Sort the off-diagonal entries in this row
        let mut row_data: Vec<(usize, f64)> = (lo..hi).map(|k| (col_ind[k], values[k])).collect();
        row_data.sort_unstable_by_key(|&(j, _)| j);

        // Sum off-diagonal weights for diagonal entry
        let diag_val: f64 = row_data.iter().map(|&(_, w)| w).sum::<f64>();

        // Insert diagonal at the start (positive: sum of weights)
        col_ind[lo] = i;
        values[lo] = diag_val;

        // Insert sorted off-diagonal entries (negative)
        for (k, &(j, w)) in row_data.iter().enumerate() {
            col_ind[lo + 1 + k] = j;
            values[lo + 1 + k] = -w;
        }

        // Update row_ptr to include diagonal
        temp_ptr[i] = hi + 1;
    }

    (row_ptr, col_ind, values)
}

/// Sparse matrix-vector product: y = A * x.
fn spmv(row_ptr: &[usize], col_ind: &[usize], vals: &[f64], x: &[f64], y: &mut [f64]) {
    for (i, yi) in y.iter_mut().enumerate() {
        let (lo, hi) = match (row_ptr.get(i), row_ptr.get(i + 1)) {
            (Some(&a), Some(&b)) => (a, b),
            _ => continue,
        };
        *yi = (lo..hi)
            .filter_map(|k| {
                let j = *col_ind.get(k)?;
                let v = *vals.get(k)?;
                let xj = *x.get(j)?;
                Some(v * xj)
            })
            .sum();
    }
}

/// Compute (I + λL) * x.
fn apply_system(row_ptr: &[usize], col_ind: &[usize], vals: &[f64], lambda: f64, x: &[f64], y: &mut [f64]) {
    spmv(row_ptr, col_ind, vals, x, y);
    for (i, yi) in y.iter_mut().enumerate() {
        *yi = x.get(i).copied().unwrap_or(0.0) + lambda * (*yi);
    }
}

/// Dot product.
fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Conjugate gradient solver for (I + λL)x = b.
/// Returns solution in x. max_iter iterations, tolerance tol.
#[allow(clippy::too_many_arguments)]
fn cg_solve(
    row_ptr: &[usize],
    col_ind: &[usize],
    vals: &[f64],
    lambda: f64,
    b: &[f64],
    x: &mut [f64],
    max_iter: usize,
    tol: f64,
) {
    let n = b.len();
    let mut r = vec![0.0; n];
    let mut p = vec![0.0; n];
    let mut ap = vec![0.0; n];

    // r = b - A*x
    apply_system(row_ptr, col_ind, vals, lambda, x, &mut ap);
    for i in 0..n {
        r[i] = b[i] - ap[i];
    }
    p.copy_from_slice(&r);

    let mut rs_old = dot(&r, &r);
    if rs_old < tol * tol {
        return;
    }

    for _ in 0..max_iter {
        apply_system(row_ptr, col_ind, vals, lambda, &p, &mut ap);
        let alpha = rs_old / dot(&p, &ap).max(1e-30);

        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }

        let rs_new = dot(&r, &r);
        if rs_new < tol * tol {
            break;
        }

        let beta = rs_new / rs_old;
        for i in 0..n {
            p[i] = r[i] + beta * p[i];
        }
        rs_old = rs_new;
    }
}

impl Node<Detail> for Smooth {
    fn type_name(&self) -> &'static str {
        "smooth"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let method = menu(p, "method")?;
        let strength = float(p, "strength")?;
        let quality = int(p, "filterquality")?;
        let constrain = toggle(p, "constrainboundary")?;

        if !(0..=MAX_QUALITY).contains(&quality) {
            return Err(bad(format!("`filterquality` must be in 0..={MAX_QUALITY}, got {quality}")));
        }
        if !strength.is_finite() {
            return Err(bad(format!("`strength` must be finite, got {strength}")));
        }

        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        if quality == 0 || strength == 0.0 || src.point_count == 0 {
            return Ok(Arc::clone(src));
        }

        let np = src.point_count as usize;
        let pts = positions(src);
        let edges = build_edges(src, &pts, method);

        let work = u64::from(quality as u32).saturating_mul((edges.len() + np) as u64);
        if work > MAX_WORK {
            return Err(failed(OP, "filterquality * edges exceeds the work cap"));
        }

        let (row_ptr, col_ind, vals) = build_laplacian_csr(np, &edges);
        let boundary = if constrain { find_boundary(src) } else { vec![false; np] };

        // Point-group selection: unselected points are left untouched (they still take part in the Laplacian).
        let sel = point_selection(src, p.string("group").unwrap_or(""))?;
        let pinned = |i: usize| (constrain && boundary.get(i).copied().unwrap_or(false)) || sel.as_ref().is_some_and(|s| !s.get(i).copied().unwrap_or(false));
        let solve = |b: &[f64]| -> Vec<f64> {
            let mut x = b.to_vec();
            cg_solve(&row_ptr, &col_ind, &vals, strength, b, &mut x, quality as usize, 1e-9);
            for (i, xi) in x.iter_mut().enumerate() {
                if pinned(i) {
                    *xi = b.get(i).copied().unwrap_or(0.0);
                }
            }
            x
        };

        // Solve for each coordinate independently
        let mut result = vec![[0.0; 3]; np];
        for axis in 0..3 {
            let b: Vec<f64> = pts.iter().map(|p| p[axis]).collect();
            for (i, xi) in solve(&b).into_iter().enumerate() {
                result[i][axis] = xi;
            }
        }

        let mut out = (**src).clone();
        *out.p_mut() = to_f32(OP, &result)?;

        // Attribute smoothing: every listed point attribute (F32 / V2 / V3 / V4 / FN) gets the same filter per channel.
        for name in p.string("attribs").unwrap_or("").split_whitespace().filter(|n| *n != "P") {
            let Some(a) = out.attr_mut(AttrClass::Point, name) else {
                return Err(failed(OP, &format!("unknown point attribute `{name}`")));
            };
            let (flat, w): (Vec<f32>, usize) = match &a.data {
                AttrData::F32(v) => (v.clone(), 1),
                AttrData::V2(v) => (v.iter().flatten().copied().collect(), 2),
                AttrData::V3(v) => (v.iter().flatten().copied().collect(), 3),
                AttrData::V4(v) => (v.iter().flatten().copied().collect(), 4),
                AttrData::FN { size, data } => (data.clone(), (*size as usize).max(1)),
                _ => return Err(failed(OP, &format!("attribute `{name}` is not a float attribute"))),
            };
            let mut res = flat.clone();
            for c in 0..w {
                let b: Vec<f64> = (0..np).map(|i| f64::from(flat.get(i * w + c).copied().unwrap_or(0.0))).collect();
                for (i, xi) in solve(&b).into_iter().enumerate() {
                    if let Some(s) = res.get_mut(i * w + c) {
                        *s = xi as f32;
                    }
                }
            }
            match &mut a.data {
                AttrData::F32(v) => *v = res,
                AttrData::V2(v) => *v = res.chunks_exact(2).map(|c| [c[0], c[1]]).collect(),
                AttrData::V3(v) => *v = res.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
                AttrData::V4(v) => *v = res.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect(),
                AttrData::FN { data, .. } => *data = res,
                _ => {}
            }
        }

        // Normal update: point N = uniform-averaged unit Newell normals of the smoothed surface.
        if toggle(p, "updatenormals")? {
            let pos = positions(&out);
            let n: Vec<[f32; 3]> = point_normals(&out, &pos).into_iter().map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]).collect();
            out.add_attr(Attribute { name: "N".into(), class: AttrClass::Point, data: AttrData::V3(n), type_info: TypeInfo::Normal });
        }
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cook_arc, cube, defaults, grid3};
    use super::*;
    use hexe_graph::ParamSet;

    fn with(method: u32, strength: f32, quality: i32) -> ParamSet {
        defaults(&*new())
            .with("method", ParamValue::Menu(method))
            .with("strength", ParamValue::F(strength))
            .with("filterquality", ParamValue::I(quality))
    }

    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "smooth");
        let params = n.params();
        assert_eq!(params.len(), 7);
        assert_eq!(params[0].name, "method");
        assert_eq!(params[1].name, "strength");
        assert_eq!(params[2].name, "filterquality");
        assert_eq!(params[3].name, "constrainboundary");
    }

    #[test]
    fn planar_grid_stays_planar() {
        let src = grid3();
        let out = cook(&*new(), src.clone(), &with(0, 0.5, 10)).expect("cooks");
        // All points should have z ~ 0
        for p in out.p() {
            assert!(p[2].abs() < 1e-5, "point {:?} not planar", p);
        }
    }

    #[test]
    fn implicit_smooth_converges() {
        // Simple test: box should converge towards centroid with enough iterations
        let src = cube();
        let out = cook(&*new(), src.clone(), &with(0, 0.8, 50)).expect("cooks");
        // All points should be close to origin (centroid of box)
        for p in out.p() {
            assert!(p[0].abs() < 0.2, "x coord should be near 0: {}", p[0]);
            assert!(p[1].abs() < 0.2, "y coord should be near 0: {}", p[1]);
            assert!(p[2].abs() < 0.2, "z coord should be near 0: {}", p[2]);
        }
    }

    #[test]
    fn boundary_pinned_when_constrain_on() {
        let src = grid3();
        let out = cook(&*new(), src.clone(), &with(0, 1.0, 10).with("constrainboundary", ParamValue::B(true)))
            .expect("cooks");

        // Grid 3x3: boundary points are all except center (index 4)
        for i in [0, 1, 2, 3, 5, 6, 7, 8] {
            assert_eq!(out.p()[i], src.p()[i], "boundary point {i} should be pinned");
        }
    }

    /// Group {4 = centre}: only the centre of grid3 moves; the other 8 points are bit-identical.
    #[test]
    fn point_group_limits_smoothing_to_selected_points() {
        let mut src = grid3();
        let mut mask = vec![false; 9];
        mask[4] = true;
        src.groups.push(hexe_geo::Group { name: "g".into(), class: AttrClass::Point, mask });
        // lift the centre so uniform smoothing has something to do
        src.p_mut()[4][2] = 1.0;
        let out = cook(&*new(), src.clone(), &with(0, 1.0, 50).with("group", ParamValue::S("g".into()))).expect("cooks");
        for i in [0, 1, 2, 3, 5, 6, 7, 8] {
            assert_eq!(out.p()[i], src.p()[i], "point {i} not in group");
        }
        assert!(out.p()[4][2] < 1.0, "centre must move toward its (z=0) neighbours: {}", out.p()[4][2]);
        let bad = cook(&*new(), src, &with(0, 1.0, 5).with("group", ParamValue::S("nope".into())));
        assert!(matches!(bad, Err(CookError::Failed(_))));
    }

    /// Attribute `w` (F32, 0 everywhere but one spike) smooths: spike drops, total stays ~conserved (Laplacian rows sum 0 -> mean preserved).
    #[test]
    fn extra_attribute_is_smoothed_and_mean_preserved_unlisted_untouched() {
        let mut src = grid3();
        let mut w = vec![0.0f32; 9];
        w[4] = 9.0;
        src.add_attr(Attribute { name: "w".into(), class: AttrClass::Point, data: AttrData::F32(w), type_info: TypeInfo::None });
        src.add_attr(Attribute { name: "u".into(), class: AttrClass::Point, data: AttrData::F32(vec![5.0; 9].into_iter().enumerate().map(|(i, v)| v + i as f32).collect()), type_info: TypeInfo::None });
        let out = cook(&*new(), src.clone(), &with(0, 1.0, 100).with("attribs", ParamValue::S("w".into()))).expect("cooks");
        let Some(AttrData::F32(ow)) = out.attr(AttrClass::Point, "w").map(|a| &a.data) else { panic!("w lost") };
        assert!(ow[4] < 9.0 && ow[4] > 0.0, "spike must drop: {}", ow[4]);
        let sum: f32 = ow.iter().sum();
        assert!((sum - 9.0).abs() < 1e-3, "mean preserved, sum {sum}");
        assert_eq!(out.attr(AttrClass::Point, "u"), src.attr(AttrClass::Point, "u"));
        let missing = cook(&*new(), src, &with(0, 1.0, 5).with("attribs", ParamValue::S("zz".into())));
        assert!(matches!(missing, Err(CookError::Failed(_))));
    }

    /// Smoothing a z-lifted grid centre then updatenormals: flat grid -> N = (0,0,+-1) exactly; centre N of the lifted grid is +-z (symmetric).
    #[test]
    fn updatenormals_writes_unit_point_normals() {
        let src = grid3();
        let off = cook(&*new(), src.clone(), &with(0, 0.5, 10)).expect("cooks");
        assert!(off.attr(AttrClass::Point, "N").is_none());
        let on = cook(&*new(), src, &with(0, 0.5, 10).with("updatenormals", ParamValue::B(true))).expect("cooks");
        let Some(AttrData::V3(n)) = on.attr(AttrClass::Point, "N").map(|a| &a.data) else { panic!("N missing") };
        assert_eq!(n.len(), 9);
        for v in n {
            assert!(v[0].abs() < 1e-5 && v[1].abs() < 1e-5 && (v[2].abs() - 1.0).abs() < 1e-5, "{v:?}");
        }
    }

    #[test]
    fn zero_quality_and_strength_share_arc() {
        let a = Arc::new(cube());
        let out1 = cook_arc(&*new(), &a, &with(0, 0.5, 0)).expect("cooks");
        assert!(Arc::ptr_eq(&out1, &a));

        let out2 = cook_arc(&*new(), &a, &with(0, 0.0, 10)).expect("cooks");
        assert!(Arc::ptr_eq(&out2, &a));
    }

    #[test]
    fn scene_loader_cooks_example() {
        let scene = r#"{"nodes":[{"name":"b","type":"box"},{"name":"s","type":"smooth","params":{"filterquality":5}}],"edges":[["b",0,"s",0]],"display":"s"}"#;
        let (mut g, id) = crate::load_scene(scene).expect("loads");
        assert_eq!(g.cook(id, 0.0).expect("cooks").point_count, 8);
    }
}
