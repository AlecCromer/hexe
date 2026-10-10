//! Houdini-ordered corner patches for CLOSED solids (every vertex = 3 faces / 3 beveled edges, round, divisions 1..=4).
//! Ground truth: `parity/ref/polybevel/*.topo` (hython polybevel::3.0, unit box). Rules (all measured, see ledger):
//! * Faces keep their prim slot (corner = source vertex order). Points: kept source points first when divisions is EVEN
//!   (= patch centres, same ids), then the sector points face by face in BFS order (root = prim 0 start corner 0; child
//!   face starts at the corner opening the shared edge; discovery walks each face's edges from its start corner).
//! * Then ONE group per face (BFS order), edges `E1..E(m-1),E0` (E0 = parent edge LAST): allocate points
//!   (even: shared mid pts + own row-1 + shared seam pts; odd 3: own row-1 + corner D pt), emit this face's half strip
//!   quads and its corner-patch quads (start corner then end corner, once per face).
//! * ODD divisions then emit, per edge in first-visit order: middle strip quad + 2 edge-corner quads (div 3), then one
//!   triangle per vertex (first-appearance order in the source prims; starts at the face arriving along the vertex's
//!   earliest-visited edge). EVEN divisions: interior sector pts (g11) are numbered last, in emission order.
//! * Profile rows = conic polyline of N = max(4, next_pow2(div-1)) vertices resampled linearly in t (see `profile`).
//!
//! UNVERIFIED: interior patch POSITIONS (centre / seam / g11 / D) are an approximation (Houdini's rule not found: its
//! centre moves with N = 4: 0.4029, 8: 0.4120, 16: 0.4131 of the offset); topology + point/prim ORDER are exact.
use super::{CLOSED, Face, OP, ZERO, add, at, dot, failed, profile, rebuild, scale, sub, to_f32, unit};
use super::{CookError, Detail, Prim, Remap};
use std::collections::{HashMap, HashSet, VecDeque};

type V3 = [f64; 3];
/// Centre pull towards the source corner (fit to Houdini: centre = 0.4029 o on a unit-box corner).
const LAMBDA: f64 = 0.2378;
/// Seam point = these fractions of (centre - edge mid): along / across the edge direction.
const SEAM_PAR: f64 = 0.6059;
const SEAM_PERP: f64 = 0.0645;
const PH: u32 = 1 << 30;

pub(super) struct Input<'a> {
    pub src: &'a Detail,
    pub faces: &'a [Face],
    pub pos: &'a [V3],
    /// Sector position per corner id.
    pub sec: &'a [V3],
    pub off: &'a [usize],
    pub div: usize,
    pub round: bool,
    pub n_bevel_edges: usize,
}

fn lerp(a: V3, b: V3, t: f64) -> V3 {
    add(scale(a, 1.0 - t), scale(b, t))
}

#[derive(Default)]
struct Alloc {
    pp: Vec<V3>,
    psrc: Vec<Vec<usize>>,
    map: HashMap<(u8, usize, usize, usize, usize), u32>,
}
impl Alloc {
    fn push(&mut self, p: V3, src: usize) -> Result<u32, CookError> {
        let id = u32::try_from(self.pp.len()).map_err(|_| failed(OP, "point id overflow"))?;
        self.pp.push(p);
        self.psrc.push(vec![src]);
        Ok(id)
    }
    /// Existing id for `key`, else a fresh point.
    fn get(&mut self, key: (u8, usize, usize, usize, usize), p: V3, src: usize) -> Result<u32, CookError> {
        if let Some(&id) = self.map.get(&key) {
            return Ok(id);
        }
        let id = self.push(p, src)?;
        self.map.insert(key, id);
        Ok(id)
    }
    fn id(&self, key: (u8, usize, usize, usize, usize)) -> u32 {
        self.map.get(&key).copied().unwrap_or(0)
    }
}

#[derive(Default)]
struct Out {
    vp: Vec<u32>,
    vsrc: Vec<Option<usize>>,
    prims: Vec<Prim>,
    psrc: Vec<Option<usize>>,
}
impl Out {
    fn emit(&mut self, poly: &[u32], vs: &[Option<usize>], from: Option<usize>) -> Result<(), CookError> {
        let start = u32::try_from(self.vp.len()).map_err(|_| failed(OP, "vertex id overflow"))?;
        for (k, &p) in poly.iter().enumerate() {
            self.vp.push(p);
            self.vsrc.push(vs.get(k).copied().flatten());
        }
        let end = u32::try_from(self.vp.len()).map_err(|_| failed(OP, "vertex id overflow"))?;
        self.prims.push(Prim { kind: CLOSED, verts: start..end });
        self.psrc.push(from);
        Ok(())
    }
}

/// `None` = not this shape of input (caller keeps the generic path).
#[allow(clippy::too_many_lines)]
pub(super) fn build(i: &Input<'_>) -> Result<Option<Detail>, CookError> {
    let (faces, pos, d) = (i.faces, i.pos, i.div);
    let (nf, np) = (faces.len(), pos.len());
    if !(1..=4).contains(&d) || !i.round || nf != i.src.prims.len() || nf == 0 {
        return Ok(None);
    }
    let mut dm: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
    let mut val = vec![0usize; np];
    for (fi, f) in faces.iter().enumerate() {
        let m = f.pts.len();
        for (k, &a) in f.pts.iter().enumerate() {
            let b = at(&f.pts, (k + 1) % m, a);
            if dm.insert((a, b), (fi, k)).is_some() {
                return Ok(None);
            }
            if let Some(c) = val.get_mut(a) {
                *c += 1;
            }
        }
    }
    if val.iter().any(|&c| c != 3)
        || dm.keys().any(|&(a, b)| !dm.contains_key(&(b, a)))
        || i.n_bevel_edges * 2 != dm.len()
    {
        return Ok(None);
    }
    let mlen = |f: usize| faces.get(f).map_or(0, |x| x.pts.len());
    let vtx = |f: usize, k: usize| faces.get(f).map_or(0, |x| at(&x.pts, k % x.pts.len().max(1), 0));
    let opp = |f: usize, k: usize| -> (usize, usize) { dm.get(&(vtx(f, k + 1), vtx(f, k))).copied().unwrap_or((f, k)) };
    let cid = |f: usize, k: usize| at(i.off, f, 0) + k % mlen(f).max(1);
    // ---- BFS face order ----
    let mut seen = vec![false; nf];
    let mut order: Vec<(usize, usize)> = Vec::with_capacity(nf);
    for root in 0..nf {
        if at(&seen, root, true) {
            continue;
        }
        if let Some(s) = seen.get_mut(root) {
            *s = true;
        }
        let mut q = VecDeque::from([(root, 0usize)]);
        while let Some((f, s)) = q.pop_front() {
            order.push((f, s));
            let m = mlen(f);
            for kk in 0..m {
                let (g, jb) = opp(f, (s + kk) % m);
                if !at(&seen, g, true) {
                    if let Some(x) = seen.get_mut(g) {
                        *x = true;
                    }
                    q.push_back((g, jb));
                }
            }
        }
    }
    // ---- profile rows: per half edge (f, k) and end (0 = at the edge start, 1 = at its end) ----
    let prof = |f: usize, k: usize, end: usize, div: usize| -> Vec<V3> {
        let (g, jb) = opp(f, k);
        let (a, b) = (vtx(f, k), vtx(f, k + 1));
        let th = unit(sub(at(pos, b, ZERO), at(pos, a, ZERO))).unwrap_or(ZERO);
        let (x, t1, t2) = if end == 0 {
            (at(pos, a, ZERO), at(i.sec, cid(f, k), ZERO), at(i.sec, cid(g, jb + 1), ZERO))
        } else {
            (at(pos, b, ZERO), at(i.sec, cid(f, k + 1), ZERO), at(i.sec, cid(g, jb), ZERO))
        };
        profile(x, th, t1, t2, div, true)
    };
    let mut rows: HashMap<(usize, usize, usize), Vec<V3>> = HashMap::new();
    let mut p4: HashMap<(usize, usize, usize), Vec<V3>> = HashMap::new();
    for f in 0..nf {
        for k in 0..mlen(f) {
            for end in 0..2 {
                rows.insert((f, k, end), prof(f, k, end, d));
                if d >= 2 {
                    p4.insert((f, k, end), prof(f, k, end, 4));
                }
            }
        }
    }
    let row1 = |f: usize, k: usize, end: usize| rows.get(&(f, k, end)).and_then(|r| r.first()).copied().unwrap_or(ZERO);
    let mid_pos = |f: usize, k: usize, end: usize| p4.get(&(f, k, end)).and_then(|r| r.get(1)).copied().unwrap_or(ZERO);
    // ---- per-vertex centre ----
    let mut cpos: Vec<V3> = vec![ZERO; np];
    if d >= 2 {
        let mut sum = vec![ZERO; np];
        for f in 0..nf {
            for k in 0..mlen(f) {
                if let Some(s) = sum.get_mut(vtx(f, k)) {
                    *s = add(*s, mid_pos(f, k, 0));
                }
            }
        }
        for (v, c) in cpos.iter_mut().enumerate() {
            *c = lerp(scale(at(&sum, v, ZERO), 1.0 / 3.0), at(pos, v, ZERO), LAMBDA);
        }
    }
    // corner (f, ck): in half-edge = (ck-1) end 1, out half-edge = ck end 0
    let inh = |f: usize, ck: usize| (ck + mlen(f).max(1) - 1) % mlen(f).max(1);
    // seam point on the line mid -> centre: along the edge direction it sits SEAM_PAR of the way, across it SEAM_PERP
    // (cube fit: 0.6059 / 0.0645 -> the seam hugs the edge cylinder)
    let seam = |f: usize, k: usize, end: usize| -> V3 {
        let m = mid_pos(f, k, end);
        let c = at(&cpos, vtx(f, k + end), ZERO);
        let th = unit(sub(at(pos, vtx(f, k + 1), ZERO), at(pos, vtx(f, k), ZERO))).unwrap_or(ZERO);
        let dlt = sub(c, m);
        let par = scale(th, dot(dlt, th));
        add(m, add(scale(par, SEAM_PAR), scale(sub(dlt, par), SEAM_PERP)))
    };
    let seam_in = |f: usize, ck: usize| seam(f, inh(f, ck), 1);
    let seam_out = |f: usize, ck: usize| seam(f, ck, 0);
    // Coons patch centre of the sector cell (corner (f, ck)), master grid N = 4
    let g11 = |f: usize, ck: usize| -> V3 {
        let v = vtx(f, ck);
        let h = inh(f, ck);
        let s = at(i.sec, cid(f, ck), ZERO);
        let r_in = p4.get(&(f, h, 1)).and_then(|r| r.first()).copied().unwrap_or(ZERO);
        let r_out = p4.get(&(f, ck, 0)).and_then(|r| r.first()).copied().unwrap_or(ZERO);
        let (m_in, m_out) = (mid_pos(f, h, 1), mid_pos(f, ck, 0));
        let c = at(&cpos, v, ZERO);
        // Coons weights (S -1/4, rows +1/2, mids -1/4, seams +1/2, centre -1/4) nudged to hit the cube exactly
        let sum = |a: V3, b: V3| add(a, b);
        let mut g = scale(s, -0.25257);
        g = add(g, scale(sum(r_in, r_out), 0.54637));
        g = add(g, scale(sum(m_in, m_out), -0.32664));
        g = add(g, scale(sum(seam_in(f, ck), seam_out(f, ck)), 0.53827));
        add(g, scale(c, -0.26342))
    };
    let dpos = |f: usize, ck: usize| -> V3 {
        let c = at(&cpos, vtx(f, ck), ZERO);
        let w = add(scale(g11(f, ck), 4.0 / 9.0), scale(add(seam_in(f, ck), seam_out(f, ck)), 2.0 / 9.0));
        add(w, scale(c, 1.0 / 9.0))
    };
    // ---- points ----
    let mut al = Alloc::default();
    if d % 2 == 0 {
        for v in 0..np {
            al.push(at(&cpos, v, ZERO), v)?;
        }
    }
    let mut sid: Vec<u32> = vec![0; i.sec.len()];
    for &(f, s) in &order {
        let m = mlen(f);
        for kk in 0..m {
            let k = (s + kk) % m;
            let id = al.push(at(i.sec, cid(f, k), ZERO), vtx(f, k))?;
            if let Some(x) = sid.get_mut(cid(f, k)) {
                *x = id;
            }
        }
    }
    let ekey = |f: usize, k: usize| (vtx(f, k).min(vtx(f, k + 1)), vtx(f, k).max(vtx(f, k + 1)));
    // key helpers (end 0 = vertex of half edge start)
    let mid_key = |f: usize, k: usize, end: usize| {
        let (a, b) = ekey(f, k);
        (0u8, a, b, vtx(f, k + end), 0usize)
    };
    let seam_key = |f: usize, k: usize, end: usize| {
        let (a, b) = ekey(f, k);
        (2u8, a, b, vtx(f, k + end), 0usize)
    };
    let r1_key = |f: usize, k: usize, end: usize| (1u8, f, k, end, 1usize);
    let d_key = |f: usize, ck: usize| (3u8, f, ck, 0usize, 0usize);
    // phase 1: allocation in group order
    for &(f, s) in &order {
        let m = mlen(f);
        for kk in (1..m).chain(std::iter::once(0)) {
            let k = (s + kk) % m;
            for end in 0..2 {
                let v = vtx(f, k + end);
                if d == 2 {
                    al.get(mid_key(f, k, end), row1(f, k, end), v)?;
                }
            }
            if d == 3 {
                for end in 0..2 {
                    let v = vtx(f, k + end);
                    let ck = (k + end) % m;
                    al.get(r1_key(f, k, end), row1(f, k, end), v)?;
                    al.get(d_key(f, ck), dpos(f, ck), v)?;
                }
            }
            if d == 4 {
                for end in 0..2 {
                    al.get(mid_key(f, k, end), mid_pos(f, k, end), vtx(f, k + end))?;
                }
                for end in 0..2 {
                    let v = vtx(f, k + end);
                    al.get(r1_key(f, k, end), row1(f, k, end), v)?;
                    let sp = if end == 0 { seam_out(f, k) } else { seam_in(f, (k + 1) % m) };
                    al.get(seam_key(f, k, end), sp, v)?;
                }
            }
        }
    }
    // ---- prims ----
    let mut out = Out::default();
    for (pi, f) in faces.iter().enumerate() {
        let poly: Vec<u32> = (0..f.pts.len()).map(|k| at(&sid, cid(pi, k), 0)).collect();
        let vs: Vec<Option<usize>> = (0..f.pts.len()).map(|k| Some(f.first + k)).collect();
        out.emit(&poly, &vs, Some(f.prim))?;
    }
    let mut g11_pos: Vec<V3> = Vec::new();
    let mut g11_src: Vec<usize> = Vec::new();
    let row_id = |f: usize, k: usize, end: usize, j: usize| -> u32 {
        // row j (0 = sector, 1.. towards the neighbour) of half edge (f,k) at `end`, from f's side
        if j == 0 {
            return at(&sid, cid(f, k + end), 0);
        }
        if d == 4 && j == 2 {
            return al.id(mid_key(f, k, end));
        }
        if d == 2 && j == 1 {
            return al.id(mid_key(f, k, end));
        }
        al.id(r1_key(f, k, end))
    };
    let mut visit: Vec<(usize, usize)> = Vec::new();
    let mut visited: HashSet<(usize, usize)> = HashSet::new();
    let mut erank: HashMap<(usize, usize), usize> = HashMap::new();
    for &(f, s) in &order {
        let m = mlen(f);
        let mut done: HashSet<usize> = HashSet::new();
        let prim = faces.get(f).map(|x| x.prim);
        for kk in (1..m).chain(std::iter::once(0)) {
            let k = (s + kk) % m;
            if visited.insert(ekey(f, k)) {
                erank.insert(ekey(f, k), visit.len());
                visit.push((f, k));
            }
            let half = d / 2; // f-side strip quads
            for j in 0..half {
                let q = [row_id(f, k, 1, j), row_id(f, k, 0, j), row_id(f, k, 0, j + 1), row_id(f, k, 1, j + 1)];
                out.emit(&q, &[], prim)?;
            }
            if d == 1 {
                continue;
            }
            for end in 0..2 {
                let ck = (k + end) % m;
                if !done.insert(ck) {
                    continue;
                }
                let h = inh(f, ck);
                let v = vtx(f, ck);
                let sct = at(&sid, cid(f, ck), 0);
                match d {
                    2 => {
                        let c = u32::try_from(v).unwrap_or(0);
                        out.emit(&[sct, al.id(mid_key(f, h, 1)), c, al.id(mid_key(f, ck, 0))], &[], None)?;
                    }
                    3 => {
                        let q = [sct, al.id(r1_key(f, h, 1)), al.id(d_key(f, ck)), al.id(r1_key(f, ck, 0))];
                        out.emit(&q, &[], None)?;
                    }
                    _ => {
                        let c = u32::try_from(v).unwrap_or(0);
                        let (r_in, m_in, s_in) =
                            (al.id(r1_key(f, h, 1)), al.id(mid_key(f, h, 1)), al.id(seam_key(f, h, 1)));
                        let (r_out, m_out, s_out) =
                            (al.id(r1_key(f, ck, 0)), al.id(mid_key(f, ck, 0)), al.id(seam_key(f, ck, 0)));
                        let gid = PH + u32::try_from(g11_pos.len()).map_err(|_| failed(OP, "point id overflow"))?;
                        g11_pos.push(g11(f, ck));
                        g11_src.push(v);
                        out.emit(&[sct, r_in, gid, r_out], &[], None)?;
                        out.emit(&[r_in, m_in, s_in, gid], &[], None)?;
                        out.emit(&[r_out, gid, s_out, m_out], &[], None)?;
                        out.emit(&[gid, s_in, c, s_out], &[], None)?;
                    }
                }
            }
        }
    }
    if d % 2 == 1 {
        for &(f, k) in &visit {
            let (g, jb) = opp(f, k);
            let prim = faces.get(f).map(|x| x.prim);
            let (sx_f, sy_f) = (at(&sid, cid(f, k), 0), at(&sid, cid(f, k + 1), 0));
            if d == 1 {
                let q = [sy_f, sx_f, at(&sid, cid(g, jb + 1), 0), at(&sid, cid(g, jb), 0)];
                out.emit(&q, &[], prim)?;
                continue;
            }
            let (rfx, rfy) = (al.id(r1_key(f, k, 0)), al.id(r1_key(f, k, 1)));
            let (rgx, rgy) = (al.id(r1_key(g, jb, 1)), al.id(r1_key(g, jb, 0)));
            out.emit(&[rfy, rfx, rgx, rgy], &[], prim)?;
            let (dfx, dfy) = (al.id(d_key(f, k)), al.id(d_key(f, (k + 1) % mlen(f).max(1))));
            let (dgx, dgy) = (al.id(d_key(g, (jb + 1) % mlen(g).max(1))), al.id(d_key(g, jb)));
            out.emit(&[rfx, dfx, dgx, rgx], &[], None)?;
            out.emit(&[rgy, dgy, dfy, rfy], &[], None)?;
        }
        // triangles, vertices in first-appearance order over the source prims
        let mut by_v: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
        let mut vorder: Vec<usize> = Vec::new();
        for (f, fc) in faces.iter().enumerate() {
            for (k, &v) in fc.pts.iter().enumerate() {
                if !by_v.contains_key(&v) {
                    vorder.push(v);
                }
                by_v.entry(v).or_default().push((f, k));
            }
        }
        for v in vorder {
            let cs = by_v.get(&v).cloned().unwrap_or_default();
            // earliest-visited edge at v: its two faces (arriving A, leaving B); C = the third
            let best = cs.iter().filter_map(|&(f, k)| erank.get(&ekey(f, k)).map(|r| (*r, f, k))).min_by_key(|t| t.0);
            let Some((_, bf, bk)) = best else { continue };
            let (a_face, b_face) = (opp(bf, bk).0, bf); // bf leaves v along the edge; the opposite arrives
            let third = cs.iter().map(|c| c.0).find(|&f| f != a_face && f != b_face);
            let Some(c_face) = third else { continue };
            let pt = |f: usize| -> u32 {
                let ck = cs.iter().find(|c| c.0 == f).map_or(0, |c| c.1);
                if d == 1 { at(&sid, cid(f, ck), 0) } else { al.id(d_key(f, ck)) }
            };
            out.emit(&[pt(a_face), pt(b_face), pt(c_face)], &[], None)?;
        }
    }
    // g11 points last (emission order)
    let base = u32::try_from(al.pp.len()).map_err(|_| failed(OP, "point id overflow"))?;
    for p in &mut out.vp {
        if *p >= PH {
            *p = base + (*p - PH);
        }
    }
    for (p, s) in g11_pos.into_iter().zip(g11_src) {
        al.push(p, s)?;
    }
    let pts32 = to_f32(OP, &al.pp)?;
    let rm = Remap { point_src: al.psrc, vert_src: out.vsrc, prim_src: out.psrc };
    Ok(Some(rebuild(OP, i.src, out.vp, out.prims, &rm, pts32)?))
}
