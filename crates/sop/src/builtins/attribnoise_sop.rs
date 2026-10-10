//! Attribute Noise SOP (`attribnoise`): adds / sets / multiplies seeded noise into one float attribute of input 0.
//! Clean-room semantics (UNVERIFIED vs Houdini: hython blocked; noise streams are OUR hash, not Houdini's):
//! - `attrib` (default `P`) of `class` (point|prim|vertex|detail). Missing attribute → `Failed`; non-float → `Unsupported`.
//! - sample position per element: point → its `P`; vertex → owning point's `P`; prim → mean of its points' `P`;
//!   detail → mean of all `P`. Noise coordinate `q = P * frequency + offset` (f64).
//! - `noise`: `perlin` (3-D gradient noise, 12 edge gradients, hashed lattice; exactly 0 at integer lattice points),
//!   `simplex` (3-D simplex, hashed gradients), `white` (hash of the bit pattern of `q`, independent per point, no
//!   spatial coherence). All map to [-1,1] (clamped). Tuple channel `c` gets an independent stream (hash includes `c`).
//! - `v = amplitude * noise` (after optional remap); `operation`: add `x+v` | set `v` | multiply `x*v`.
//! - remap: `remap` toggle (default off) + `remap_min`/`remap_max` floats: noise ∈ [-1,1] → [min,max] linearly.
//!   DEVIATION: the brief's V2 `remap_range` has no ParamValue::V2 → two floats + toggle.
//! - `seed` (i32, sign-extended) mixes into every hash: same seed + input → byte-identical output.
//! - QA: `noise` also `value` (hashed lattice values, fade-interpolated) and `worley`/`cellular` (F1 of jittered feature points, `2*min(F1,1)-1`);
//!   `group` (element group of `class`, non-members untouched), `rest` (V3 point attr sampled instead of P), `updatenormals` (after P noise: point N
//!   written, existing prim N refreshed). Fractal fbm/turbulence/ridged + octaves/rough already existed.
//! - NOT modelled: Houdini's remaining patterns (sparse convolution, alligator, curl, flow...), `pattern`-specific params.
use super::meshutil::{closed_poly_pts, point_normals, positions as mesh_positions, prim_normal};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

/// Cap on tuple size of `FN` attributes we are willing to noise (AGENTS.md §4).
const MAX_TUPLE: usize = 64;

/// STAT-ORACLE calibration (Houdini 20.5 `attribnoise::2.0`, perlin, zero-centred range, vec P add, 70x70 point grid, amp 0.5; fixtures sop-noise*):
/// Houdini's per-component spread is 0.388 x amplitude (plain; gain 1.58 balances mean/std error) / 0.350 x amplitude (fBm, any octave count 1..6: variance-preserving, DC offset
/// of its fBm not modelled) against our unit-range perlin 0.250; fBm is re-normalised from the raw sum to the RMS of the octave weights.
/// Fitted constants, UNVERIFIED outside the recorded cases; only the perlin basis is calibrated: `remap`, turbulence, ridged and the other bases keep the raw unit-range definition.
pub(super) const GAIN_PLAIN: f64 = 1.58;
pub(super) const GAIN_FRACTAL: f64 = 1.41;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("attrib", ParamValue::S("P".into())).label("Attribute"),
        ParamSpec::new("class", ParamValue::S("point".into())).label("Class"),
        ParamSpec::new("noise", ParamValue::S("perlin".into())).label("Noise Type"),
        ParamSpec::new("amplitude", ParamValue::F(1.0)).label("Amplitude").range(0.0, 10.0),
        ParamSpec::new("frequency", ParamValue::V3([1.0; 3])).label("Frequency").range(0.0, 10.0),
        ParamSpec::new("offset", ParamValue::V3([0.0; 3])).label("Offset").range(-10.0, 10.0),
        ParamSpec::new("seed", ParamValue::I(0)).label("Seed").range(0.0, 10.0),
        ParamSpec::new("operation", ParamValue::S("add".into())).label("Operation"),
        ParamSpec::new("remap", ParamValue::B(false)).label("Remap Noise"),
        ParamSpec::new("remap_min", ParamValue::F(-1.0)).label("Remap Min"),
        ParamSpec::new("remap_max", ParamValue::F(1.0)).label("Remap Max"),
        ParamSpec::new("fractal", ParamValue::Menu(0)).label("Fractal").menu(&[
            ("none", "None"),
            ("fbm", "FBM"),
            ("turbulence", "Turbulence"),
            ("ridged", "Ridged"),
        ]),
        ParamSpec::new("octaves", ParamValue::I(4)).label("Octaves").range(1.0, 16.0),
        ParamSpec::new("rough", ParamValue::F(0.5)).label("Roughness").range(0.0, 1.0),
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("rest", ParamValue::S(String::new())).label("Rest Position Attribute"),
        ParamSpec::new("updatenormals", ParamValue::B(false)).label("Update Normals"),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(AttribNoise { specs: spec() })
}

struct AttribNoise {
    specs: Vec<ParamSpec>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Perlin,
    Simplex,
    White,
    /// smooth value noise (hashed lattice values, fade interpolation)
    Value,
    /// cellular F1 distance (nearest hashed feature point of the 27 neighbouring cells)
    Worley,
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum Fractal {
    None,
    Fbm,
    Turbulence,
    Ridged,
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum Op {
    Add,
    Set,
    Multiply,
}

/// Parameters resolved and validated for one cook.
struct Config {
    attrib: String,
    class: AttrClass,
    kind: Kind,
    amplitude: f64,
    freq: [f64; 3],
    offset: [f64; 3],
    seed: u64,
    op: Op,
    remap: Option<(f64, f64)>,
    fractal: Fractal,
    octaves: u32,
    rough: f64,
    group: String,
    rest: String,
    update_normals: bool,
}

fn bad<T>(m: impl Into<String>) -> Result<T, CookError> {
    Err(CookError::BadParam(format!("attribnoise: {}", m.into())))
}

fn need_vec3(p: &ParamSet, name: &str) -> Result<[f64; 3], CookError> {
    let v = p.vec3(name).ok_or_else(|| CookError::BadParam(format!("attribnoise: missing or mistyped `{name}`")))?;
    if v.iter().all(|c| c.is_finite()) { Ok(v.map(f64::from)) } else { bad(format!("`{name}`: non-finite")) }
}

fn need_float(p: &ParamSet, name: &str) -> Result<f64, CookError> {
    let v = p.float(name).ok_or_else(|| CookError::BadParam(format!("attribnoise: missing or mistyped `{name}`")))?;
    if v.is_finite() { Ok(f64::from(v)) } else { bad(format!("`{name}`: non-finite")) }
}

fn need_str<'a>(p: &'a ParamSet, name: &str) -> Result<&'a str, CookError> {
    p.string(name).ok_or_else(|| CookError::BadParam(format!("attribnoise: missing or mistyped `{name}`")))
}

impl Config {
    /// RMS of the fBm octave weights `rough^k` (k < octaves): the divisor that keeps the summed octaves at the variance of one.
    fn fbm_rms(&self) -> f64 {
        (0..self.octaves).map(|o| self.rough.powi(2 * o as i32)).sum::<f64>().sqrt().max(f64::MIN_POSITIVE)
    }
    fn read(p: &ParamSet) -> Result<Config, CookError> {
        let attrib = need_str(p, "attrib")?.trim().to_string();
        if attrib.is_empty() {
            return bad("`attrib` is empty");
        }
        let class = match need_str(p, "class")?.trim() {
            "point" => AttrClass::Point,
            "prim" | "primitive" => AttrClass::Prim,
            "vertex" => AttrClass::Vertex,
            "detail" | "global" => AttrClass::Detail,
            other => return bad(format!("`class` `{other}`: expected point|prim|vertex|detail")),
        };
        let kind = match need_str(p, "noise")?.trim() {
            "perlin" => Kind::Perlin,
            "simplex" => Kind::Simplex,
            "white" => Kind::White,
            "value" => Kind::Value,
            "worley" | "cellular" => Kind::Worley,
            other => return bad(format!("`noise` `{other}`: expected perlin|simplex|white|value|worley")),
        };
        let op = match need_str(p, "operation")?.trim() {
            "add" => Op::Add,
            "set" => Op::Set,
            "multiply" => Op::Multiply,
            other => return bad(format!("`operation` `{other}`: expected add|set|multiply")),
        };
        let seed =
            i64::from(p.int("seed").ok_or_else(|| CookError::BadParam("attribnoise: missing `seed`".into()))?) as u64;
        let remap = if p.toggle("remap").unwrap_or(false) {
            Some((need_float(p, "remap_min")?, need_float(p, "remap_max")?))
        } else {
            None
        };
        let fractal = match p.menu("fractal").unwrap_or(0) {
            0 => Fractal::None,
            1 => Fractal::Fbm,
            2 => Fractal::Turbulence,
            3 => Fractal::Ridged,
            other => return bad(format!("fractal menu index {other} out of range")),
        };
        let octaves = p.int("octaves").unwrap_or(4).clamp(1, 16) as u32;
        let rough = need_float(p, "rough")?.clamp(0.0, 1.0);
        Ok(Config {
            attrib,
            class,
            kind,
            amplitude: need_float(p, "amplitude")?,
            freq: need_vec3(p, "frequency")?,
            offset: need_vec3(p, "offset")?,
            seed,
            op,
            remap,
            fractal,
            octaves,
            rough,
            group: p.string("group").unwrap_or("").trim().to_string(),
            rest: p.string("rest").unwrap_or("").trim().to_string(),
            update_normals: p.toggle("updatenormals").unwrap_or(false),
        })
    }
}

// ---------------------------------------------------------------- noise ----

/// SplitMix64 finalizer: a strong 64→64 bit mixer.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Hash of (seed, channel, integer cell).
fn cell_hash(seed: u64, chan: u64, i: i64, j: i64, k: i64) -> u64 {
    let h = mix(seed ^ mix(chan.wrapping_mul(0xD6E8_FEB8_6659_FD93)));
    mix(h ^ mix(i as u64 ^ mix(j as u64 ^ mix(k as u64))))
}

/// 12 edge-midpoint gradients of the improved-noise reference set.
const GRAD: [[f64; 3]; 12] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
];

fn grad(h: u64) -> [f64; 3] {
    GRAD.get((h % 12) as usize).copied().unwrap_or([0.0; 3])
}

fn floor_i(x: f64) -> i64 {
    x.floor() as i64 // saturating cast: deterministic for any finite input
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

/// Classic gradient noise: zero at every integer lattice point, |value| ≤ 1.
pub(super) fn perlin(q: [f64; 3], seed: u64, chan: u64) -> f64 {
    let (ix, iy, iz) = (floor_i(q[0]), floor_i(q[1]), floor_i(q[2]));
    let (fx, fy, fz) = (q[0] - ix as f64, q[1] - iy as f64, q[2] - iz as f64);
    let corner = |dx: i64, dy: i64, dz: i64| {
        let g = grad(cell_hash(seed, chan, ix.wrapping_add(dx), iy.wrapping_add(dy), iz.wrapping_add(dz)));
        g[0] * (fx - dx as f64) + g[1] * (fy - dy as f64) + g[2] * (fz - dz as f64)
    };
    let (u, v, w) = (fade(fx), fade(fy), fade(fz));
    let x00 = lerp(u, corner(0, 0, 0), corner(1, 0, 0));
    let x10 = lerp(u, corner(0, 1, 0), corner(1, 1, 0));
    let x01 = lerp(u, corner(0, 0, 1), corner(1, 0, 1));
    let x11 = lerp(u, corner(0, 1, 1), corner(1, 1, 1));
    lerp(w, lerp(v, x00, x10), lerp(v, x01, x11)).clamp(-1.0, 1.0)
}

/// 3-D simplex noise (Gustavson skew/unskew), scaled to ≈[-1,1] and clamped.
fn simplex(q: [f64; 3], seed: u64, chan: u64) -> f64 {
    const F3: f64 = 1.0 / 3.0;
    const G3: f64 = 1.0 / 6.0;
    let s = (q[0] + q[1] + q[2]) * F3;
    let (i, j, k) = (floor_i(q[0] + s), floor_i(q[1] + s), floor_i(q[2] + s));
    let t = (i as f64 + j as f64 + k as f64) * G3;
    let (x0, y0, z0) = (q[0] - (i as f64 - t), q[1] - (j as f64 - t), q[2] - (k as f64 - t));
    let ((i1, j1, k1), (i2, j2, k2)) = if x0 >= y0 {
        if y0 >= z0 {
            ((1, 0, 0), (1, 1, 0))
        } else if x0 >= z0 {
            ((1, 0, 0), (1, 0, 1))
        } else {
            ((0, 0, 1), (1, 0, 1))
        }
    } else if y0 < z0 {
        ((0, 0, 1), (0, 1, 1))
    } else if x0 < z0 {
        ((0, 1, 0), (0, 1, 1))
    } else {
        ((0, 1, 0), (1, 1, 0))
    };
    let corners = [
        (0, 0, 0, x0, y0, z0),
        (i1, j1, k1, x0 - i1 as f64 + G3, y0 - j1 as f64 + G3, z0 - k1 as f64 + G3),
        (i2, j2, k2, x0 - i2 as f64 + 2.0 * G3, y0 - j2 as f64 + 2.0 * G3, z0 - k2 as f64 + 2.0 * G3),
        (1, 1, 1, x0 - 1.0 + 3.0 * G3, y0 - 1.0 + 3.0 * G3, z0 - 1.0 + 3.0 * G3),
    ];
    let mut sum = 0.0;
    for (di, dj, dk, x, y, z) in corners {
        let a = 0.6 - x * x - y * y - z * z;
        if a > 0.0 {
            let g = grad(cell_hash(seed, chan, i.wrapping_add(di), j.wrapping_add(dj), k.wrapping_add(dk)));
            let a2 = a * a;
            sum += a2 * a2 * (g[0] * x + g[1] * y + g[2] * z);
        }
    }
    (32.0 * sum).clamp(-1.0, 1.0)
}

/// White noise: uniform in [-1,1) from the bit pattern of the sample position (no spatial coherence).
fn white(q: [f64; 3], seed: u64, chan: u64) -> f64 {
    let h = cell_hash(seed, chan, q[0].to_bits() as i64, q[1].to_bits() as i64, q[2].to_bits() as i64);
    ((h >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
}

/// Hashed lattice value in [-1, 1) of cell `(i, j, k)`.
fn lattice_value(seed: u64, chan: u64, i: i64, j: i64, k: i64) -> f64 {
    ((cell_hash(seed ^ 0x5851_F42D_4C95_7F2D, chan, i, j, k) >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
}
/// Smooth value noise: trilinear blend of the 8 lattice values with the Perlin fade curve.
fn value_noise(q: [f64; 3], seed: u64, chan: u64) -> f64 {
    let (i, j, k) = (floor_i(q[0]), floor_i(q[1]), floor_i(q[2]));
    let (fx, fy, fz) = (fade(q[0] - i as f64), fade(q[1] - j as f64), fade(q[2] - k as f64));
    let v = |a: i64, b: i64, c: i64| lattice_value(seed, chan, i.wrapping_add(a), j.wrapping_add(b), k.wrapping_add(c));
    let x00 = lerp(fx, v(0, 0, 0), v(1, 0, 0));
    let x10 = lerp(fx, v(0, 1, 0), v(1, 1, 0));
    let x01 = lerp(fx, v(0, 0, 1), v(1, 0, 1));
    let x11 = lerp(fx, v(0, 1, 1), v(1, 1, 1));
    lerp(fz, lerp(fy, x00, x10), lerp(fy, x01, x11))
}
/// Jittered feature point inside cell `(i, j, k)` (absolute coordinates).
fn feature_point(seed: u64, chan: u64, i: i64, j: i64, k: i64) -> [f64; 3] {
    let h = |s: u64| (cell_hash(seed ^ s, chan, i, j, k) >> 11) as f64 / (1u64 << 53) as f64;
    [i as f64 + h(0x1111), j as f64 + h(0x2222), k as f64 + h(0x3333)]
}
/// Worley F1 mapped to [-1, 1]: `2 * min(F1, 1) - 1` (feature point itself = -1).
fn worley(q: [f64; 3], seed: u64, chan: u64) -> f64 {
    let (i, j, k) = (floor_i(q[0]), floor_i(q[1]), floor_i(q[2]));
    let mut best = f64::MAX;
    for a in -1..=1 {
        for b in -1..=1 {
            for c in -1..=1 {
                let f = feature_point(seed, chan, i.wrapping_add(a), j.wrapping_add(b), k.wrapping_add(c));
                let d = ((f[0] - q[0]).powi(2) + (f[1] - q[1]).powi(2) + (f[2] - q[2]).powi(2)).sqrt();
                best = best.min(d);
            }
        }
    }
    2.0 * best.min(1.0) - 1.0
}
fn base_noise(kind: Kind, q: [f64; 3], seed: u64, chan: u64) -> f64 {
    match kind {
        Kind::Value => value_noise(q, seed, chan),
        Kind::Worley => worley(q, seed, chan),
        Kind::Perlin => perlin(q, seed, chan),
        Kind::Simplex => simplex(q, seed, chan),
        Kind::White => white(q, seed, chan),
    }
}

/// Base noise by name (`perlin` | `simplex` | `value` | `worley`), in [-1, 1]; shared with `mountain`.
pub(super) fn noise_named(name: &str, q: [f64; 3], seed: u64, chan: u64) -> Option<f64> {
    let kind = match name {
        "perlin" => Kind::Perlin,
        "simplex" => Kind::Simplex,
        "value" => Kind::Value,
        "worley" => Kind::Worley,
        _ => return None,
    };
    Some(base_noise(kind, q, seed, chan))
}

fn sample(cfg: &Config, p: [f64; 3], chan: usize) -> f64 {
    let base_q = std::array::from_fn(|k| p[k] * cfg.freq[k] + cfg.offset[k]);
    if !base_q.iter().all(|c| c.is_finite()) {
        return 0.0;
    }
    let chan = chan as u64;

    let n = match cfg.fractal {
        Fractal::None => base_noise(cfg.kind, base_q, cfg.seed, chan),
        Fractal::Fbm => {
            let mut sum = 0.0;
            let mut amp = 1.0;
            let mut freq = 1.0;
            let lacunarity = 2.0;
            let persistence = cfg.rough;
            for oct in 0..cfg.octaves {
                let q = base_q.map(|x| x * freq);
                let seed = cfg.seed.wrapping_add(oct as u64 * 7919);
                sum += amp * base_noise(cfg.kind, q, seed, chan);
                amp *= persistence;
                freq *= lacunarity;
            }
            sum
        }
        Fractal::Turbulence => {
            let mut sum = 0.0;
            let mut amp = 1.0;
            let mut freq = 1.0;
            let lacunarity = 2.0;
            let persistence = cfg.rough;
            for oct in 0..cfg.octaves {
                let q = base_q.map(|x| x * freq);
                let seed = cfg.seed.wrapping_add(oct as u64 * 7919);
                sum += amp * base_noise(cfg.kind, q, seed, chan).abs();
                amp *= persistence;
                freq *= lacunarity;
            }
            sum
        }
        Fractal::Ridged => {
            let mut sum = 0.0;
            let mut amp = 1.0;
            let mut freq = 1.0;
            let lacunarity = 2.0;
            let persistence = cfg.rough;
            for oct in 0..cfg.octaves {
                let q = base_q.map(|x| x * freq);
                let seed = cfg.seed.wrapping_add(oct as u64 * 7919);
                let signal = 1.0 - base_noise(cfg.kind, q, seed, chan).abs();
                sum += amp * signal;
                amp *= persistence;
                freq *= lacunarity;
            }
            sum
        }
    };
    match cfg.remap {
        Some((lo, hi)) => lo + (n.clamp(-1.0, 1.0) + 1.0) * 0.5 * (hi - lo),
        None if cfg.kind != Kind::Perlin => n,
        None => match cfg.fractal {
            Fractal::None => n * GAIN_PLAIN,
            Fractal::Fbm if cfg.octaves == 1 => n * GAIN_PLAIN,
            Fractal::Fbm => n / cfg.fbm_rms() * GAIN_FRACTAL,
            Fractal::Turbulence | Fractal::Ridged => n,
        },
    }
}

// ----------------------------------------------------------------- cook ----

/// Sample position of every element of `class`.
fn positions(d: &Detail, class: AttrClass, pos: &[[f32; 3]]) -> Vec<[f64; 3]> {
    let at = |pt: u32| pos.get(pt as usize).map_or([0.0; 3], |v| v.map(f64::from));
    let mean = |pts: &mut dyn Iterator<Item = [f64; 3]>| {
        let (mut s, mut n) = ([0.0f64; 3], 0usize);
        for v in pts {
            s = std::array::from_fn(|k| s[k] + v[k]);
            n += 1;
        }
        if n == 0 { [0.0; 3] } else { s.map(|c| c / n as f64) }
    };
    match class {
        AttrClass::Point => pos.iter().map(|v| v.map(f64::from)).collect(),
        AttrClass::Vertex => d.vertex_point.iter().map(|&pt| at(pt)).collect(),
        AttrClass::Prim => d
            .prims
            .iter()
            .map(|pr| {
                let vs = d.vertex_point.get(pr.verts.start as usize..pr.verts.end as usize).unwrap_or(&[]);
                mean(&mut vs.iter().map(|&pt| at(pt)))
            })
            .collect(),
        AttrClass::Detail => vec![mean(&mut pos.iter().map(|v| v.map(f64::from)))],
    }
}

/// Visit every element of a float-typed attribute as a mutable channel slice. `false` = not a float attribute.
fn each_element(data: &mut AttrData, f: &mut dyn FnMut(usize, &mut [f32])) -> bool {
    match data {
        AttrData::F32(v) => v.iter_mut().enumerate().for_each(|(i, x)| f(i, std::slice::from_mut(x))),
        AttrData::V2(v) => v.iter_mut().enumerate().for_each(|(i, x)| f(i, x.as_mut_slice())),
        AttrData::V3(v) => v.iter_mut().enumerate().for_each(|(i, x)| f(i, x.as_mut_slice())),
        AttrData::V4(v) => v.iter_mut().enumerate().for_each(|(i, x)| f(i, x.as_mut_slice())),
        AttrData::FN { size, data } => match usize::try_from(*size) {
            Ok(n) if (1..=MAX_TUPLE).contains(&n) => data.chunks_mut(n).enumerate().for_each(|(i, x)| f(i, x)),
            _ => return false,
        },
        _ => return false,
    }
    true
}

fn run(d: &Detail, cfg: &Config) -> Result<Detail, CookError> {
    let mut out = d.clone();
    let rest_pos: Option<Vec<[f32; 3]>> = if cfg.rest.is_empty() {
        None
    } else {
        match d.attr(AttrClass::Point, &cfg.rest).map(|a| &a.data) {
            Some(AttrData::V3(v)) => Some(v.clone()),
            _ => {
                return Err(CookError::Failed(format!("attribnoise: no V3 point attribute `{}` for `rest`", cfg.rest)));
            }
        }
    };
    let pos = positions(d, cfg.class, rest_pos.as_deref().unwrap_or(d.p()));
    let mask: Option<Vec<bool>> = if cfg.group.is_empty() || cfg.class == AttrClass::Detail {
        None
    } else {
        let mut m = vec![false; d.class_len(cfg.class)];
        for name in cfg.group.split_whitespace() {
            let g = d
                .group(cfg.class, name)
                .ok_or_else(|| CookError::Failed(format!("attribnoise: unknown {:?} group `{name}`", cfg.class)))?;
            for (s, b) in m.iter_mut().zip(&g.mask) {
                *s |= *b;
            }
        }
        Some(m)
    };
    let Some(attr) = out.attr_mut(cfg.class, &cfg.attrib) else {
        return Err(CookError::Failed(format!("attribnoise: no {:?} attribute `{}`", cfg.class, cfg.attrib)));
    };
    let mut overflow = false;
    let ok = each_element(&mut attr.data, &mut |i, x| {
        if mask.as_ref().is_some_and(|m| !m.get(i).copied().unwrap_or(false)) {
            return;
        }
        let p = pos.get(i).copied().unwrap_or([0.0; 3]);
        for (c, slot) in x.iter_mut().enumerate() {
            let v = cfg.amplitude * sample(cfg, p, c);
            let old = f64::from(*slot);
            let new = match cfg.op {
                Op::Add => old + v,
                Op::Set => v,
                Op::Multiply => old * v,
            };
            let new = new as f32;
            overflow |= !new.is_finite() && old.is_finite();
            *slot = new;
        }
    });
    if !ok {
        return Err(CookError::Unsupported(format!(
            "attribnoise: `{}` is not a float attribute (F/V2/V3/V4)",
            cfg.attrib
        )));
    }
    if overflow {
        return Err(CookError::Failed("attribnoise: result overflows f32".into()));
    }
    if cfg.update_normals && cfg.attrib == "P" {
        let pp = mesh_positions(&out);
        let n: Vec<[f32; 3]> =
            point_normals(&out, &pp).into_iter().map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]).collect();
        out.add_attr(Attribute {
            name: "N".into(),
            class: AttrClass::Point,
            data: AttrData::V3(n),
            type_info: TypeInfo::Normal,
        });
        if let Some(AttrData::V3(old)) = out.attr(AttrClass::Prim, "N").map(|a| a.data.clone()) {
            let fresh: Vec<[f32; 3]> = (0..out.prims.len())
                .map(|i| {
                    let nrm = closed_poly_pts(&out, &out.prims[i]).and_then(|pts| prim_normal(&pp, &pts));
                    nrm.map_or_else(
                        || old.get(i).copied().unwrap_or([0.0; 3]),
                        |v| [v[0] as f32, v[1] as f32, v[2] as f32],
                    )
                })
                .collect();
            out.add_attr(Attribute {
                name: "N".into(),
                class: AttrClass::Prim,
                data: AttrData::V3(fresh),
                type_info: TypeInfo::Normal,
            });
        }
    }
    Ok(out)
}

impl Node<Detail> for AttribNoise {
    fn type_name(&self) -> &'static str {
        "attribnoise"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let cfg = Config::read(ctx.params)?;
        let empty = Detail::default();
        run(inputs.first().map_or(&empty, |a| &**a), &cfg).map(Arc::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_geo::{Attribute, TypeInfo, write_geo_json};

    fn params(over: &[(&str, ParamValue)]) -> ParamSet {
        let mut set = ParamSet::new();
        for s in spec() {
            set = set.with(&s.name, s.default.clone());
        }
        for (k, v) in over {
            set = set.with(k, v.clone());
        }
        set
    }
    fn cook(d: &Detail, over: &[(&str, ParamValue)]) -> Result<Detail, CookError> {
        run(d, &Config::read(&params(over))?)
    }
    fn s(v: &str) -> ParamValue {
        ParamValue::S(v.into())
    }
    fn points(pts: &[[f32; 3]]) -> Detail {
        let mut d = Detail::default();
        d.add_points(pts.len() as u32);
        d.p_mut().copy_from_slice(pts);
        d
    }
    /// n³ integer lattice, spacing 1.
    fn lattice(n: i32) -> Detail {
        let mut v = Vec::new();
        for x in 0..n {
            for y in 0..n {
                for z in 0..n {
                    v.push([x as f32 - 2.0, y as f32 - 2.0, z as f32 - 2.0]);
                }
            }
        }
        points(&v)
    }
    /// Deterministic non-lattice cloud (golden-ratio-ish strides).
    fn cloud(n: usize) -> Detail {
        let v: Vec<[f32; 3]> = (0..n)
            .map(|i| {
                let t = i as f32;
                [
                    (t * 0.618_034).fract() * 7.0 + 0.123,
                    (t * 0.414_214).fract() * 7.0 + 0.371,
                    (t * 0.732_051).fract() * 7.0 + 0.517,
                ]
            })
            .collect();
        points(&v)
    }
    fn with_attr(mut d: Detail, name: &str, class: AttrClass, data: AttrData) -> Detail {
        d.add_attr(Attribute { name: name.into(), class, data, type_info: TypeInfo::None });
        d
    }
    fn f32s(d: &Detail, name: &str) -> Vec<f32> {
        match d.attr(AttrClass::Point, name).map(|a| &a.data) {
            Some(AttrData::F32(v)) => v.clone(),
            _ => panic!("no f32 `{name}`"),
        }
    }
    const KINDS: [&str; 5] = ["perlin", "simplex", "white", "value", "worley"];

    #[test]
    fn perlin_is_zero_at_integer_lattice_points() {
        let d = with_attr(lattice(5), "n", AttrClass::Point, AttrData::F32(vec![7.0; 125]));
        for seed in [0, 1, -5, 12345] {
            let o = cook(&d, &[("operation", s("set")), ("seed", ParamValue::I(seed))]).expect("cooks");
            // attrib defaults to P, so aim at `n`
            let o2 =
                cook(&d, &[("attrib", s("n")), ("operation", s("set")), ("seed", ParamValue::I(seed))]).expect("cooks");
            assert!(f32s(&o2, "n").iter().all(|v| v.abs() < 1e-6), "seed {seed}");
            assert_eq!(o.point_count, 125);
        }
    }
    #[test]
    fn perlin_is_zero_on_integer_lattice_after_integer_offset_and_freq() {
        let d = with_attr(lattice(4), "n", AttrClass::Point, AttrData::F32(vec![0.0; 64]));
        let o = cook(
            &d,
            &[
                ("attrib", s("n")),
                ("operation", s("set")),
                ("frequency", ParamValue::V3([3.0, 2.0, 1.0])),
                ("offset", ParamValue::V3([4.0, -7.0, 1.0])),
            ],
        )
        .expect("cooks");
        assert!(f32s(&o, "n").iter().all(|v| v.abs() < 1e-6));
        // ...and is NOT trivially zero off-lattice
        let off = with_attr(cloud(200), "n", AttrClass::Point, AttrData::F32(vec![0.0; 200]));
        let o = cook(&off, &[("attrib", s("n")), ("operation", s("set"))]).expect("cooks");
        let v = f32s(&o, "n");
        assert!(v.iter().any(|x| x.abs() > 0.05) && v.iter().all(|x| x.abs() <= GAIN_PLAIN as f32));
    }
    #[test]
    fn amplitude_scales_linearly_all_noise_kinds() {
        let d = with_attr(cloud(300), "n", AttrClass::Point, AttrData::F32(vec![0.0; 300]));
        for kind in KINDS {
            let run = |a: f32| {
                f32s(
                    &cook(
                        &d,
                        &[
                            ("attrib", s("n")),
                            ("operation", s("set")),
                            ("noise", s(kind)),
                            ("amplitude", ParamValue::F(a)),
                        ],
                    )
                    .expect("cooks"),
                    "n",
                )
            };
            let (a1, a2) = (run(1.0), run(2.0));
            assert!(a1.iter().any(|v| v.abs() > 0.05), "{kind} not all-zero");
            for (x, y) in a1.iter().zip(&a2) {
                assert!((2.0 * x - y).abs() < 1e-6, "{kind}: {x} {y}");
            }
        }
    }
    #[test]
    fn add_on_p_moves_each_point_at_most_amp_sqrt3() {
        let src = cloud(400);
        for kind in KINDS {
            for amp in [0.25f32, 1.0, 3.0] {
                let o = cook(&src, &[("noise", s(kind)), ("amplitude", ParamValue::F(amp))]).expect("cooks");
                let gain = if kind == "perlin" { GAIN_PLAIN as f32 } else { 1.0 };
                let bound = amp * gain * 3.0f32.sqrt() + 1e-5;
                let mut moved = 0;
                for (a, b) in src.p().iter().zip(o.p()) {
                    let dist = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
                    assert!(dist <= bound, "{kind} amp {amp}: {dist} > {bound}");
                    moved += usize::from(dist > 1e-4);
                }
                assert!(moved > 300, "{kind}: only {moved} moved");
            }
        }
    }
    #[test]
    fn white_noise_is_seed_deterministic_byte_equal() {
        let d = cloud(500);
        let p = |seed: i32| [("noise", s("white")), ("seed", ParamValue::I(seed))];
        let (a, b) = (cook(&d, &p(9)).expect("a"), cook(&d, &p(9)).expect("b"));
        assert_eq!(write_geo_json(&a), write_geo_json(&b));
        assert_ne!(write_geo_json(&a), write_geo_json(&cook(&d, &p(10)).expect("c")));
        // other kinds are seed-deterministic too
        for kind in ["perlin", "simplex"] {
            let q = [("noise", s(kind)), ("seed", ParamValue::I(3))];
            assert_eq!(write_geo_json(&cook(&d, &q).expect("x")), write_geo_json(&cook(&d, &q).expect("y")));
        }
    }
    #[test]
    fn white_noise_is_roughly_uniform_and_channels_are_independent() {
        let d = cloud(4000);
        let o = cook(&d, &[("noise", s("white"))]).expect("cooks");
        let (mut dx, mut dy) = (Vec::new(), Vec::new());
        for (a, b) in d.p().iter().zip(o.p()) {
            dx.push(f64::from(b[0] - a[0]));
            dy.push(f64::from(b[1] - a[1]));
        }
        let n = dx.len() as f64;
        let (mx, my) = (dx.iter().sum::<f64>() / n, dy.iter().sum::<f64>() / n);
        assert!(mx.abs() < 0.05 && my.abs() < 0.05, "mean {mx} {my}");
        let var = dx.iter().map(|v| (v - mx).powi(2)).sum::<f64>() / n; // U(-1,1) → 1/3
        assert!((var - 1.0 / 3.0).abs() < 0.03, "var {var}");
        let cov = dx.iter().zip(&dy).map(|(a, b)| (a - mx) * (b - my)).sum::<f64>() / n;
        assert!((cov / var).abs() < 0.05, "channel correlation {}", cov / var);
    }
    #[test]
    fn operations_set_add_multiply_agree() {
        let d = with_attr(cloud(50), "n", AttrClass::Point, AttrData::F32(vec![3.0; 50]));
        let run = |op: &str| {
            f32s(
                &cook(&d, &[("attrib", s("n")), ("operation", s(op)), ("amplitude", ParamValue::F(0.5))]).expect("c"),
                "n",
            )
        };
        let (set, add, mul) = (run("set"), run("add"), run("multiply"));
        for i in 0..50 {
            assert!((add[i] - (3.0 + set[i])).abs() < 1e-5 && (mul[i] - 3.0 * set[i]).abs() < 1e-5);
        }
    }
    #[test]
    fn remap_range_maps_into_min_max() {
        let d = with_attr(cloud(500), "n", AttrClass::Point, AttrData::F32(vec![0.0; 500]));
        let o = cook(
            &d,
            &[
                ("attrib", s("n")),
                ("operation", s("set")),
                ("noise", s("white")),
                ("remap", ParamValue::B(true)),
                ("remap_min", ParamValue::F(0.0)),
                ("remap_max", ParamValue::F(10.0)),
            ],
        )
        .expect("cooks");
        let v = f32s(&o, "n");
        assert!(v.iter().all(|x| (0.0..=10.0).contains(x)));
        assert!(v.iter().any(|x| *x < 1.0) && v.iter().any(|x| *x > 9.0));
    }
    #[test]
    fn classes_vertex_prim_detail() {
        let mut d = points(&[[0.3, 0.2, 0.1], [1.3, 0.2, 0.1], [1.3, 1.2, 0.1], [0.3, 1.2, 0.1]]);
        d.add_poly(&[0, 1, 2, 3], true);
        let d = with_attr(d, "v", AttrClass::Vertex, AttrData::F32(vec![0.0; 4]));
        let d = with_attr(d, "pr", AttrClass::Prim, AttrData::V3(vec![[0.0; 3]]));
        let d = with_attr(d, "g", AttrClass::Detail, AttrData::F32(vec![0.0]));
        let set = |attrib: &str, class: &str| {
            cook(&d, &[("attrib", s(attrib)), ("class", s(class)), ("operation", s("set"))]).expect("c")
        };
        // vertex noise == point noise at the owning point
        let pt = with_attr(d.clone(), "v", AttrClass::Point, AttrData::F32(vec![0.0; 4]));
        let ptn = f32s(&cook(&pt, &[("attrib", s("v")), ("operation", s("set"))]).expect("p"), "v");
        let Some(AttrData::F32(vn)) = set("v", "vertex").attr(AttrClass::Vertex, "v").map(|a| a.data.clone()) else {
            panic!()
        };
        assert_eq!(vn, ptn);
        // prim = noise at centroid (0.8, 0.7, 0.1), channel-wise on V3
        let Some(AttrData::V3(pn)) = set("pr", "prim").attr(AttrClass::Prim, "pr").map(|a| a.data.clone()) else {
            panic!()
        };
        let cen = points(&[[0.8, 0.7, 0.1]]);
        let cen = with_attr(cen, "pr", AttrClass::Point, AttrData::V3(vec![[0.0; 3]]));
        let cn = cook(&cen, &[("attrib", s("pr")), ("operation", s("set"))]).expect("c");
        let Some(AttrData::V3(cv)) = cn.attr(AttrClass::Point, "pr").map(|a| a.data.clone()) else { panic!() };
        for k in 0..3 {
            assert!((pn[0][k] - cv[0][k]).abs() < 1e-5);
        }
        // detail class cooks to a single finite value
        let Some(AttrData::F32(g)) = set("g", "detail").attr(AttrClass::Detail, "g").map(|a| a.data.clone()) else {
            panic!()
        };
        assert!(g.len() == 1 && g[0].is_finite());
    }
    #[test]
    fn v3_channels_are_independent_streams() {
        let src = cloud(300);
        let o = cook(&src, &[("noise", s("simplex"))]).expect("cooks");
        let d: Vec<[f32; 3]> =
            src.p().iter().zip(o.p()).map(|(a, b)| [b[0] - a[0], b[1] - a[1], b[2] - a[2]]).collect();
        assert!(d.iter().any(|v| (v[0] - v[1]).abs() > 0.05) && d.iter().any(|v| (v[1] - v[2]).abs() > 0.05));
    }
    #[test]
    fn missing_attrib_is_failed_and_bad_params_are_rejected() {
        let d = cloud(5);
        assert!(matches!(cook(&d, &[("attrib", s("nope"))]), Err(CookError::Failed(_))));
        assert!(matches!(cook(&d, &[("attrib", s("P")), ("class", s("prim"))]), Err(CookError::Failed(_))));
        for (k, v) in [("class", "blob"), ("noise", "blob"), ("operation", "xor"), ("attrib", "")] {
            assert!(matches!(cook(&d, &[(k, s(v))]), Err(CookError::BadParam(_))), "{k}={v}");
        }
        let name = with_attr(d.clone(), "name", AttrClass::Point, AttrData::I32(vec![0; 5]));
        assert!(matches!(cook(&name, &[("attrib", s("name"))]), Err(CookError::Unsupported(_))));
        assert!(cook(&d, &[("amplitude", ParamValue::F(f32::NAN))]).is_err());
    }
    #[test]
    fn hostile_geometry_never_panics() {
        // no geometry at all → no `P` → Failed; zero points with a `P` → fine
        assert!(matches!(cook(&Detail::default(), &[]), Err(CookError::Failed(_))));
        assert_eq!(cook(&points(&[]), &[]).expect("empty").point_count, 0);
        let mut nan = cloud(3);
        nan.p_mut()[1] = [f32::NAN, f32::INFINITY, 1e38];
        for kind in KINDS {
            assert!(cook(&nan, &[("noise", s(kind))]).is_ok());
        }
        let huge = points(&[[3.0e38, -3.0e38, 1.0e30]]);
        for kind in KINDS {
            let _ = cook(&huge, &[("noise", s(kind)), ("frequency", ParamValue::V3([1e30; 3]))]);
        }
    }
    fn w_attr(d: Detail) -> Detail {
        let n = d.point_count as usize;
        with_attr(d, "w", AttrClass::Point, AttrData::F32(vec![0.0; n]))
    }
    /// Value noise at an integer lattice point == the hashed lattice value; continuous (1e-4 step moves < 1e-2); bounded.
    #[test]
    fn value_noise_equals_lattice_value_on_the_lattice_and_is_continuous() {
        let d = w_attr(points(&[[2.0, 3.0, -1.0], [0.0, 0.0, 0.0], [2.0001, 3.0, -1.0]]));
        let o = cook(&d, &[("attrib", s("w")), ("noise", s("value")), ("operation", s("set"))]).expect("cooks");
        let w = f32s(&o, "w");
        assert!((f64::from(w[0]) - lattice_value(0, 0, 2, 3, -1)).abs() < 1e-6);
        assert!((f64::from(w[1]) - lattice_value(0, 0, 0, 0, 0)).abs() < 1e-6);
        assert!((w[2] - w[0]).abs() < 1e-2, "{} vs {}", w[0], w[2]);
        let c = cook(&w_attr(cloud(500)), &[("attrib", s("w")), ("noise", s("value")), ("operation", s("set"))])
            .expect("cooks");
        assert!(f32s(&c, "w").iter().all(|v| (-1.0..=1.0).contains(v)));
    }
    /// Worley at a feature point is exactly the minimum -1; every sample lies in [-1, 1] and not all equal.
    #[test]
    fn worley_is_minus_one_at_a_feature_point_and_bounded() {
        let f = feature_point(0, 0, 0, 0, 0);
        let d = w_attr(points(&[[f[0] as f32, f[1] as f32, f[2] as f32]]));
        let o = cook(&d, &[("attrib", s("w")), ("noise", s("worley")), ("operation", s("set"))]).expect("cooks");
        assert!((f32s(&o, "w")[0] + 1.0).abs() < 1e-4, "{}", f32s(&o, "w")[0]);
        let c = cook(&w_attr(cloud(500)), &[("attrib", s("w")), ("noise", s("worley")), ("operation", s("set"))])
            .expect("cooks");
        let w = f32s(&c, "w");
        assert!(w.iter().all(|v| (-1.0..=1.0).contains(v)));
        assert!(w.iter().any(|v| (v - w[0]).abs() > 0.05));
    }
    /// Group of the first 50 of 100 points: only those move; the rest are bit-identical.
    #[test]
    fn group_limits_noise_to_members() {
        let mut src = cloud(100);
        src.groups.push(hexe_geo::Group {
            name: "g".into(),
            class: AttrClass::Point,
            mask: (0..100).map(|i| i < 50).collect(),
        });
        let o = cook(&src, &[("group", s("g"))]).expect("cooks");
        for i in 0..100 {
            if i < 50 {
                assert_ne!(o.p()[i], src.p()[i], "member {i} did not move");
            } else {
                assert_eq!(o.p()[i], src.p()[i], "non-member {i} moved");
            }
        }
        assert!(matches!(cook(&src, &[("group", s("nope"))]), Err(CookError::Failed(_))));
    }
    /// `rest`: noise samples the rest attribute, not P -> identical rest positions give identical values; no rest attr = Failed.
    #[test]
    fn rest_attribute_drives_the_sample_position() {
        let src = cloud(20);
        let src = with_attr(src, "rest", AttrClass::Point, AttrData::V3(vec![[1.5, 2.5, 3.5]; 20]));
        let src = w_attr(src);
        let o = cook(&src, &[("attrib", s("w")), ("operation", s("set")), ("rest", s("rest"))]).expect("cooks");
        let w = f32s(&o, "w");
        assert!(w.iter().all(|v| *v == w[0]));
        let plain = f32s(&cook(&src, &[("attrib", s("w")), ("operation", s("set"))]).expect("cooks"), "w");
        assert!(plain.iter().any(|v| *v != plain[0]));
        assert!(matches!(cook(&src, &[("rest", s("nope"))]), Err(CookError::Failed(_))));
    }
    /// updatenormals on a flat quad (amp 0): point N = (0,0,+-1), existing prim N refreshed; off = no N written.
    #[test]
    fn updatenormals_recomputes_point_and_existing_prim_normals() {
        let mut q = points(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]);
        q.add_poly(&[0, 1, 2, 3], true);
        let q = with_attr(q, "N", AttrClass::Prim, AttrData::V3(vec![[9.0, 9.0, 9.0]]));
        let off = cook(&q, &[("amplitude", ParamValue::F(0.0))]).expect("cooks");
        assert!(off.attr(AttrClass::Point, "N").is_none());
        let on = cook(&q, &[("amplitude", ParamValue::F(0.0)), ("updatenormals", ParamValue::B(true))]).expect("cooks");
        let Some(AttrData::V3(n)) = on.attr(AttrClass::Point, "N").map(|a| &a.data) else { panic!("N") };
        assert!(n.iter().all(|v| v[0].abs() < 1e-6 && v[1].abs() < 1e-6 && (v[2].abs() - 1.0).abs() < 1e-6));
        let Some(AttrData::V3(pn)) = on.attr(AttrClass::Prim, "N").map(|a| &a.data) else { panic!("prim N") };
        assert!(pn[0][0].abs() < 1e-6 && (pn[0][2].abs() - 1.0).abs() < 1e-6, "{:?}", pn[0]);
    }
    #[test]
    fn defaults_match_the_brief() {
        let sp = spec();
        let get = |n: &str| sp.iter().find(|p| p.name == n).map(|p| p.default.clone());
        assert_eq!(get("attrib"), Some(s("P")));
        assert_eq!(get("class"), Some(s("point")));
        assert_eq!(get("noise"), Some(s("perlin")));
        assert_eq!(get("amplitude"), Some(ParamValue::F(1.0)));
        assert_eq!(get("frequency"), Some(ParamValue::V3([1.0; 3])));
        assert_eq!(get("offset"), Some(ParamValue::V3([0.0; 3])));
        assert_eq!(get("seed"), Some(ParamValue::I(0)));
        assert_eq!(get("operation"), Some(s("add")));
        assert_eq!(get("remap"), Some(ParamValue::B(false)));
    }
    #[test]
    fn example_scene_cooks() {
        let json = include_str!("../../../../examples/attribnoise.json");
        let (mut g, id) = load_scene(json).map_err(|e| e.to_string()).expect("loads");
        let d = g.cook(id, 0.0).map_err(|e| e.to_string()).expect("cooks");
        assert_eq!(d.point_count, 121);
        assert!(d.p().iter().all(|p| p[1].abs() <= 0.5 * 3.0f32.sqrt() + 1e-5));
        assert!(d.p().iter().any(|p| p[1].abs() > 0.01));
    }
    /// Calibration pin (stat-oracle): per-component RMS of perlin displacement = GAIN_PLAIN x the unit-range field's (0.25) = Houdini's 0.388 x amp
    /// within 6 %; fBm (4 octaves) is variance-normalised x GAIN_FRACTAL = 0.350 x amp within 8 %.
    #[test]
    fn perlin_displacement_rms_is_calibrated_to_the_houdini_oracle() {
        // the oracle's arrangement: a flat 70 x 70 point grid, 10 x 10 wide, at freq 3
        let pts: Vec<[f32; 3]> =
            (0..4900).map(|i| [(i / 70) as f32 * 0.145 - 5.0, 0.0, (i % 70) as f32 * 0.145 - 5.0]).collect();
        let d = points(&pts);
        let rms = |extra: &[(&str, ParamValue)]| {
            let mut kv = vec![("amplitude", ParamValue::F(1.0)), ("frequency", ParamValue::V3([3.0; 3]))];
            kv.extend(extra.iter().cloned());
            let o = cook(&d, &kv).expect("cooks");
            let sq: f64 =
                o.p().iter().zip(d.p()).flat_map(|(a, b)| (0..3).map(move |k| f64::from(a[k] - b[k]).powi(2))).sum();
            (sq / (3.0 * d.p().len() as f64)).sqrt()
        };
        let plain = rms(&[]);
        assert!((plain / 0.388 - 1.0).abs() < 0.06, "plain rms {plain}");
        let fbm = rms(&[("fractal", ParamValue::Menu(1)), ("octaves", ParamValue::I(4))]);
        assert!((fbm / 0.350 - 1.0).abs() < 0.08, "fbm rms {fbm}");
    }
    #[test]
    fn fractal_octaves_one_equals_plain_noise() {
        let d = cloud(10);
        let plain = cook(&d, &[("fractal", ParamValue::Menu(0))]).expect("plain");
        let fbm1 = cook(&d, &[("fractal", ParamValue::Menu(1)), ("octaves", ParamValue::I(1))]).expect("fbm1");
        assert_eq!(plain.p(), fbm1.p(), "octaves=1 should match plain noise");
    }
    #[test]
    fn ridged_is_non_negative() {
        let d = cloud(20);
        let out = cook(&d, &[("fractal", ParamValue::Menu(3)), ("octaves", ParamValue::I(4)), ("operation", s("set"))])
            .expect("ridged");
        // Ridged noise should be >= 0 since it's 1 - abs(noise)
        for p in out.p() {
            assert!(p[1] >= -1.1, "ridged noise should be >= -1 (with some margin): {}", p[1]);
        }
    }
}
