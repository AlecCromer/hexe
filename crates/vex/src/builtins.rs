//! Operators on [`Value`]s and the builtin function table.
use crate::Value;
use crate::ast::BinOp;
use crate::interp::{Host, R, fail};

/// Scalar op on f64 (also the per-component op of vectors). Division by zero follows IEEE (inf/NaN).
fn num(op: BinOp, a: f64, b: f64) -> f64 {
    match op {
        BinOp::Add => a + b,
        BinOp::Sub => a - b,
        BinOp::Mul => a * b,
        BinOp::Div => a / b,
        BinOp::Mod => a % b,
        _ => f64::NAN,
    }
}

fn cmp(op: BinOp, a: f64, b: f64) -> Option<bool> {
    Some(match op {
        BinOp::Eq => a == b,
        BinOp::Ne => a != b,
        BinOp::Lt => a < b,
        BinOp::Le => a <= b,
        BinOp::Gt => a > b,
        BinOp::Ge => a >= b,
        _ => return None,
    })
}

fn mismatch<T>(op: BinOp, a: &Value, b: &Value) -> R<T> {
    fail(format!("type mismatch: {} {op:?} {}", a.ty().name(), b.ty().name()))
}

/// Binary arithmetic / comparison (`And`/`Or` are short-circuited by the interpreter).
pub(crate) fn arith(op: BinOp, a: &Value, b: &Value) -> R<Value> {
    let is_cmp = matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge);
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok(match op {
            BinOp::Add => Value::Int(x.wrapping_add(*y)),
            BinOp::Sub => Value::Int(x.wrapping_sub(*y)),
            BinOp::Mul => Value::Int(x.wrapping_mul(*y)),
            BinOp::Div if *y == 0 => Value::Float(*x as f64 / 0.0),
            BinOp::Div => Value::Int(x.wrapping_div(*y)),
            BinOp::Mod if *y == 0 => Value::Int(0),
            BinOp::Mod => Value::Int(x.wrapping_rem(*y)),
            _ => Value::Int(i64::from(cmp(op, *x as f64, *y as f64).unwrap_or(false))),
        }),
        _ if a.as_f64().is_some() && b.as_f64().is_some() => {
            let (x, y) = (a.as_f64().unwrap_or(0.0), b.as_f64().unwrap_or(0.0));
            Ok(if is_cmp { Value::Int(i64::from(cmp(op, x, y).unwrap_or(false))) } else { Value::Float(num(op, x, y)) })
        }
        (Value::Str(x), Value::Str(y)) => match op {
            BinOp::Eq => Ok(Value::Int(i64::from(x == y))),
            BinOp::Ne => Ok(Value::Int(i64::from(x != y))),
            BinOp::Add => Ok(Value::Str(format!("{x}{y}"))),
            _ => mismatch(op, a, b),
        },
        _ => {
            let (ca, cb) = (a.comps(), b.comps());
            let (xs, ys): (Vec<f64>, Vec<f64>) = match (ca, cb, a.as_f64(), b.as_f64()) {
                (Some(x), Some(y), _, _) if x.len() == y.len() => (x.to_vec(), y.to_vec()),
                (Some(x), None, _, Some(s)) => (x.to_vec(), vec![s; x.len()]),
                (None, Some(y), Some(s), _) => (vec![s; y.len()], y.to_vec()),
                _ => return mismatch(op, a, b),
            };
            if is_cmp {
                return match op {
                    BinOp::Eq => Ok(Value::Int(i64::from(xs == ys))),
                    BinOp::Ne => Ok(Value::Int(i64::from(xs != ys))),
                    _ => fail("vectors support only == and !="),
                };
            }
            let out: Vec<f64> = xs.iter().zip(&ys).map(|(p, q)| num(op, *p, *q)).collect();
            Value::from_comps(&out).map_or_else(|| fail("bad vector"), Ok)
        }
    }
}

fn scalar(v: &Value, f: &str) -> R<f64> {
    v.as_f64().map_or_else(|| fail(format!("{f}: expected a number, got {}", v.ty().name())), Ok)
}

fn vec_of(v: &Value, f: &str) -> R<Vec<f64>> {
    match v.comps() {
        Some(c) => Ok(c.to_vec()),
        None => fail(format!("{f}: expected a vector, got {}", v.ty().name())),
    }
}

fn want(f: &str, a: &[Value], n: usize) -> R<()> {
    if a.len() == n { Ok(()) } else { fail(format!("{f}: expected {n} argument(s), got {}", a.len())) }
}

/// Apply `f` per component (scalars → float).
fn map1(f: &str, v: &Value, g: impl Fn(f64) -> f64) -> R<Value> {
    match v.comps() {
        Some(c) => {
            Value::from_comps(&c.iter().map(|x| g(*x)).collect::<Vec<_>>()).map_or_else(|| fail("bad vector"), Ok)
        }
        None => Ok(Value::Float(g(scalar(v, f)?))),
    }
}

/// Broadcasting binary map (scalar·scalar, vec·vec same dim, vec·scalar, scalar·vec).
fn map2(f: &str, a: &Value, b: &Value, g: impl Fn(f64, f64) -> f64) -> R<Value> {
    match (a.comps(), b.comps()) {
        (None, None) => Ok(Value::Float(g(scalar(a, f)?, scalar(b, f)?))),
        (Some(x), Some(y)) if x.len() == y.len() => {
            Value::from_comps(&x.iter().zip(y).map(|(p, q)| g(*p, *q)).collect::<Vec<_>>())
                .map_or_else(|| fail("bad vector"), Ok)
        }
        (Some(x), None) => {
            let s = scalar(b, f)?;
            Value::from_comps(&x.iter().map(|p| g(*p, s)).collect::<Vec<_>>()).map_or_else(|| fail("bad vector"), Ok)
        }
        (None, Some(y)) => {
            let s = scalar(a, f)?;
            Value::from_comps(&y.iter().map(|q| g(s, *q)).collect::<Vec<_>>()).map_or_else(|| fail("bad vector"), Ok)
        }
        _ => fail(format!("{f}: vector size mismatch")),
    }
}

fn len(c: &[f64]) -> f64 {
    c.iter().map(|x| x * x).sum::<f64>().sqrt()
}

// ---- hashing / noise (OUR streams, not Houdini's) ----
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Uniform [0,1) from the bit patterns of the inputs (`-0.0` folded onto `0.0`).
fn rand_of(c: &[f64]) -> f64 {
    let mut h = 0x5EED_u64;
    for x in c {
        let x = if *x == 0.0 { 0.0 } else { *x };
        h = mix(h ^ x.to_bits());
    }
    (h >> 11) as f64 / (1u64 << 53) as f64
}

fn grad(i: i64, j: i64, k: i64) -> [f64; 3] {
    const G: [[f64; 3]; 12] = [
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
    let h = mix(i as u64 ^ mix(j as u64 ^ mix(k as u64)));
    G.get((h % 12) as usize).copied().unwrap_or([0.0; 3])
}

/// 3-D gradient noise mapped to [0,1] (0.5 at integer lattice points).
fn noise3(q: [f64; 3]) -> f64 {
    let f = |x: f64| x.floor();
    let (ix, iy, iz) = (f(q[0]), f(q[1]), f(q[2]));
    let (fx, fy, fz) = (q[0] - ix, q[1] - iy, q[2] - iz);
    let (ix, iy, iz) = (ix as i64, iy as i64, iz as i64);
    let fade = |t: f64| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let lerp = |t: f64, a: f64, b: f64| a + t * (b - a);
    let corner = |dx: i64, dy: i64, dz: i64| {
        let g = grad(ix.wrapping_add(dx), iy.wrapping_add(dy), iz.wrapping_add(dz));
        g[0] * (fx - dx as f64) + g[1] * (fy - dy as f64) + g[2] * (fz - dz as f64)
    };
    let (u, v, w) = (fade(fx), fade(fy), fade(fz));
    let x00 = lerp(u, corner(0, 0, 0), corner(1, 0, 0));
    let x10 = lerp(u, corner(0, 1, 0), corner(1, 1, 0));
    let x01 = lerp(u, corner(0, 0, 1), corner(1, 0, 1));
    let x11 = lerp(u, corner(0, 1, 1), corner(1, 1, 1));
    let n = lerp(w, lerp(v, x00, x10), lerp(v, x01, x11)).clamp(-1.0, 1.0);
    0.5 + 0.5 * n
}

fn handle(f: &str, v: &Value) -> R<()> {
    match v {
        Value::Int(0) => Ok(()),
        _ => fail(format!("{f}: only geometry handle 0 (the input) is supported")),
    }
}

fn str_arg<'a>(f: &str, v: &'a Value) -> R<&'a str> {
    match v {
        Value::Str(s) => Ok(s),
        _ => fail(format!("{f}: expected a string, got {}", v.ty().name())),
    }
}

fn int_arg(f: &str, v: &Value) -> R<i64> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Float(x) => Ok(*x as i64),
        _ => fail(format!("{f}: expected an int, got {}", v.ty().name())),
    }
}

/// Builtin dispatch. Unknown name → error.
pub(crate) fn call(host: &mut dyn Host, name: &str, a: &[Value]) -> R<Value> {
    let f = name;
    match name {
        "length" => {
            want(f, a, 1)?;
            match a[0].comps() {
                Some(c) => Ok(Value::Float(len(c))),
                None => Ok(Value::Float(scalar(&a[0], f)?.abs())),
            }
        }
        "normalize" => {
            want(f, a, 1)?;
            let c = vec_of(&a[0], f)?;
            let l = len(&c);
            let out: Vec<f64> = c.iter().map(|x| if l > 0.0 { x / l } else { 0.0 }).collect();
            Value::from_comps(&out).map_or_else(|| fail("bad vector"), Ok)
        }
        "dot" => {
            want(f, a, 2)?;
            let (x, y) = (vec_of(&a[0], f)?, vec_of(&a[1], f)?);
            if x.len() != y.len() {
                return fail("dot: vector size mismatch");
            }
            Ok(Value::Float(x.iter().zip(&y).map(|(p, q)| p * q).sum()))
        }
        "cross" => {
            want(f, a, 2)?;
            match (&a[0], &a[1]) {
                (Value::Vec3(p), Value::Vec3(q)) => {
                    Ok(Value::Vec3([p[1] * q[2] - p[2] * q[1], p[2] * q[0] - p[0] * q[2], p[0] * q[1] - p[1] * q[0]]))
                }
                _ => fail("cross: expected two vectors"),
            }
        }
        "distance" => {
            want(f, a, 2)?;
            let d = arith(BinOp::Sub, &a[0], &a[1])?;
            match d.comps() {
                Some(c) => Ok(Value::Float(len(c))),
                None => Ok(Value::Float(scalar(&d, f)?.abs())),
            }
        }
        "abs" => {
            want(f, a, 1)?;
            match &a[0] {
                Value::Int(i) => Ok(Value::Int(i.wrapping_abs())),
                v => map1(f, v, f64::abs),
            }
        }
        "floor" | "ceil" | "sqrt" | "sin" | "cos" | "tan" | "exp" | "log" | "fract" | "rint" | "sign" => {
            want(f, a, 1)?;
            let g: fn(f64) -> f64 = match name {
                "floor" => f64::floor,
                "ceil" => f64::ceil,
                "sqrt" => f64::sqrt,
                "sin" => f64::sin,
                "cos" => f64::cos,
                "tan" => f64::tan,
                "exp" => f64::exp,
                "log" => f64::ln,
                "fract" => |x| x - x.floor(),
                "rint" => f64::round_ties_even,
                _ => |x| {
                    if x > 0.0 {
                        1.0
                    } else if x < 0.0 {
                        -1.0
                    } else {
                        0.0
                    }
                },
            };
            map1(f, &a[0], g)
        }
        "pow" | "atan2" => {
            want(f, a, 2)?;
            map2(f, &a[0], &a[1], if name == "pow" { f64::powf } else { f64::atan2 })
        }
        "min" | "max" => {
            want(f, a, 2)?;
            if let (Value::Int(x), Value::Int(y)) = (&a[0], &a[1]) {
                return Ok(Value::Int(if name == "min" { *x.min(y) } else { *x.max(y) }));
            }
            map2(f, &a[0], &a[1], if name == "min" { f64::min } else { f64::max })
        }
        "clamp" => {
            want(f, a, 3)?;
            if let (Value::Int(x), Value::Int(lo), Value::Int(hi)) = (&a[0], &a[1], &a[2]) {
                return Ok(Value::Int(*x.max(lo).min(hi)));
            }
            let lo = map2(f, &a[0], &a[1], f64::max)?;
            map2(f, &lo, &a[2], f64::min)
        }
        "lerp" => {
            want(f, a, 3)?;
            let d = arith(BinOp::Sub, &a[1], &a[0])?;
            let m = arith(BinOp::Mul, &d, &a[2])?;
            arith(BinOp::Add, &a[0], &m).map(|v| match v {
                Value::Int(i) => Value::Float(i as f64),
                v => v,
            })
        }
        "fit" => {
            want(f, a, 5)?;
            let n: Vec<f64> = a.iter().map(|v| scalar(v, f)).collect::<R<_>>()?;
            let (v, omin, omax, nmin, nmax) = (n[0], n[1], n[2], n[3], n[4]);
            let t = if omax == omin { 0.0 } else { ((v - omin) / (omax - omin)).clamp(0.0, 1.0) };
            Ok(Value::Float(nmin + t * (nmax - nmin)))
        }
        "fit01" => {
            want(f, a, 3)?;
            let n: Vec<f64> = a.iter().map(|v| scalar(v, f)).collect::<R<_>>()?;
            Ok(Value::Float(n[1] + n[0].clamp(0.0, 1.0) * (n[2] - n[1])))
        }
        "rand" => {
            want(f, a, 1)?;
            match a[0].comps() {
                Some(c) => Ok(Value::Float(rand_of(c))),
                None => Ok(Value::Float(rand_of(&[scalar(&a[0], f)?]))),
            }
        }
        "noise" => {
            want(f, a, 1)?;
            let q = match a[0].comps() {
                Some([x, y, z, ..]) => [*x, *y, *z],
                Some([x, y]) => [*x, *y, 0.0],
                _ => [scalar(&a[0], f)?, 0.0, 0.0],
            };
            Ok(Value::Float(noise3(q)))
        }
        "set" => {
            let c: Vec<f64> = a.iter().map(|v| scalar(v, f)).collect::<R<_>>()?;
            Value::from_comps(&c).map_or_else(|| fail("set: expected 2 to 4 numbers"), Ok)
        }
        "vector" => {
            want(f, a, 1)?;
            match &a[0] {
                Value::Vec3(v) => Ok(Value::Vec3(*v)),
                v => {
                    let s = scalar(v, f)?;
                    Ok(Value::Vec3([s; 3]))
                }
            }
        }
        "float" => {
            want(f, a, 1)?;
            Ok(Value::Float(scalar(&a[0], f)?))
        }
        "int" => {
            want(f, a, 1)?;
            Ok(Value::Int(scalar(&a[0], f)? as i64))
        }
        "printf" => Ok(Value::Int(0)),
        "npoints" => {
            want(f, a, 1)?;
            handle(f, &a[0])?;
            Ok(Value::Int(host.npoints()))
        }
        "addpoint" => {
            want(f, a, 2)?;
            handle(f, &a[0])?;
            match &a[1] {
                Value::Vec3(p) => Ok(Value::Int(host.add_point(*p)?)),
                v => fail(format!("addpoint: expected a vector position, got {}", v.ty().name())),
            }
        }
        "removepoint" => {
            want(f, a, 2)?;
            handle(f, &a[0])?;
            host.remove_point(int_arg(f, &a[1])?)?;
            Ok(Value::Int(0))
        }
        "addprim" => {
            if a.len() < 2 {
                return fail("addprim: expected (0, 'poly'|'polyline', point numbers...)");
            }
            handle(f, &a[0])?;
            let kind = str_arg(f, &a[1])?;
            let pts: Vec<i64> = a[2..].iter().map(|v| int_arg(f, v)).collect::<R<_>>()?;
            Ok(Value::Int(host.add_prim(kind, &pts)?))
        }
        "setpointattrib" => {
            want(f, a, 4)?;
            handle(f, &a[0])?;
            host.set_point_attrib(str_arg(f, &a[1])?, int_arg(f, &a[2])?, a[3].clone())?;
            Ok(Value::Int(0))
        }
        "point" => {
            want(f, a, 3)?;
            handle(f, &a[0])?;
            Ok(host.point(str_arg(f, &a[1])?, int_arg(f, &a[2])?)?)
        }
        _ => fail(format!("unknown function `{name}`")),
    }
}
