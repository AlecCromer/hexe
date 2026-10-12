//! `cargo xtask boolstart`: START-VERTEX hypothesis table for the Houdini boolean::2.0 corpus (`parity/ref/boolean/bx*.topo`, H35).
//! Analysis tool, not a gate: always exits 0 and prints how many SPLIT polys (not a full unit face) of the hython output each
//! hypothesis puts at index 0. A hypothesis scores a poly only when it names ONE vertex (ties / not applicable = miss).
//! Box A = unit box, box B = unit box translated by `t/4` (name `bx<tx>_<ty>_<tz>_<op>`), source faces = hython `box` prim cycles.
//! A poly belongs to the A/B face whose plane holds all its vertices; polys lying on one A plane AND one B plane (coplanar
//! offsets) or on none are ambiguous and skipped. Kept corner = poly vertex equal to a corner of its source face, else new.
//! `--sweep` additionally prints the plan of the hython offset sweep (`oracle/houdini/probes/probe-boolean-start-sweep.py`).
use crate::{Res, root};
use std::path::Path;

type V = [i64; 3];
const BOX: [V; 8] = [
    [-500, -500, -500],
    [500, -500, -500],
    [500, -500, 500],
    [-500, -500, 500],
    [-500, 500, -500],
    [500, 500, -500],
    [500, 500, 500],
    [-500, 500, 500],
];
const FACES: [[usize; 4]; 6] = [[1, 5, 4, 0], [2, 6, 5, 1], [3, 7, 6, 2], [0, 4, 7, 3], [2, 1, 0, 3], [5, 6, 7, 4]];

struct Rec {
    mesh: Vec<V>,
    t: V,
    pos: Vec<V>,
    /// index in the output-oriented source cycle (None = new vertex)
    ks: Vec<Option<usize>>,
    /// source point id of a kept corner
    sid: Vec<Option<usize>>,
    coplanar: bool,
}

fn parse_topo(text: &str) -> (Vec<V>, Vec<Vec<usize>>) {
    let (mut p, mut f) = (Vec::new(), Vec::new());
    for l in text.lines() {
        let w: Vec<&str> = l.split_whitespace().collect();
        match w.first() {
            Some(&"P") if w.len() >= 5 => {
                let q = |s: &str| (s.parse::<f64>().unwrap_or(0.0) * 1000.0).round() as i64;
                p.push([q(w[2]), q(w[3]), q(w[4])]);
            }
            Some(&"F") => f.push(w.iter().skip(2).filter_map(|s| s.parse().ok()).collect()),
            _ => {}
        }
    }
    (p, f)
}

fn newell(p: &[V]) -> [i64; 3] {
    let mut n = [0i64; 3];
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}

/// Does every vertex of `poly` lie on the plane of box face `f` of the box translated by `off`?
fn on_face(poly: &[V], f: usize, off: V) -> bool {
    let c: Vec<V> = FACES[f].iter().map(|&i| [0, 1, 2].map(|a| BOX[i][a] + off[a])).collect();
    (0..3).any(|a| c.iter().all(|q| q[a] == c[0][a]) && poly.iter().all(|q| q[a] == c[0][a]))
}

fn records(dir: &Path) -> Result<(Vec<Rec>, usize), String> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "topo"))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("bx")))
        .collect();
    files.sort();
    let (mut out, mut amb) = (Vec::new(), 0usize);
    for file in files {
        let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let body = stem.trim_start_matches("bx");
        let offs = body.rsplit_once('_').map_or("", |x| x.0);
        let q: Vec<i64> = offs.split('_').filter_map(|s| s.parse().ok()).collect();
        if q.len() != 3 {
            continue;
        }
        let tb = [q[0] * 250, q[1] * 250, q[2] * 250];
        let (pts, faces) = parse_topo(&std::fs::read_to_string(&file).map_err(|e| e.to_string())?);
        for f in &faces {
            let pos: Vec<V> = f.iter().filter_map(|&i| pts.get(i).copied()).collect();
            let mut src: Vec<(bool, usize)> = Vec::new();
            for (b, off) in [(false, [0; 3]), (true, tb)] {
                src.extend((0..6).filter(|&k| on_face(&pos, k, off)).map(|k| (b, k)));
            }
            let [(b, k)] = src[..] else {
                amb += 1;
                continue;
            };
            let off = if b { tb } else { [0; 3] };
            let corners: Vec<V> = FACES[k].iter().map(|&i| [0, 1, 2].map(|a| BOX[i][a] + off[a])).collect();
            let kidx: Vec<Option<usize>> = pos.iter().map(|p| corners.iter().position(|c| c == p)).collect();
            if pos.len() == 4 && kidx.iter().all(Option::is_some) {
                continue; // untouched face
            }
            let sn = newell(&corners);
            let on = newell(&pos);
            let flip = (0..3).map(|a| sn[a] * on[a]).sum::<i64>() < 0;
            let orient = |i: usize| if flip { [0, 3, 2, 1][i] } else { i };
            out.push(Rec {
                mesh: pts.clone(),
                t: tb,
                ks: kidx.iter().map(|k| k.map(orient)).collect(),
                sid: kidx.iter().map(|c| c.map(|i| FACES[k][i])).collect(),
                pos,
                coplanar: q.contains(&0),
            });
        }
    }
    Ok((out, amb))
}

type Hyp = Box<dyn Fn(&Rec) -> Option<usize>>;

/// The single index minimising `key` over `idx`, or None on a tie / empty set.
fn unique_min<K: Ord + Copy>(idx: impl Iterator<Item = usize>, key: impl Fn(usize) -> K) -> Option<usize> {
    let mut best: Option<(usize, K, bool)> = None;
    for i in idx {
        let k = key(i);
        best = match best {
            None => Some((i, k, false)),
            Some((_, bk, _)) if k < bk => Some((i, k, false)),
            Some((bi, bk, _)) if bk == k => Some((bi, bk, true)),
            b => b,
        };
    }
    best.and_then(|(i, _, tie)| (!tie).then_some(i))
}

/// Cyclic structure when the poly is ONE run of kept corners + ONE run of new points: (first kept, last kept, new vertices in order).
fn runs(r: &Rec) -> Option<(usize, usize, Vec<usize>)> {
    let n = r.ks.len();
    let kept = |i: usize| r.ks[i % n].is_some();
    let starts: Vec<usize> = (0..n).filter(|&i| kept(i) && !kept(i + n - 1)).collect();
    let [first] = starts[..] else { return None };
    let len = (0..n).take_while(|&j| kept(first + j)).count();
    let news: Vec<usize> = (len..n).map(|j| (first + j) % n).collect();
    (!news.is_empty()).then_some((first, (first + len - 1) % n, news))
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

fn hypotheses() -> Vec<(String, Hyp)> {
    let mut h: Vec<(String, Hyp)> = Vec::new();
    let mut add = |n: &str, f: Hyp| h.push((n.to_string(), f));
    add(
        "kept corner, lowest source-cycle slot (first kept corner)",
        Box::new(|r| unique_min((0..r.ks.len()).filter(|&i| r.ks[i].is_some()), |i| r.ks[i])),
    );
    add(
        "kept corner, highest source-cycle slot",
        Box::new(|r| unique_min((0..r.ks.len()).filter(|&i| r.ks[i].is_some()), |i| std::cmp::Reverse(r.ks[i]))),
    );
    add(
        "kept corner, lowest source point index",
        Box::new(|r| unique_min((0..r.ks.len()).filter(|&i| r.sid[i].is_some()), |i| r.sid[i])),
    );
    add(
        "kept corner, highest source point index",
        Box::new(|r| unique_min((0..r.ks.len()).filter(|&i| r.sid[i].is_some()), |i| std::cmp::Reverse(r.sid[i]))),
    );
    add(
        "first new vertex of the cut chain (after the last kept corner)",
        Box::new(|r| runs(r).and_then(|x| x.2.first().copied())),
    );
    add("last new vertex of the cut chain", Box::new(|r| runs(r).and_then(|x| x.2.last().copied())));
    add(
        "middle new vertex of the cut chain (odd length)",
        Box::new(|r| runs(r).and_then(|x| (x.2.len() % 2 == 1).then(|| x.2[x.2.len() / 2]))),
    );
    add("first kept corner of the kept run (winding-relative)", Box::new(|r| runs(r).map(|x| x.0)));
    add("last kept corner of the kept run (winding-relative)", Box::new(|r| runs(r).map(|x| x.1)));
    add("start of the longest edge", Box::new(|r| edge_pick(r, true, false)));
    add("end of the longest edge", Box::new(|r| edge_pick(r, true, true)));
    add("start of the shortest edge", Box::new(|r| edge_pick(r, false, false)));
    add("end of the shortest edge", Box::new(|r| edge_pick(r, false, true)));
    let f = |r: &Rec| r.pos.iter().map(|p| p.map(|c| c as f64)).collect::<Vec<_>>();
    type Refp = fn(&Rec, &[[f64; 3]]) -> [f64; 3];
    let refs: [(&str, Refp); 4] = [
        ("polygon vertex mean", |_, v| [0, 1, 2].map(|a| v.iter().map(|p| p[a]).sum::<f64>() / v.len().max(1) as f64)),
        ("origin (centre of A)", |_, _| [0.0; 3]),
        ("centre of B", |r, _| r.t.map(|c| c as f64)),
        ("mean of all result points", |r, _| {
            let m: Vec<[f64; 3]> = r.mesh.iter().map(|p| p.map(|c| c as f64)).collect();
            [0, 1, 2].map(|a| m.iter().map(|p| p[a]).sum::<f64>() / m.len().max(1) as f64)
        }),
    ];
    for (n, rf) in refs {
        add(
            &format!("nearest to {n}"),
            Box::new(move |r| {
                let v = f(r);
                let c = rf(r, &v);
                unique_min(0..v.len(), |i| (dist2(v[i], c) * 1e3).round() as i64)
            }),
        );
        add(
            &format!("farthest from {n}"),
            Box::new(move |r| {
                let v = f(r);
                let c = rf(r, &v);
                unique_min(0..v.len(), |i| -((dist2(v[i], c) * 1e3).round() as i64))
            }),
        );
    }
    h
}

fn edge_pick(r: &Rec, longest: bool, end: bool) -> Option<usize> {
    let n = r.pos.len();
    let len = |i: usize| {
        let (a, b) = (r.pos[i], r.pos[(i + 1) % n]);
        let l: i64 = (0..3).map(|k| (a[k] - b[k]).pow(2)).sum();
        if longest { -l } else { l }
    };
    unique_min(0..n, len).map(|e| if end { (e + 1) % n } else { e })
}

/// Best member of a parametrised family: (hits per subset, label of the best-by-total member).
fn family<C: Copy + std::fmt::Debug>(
    recs: &[Rec],
    cfgs: &[C],
    pick: impl Fn(&Rec, C) -> Option<usize>,
) -> ([usize; 3], String) {
    let mut best = ([0usize; 3], String::new());
    let mut top = 0;
    for &c in cfgs {
        let mut hits = [0usize; 3];
        for r in recs {
            if pick(r, c) == Some(0) {
                hits[0] += 1;
                hits[if r.coplanar { 2 } else { 1 }] += 1;
            }
        }
        if hits[0] > top {
            top = hits[0];
            best = (hits, format!("{c:?}"));
        }
    }
    best
}

pub fn run(args: &[String]) -> Res {
    let dir = root().join("parity").join("ref").join("boolean");
    let (recs, amb) = records(&dir)?;
    let tot = [recs.len(), recs.iter().filter(|r| !r.coplanar).count(), recs.iter().filter(|r| r.coplanar).count()];
    println!(
        "boolstart: {} split polys scored (non-coplanar offsets {}, coplanar offsets {}), {amb} ambiguous skipped; hit = hypothesis names exactly the Houdini start vertex",
        tot[0], tot[1], tot[2]
    );
    println!("{:<68} {:>9} {:>9} {:>9}", "hypothesis", "all", "non-copl", "coplanar");
    let pct = |h: [usize; 3]| (0..3).map(|i| format!("{:>4}/{:<4}", h[i], tot[i])).collect::<Vec<_>>();
    for (name, f) in hypotheses() {
        let mut hits = [0usize; 3];
        for r in &recs {
            if f(r) == Some(0) {
                hits[0] += 1;
                hits[if r.coplanar { 2 } else { 1 }] += 1;
            }
        }
        let p = pct(hits);
        println!("{name:<68} {} {} {}", p[0], p[1], p[2]);
    }
    // axis-lexicographic order over (all | new | kept) vertices, all 48 permutation/sign choices
    let mut lex = Vec::new();
    for sel in 0..3u8 {
        for perm in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
            for s in 0..8u8 {
                lex.push((
                    sel,
                    perm,
                    [(s & 1) as i64 * 2 - 1, ((s >> 1) & 1) as i64 * 2 - 1, ((s >> 2) & 1) as i64 * 2 - 1],
                ));
            }
        }
    }
    let (h, l) = family(&recs, &lex, |r, (sel, perm, sg)| {
        let ok = |i: usize| match sel {
            0 => true,
            1 => r.ks[i].is_none(),
            _ => r.ks[i].is_some(),
        };
        unique_min((0..r.pos.len()).filter(|&i| ok(i)), |i| lexkey(r, i, perm, sg))
    });
    let p = pct(h);
    println!("{:<68} {} {} {}", "best axis-lexicographic order (sel,perm,signs) of 144", p[0], p[1], p[2]);
    println!("    best = {l}");
    // one global linear key a.p over a golden-spiral sample of directions, min and max
    let dirs: Vec<(bool, usize)> = (0..4000).flat_map(|i| [(false, i), (true, i)]).collect();
    let (h, l) = family(&recs, &dirs, |r, (mx, i)| {
        let z = 1.0 - 2.0 * (i as f64 + 0.5) / 4000.0;
        let (rr, ph) = ((1.0 - z * z).sqrt(), i as f64 * 2.399_963_229_728_653);
        let a = [rr * ph.cos(), rr * ph.sin(), z];
        let k = |j: usize| {
            let v: f64 = (0..3).map(|c| a[c] * r.pos[j][c] as f64).sum();
            ((if mx { -v } else { v }) * 1e6).round() as i64
        };
        unique_min(0..r.pos.len(), k)
    });
    let p = pct(h);
    println!("{:<68} {} {} {}", "best single global linear key a.p (4000 directions, min|max)", p[0], p[1], p[2]);
    println!("    best = {l}");
    if args.iter().any(|a| a == "--sweep") {
        println!(
            "sweep: parity/ref/boolean/start-sweep.txt (hython, oracle/houdini/probes/probe-boolean-start-sweep.py)"
        );
    }
    Ok(())
}

fn lexkey(r: &Rec, i: usize, perm: [usize; 3], sg: [i64; 3]) -> (i64, i64, i64) {
    (sg[0] * r.pos[i][perm[0]], sg[1] * r.pos[i][perm[1]], sg[2] * r.pos[i][perm[2]])
}
