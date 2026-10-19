//! Attribute Adjust Float / Integer / Color SOPs (tokens `attribadjustfloat`, `attribadjustinteger`, `attribadjustcolor`;
//! Houdini 20.5 HDAs whose core is one attribute wrangle each).
//!
//! # Semantics (hython 20.5: HDA internals + `oracle/houdini/attribadjust/sop_attribadjust_cases.py`, pinned in the tests)
//! Per element of `class` (point | vertex | primitive | detail; detail ignores `group`) in `group`:
//! * value: `const` = `singlevalue`; `attrib` = adjustment attribute * scale + offset (int: `rint`); otherwise a
//!   pattern `d` in [0,1] - `rand` = VEX `rand(separation, randomseed)` (separation = element number or the
//!   `randattrib` attribute), `line` (position projected on start->end), `radial` (distance from start / |end-start|),
//!   `bbox` (position along `bounddir` inside the input bbox or `boundmin`/`boundmax`), `remapattrib` (clamp: fit,
//!   cycle: efit mod 1) - turned into a value by `rangemethod` (min/max, min + range, mid +- range/2, specific values
//!   drawn by weight) and `distribinrange` (uniform | remap ramp | outliers: `outlierpercent` % of elements land in an
//!   extra band of `outlierrange` above / below the range), optional stepping.
//! * VEX `rand(a, b..)` = mantissa of `lcg(wang(h))`, `h = fold(wang(h + float_bits(x)))` from 0; vector rand = successive
//!   `lcg` steps; `random_fhash` (integer random) = `h` as a signed int, taken `mod` the range width (non-negative).
//! * operation (init: keep existing / set / add / sub / mult / min / max; color adds overlay = `1-2(1-a)(1-b)` (VEX
//!   `len()` of a vector is 3, so the multiply branch never fires), screen, hard mix, difference); a missing attribute is
//!   created (float / int default 0 or `default`, color (1,1,1) or `default`, typed Color) and every op then just sets.
//! * `doblend` lerps original -> new (int: `orig + rint(w * (new - orig))`); post-process clamps (color: complement).
//! * Color: optional pre-process (delete all `Cd`, initial value from constant / float attribute), colour correction
//!   (hue shift `/180`, sat, val, brightness, contrast, gain, gamma) and a final `hsvtorgb(rgbtohsv(c))` round trip
//!   (negative maxima collapse to grey, saturation clamps to 1). The component scope menus are not wired to the
//!   wrangle in the HDA (always "all") and are ignored here too.
//! * Float on an int scalar casts the attribute to float first. An existing attribute of the wrong type / size leaves
//!   the geometry unchanged (Houdini warns).
//!
//! Not modelled (-> `Unsupported("unsupported: <parm>=<value>")`): `noise` and `cmap` value types, unit conversion,
//! vector targets of the float SOP (length / component adjust), list-of-values entries, ramp bases other than constant /
//! linear, color blending (HSV colour mix), color min/max with a non-random pattern, color specific values, attribute
//! driven colour correction, spatial patterns on primitive / detail class, a missing helper attribute (blend /
//! separation / adjustment / remap).
use super::group_sop::parse_pattern;
use super::meshutil::{bad, float, int, menu, toggle};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const MAX_KEYS: usize = 8;
const MAX_VALUES: usize = 8;
const CLASSES: [(&str, &str); 4] =
    [("point", "Point"), ("vertex", "Vertex"), ("primitive", "Primitive"), ("detail", "Detail")];
const VALUETYPES: [(&str, &str); 9] = [
    ("const", "Constant"),
    ("rand", "Random"),
    ("noise", "Noise"),
    ("attrib", "Attribute"),
    ("remapattrib", "Remap Attribute"),
    ("cmap", "Color Map"),
    ("line", "Line"),
    ("radial", "Radial"),
    ("bbox", "Bounding Box"),
];
const VT_CONST: u32 = 0;
const VT_RAND: u32 = 1;
const VT_NOISE: u32 = 2;
const VT_ATTRIB: u32 = 3;
const VT_REMAP: u32 = 4;
const VT_CMAP: u32 = 5;
const VT_LINE: u32 = 6;
const VT_RADIAL: u32 = 7;
const OPS: [(&str, &str); 7] = [
    ("init", "Set Initial"),
    ("set", "Set Always"),
    ("add", "Add"),
    ("sub", "Subtract"),
    ("mult", "Multiply"),
    ("min", "Minimum"),
    ("max", "Maximum"),
];
const COLOR_OPS: [(&str, &str); 11] = [
    ("init", "Set Initial"),
    ("set", "Set Always"),
    ("add", "Add"),
    ("sub", "Subtract"),
    ("mult", "Multiply"),
    ("overlay", "Overlay"),
    ("screen", "Screen"),
    ("hardmix", "Hard Mix"),
    ("diff", "Difference"),
    ("min", "Minimum"),
    ("max", "Maximum"),
];
const RANGES: [(&str, &str); 4] = [
    ("minmax", "Min/Max"),
    ("minplusrange", "Min + Range Length"),
    ("midplusminusrange", "Middle +- Range Length"),
    ("specific", "Specific Values"),
];
const COLOR_RANGES: [(&str, &str); 3] =
    [("minmax", "Min/Max"), ("ramp", "Color Ramp"), ("specific", "Specific Values")];
const DISTRIBS: [(&str, &str); 3] =
    [("uniform", "Uniform"), ("remap", "Uniform with Remapping"), ("outliers", "Uniform with Outliers")];
const BASIS: [(&str, &str); 7] = [
    ("constant", "Constant"),
    ("linear", "Linear"),
    ("catmull-rom", "Catmull-Rom"),
    ("monotonecubic", "Monotone Cubic"),
    ("bezier", "Bezier"),
    ("bspline", "B-Spline"),
    ("hermite", "Hermite"),
];
const MODE: [(&str, &str); 2] = [("uniform", "Set Uniform"), ("attrib", "Use Attribute")];

// ---------------------------------------------------------------- VEX numerics (f32, as the wrangle runs them)

fn wang(mut k: u32) -> u32 {
    k = k.wrapping_add(!(k << 16));
    k ^= k >> 5;
    k = k.wrapping_add(k << 3);
    k ^= k >> 13;
    k = k.wrapping_add(!(k << 9));
    k ^ (k >> 17)
}
/// VEX `random_fhash(a, b, ..)`: Wang-hash fold of the float bit patterns.
fn fhash(v: &[f32]) -> u32 {
    v.iter().fold(0u32, |h, x| wang(h.wrapping_add(x.to_bits())))
}
fn lcg(s: u32) -> u32 {
    s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223)
}
fn h01(h: u32) -> f32 {
    f32::from_bits(0x3f80_0000 | (h & 0x007f_ffff)) - 1.0
}
/// VEX `rand(v..)` returning `N` components (successive LCG steps).
fn vrand<const N: usize>(v: &[f32]) -> [f32; N] {
    let mut s = wang(fhash(v));
    let mut o = [0.0f32; N];
    for x in &mut o {
        s = lcg(s);
        *x = h01(s);
    }
    o
}
fn rand1(v: &[f32]) -> f32 {
    vrand::<1>(v)[0]
}
/// VEX `fit` (input clamped to the source range; a degenerate range gives the middle of the target).
fn fit(v: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    if a == b {
        return (c + d) * 0.5;
    }
    let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
    c + (d - c) * t
}
fn efit(v: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    if a == b {
        return (c + d) * 0.5;
    }
    c + (d - c) * ((v - a) / (b - a))
}
fn fit01(t: f32, a: f32, b: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
/// VEX `rgbtohsv` (saturation unclamped; a non-positive maximum is grey).
fn rgb2hsv(c: [f32; 3]) -> [f32; 3] {
    let (r, g, b) = (c[0], c[1], c[2]);
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let delta = mx - mn;
    let s = if mx > 0.0 { delta / mx } else { 0.0 };
    if s == 0.0 || delta == 0.0 {
        return [0.0, s, mx];
    }
    let mut h = if r == mx {
        (g - b) / delta
    } else if g == mx {
        2.0 + (b - r) / delta
    } else {
        4.0 + (r - g) / delta
    };
    h /= 6.0;
    if h < 0.0 {
        h += 1.0;
    }
    [h, s, mx]
}
/// VEX `hsvtorgb` (hue wraps, saturation clamped to [0,1]).
fn hsv2rgb(c: [f32; 3]) -> [f32; 3] {
    let (h, s, v) = (c[0], c[1].min(1.0), c[2]);
    if s.is_nan() || s <= 0.0 {
        return [v; 3];
    }
    let h6 = (h - h.floor()) * 6.0;
    let i = h6.floor();
    let f = h6 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i as i32 {
        0 | 6 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}
/// VEX `sample_discrete`: the weight bucket `u` falls in.
fn sample_discrete(w: &[f32], u: f32) -> usize {
    let total: f32 = w.iter().map(|x| x.max(0.0)).sum();
    if w.is_empty() || total <= 0.0 {
        return 0;
    }
    let mut x = u * total;
    for (i, wi) in w.iter().enumerate() {
        let wi = wi.max(0.0);
        if x < wi {
            return i;
        }
        x -= wi;
    }
    w.len() - 1
}

// ---------------------------------------------------------------- params

fn num(v: f64, is_int: bool) -> ParamValue {
    if is_int {
        ParamValue::I(v as i32)
    } else {
        ParamValue::F(v as f32)
    }
}
fn unsup(op: &str, parm: &str, value: impl std::fmt::Display) -> CookError {
    CookError::Unsupported(format!("{op}: unsupported: {parm}={value}"))
}
fn s(name: &str, v: &str) -> ParamSpec {
    ParamSpec::new(name, ParamValue::S(v.into()))
}
fn f(name: &str, v: f32) -> ParamSpec {
    ParamSpec::new(name, ParamValue::F(v))
}
fn b(name: &str) -> ParamSpec {
    ParamSpec::new(name, ParamValue::B(false))
}
fn m(name: &str, v: u32, items: &[(&str, &str)]) -> ParamSpec {
    ParamSpec::new(name, ParamValue::Menu(v)).menu(items)
}

fn head(attrib: &str, color: bool) -> Vec<ParamSpec> {
    let mut v = vec![
        s("group", "").label("Group"),
        b("doblend").label("Blend"),
        f("blendweight", 1.0).label("Blend Weight"),
        m("blendmode", 0, &[("value", "Constant"), ("attrib", "Use Attribute")]).label("Blend Mode"),
    ];
    if color {
        v.push(m("blendspace", 0, &[("rgb", "RGB"), ("hsv", "HSV")]).label("Blend Space"));
    }
    v.push(s("blendattrib", "mask").label("Blend Attribute"));
    v.push(s("attrib", attrib).label("Attribute"));
    if color {
        v.push(s("componentscope_rgb", "111").label("RGB Scope"));
        v.push(s("componentscope_hsv", "111").label("HSV Scope"));
    }
    v.push(m("class", 0, &CLASSES).label("Class"));
    v
}
fn pattern_specs(seed: i32) -> Vec<ParamSpec> {
    let mut v = vec![
        m("randmethod", 0, &[("ppnum", "Element Number"), ("custom", "Custom Attribute")]).label("Seed Method"),
        s("randattrib", "id").label("Seed Attribute"),
        ParamSpec::new("randomseed", ParamValue::I(seed)).label("Seed"),
    ];
    for (n, d) in [("startpos", [0.0, 0.0, 0.0]), ("endpos", [5.0, 0.0, 0.0])] {
        for (a, x) in ["x", "y", "z"].iter().zip(d) {
            v.push(f(&format!("{n}{a}"), x).label(n));
        }
    }
    v.extend([
        m("bounddir", 0, &[("x", "X"), ("y", "Y"), ("z", "Z")]).label("Direction"),
        b("doboundmin").label("Bound Min"),
        f("boundmin", 0.0).label("Bound Min"),
        b("doboundmax").label("Bound Max"),
        f("boundmax", 1.0).label("Bound Max"),
        s("remapattrib", "mask").label("Remap Attribute"),
        f("remapattrib_rangemin", 0.0).label("Range Min"),
        f("remapattrib_rangemax", 1.0).label("Range Max"),
        m("remapattrib_rangemode", 0, &[("clamp", "Clamp"), ("cycle", "Cycle")]).label("Range Mode"),
    ]);
    v
}
/// Ramp parms: the key count `<name>` then `<name>Npos`, value(s), `<name>Ninterp` for N in 1..=MAX_KEYS.
fn ramp_specs(name: &str, keys: &[(f32, [f32; 3])], color: bool) -> Vec<ParamSpec> {
    let mut v = vec![ParamSpec::new(name, ParamValue::I(keys.len() as i32)).label(name)];
    for k in 0..MAX_KEYS {
        let (pos, val) = keys.get(k).copied().unwrap_or((0.0, [0.0; 3]));
        let n = k + 1;
        v.push(f(&format!("{name}{n}pos"), pos));
        if color {
            for (c, x) in ["cr", "cg", "cb"].iter().zip(val) {
                v.push(f(&format!("{name}{n}{c}"), x));
            }
        } else {
            v.push(f(&format!("{name}{n}value"), val[0]));
        }
        v.push(m(&format!("{name}{n}interp"), 1, &BASIS));
    }
    v
}

fn scalar_spec(is_int: bool) -> Vec<ParamSpec> {
    let (attrib, seed, oseed, maxv) = if is_int { ("variant", 30, 14, 9.0) } else { ("pscale", 40, 3, 1.0) };
    let mut v = head(attrib, false);
    if !is_int {
        v.push(
            m(
                "unitsettings",
                0,
                &[
                    ("none", "None"),
                    ("duration", "Duration"),
                    ("time", "Time"),
                    ("angle", "Angle"),
                    ("custom", "Custom"),
                ],
            )
            .label("Unit Settings"),
        );
    }
    v.extend([
        ParamSpec::new("adjustvalue", ParamValue::B(true)).label("Adjust Value"),
        m("operation", 2, &OPS).label("Operation"),
        m("valuetype", 0, &VALUETYPES).label("Value"),
        m("rangemethod", 0, &RANGES).label("Range"),
        ParamSpec::new("singlevalue", num(0.0, is_int)).label("Value"),
        ParamSpec::new("minvalue", num(0.0, is_int)).label("Min Value"),
        ParamSpec::new("maxvalue", num(maxv, is_int)).label("Max Value"),
        ParamSpec::new("midvalue", num(0.0, is_int)).label("Mid Value"),
        ParamSpec::new("rangevalue", num(maxv, is_int)).label("Range Length"),
        m("distribinrange", 1, &DISTRIBS).label("Distribution"),
        f("outlierpercent", 15.0).label("Outlier Percent"),
        f("outlierrange", 1.0).label("Outlier Range"),
        m("outlieroperation", 0, &[("add", "Additive"), ("mult", "Multiplicative")]).label("Outlier Operation"),
        m("outlierside", 0, &[("above", "Above Max Value"), ("below", "Below Min Value"), ("both", "Both")])
            .label("Outlier Side"),
        ParamSpec::new("outlierseed", ParamValue::I(oseed)).label("Outlier Seed"),
        b("outlierviz").label("Outlier Attribute"),
        s("outlierattrib", "outlier").label("Outlier Attribute Name"),
        b("enablestepping").label("Stepping"),
        ParamSpec::new("stepsize", num(1.0, is_int)).label("Step Size"),
        ParamSpec::new("nvalues", ParamValue::I(1)).label("Number of Values"),
        s("adjustmentattrib", "").label("Adjustment Attribute"),
        f("adjustmentattribscale", 1.0).label("Scale"),
        f("adjustmentattriboffset", 0.0).label("Offset"),
    ]);
    v.extend(ramp_specs("remapramp", &[(0.0, [0.0; 3]), (1.0, [1.0; 3])], false));
    for i in 0..MAX_VALUES {
        v.push(ParamSpec::new(&format!("value{i}"), num(0.0, is_int)).label("Value"));
        v.push(m(&format!("valuetype{i}"), 0, &[("const", "Constant"), ("list", "List of Values")]));
        v.push(f(&format!("weight{i}"), 1.0).label("Weight"));
        v.push(s(&format!("values{i}"), "").label("Values"));
    }
    v.extend(pattern_specs(seed));
    v.extend([
        b("enable_postprocess").label("Post-Process"),
        b("doclampmin").label("Clamp Min"),
        ParamSpec::new("clampminvalue", num(0.0, is_int)).label("Clamp Min"),
        b("doclampmax").label("Clamp Max"),
        ParamSpec::new("clampmaxvalue", num(1.0, is_int)).label("Clamp Max"),
        b("dodefault").label("Set Default"),
        ParamSpec::new("default", num(0.0, is_int)).label("Default"),
    ]);
    v
}
pub fn float_spec() -> Vec<ParamSpec> {
    scalar_spec(false)
}
pub fn int_spec() -> Vec<ParamSpec> {
    scalar_spec(true)
}
pub fn color_spec() -> Vec<ParamSpec> {
    let mut v = head("Cd", true);
    v.extend([
        b("enable_preprocess").label("Pre-Process"),
        b("deleteallcolorattribs").label("Delete All Cd"),
        b("doinitvalue").label("Initial Value"),
        m(
            "initvaluefrom",
            0,
            &[("const", "Constant"), ("colorattrib", "Color Attribute"), ("floatattrib", "Float Attribute")],
        )
        .label("Initial Value From"),
        f("initvaluer", 1.0),
        f("initvalueg", 1.0),
        f("initvalueb", 1.0),
        s("initcolorattrib", "Cd"),
        s("componentscope_initcolor", "111"),
        s("initfloatattrib", "pscale"),
        s("componentscope_initfloat", "1"),
        ParamSpec::new("adjustvalue", ParamValue::B(true)).label("Adjust Value"),
        m("operation", 1, &COLOR_OPS).label("Operation"),
        m("valuetype", 0, &VALUETYPES).label("Value"),
        m("rangemethod", 1, &COLOR_RANGES).label("Range"),
        f("singlevaluer", 1.0),
        f("singlevalueg", 1.0),
        f("singlevalueb", 1.0),
        m("randomcolormodel", 0, &[("rgb", "RGB"), ("hsv", "HSV")]).label("Color Model"),
        b("dobasecolor").label("Base Color"),
        f("basecolorr", 1.0),
        f("basecolorg", 1.0),
        f("basecolorb", 1.0),
    ]);
    for (n, lo, hi) in [
        ("randr", 0.0, 1.0),
        ("randg", 0.0, 1.0),
        ("randb", 0.0, 1.0),
        ("randhue", -180.0, 180.0),
        ("randsat", 0.0, 1.0),
        ("randval", 0.0, 1.0),
    ] {
        v.push(f(&format!("{n}min"), lo));
        v.push(f(&format!("{n}max"), hi));
    }
    v.extend(ramp_specs(
        "colorramp",
        &[
            (0.0, [0.2, 0.0, 1.0]),
            (0.25, [0.0, 0.85, 1.0]),
            (0.5, [0.0, 1.0, 0.1]),
            (0.75, [0.95, 1.0, 0.0]),
            (1.0, [1.0, 0.0, 0.0]),
        ],
        true,
    ));
    v.extend([
        ParamSpec::new("nvalues", ParamValue::I(1)).label("Number of Values"),
        s("adjustmentattrib", "").label("Adjustment Attribute"),
        s("componentscope_adjustmentattrib", "111"),
        f("adjustmentattribgain", 1.0).label("Gain"),
        f("adjustmentattribbrightness", 0.0).label("Brightness"),
    ]);
    v.extend(pattern_specs(36));
    v.push(b("enable_cc").label("Color Correction"));
    for (n, d) in [
        ("hueshift", 0.0),
        ("sat", 1.0),
        ("val", 1.0),
        ("brightness", 0.0),
        ("contrast", 1.0),
        ("gain", 1.0),
        ("gamma", 1.0),
    ] {
        v.push(b(&format!("do{n}")));
        v.push(f(n, d));
        v.push(m(&format!("{n}mode"), 0, &MODE));
        let attr = match n {
            "sat" => "saturation",
            "val" => "value",
            _ => n,
        };
        v.push(s(&format!("{n}attrib"), attr));
    }
    v.extend([
        b("deleteallccattrib"),
        b("enable_postprocess").label("Post-Process"),
        b("doclampmin").label("Clamp Min"),
        f("clampminvaluer", 0.0),
        f("clampminvalueg", 0.0),
        f("clampminvalueb", 0.0),
        b("doclampmax").label("Clamp Max"),
        f("clampmaxvaluer", 1.0),
        f("clampmaxvalueg", 1.0),
        f("clampmaxvalueb", 1.0),
        b("docomplement").label("Complement"),
        b("dodefault").label("Set Default"),
        f("defaultr", 1.0),
        f("defaultg", 1.0),
        f("defaultb", 1.0),
        b("dotypeinfo").label("Set Type Info"),
        m("typeinfo", 1, &[("none", "None"), ("color", "Color")]).label("Type Info"),
    ]);
    v
}

pub fn float_new() -> Box<SopNode> {
    Box::new(Scalar { is_int: false, specs: float_spec() })
}
pub fn int_new() -> Box<SopNode> {
    Box::new(Scalar { is_int: true, specs: int_spec() })
}
pub fn color_new() -> Box<SopNode> {
    Box::new(Color { specs: color_spec() })
}

// ---------------------------------------------------------------- shared evaluation

fn ff(p: &ParamSet, n: &str) -> Result<f32, CookError> {
    Ok(float(p, n)? as f32)
}
/// Int or float parm as f64 (int SOP parms are ints, float SOP parms floats).
fn numf(p: &ParamSet, n: &str) -> Result<f64, CookError> {
    match p.get(n) {
        Some(ParamValue::I(v)) => Ok(f64::from(*v)),
        _ => float(p, n),
    }
}
fn v3p(p: &ParamSet, n: &str) -> Result<[f32; 3], CookError> {
    Ok([ff(p, &format!("{n}x"))?, ff(p, &format!("{n}y"))?, ff(p, &format!("{n}z"))?])
}

struct Ramp {
    keys: Vec<(f32, [f32; 3], u32)>,
}
impl Ramp {
    fn read(op: &str, p: &ParamSet, name: &str, color: bool) -> Result<Ramp, CookError> {
        let n = int(p, name)?;
        if n < 0 || n as usize > MAX_KEYS {
            return Err(unsup(op, name, n));
        }
        let mut keys = Vec::with_capacity(n as usize);
        for k in 1..=n as usize {
            let pos = ff(p, &format!("{name}{k}pos"))?;
            let val = if color {
                [ff(p, &format!("{name}{k}cr"))?, ff(p, &format!("{name}{k}cg"))?, ff(p, &format!("{name}{k}cb"))?]
            } else {
                [ff(p, &format!("{name}{k}value"))?; 3]
            };
            let basis = menu(p, &format!("{name}{k}interp"))?;
            keys.push((pos, val, basis));
        }
        keys.sort_by(|a, b| a.0.total_cmp(&b.0));
        // the last key's basis never interpolates anything
        for (k, key) in keys.iter().enumerate().take(keys.len().saturating_sub(1)) {
            if key.2 > 1 {
                let tok = BASIS.get(key.2 as usize).map_or("?", |b| b.0);
                return Err(unsup(op, &format!("{name}{}interp", k + 1), tok));
            }
        }
        Ok(Ramp { keys })
    }
    fn eval(&self, t: f32) -> [f32; 3] {
        let (Some(first), Some(last)) = (self.keys.first(), self.keys.last()) else { return [0.0; 3] };
        if t.is_nan() || t <= first.0 {
            return first.1;
        }
        if t >= last.0 {
            return last.1;
        }
        for w in self.keys.windows(2) {
            let (a, c) = (&w[0], &w[1]);
            if t >= a.0 && t < c.0 {
                if a.2 == 0 || c.0 == a.0 {
                    return a.1;
                }
                let u = (t - a.0) / (c.0 - a.0);
                return [lerp(a.1[0], c.1[0], u), lerp(a.1[1], c.1[1], u), lerp(a.1[2], c.1[2], u)];
            }
        }
        last.1
    }
}

fn class_of(p: &ParamSet) -> Result<AttrClass, CookError> {
    Ok(match menu(p, "class")? {
        0 => AttrClass::Point,
        1 => AttrClass::Vertex,
        2 => AttrClass::Prim,
        3 => AttrClass::Detail,
        c => return Err(bad(format!("`class`: {c} out of range"))),
    })
}
fn attr_name(p: &ParamSet, n: &str) -> Result<String, CookError> {
    let a = p.string(n).ok_or_else(|| bad(format!("`{n}`: missing or not a string")))?.trim();
    if a.is_empty() || a.contains(char::is_whitespace) {
        return Err(bad(format!("`{n}`: must be one non-empty token")));
    }
    Ok(a.to_string())
}
/// A numeric scalar helper attribute (float or int, size 1) as f32 per element.
fn scalar_attr(op: &str, src: &Detail, class: AttrClass, parm: &str, name: &str) -> Result<Vec<f32>, CookError> {
    match src.attr(class, name).map(|a| &a.data) {
        Some(AttrData::F32(v)) => Ok(v.clone()),
        Some(AttrData::FN { size: 1, data }) => Ok(data.clone()),
        Some(AttrData::I32(v)) => Ok(v.iter().map(|x| *x as f32).collect()),
        Some(AttrData::IN { size: 1, data }) => Ok(data.iter().map(|x| *x as f32).collect()),
        Some(AttrData::V3(v)) if parm == "remapattrib" => {
            Ok(v.iter().map(|c| (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt()).collect())
        }
        Some(_) => Err(unsup(op, parm, format!("{name} (not a numeric scalar)"))),
        None => Err(unsup(op, parm, format!("{name} (missing attribute)"))),
    }
}
fn vec3_attr(src: &Detail, class: AttrClass, name: &str) -> Option<Vec<[f32; 3]>> {
    match src.attr(class, name).map(|a| &a.data) {
        Some(AttrData::V3(v)) => Some(v.clone()),
        Some(AttrData::FN { size: 3, data }) => Some(data.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()),
        _ => None,
    }
}
/// Element positions for spatial patterns (point / vertex class).
fn positions(op: &str, src: &Detail, class: AttrClass, vt: u32) -> Result<Vec<[f32; 3]>, CookError> {
    let p = src.p();
    match class {
        AttrClass::Point => Ok(p.to_vec()),
        AttrClass::Vertex => {
            Ok(src.vertex_point.iter().map(|&q| p.get(q as usize).copied().unwrap_or([0.0; 3])).collect())
        }
        _ => Err(unsup(op, "valuetype", format!("{} on primitive / detail class", VALUETYPES[vt as usize].0))),
    }
}
fn selection(op: &str, src: &Detail, class: AttrClass, p: &ParamSet) -> Result<Vec<bool>, CookError> {
    let n = src.class_len(class);
    if class == AttrClass::Detail {
        return Ok(vec![true; n]);
    }
    let pat = p.string("group").ok_or_else(|| bad("`group`: missing or not a string"))?;
    Ok(parse_pattern(src, class, pat, op)?.unwrap_or_else(|| vec![true; n]))
}

/// Pattern source: `rand` (VEX rand of separation and seed) or a spatial / attribute distribution in [0,1].
struct Pattern {
    vt: u32,
    seed: f32,
    sep: Option<Vec<f32>>,
    pos: Vec<[f32; 3]>,
    a: [f32; 3],
    bdir: [f32; 3],
    lo: f32,
    hi: f32,
    axis: usize,
    remap: Vec<f32>,
    cycle: bool,
}
impl Pattern {
    fn read(op: &str, src: &Detail, class: AttrClass, p: &ParamSet, vt: u32) -> Result<Pattern, CookError> {
        let sep = match menu(p, "randmethod")? {
            0 => None,
            _ => Some(scalar_attr(op, src, class, "randattrib", &attr_name(p, "randattrib")?)?),
        };
        let mut pt = Pattern {
            vt,
            seed: int(p, "randomseed")? as f32,
            sep,
            pos: Vec::new(),
            a: [0.0; 3],
            bdir: [0.0; 3],
            lo: 0.0,
            hi: 1.0,
            axis: 0,
            remap: Vec::new(),
            cycle: false,
        };
        match vt {
            VT_RAND => {}
            VT_REMAP => {
                pt.remap = scalar_attr(op, src, class, "remapattrib", &attr_name(p, "remapattrib")?)?;
                pt.lo = ff(p, "remapattrib_rangemin")?;
                pt.hi = ff(p, "remapattrib_rangemax")?;
                pt.cycle = menu(p, "remapattrib_rangemode")? == 1;
            }
            VT_LINE | VT_RADIAL => {
                pt.pos = positions(op, src, class, vt)?;
                let (s0, e0) = (v3p(p, "startpos")?, v3p(p, "endpos")?);
                pt.a = s0;
                let d = [e0[0] - s0[0], e0[1] - s0[1], e0[2] - s0[2]];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if vt == VT_RADIAL {
                    pt.hi = len;
                } else {
                    pt.bdir = if len > 0.0 { [d[0] / len, d[1] / len, d[2] / len] } else { [1.0, 0.0, 0.0] };
                    pt.lo = dot(s0, pt.bdir);
                    pt.hi = dot(e0, pt.bdir);
                }
            }
            _ => {
                // bbox
                pt.pos = positions(op, src, class, vt)?;
                pt.axis = menu(p, "bounddir")?.min(2) as usize;
                let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
                for q in src.p() {
                    lo = lo.min(q[pt.axis]);
                    hi = hi.max(q[pt.axis]);
                }
                if lo > hi {
                    (lo, hi) = (0.0, 0.0);
                }
                pt.lo = if toggle(p, "doboundmin")? { ff(p, "boundmin")? } else { lo };
                pt.hi = if toggle(p, "doboundmax")? { ff(p, "boundmax")? } else { hi };
            }
        }
        Ok(pt)
    }
    fn separation(&self, i: usize) -> f32 {
        match &self.sep {
            Some(v) => v.get(i).copied().unwrap_or(0.0),
            None => i as f32,
        }
    }
    fn distrib(&self, i: usize) -> f32 {
        let q = self.pos.get(i).copied().unwrap_or([0.0; 3]);
        match self.vt {
            VT_RAND => rand1(&[self.separation(i), self.seed]),
            VT_REMAP => {
                let v = self.remap.get(i).copied().unwrap_or(0.0);
                if self.cycle {
                    efit(v, self.lo, self.hi, 0.0, 1.0) % 1.0
                } else {
                    fit(v, self.lo, self.hi, 0.0, 1.0)
                }
            }
            VT_LINE => fit(dot(q, self.bdir), self.lo, self.hi, 0.0, 1.0),
            VT_RADIAL => {
                let d = [self.a[0] - q[0], self.a[1] - q[1], self.a[2] - q[2]];
                fit((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt(), 0.0, self.hi, 0.0, 1.0)
            }
            _ => fit(q[self.axis], self.lo, self.hi, 0.0, 1.0),
        }
    }
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `distribinrange` / outlier / stepping settings of the float and int SOPs.
struct Range {
    distrib: u32,
    ramp: Ramp,
    pct: f32,
    orange: f32,
    omult: bool,
    side: u32,
    oseed: i32,
    step: Option<f64>,
}
impl Range {
    fn read(op: &str, p: &ParamSet) -> Result<Range, CookError> {
        let distrib = menu(p, "distribinrange")?;
        Ok(Range {
            distrib,
            ramp: if distrib == 1 { Ramp::read(op, p, "remapramp", false)? } else { Ramp { keys: Vec::new() } },
            pct: ff(p, "outlierpercent")?,
            orange: ff(p, "outlierrange")?,
            omult: menu(p, "outlieroperation")? == 1,
            side: menu(p, "outlierside")?,
            oseed: int(p, "outlierseed")?,
            step: if toggle(p, "enablestepping")? { Some(numf(p, "stepsize")?) } else { None },
        })
    }
}

/// Specific values (multiparm `nvalues`: `value<i>`, `weight<i>`; list entries unsupported).
fn specific(op: &str, p: &ParamSet) -> Result<(Vec<f64>, Vec<f32>), CookError> {
    let n = int(p, "nvalues")?;
    if n < 0 || n as usize > MAX_VALUES {
        return Err(unsup(op, "nvalues", n));
    }
    let (mut vals, mut ws) = (Vec::new(), Vec::new());
    for i in 0..n as usize {
        if menu(p, &format!("valuetype{i}"))? != 0 {
            return Err(unsup(op, &format!("valuetype{i}"), "list"));
        }
        vals.push(numf(p, &format!("value{i}"))?);
        ws.push(ff(p, &format!("weight{i}"))?);
    }
    Ok((vals, ws))
}

fn stepping(d: f32, lo: f32, hi: f32, step: f32) -> f32 {
    let nsteps = (1.00001f32 * ((hi - lo) / step + 1.0)).floor().max(1.0);
    let v = (d * nsteps).floor() * step + lo;
    v.max(lo).min(hi)
}
/// Houdini int parm evaluating a float expression (round half up).
fn round_half_up(x: f64) -> i32 {
    (x + 0.5).floor() as i32
}

// ---------------------------------------------------------------- float / integer

struct Scalar {
    is_int: bool,
    specs: Vec<ParamSpec>,
}

/// Original values of the target attribute: `None` = wrong type (Houdini leaves the geometry alone).
enum Target {
    Floats(Vec<f32>),
    Ints(Vec<i32>),
}

impl Scalar {
    fn op(&self) -> &'static str {
        if self.is_int {
            "attribadjustinteger"
        } else {
            "attribadjustfloat"
        }
    }
}

impl Node<Detail> for Scalar {
    fn type_name(&self) -> &'static str {
        self.op()
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let op = self.op();
        let p = ctx.params;
        let class = class_of(p)?;
        let name = attr_name(p, "attrib")?;
        if name.contains(['.', '[']) {
            return Err(unsup(op, "attrib", format!("{name} (component syntax)")));
        }
        if !self.is_int && menu(p, "unitsettings")? != 0 {
            return Err(unsup(op, "unitsettings", menu(p, "unitsettings")?));
        }
        let adjust = toggle(p, "adjustvalue")?;
        let opn = menu(p, "operation")?;
        if opn as usize >= OPS.len() {
            return Err(bad(format!("`operation`: {opn} out of range")));
        }
        let vt = menu(p, "valuetype")?;
        if vt as usize >= VALUETYPES.len() {
            return Err(bad(format!("`valuetype`: {vt} out of range")));
        }
        if adjust && (vt == VT_NOISE || vt == VT_CMAP) {
            return Err(unsup(op, "valuetype", VALUETYPES[vt as usize].0));
        }
        let rm = menu(p, "rangemethod")?;
        let single = numf(p, "singlevalue")?;
        let (minv, maxv, midv, rangev) =
            (numf(p, "minvalue")?, numf(p, "maxvalue")?, numf(p, "midvalue")?, numf(p, "rangevalue")?);
        let (blend, bw, bmode) = (toggle(p, "doblend")?, ff(p, "blendweight")?, menu(p, "blendmode")?);
        let post = toggle(p, "enable_postprocess")?;
        let (cmin, cmax) = (toggle(p, "doclampmin")?, toggle(p, "doclampmax")?);
        let (cminv, cmaxv) = (numf(p, "clampminvalue")?, numf(p, "clampmaxvalue")?);
        let default = if toggle(p, "dodefault")? { numf(p, "default")? } else { 0.0 };
        let (ascale, aoff) = (ff(p, "adjustmentattribscale")?, ff(p, "adjustmentattriboffset")?);
        let range = Range::read(op, p)?;
        let spec_vals =
            if adjust && vt != VT_CONST && vt != VT_ATTRIB && rm == 3 { Some(specific(op, p)?) } else { None };
        let Some(src) = inputs.first() else { return Ok(Arc::default()) };
        let n = src.class_len(class);
        let sel = selection(op, src, class, p)?;
        // target attribute
        let existing = src.attr(class, &name);
        let target = match existing.map(|a| &a.data) {
            None => {
                if self.is_int {
                    Target::Ints(vec![round_half_up(default); n])
                } else {
                    Target::Floats(vec![default as f32; n])
                }
            }
            Some(AttrData::F32(v)) if !self.is_int => Target::Floats(v.clone()),
            Some(AttrData::FN { size: 1, data }) if !self.is_int => Target::Floats(data.clone()),
            Some(AttrData::I32(v)) if !self.is_int => Target::Floats(v.iter().map(|x| *x as f32).collect()),
            Some(AttrData::IN { size: 1, data }) if !self.is_int => {
                Target::Floats(data.iter().map(|x| *x as f32).collect())
            }
            Some(AttrData::V2(_) | AttrData::V3(_) | AttrData::V4(_) | AttrData::FN { .. }) if !self.is_int => {
                return Err(unsup(op, "attrib", format!("{name} (vector attribute: length adjust)")));
            }
            Some(AttrData::I32(v)) => Target::Ints(v.clone()),
            Some(AttrData::IN { size: 1, data }) => Target::Ints(data.clone()),
            Some(_) => {
                (ctx.log)(&format!("{op}: `{name}` exists with the wrong type or size: geometry unchanged"));
                return Ok(Arc::clone(src));
            }
        };
        let has = existing.is_some();
        let needs_pattern = adjust && !matches!(vt, VT_CONST | VT_ATTRIB);
        let pattern = if needs_pattern { Some(Pattern::read(op, src, class, p, vt)?) } else { None };
        let adj = if adjust && vt == VT_ATTRIB {
            scalar_attr(op, src, class, "adjustmentattrib", &attr_name(p, "adjustmentattrib")?)?
        } else {
            Vec::new()
        };
        let battr = if blend && bmode == 1 {
            scalar_attr(op, src, class, "blendattrib", &attr_name(p, "blendattrib")?)?
        } else {
            Vec::new()
        };
        let weight = |i: usize| if bmode == 1 { battr.get(i).copied().unwrap_or(0.0) } else { bw };
        let data = match target {
            Target::Floats(mut cur) => {
                cur.resize(n, default as f32);
                // float range bounds (wrangle `minrange` / `maxrange`, double expressions read back as float)
                let (lo, hi) = match rm {
                    1 => (minv, minv + rangev),
                    2 => (midv - rangev * 0.5, midv + rangev * 0.5),
                    _ => (minv, maxv),
                };
                let (lo, hi) = (lo as f32, hi as f32);
                for (i, x) in cur.iter_mut().enumerate() {
                    if !sel.get(i).copied().unwrap_or(false) {
                        continue;
                    }
                    let orig = *x;
                    let mut out = orig;
                    if adjust {
                        let value = match (vt, &pattern) {
                            (VT_ATTRIB, _) => adj.get(i).copied().unwrap_or(0.0) * ascale + aoff,
                            (_, Some(pt)) => {
                                let d = pt.distrib(i);
                                match &spec_vals {
                                    Some((vals, ws)) => vals.get(sample_discrete(ws, d)).copied().unwrap_or(0.0) as f32,
                                    None => float_range(&range, d, pt.separation(i), lo, hi),
                                }
                            }
                            _ => single as f32,
                        };
                        out = match opn {
                            0 if has => orig,
                            _ if !has => value,
                            2 => orig + value,
                            3 => orig - value,
                            4 => orig * value,
                            5 => orig.min(value),
                            6 => orig.max(value),
                            _ => value,
                        };
                    }
                    if blend {
                        out = lerp(orig, out, weight(i));
                    }
                    if post {
                        if cmin && out < cminv as f32 {
                            out = cminv as f32;
                        }
                        if cmax && out > cmaxv as f32 {
                            out = cmaxv as f32;
                        }
                    }
                    *x = out;
                }
                AttrData::F32(cur)
            }
            Target::Ints(mut cur) => {
                cur.resize(n, round_half_up(default));
                let min_val = if rm < 2 { round_half_up(minv) } else { round_half_up(midv - rangev * 0.5) };
                let width = if rm == 0 { round_half_up(maxv - minv) } else { round_half_up(rangev) };
                let seed = int(p, "randomseed")? as f32;
                for (i, x) in cur.iter_mut().enumerate() {
                    if !sel.get(i).copied().unwrap_or(false) {
                        continue;
                    }
                    let orig = *x;
                    let mut out = orig;
                    if adjust {
                        let value = match (vt, &pattern) {
                            (VT_ATTRIB, _) => {
                                (adj.get(i).copied().unwrap_or(0.0) * ascale + aoff).round_ties_even() as i32
                            }
                            (_, Some(pt)) => {
                                let l_id = match &pt.sep {
                                    Some(v) => v.get(i).copied().unwrap_or(0.0) as i32,
                                    None => i as i32,
                                };
                                let random = vt == VT_RAND;
                                let irand = if random { fhash(&[l_id as f32, seed]) as i32 } else { 0 };
                                let frand = if random {
                                    if spec_vals.is_some() {
                                        rand1(&[irand as f32])
                                    } else {
                                        0.0
                                    }
                                } else {
                                    pt.distrib(i)
                                };
                                match &spec_vals {
                                    Some((vals, ws)) => {
                                        vals.get(sample_discrete(ws, frand)).copied().unwrap_or(0.0) as i32
                                    }
                                    None => {
                                        let r = IntRand { use_flt: !random, irand, frand, l_id };
                                        int_range(&range, &r, min_val, width)
                                    }
                                }
                            }
                            _ => single as i32,
                        };
                        out = match opn {
                            0 if has => orig,
                            _ if !has => value,
                            2 => orig.wrapping_add(value),
                            3 => orig.wrapping_sub(value),
                            4 => orig.wrapping_mul(value),
                            5 => orig.min(value),
                            6 => orig.max(value),
                            _ => value,
                        };
                    }
                    if blend {
                        let dlt = out.wrapping_sub(orig) as f32;
                        out = orig.wrapping_add((weight(i) * dlt).round_ties_even() as i32);
                    }
                    if post {
                        if cmin {
                            out = out.max(cminv as i32);
                        }
                        if cmax {
                            out = out.min(cmaxv as i32);
                        }
                    }
                    *x = out;
                }
                AttrData::I32(cur)
            }
        };
        let mut out = (**src).clone();
        let type_info = match (existing, self.is_int) {
            (Some(a), _) if matches!(a.data, AttrData::F32(_) | AttrData::FN { .. }) => a.type_info,
            _ => TypeInfo::None,
        };
        out.add_attr(Attribute { name, class, data, type_info });
        Ok(Arc::new(out))
    }
}

/// aa_getRangeValue (float): outlier band, remap ramp, stepping, fit into [lo, hi].
fn float_range(r: &Range, d: f32, sep: f32, lo: f32, hi: f32) -> f32 {
    let (mut lo, mut hi, mut dd) = (lo, hi, d);
    if r.distrib == 2 && rand1(&[sep, r.oseed.wrapping_add(1) as f32]) < r.pct * 0.01 {
        let above = r.side == 0 || (r.side == 2 && d > 0.5);
        let below = r.side == 1 || (r.side == 2 && d <= 0.5);
        let span = if r.omult { r.orange * (hi - lo) } else { r.orange };
        if above {
            lo = hi;
            hi += span;
            if r.side == 2 {
                dd = fit(d, 0.5, 1.0, 0.0, 1.0);
            }
        } else if below {
            hi = lo;
            lo -= span;
            if r.side == 2 {
                dd = fit(d, 0.0, 0.5, 0.0, 1.0);
            }
        }
    }
    if r.distrib == 1 {
        dd = r.ramp.eval(d)[0];
    }
    match r.step {
        Some(s) => stepping(dd, lo, hi, s as f32),
        None => fit01(dd, lo, hi),
    }
}

struct IntRand {
    use_flt: bool,
    irand: i32,
    frand: f32,
    l_id: i32,
}
/// aa_getRangeValue (int): `irand mod width` (or a float distribution scaled), outliers, ramp, stepping.
fn int_range(r: &Range, x: &IntRand, min_val: i32, range_width: i32) -> i32 {
    let mut new_min = min_val;
    let mut width = range_width.wrapping_add(1);
    if r.distrib == 2 {
        let [ox, oy] = vrand::<2>(&[r.oseed as f32, x.l_id as f32]);
        if ox < r.pct * 0.01 {
            width = if r.omult {
                (r.orange * range_width as f32).round_ties_even() as i32
            } else {
                r.orange.round_ties_even() as i32
            };
            if r.side == 0 || (r.side == 2 && oy > 0.5) {
                new_min = new_min.wrapping_add(range_width).wrapping_add(1);
            } else {
                new_min = new_min.wrapping_sub(width);
            }
        }
    }
    let modw = |v: i32| if width == 0 { 0 } else { v.rem_euclid(width) };
    let span = width.wrapping_sub(1) as f32;
    let mut val = if r.distrib == 1 {
        let rv = if x.use_flt { x.frand } else { modw(x.irand) as f32 / span };
        (r.ramp.eval(rv)[0] * span).round_ties_even() as i32
    } else if x.use_flt {
        (x.frand * span).round_ties_even() as i32
    } else {
        modw(x.irand)
    };
    if let Some(s) = r.step {
        let s = s as i32;
        val = val.checked_div(s).unwrap_or(0).wrapping_mul(s);
    }
    new_min.wrapping_add(val)
}

// ---------------------------------------------------------------- color

struct Color {
    specs: Vec<ParamSpec>,
}

fn c3(p: &ParamSet, n: &str) -> Result<[f32; 3], CookError> {
    Ok([ff(p, &format!("{n}r"))?, ff(p, &format!("{n}g"))?, ff(p, &format!("{n}b"))?])
}
fn map3(a: [f32; 3], b: [f32; 3], f: impl Fn(f32, f32) -> f32) -> [f32; 3] {
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

/// Settings of the colour-correction block (uniform modes only).
struct Cc {
    hue: Option<f32>,
    sat: Option<f32>,
    val: Option<f32>,
    bright: Option<f32>,
    contrast: Option<f32>,
    gain: Option<f32>,
    gamma: Option<f32>,
}
impl Cc {
    fn read(op: &str, p: &ParamSet) -> Result<Option<Cc>, CookError> {
        if !toggle(p, "enable_cc")? {
            return Ok(None);
        }
        let get = |n: &str| -> Result<Option<f32>, CookError> {
            if !toggle(p, &format!("do{n}"))? {
                return Ok(None);
            }
            if menu(p, &format!("{n}mode"))? != 0 {
                return Err(unsup(op, &format!("{n}mode"), "attrib"));
            }
            Ok(Some(ff(p, n)?))
        };
        Ok(Some(Cc {
            hue: get("hueshift")?,
            sat: get("sat")?,
            val: get("val")?,
            bright: get("brightness")?,
            contrast: get("contrast")?,
            gain: get("gain")?,
            gamma: get("gamma")?,
        }))
    }
    fn apply(&self, mut c: [f32; 3]) -> [f32; 3] {
        if let Some(h) = self.hue {
            let mut hsv = rgb2hsv(c);
            hsv[0] += h / 180.0;
            c = hsv2rgb(hsv);
        }
        if let Some(s) = self.sat {
            let mut hsv = rgb2hsv(c);
            hsv[1] *= s;
            c = hsv2rgb(hsv);
        }
        if let Some(v) = self.val {
            let mut hsv = rgb2hsv(c);
            hsv[2] *= v;
            c = hsv2rgb(hsv);
        }
        if let Some(b) = self.bright {
            c = c.map(|x| x + b);
        }
        if let Some(k) = self.contrast {
            c = c.map(|x| (x - 0.5) * k + 0.5);
        }
        if let Some(g) = self.gain {
            c = c.map(|x| x * g);
        }
        if let Some(g) = self.gamma {
            let e = 1.0 / g;
            c = c.map(|x| x.powf(e));
        }
        c
    }
}

/// Random colour settings (`rangemethod` min/max with the `rand` pattern).
struct RandColor {
    hsv: bool,
    base: Option<[f32; 3]>,
    lo: [f32; 3],
    hi: [f32; 3],
}
impl RandColor {
    fn read(p: &ParamSet) -> Result<RandColor, CookError> {
        let hsv = menu(p, "randomcolormodel")? == 1;
        let names = if hsv { ["randhue", "randsat", "randval"] } else { ["randr", "randg", "randb"] };
        let mut lo = [0.0; 3];
        let mut hi = [0.0; 3];
        for (k, n) in names.iter().enumerate() {
            lo[k] = ff(p, &format!("{n}min"))?;
            hi[k] = ff(p, &format!("{n}max"))?;
        }
        if hsv {
            lo[0] = efit(lo[0], -180.0, 180.0, -0.5, 0.5);
            hi[0] = efit(hi[0], -180.0, 180.0, -0.5, 0.5);
        }
        let base = if toggle(p, "dobasecolor")? { Some(c3(p, "basecolor")?) } else { None };
        Ok(RandColor { hsv, base, lo, hi })
    }
    fn eval(&self, sep: f32, seed: f32) -> [f32; 3] {
        const GOLDEN: f32 = 0.618_034;
        let r = vrand::<3>(&[sep, seed]).map(|x| (x + GOLDEN) % 1.0);
        let fitc = |v: [f32; 3], base: [f32; 3]| {
            [
                fit01(v[0], base[0] + self.lo[0], base[0] + self.hi[0]),
                fit01(v[1], base[1] + self.lo[1], base[1] + self.hi[1]),
                fit01(v[2], base[2] + self.lo[2], base[2] + self.hi[2]),
            ]
        };
        if self.hsv {
            let base = self.base.map_or([0.0; 3], rgb2hsv);
            hsv2rgb(fitc(rgb2hsv(r), base))
        } else {
            fitc(r, self.base.unwrap_or([0.0; 3]))
        }
    }
}

fn color_op(o: [f32; 3], n: [f32; 3], has: bool, opn: u32) -> [f32; 3] {
    match opn {
        0 if has => o,
        _ if !has => n,
        2 => map3(o, n, |a, b| a + b),
        3 => map3(o, n, |a, b| a - b),
        4 => map3(o, n, |a, b| a * b),
        // VEX `len(vector)` is 3, so only the screen-like branch of the overlay formula survives
        5 => map3(o, n, |a, b| 1.0 - 2.0 * (1.0 - a) * (1.0 - b)),
        6 => map3(o, n, |a, b| 1.0 - (1.0 - a) * (1.0 - b)),
        7 => map3(o, n, |a, b| if a + b >= 1.0 { 1.0 } else { 0.0 }),
        8 => map3(o, n, |a, b| (a - b).abs()),
        9 => map3(o, n, f32::min),
        10 => map3(o, n, f32::max),
        _ => n,
    }
}

impl Node<Detail> for Color {
    fn type_name(&self) -> &'static str {
        "attribadjustcolor"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let op = "attribadjustcolor";
        let p = ctx.params;
        let class = class_of(p)?;
        let name = attr_name(p, "attrib")?;
        let adjust = toggle(p, "adjustvalue")?;
        let opn = menu(p, "operation")?;
        if opn as usize >= COLOR_OPS.len() {
            return Err(bad(format!("`operation`: {opn} out of range")));
        }
        let vt = menu(p, "valuetype")?;
        if vt as usize >= VALUETYPES.len() {
            return Err(bad(format!("`valuetype`: {vt} out of range")));
        }
        let rm = menu(p, "rangemethod")?;
        let pattern_vt = !matches!(vt, VT_CONST | VT_ATTRIB);
        if adjust {
            if vt == VT_NOISE || vt == VT_CMAP {
                return Err(unsup(op, "valuetype", VALUETYPES[vt as usize].0));
            }
            if pattern_vt && rm == 2 {
                return Err(unsup(op, "rangemethod", "specific"));
            }
            if pattern_vt && rm == 0 && vt != VT_RAND {
                return Err(unsup(op, "rangemethod", format!("minmax with valuetype={}", VALUETYPES[vt as usize].0)));
            }
        }
        ff(p, "blendweight")?;
        if toggle(p, "doblend")? {
            return Err(unsup(op, "doblend", 1));
        }
        let single = c3(p, "singlevalue")?;
        let (gain, bright) = (ff(p, "adjustmentattribgain")?, ff(p, "adjustmentattribbrightness")?);
        let cc = Cc::read(op, p)?;
        let pre = toggle(p, "enable_preprocess")?;
        let init_on = pre && toggle(p, "doinitvalue")?;
        let init_from = menu(p, "initvaluefrom")?;
        let init_c = c3(p, "initvalue")?;
        let post = toggle(p, "enable_postprocess")?;
        let (cmin, cmax, compl) = (toggle(p, "doclampmin")?, toggle(p, "doclampmax")?, toggle(p, "docomplement")?);
        let (cminv, cmaxv) = (c3(p, "clampminvalue")?, c3(p, "clampmaxvalue")?);
        let default = if toggle(p, "dodefault")? { c3(p, "default")? } else { [1.0; 3] };
        let typeinfo = if toggle(p, "dotypeinfo")? { Some(menu(p, "typeinfo")? == 1) } else { None };
        let ramp = if adjust && pattern_vt && rm == 1 { Some(Ramp::read(op, p, "colorramp", true)?) } else { None };
        let randc = if adjust && pattern_vt && rm == 0 { Some(RandColor::read(p)?) } else { None };
        let Some(src0) = inputs.first() else { return Ok(Arc::default()) };
        // pre-process: delete every `Cd`
        let mut work: Detail = (**src0).clone();
        if pre && toggle(p, "deleteallcolorattribs")? {
            work.attrs.retain(|a| a.name != "Cd");
        }
        let src = &work;
        let n = src.class_len(class);
        let sel = selection(op, src, class, p)?;
        let existing = src.attr(class, &name);
        let mut cur: Vec<[f32; 3]> = match existing {
            None => vec![default; n],
            Some(_) => match vec3_attr(src, class, &name) {
                Some(v) => v,
                None => {
                    if matches!(existing.map(|a| &a.data), Some(AttrData::V4(_) | AttrData::FN { size: 4, .. })) {
                        return Err(unsup(op, "attrib", format!("{name} (4-component color)")));
                    }
                    (ctx.log)(&format!("{op}: `{name}` exists with the wrong type or size: geometry unchanged"));
                    return Ok(Arc::clone(src0));
                }
            },
        };
        cur.resize(n, default);
        let has = existing.is_some() || init_on;
        let pattern = if adjust && pattern_vt { Some(Pattern::read(op, src, class, p, vt)?) } else { None };
        let adj = if adjust && vt == VT_ATTRIB {
            let an = attr_name(p, "adjustmentattrib")?;
            Some(
                vec3_attr(src, class, &an)
                    .ok_or_else(|| unsup(op, "adjustmentattrib", format!("{an} (not a color)")))?,
            )
        } else {
            None
        };
        let init_attr: Option<Vec<[f32; 3]>> = match (init_on, init_from) {
            (false, _) | (true, 0) => None,
            (true, 1) => {
                let an = attr_name(p, "initcolorattrib")?;
                Some(
                    vec3_attr(src, class, &an)
                        .ok_or_else(|| unsup(op, "initcolorattrib", format!("{an} (not a color)")))?,
                )
            }
            (true, _) => {
                let v = scalar_attr(op, src, class, "initfloatattrib", &attr_name(p, "initfloatattrib")?)?;
                Some(v.into_iter().map(|x| [x; 3]).collect())
            }
        };
        for (i, x) in cur.iter_mut().enumerate() {
            if !sel.get(i).copied().unwrap_or(false) {
                continue;
            }
            let mut color = *x;
            if init_on {
                color = match &init_attr {
                    Some(v) => v.get(i).copied().unwrap_or([0.0; 3]),
                    None => init_c,
                };
            }
            let initial = color;
            if adjust {
                let value = match (&pattern, &adj) {
                    (_, Some(a)) => a.get(i).copied().unwrap_or([0.0; 3]).map(|c| c * gain + bright),
                    (Some(pt), _) => match (&ramp, &randc) {
                        (Some(r), _) => r.eval(pt.distrib(i)),
                        (_, Some(rc)) => rc.eval(pt.separation(i), pt.seed),
                        _ => initial,
                    },
                    _ => single,
                };
                color = color_op(initial, value, has, opn);
            }
            if let Some(c) = &cc {
                color = c.apply(color);
            }
            if post {
                for k in 0..3 {
                    if cmin && color[k] < cminv[k] {
                        color[k] = cminv[k];
                    }
                    if cmax && color[k] > cmaxv[k] {
                        color[k] = cmaxv[k];
                    }
                }
                if compl {
                    color = color.map(|c| 1.0 - c);
                }
            }
            *x = hsv2rgb(rgb2hsv(color));
        }
        let type_info = match (typeinfo, existing) {
            (Some(true), _) => TypeInfo::Color,
            (Some(false), _) => TypeInfo::None,
            (None, Some(a)) => a.type_info,
            (None, None) => TypeInfo::Color,
        };
        let mut out = work.clone();
        out.add_attr(Attribute { name, class, data: AttrData::V3(cur), type_info });
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, defaults};
    use super::*;
    use serde_json::Value;

    const FX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/sop-attribadjust/");

    fn node(t: &str) -> Box<SopNode> {
        match t {
            "attribadjustfloat" => float_new(),
            "attribadjustinteger" => int_new(),
            _ => color_new(),
        }
    }
    fn input() -> Detail {
        hexe_geo::read_geo(&std::fs::read(format!("{FX}input.geo")).expect("input.geo")).expect("geo")
    }
    /// Houdini parm values (menus as tokens) -> hexe params by the spec's kind.
    fn params(n: &SopNode, parms: &serde_json::Map<String, Value>) -> ParamSet {
        let mut p = defaults(n);
        for (k, v) in parms {
            let spec = n.params().iter().find(|s| &s.name == k).unwrap_or_else(|| panic!("no hexe parm {k}"));
            let pv = match (&spec.default, v) {
                (ParamValue::Menu(_), Value::String(t)) => ParamValue::Menu(
                    spec.menu.iter().position(|m| &m.0 == t).unwrap_or_else(|| panic!("{k}={t}")) as u32,
                ),
                (ParamValue::S(_), Value::String(t)) => ParamValue::S(t.clone()),
                (ParamValue::F(_), v) => ParamValue::F(v.as_f64().expect("num") as f32),
                (ParamValue::I(_), v) => ParamValue::I(v.as_f64().expect("num") as i32),
                (ParamValue::B(_), v) => ParamValue::B(v.as_f64().expect("num") != 0.0),
                (d, v) => panic!("{k}: {d:?} vs {v}"),
            };
            p = p.with(k, pv);
        }
        p
    }
    fn class(s: &str) -> AttrClass {
        match s {
            "point" => AttrClass::Point,
            "vertex" => AttrClass::Vertex,
            "primitive" => AttrClass::Prim,
            _ => AttrClass::Detail,
        }
    }
    fn close(a: f32, b: f64) -> bool {
        (f64::from(a) - b).abs() <= 2e-6 * b.abs().max(1.0)
    }
    /// Every recorded case of `ty`: hexe output attribute vs Houdini's, element by element.
    fn check_cases(ty: &str) -> usize {
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(format!("{FX}cases.json")).expect("cases.json"))
            .expect("json");
        let base = input();
        let mut fails = Vec::new();
        let mut count = 0;
        for c in doc["cases"].as_array().expect("cases") {
            if c["type"] != ty {
                continue;
            }
            count += 1;
            let name = c["name"].as_str().expect("name");
            let n = node(ty);
            let p = params(&*n, c["parms"].as_object().expect("parms"));
            let out = match cook(&*n, base.clone(), &p) {
                Ok(o) => o,
                Err(e) => {
                    fails.push(format!("{name}: {e:?}"));
                    continue;
                }
            };
            let (cl, an) = (class(c["class"].as_str().expect("class")), c["attrib"].as_str().expect("attrib"));
            if c["unchanged"] == true {
                if out.attr(cl, an) != base.attr(cl, an) {
                    fails.push(format!("{name}: expected unchanged"));
                }
                continue;
            }
            let Some(a) = out.attr(cl, an) else {
                fails.push(format!("{name}: attribute missing"));
                continue;
            };
            let want = c["values"].as_array().expect("values");
            let got: Vec<Vec<f32>> = match &a.data {
                AttrData::F32(v) => v.iter().map(|x| vec![*x]).collect(),
                AttrData::I32(v) => v.iter().map(|x| vec![*x as f32]).collect(),
                AttrData::V3(v) => v.iter().map(|x| x.to_vec()).collect(),
                d => {
                    fails.push(format!("{name}: unexpected data {d:?}"));
                    continue;
                }
            };
            let dt = c["data_type"].as_str().expect("dt");
            let kind_ok =
                matches!((dt, &a.data), ("Float", AttrData::F32(_) | AttrData::V3(_)) | ("Int", AttrData::I32(_)));
            if !kind_ok || got.len() != want.len() {
                fails.push(format!("{name}: kind {dt} / len {} vs {}", got.len(), want.len()));
                continue;
            }
            let mut bad_n = 0;
            for (k, (g, w)) in got.iter().zip(want).enumerate() {
                let w: Vec<f64> = match w {
                    Value::Array(x) => x.iter().map(|y| y.as_f64().expect("num")).collect(),
                    x => vec![x.as_f64().expect("num")],
                };
                if g.len() != w.len() || !g.iter().zip(&w).all(|(x, y)| close(*x, *y)) {
                    if bad_n < 3 {
                        fails.push(format!("{name}[{k}]: hexe {g:?} houdini {w:?}"));
                    }
                    bad_n += 1;
                }
            }
            if bad_n > 0 {
                fails.push(format!("{name}: {bad_n} / {} elements differ", want.len()));
            }
            if dt == "Float" && c["size"] == 3 && c["type_info"] == "Color" && a.type_info != TypeInfo::Color {
                fails.push(format!("{name}: type info {:?}", a.type_info));
            }
        }
        assert!(fails.is_empty(), "{} failures:\n{}", fails.len(), fails.join("\n"));
        count
    }

    /// Houdini 20.5 `oracle/houdini/attribadjust/sop_attribadjust_cases.py` (`f_*`): constant / attribute / random
    /// (seeded, custom seed attribute, uniform / remap ramp / outliers / stepping / specific values) / line / radial /
    /// bbox / remap-attribute values, every operation, blend, clamp, int cast, wrong type, defaults, all four classes.
    #[test]
    fn float_cases_match_houdini() {
        assert!(check_cases("attribadjustfloat") >= 30);
    }
    /// `i_*`: integer random (`random_fhash mod width`, min/max, min+range, mid+-range rounding), ramp, outliers, stepping,
    /// line, attribute (rint), specific values, blend, clamp, primitive class, wrong type, new attribute in a group.
    #[test]
    fn integer_cases_match_houdini() {
        assert!(check_cases("attribadjustinteger") >= 18);
    }
    /// `c_*`: constant / random RGB / HSV (with base colour) / ramp (random, line) / attribute values, all 11 operations,
    /// colour correction, clamp + complement, pre-process initial value, HSV round trip, new attribute default + type.
    #[test]
    fn color_cases_match_houdini() {
        assert!(check_cases("attribadjustcolor") >= 25);
    }

    #[test]
    fn vex_rand_matches_recorded_values() {
        // hython: rand(0) .. rand(2), rand(0, 40), rand(0.1), vector rand(3) and random_fhash(0, 30)
        assert_eq!(rand1(&[0.0]), 0.417_231_8);
        assert_eq!(rand1(&[1.0]), 0.002_925_157_5);
        assert_eq!(rand1(&[0.0, 40.0]), 0.603_710_17);
        assert_eq!(rand1(&[0.1]), 0.879_700_2);
        assert_eq!(vrand::<3>(&[3.0]), [0.262_380_48, 0.736_549_6, 0.116_141_44]);
        assert_eq!(fhash(&[0.0, 30.0]) as i32, 431_185_554);
        assert_eq!(fhash(&[2.0, 30.0]) as i32, -1_300_907_755);
    }

    #[test]
    fn hsv_matches_vex_probe() {
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        assert!(near(rgb2hsv([0.2, 0.5, 0.9]), [0.595_238_1, 0.777_777_8, 0.9]));
        assert!(near(rgb2hsv([1.5, -0.2, 0.4]), [0.941_176_4, 1.133_333_3, 1.5]));
        assert!(near(rgb2hsv([-0.5, -0.2, -0.1]), [0.0, 0.0, -0.1]));
        assert!(near(hsv2rgb([0.2, 0.5, 0.9]), [0.81, 0.9, 0.45]));
        assert!(near(hsv2rgb([1.5, -0.2, 0.4]), [0.4; 3]));
        assert!(near(hsv2rgb([0.9, 0.8, 0.1]), [0.1, 0.02, 0.068]));
    }

    fn grid() -> Detail {
        let mut d = Detail::default();
        d.add_points(9);
        *d.p_mut() = (0..9).map(|i| [(i % 3) as f32, 0.0, (i / 3) as f32]).collect();
        d.add_poly(&[0, 1, 4, 3], true);
        d
    }
    fn run(n: &SopNode, d: Detail, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        cook(n, d, &ov.iter().fold(defaults(n), |p, (k, v)| p.with(k, v.clone())))
    }

    #[test]
    fn hostile_inputs_never_panic() {
        let (fl, it, co) = (float_new(), int_new(), color_new());
        for n in [&fl, &it, &co] {
            let n = &**n;
            assert!(matches!(run(n, grid(), &[("blendweight", ParamValue::F(f32::NAN))]), Err(CookError::BadParam(_))));
            assert!(run(n, grid(), &[("group", ParamValue::S("nope".into()))]).is_err());
            assert!(run(n, grid(), &[("attrib", ParamValue::S("a b".into()))]).is_err());
            assert!(run(n, grid(), &[("class", ParamValue::Menu(9))]).is_err());
            assert!(run(n, grid(), &[("valuetype", ParamValue::Menu(99))]).is_err());
            assert!(matches!(
                run(n, grid(), &[("valuetype", ParamValue::Menu(VT_NOISE))]),
                Err(CookError::Unsupported(_))
            ));
            // missing helper attributes -> Unsupported, never a guess
            let custom = [("valuetype", ParamValue::Menu(VT_RAND)), ("randmethod", ParamValue::Menu(1))];
            assert!(matches!(run(n, grid(), &custom), Err(CookError::Unsupported(_))));
            // spatial pattern on detail class
            let det = [("class", ParamValue::Menu(3)), ("valuetype", ParamValue::Menu(VT_LINE))];
            assert!(matches!(run(n, grid(), &det), Err(CookError::Unsupported(_))));
            // ramp with too many keys / spline basis
            assert!(run(n, Detail::default(), &[]).is_ok());
        }
        // wrong attribute type: geometry unchanged
        let mut d = grid();
        d.add_attr(Attribute {
            name: "pscale".into(),
            class: AttrClass::Point,
            data: AttrData::Str { idx: vec![0; 9], table: vec!["x".into()] },
            type_info: TypeInfo::None,
        });
        let o = run(&*fl, d.clone(), &[]).expect("passthrough");
        assert_eq!(o.attr(AttrClass::Point, "pscale"), d.attr(AttrClass::Point, "pscale"));
        // NaN attribute values flow through without panicking
        d.add_attr(Attribute {
            name: "pscale".into(),
            class: AttrClass::Point,
            data: AttrData::F32(vec![f32::NAN; 9]),
            type_info: TypeInfo::None,
        });
        let o = run(&*fl, d.clone(), &[("valuetype", ParamValue::Menu(VT_RAND))]).expect("cooks");
        assert!(matches!(&o.attr(AttrClass::Point, "pscale").expect("pscale").data, AttrData::F32(v) if v.len() == 9));
        d.add_attr(Attribute {
            name: "Cd".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[f32::NAN, f32::INFINITY, -1.0]; 9]),
            type_info: TypeInfo::Color,
        });
        assert!(run(&*co, d.clone(), &[("enable_cc", ParamValue::B(true)), ("dogamma", ParamValue::B(true))]).is_ok());
        // vector target of the float SOP / blend on color / spline ramp: Unsupported
        assert!(matches!(
            run(&*fl, d.clone(), &[("attrib", ParamValue::S("P".into()))]),
            Err(CookError::Unsupported(_))
        ));
        assert!(matches!(run(&*co, d.clone(), &[("doblend", ParamValue::B(true))]), Err(CookError::Unsupported(_))));
        let spline = [("valuetype", ParamValue::Menu(VT_RAND)), ("remapramp1interp", ParamValue::Menu(2))];
        assert!(matches!(run(&*fl, d.clone(), &spline), Err(CookError::Unsupported(_))));
        assert!(matches!(
            run(&*it, d.clone(), &[("valuetype", ParamValue::Menu(VT_RAND)), ("remapramp", ParamValue::I(99))]),
            Err(CookError::Unsupported(_))
        ));
        // degenerate int range / step: no division by zero
        let z = [
            ("valuetype", ParamValue::Menu(VT_RAND)),
            ("minvalue", ParamValue::I(0)),
            ("maxvalue", ParamValue::I(-1)),
            ("enablestepping", ParamValue::B(true)),
            ("stepsize", ParamValue::I(0)),
            ("distribinrange", ParamValue::Menu(0)),
        ];
        assert!(run(&*it, d, &z).is_ok());
    }
}
