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
//! * `outlierviz` (float / int, outlier distribution): the range branch (pattern value type, not specific values) also
//!   writes int attribute `outlierattrib` = 1 for elements that landed in the outlier band, 0 otherwise (elements
//!   outside `group` keep an existing value, else 0); other value types write nothing.
//!
//! # Parms
//! Every Houdini 20.5 parm tuple of the three HDAs is declared under its tuple name (pinned against the hython dump
//! `fixtures/sop-attribadjust/parms.json`), so a saved `.hip` drops nothing. Float tuples of 2 or 3 components are `V3`
//! (`startpos`, `endpos`, colour `singlevalue` / `initvalue` / `basecolor` / `clampminvalue` / `clampmaxvalue` /
//! `default`, ramp keys `colorrampNc`; the 2-tuples `remapattrib_range`, `randr` .. `randval` = (min, max, unused)).
//! Declared but never read:
//! * UI only: folders (`folderN`, `xn__`, multiparm `folder3_N` / `folder3_N_2`), separators (`sepparm`, `sepparm2`),
//!   the compute-range buttons, `gammapresets` (a preset menu writing `gamma`), `colorpalette` (ramp presets),
//!   `deleteblendattrib`, and the component scope menus (see above).
//! * Only used by unsupported modes, which cook to `Unsupported`: noise (`basis`, `locationattrib`, `element*`,
//!   `offset*`, `usenoiseexpression`, `noiseexpression`, `vex_cwdpath`, `animated`, `pulseduration`, `fractal`, `oct`,
//!   `lac`, `rough`, `disp`, `dispesize`, `gflow`, `flowrot`, `accuml`, `accumg`), colour map (`cmap*`,
//!   `componentscope_cmap`, `srccolorspace`, `uvattrib`, `t`, `r`, `scale`, `scaletype`, `s`, `p`), units
//!   (`attribunit`, `parmunit`, `unitscale`, `unitoffset`), colour specific values (`colorN`, `colorschemeN`, `weightN`,
//!   the scheme angles / distances), colour blending (`blendspace`), attribute-driven colour correction (`*attrib`,
//!   `deleteallccattrib`).
//!
//! Not modelled (-> `Unsupported("unsupported: <parm>=<value>")`): `noise` and `cmap` value types, unit conversion,
//! vector targets of the float SOP (length / component adjust), list-of-values entries, ramp bases other than constant /
//! linear, color blending (HSV colour mix), color min/max with a non-random pattern, color specific values, attribute
//! driven colour correction, spatial patterns on primitive / detail class, a missing helper attribute (blend /
//! separation / adjustment / remap).
use super::group_sop::parse_pattern;
use super::meshutil::{bad, float, int, menu, toggle, v3};
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
    if is_int { ParamValue::I(v as i32) } else { ParamValue::F(v as f32) }
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
/// A Houdini parm tuple of 2 or 3 floats (2-tuples use the first two slots; the third is unused).
fn v3s(name: &str, v: [f32; 3]) -> ParamSpec {
    ParamSpec::new(name, ParamValue::V3(v))
}
fn ip(name: &str, v: i32) -> ParamSpec {
    ParamSpec::new(name, ParamValue::I(v))
}

const XFORM_MENU: &str = "BUTTONS_decompose_transforms";
const NOISE_EXPR: &str =
    "// Parameters to modify:\n// Location Attribute - pos\n// Element Size - elementsize\n// Offset - offset\n\n";
const NOISE_BASIS: [(&str, &str); 12] = [
    ("value_fast", "Fast"),
    ("sparse", "Sparse Convolution"),
    ("alligator", "Alligator"),
    ("perlin", "Perlin"),
    ("flow", "Perlin Flow"),
    ("simplex", "Simplex"),
    ("worleyFA", "Worley Cellular F1"),
    ("worleyFB", "Worley Cellular F2-F1"),
    ("mworleyFA", "Manhattan Cellular F1"),
    ("mworleyFB", "Manhattan Cellular F2-F1"),
    ("cworleyFA", "Chebyshev Cellular F1"),
    ("cworleyFB", "Chebyshev Cellular F2-F1"),
];
const SCHEMES: [(&str, &str); 8] = [
    ("const", "Constant"),
    ("comp", "Complementary"),
    ("twoanalog", "Two Analogous"),
    ("fouranalog", "Four Analogous"),
    ("triadic", "Triadic"),
    ("tetradic", "Tetradic"),
    ("fiveshades", "Five Shades"),
    ("fivetints", "Five Tints"),
];
/// Houdini folder / separator names of each HDA (UI only: declared so a saved `.hip` keeps them, never read).
const FLOAT_FOLDERS: [&str; 15] = [
    "folder1", "folder7", "xn__", "folder4", "folder0", "folder5", "folder9", "folder8", "folder2", "folder10",
    "folder11", "folder12", "folder13", "folder3", "folder6",
];
const INT_FOLDERS: [&str; 13] = [
    "folder1", "folder4", "folder0", "folder5", "folder9", "folder8", "folder2", "folder13", "folder10", "folder11",
    "folder12", "folder3", "folder6",
];
const COLOR_FOLDERS: [&str; 15] = [
    "folder0", "folder13", "folder1", "folder4", "folder5", "folder9", "folder8", "folder6", "folder7", "folder12",
    "folder10", "folder2", "folder14", "folder3", "folder11",
];

/// Parms hexe declares but never reads (module doc): UI folders / separator / buttons, and the noise / colour-map
/// settings, which only matter for `valuetype` noise / cmap (cook -> `Unsupported`).
fn ignored_specs(folders: &[&str], sep: &str, color: bool, t: f32) -> Vec<ParamSpec> {
    let mut v: Vec<ParamSpec> = folders.iter().map(|n| ip(n, 0)).collect();
    v.extend([s(sep, ""), ip("bbox_computerange", 0), ip("remapattrib_computerange", 0)]);
    v.extend([
        s("cmap", "$HH/pic/Mandril.pic").label("Color Map"),
        m("cmapmode", 0, &[("path", "File"), ("attrib", "Attribute")]).label("Color Map Source"),
        s("cmapattrib", "path").label("Color Map Attribute"),
        s("cmapchannel", "").label("Channel"),
    ]);
    v.push(if color {
        s("componentscope_cmap", "111")
    } else {
        m("componentscope_cmap", 3, &[("r", "R"), ("g", "G"), ("b", "B"), ("a", "A")])
    });
    v.extend([
        v3s("cmapborder", [0.0; 3]).label("Border Color"),
        m("srccolorspace", 0, &[("auto", "Automatic"), ("linear", "Linear")]).label("Source Color Space"),
        s("cmapwrap", "repeat").label("Wrap"),
        s("uvattrib", "uv").label("UV Attribute"),
        v3s("t", [t, t, 0.0]).label("Translate"),
        f("r", 0.0).label("Rotate"),
        f("scale", 1.0).label("Uniform Scale"),
        s("scaletype", XFORM_MENU),
        v3s("s", [1.0, 1.0, 0.0]).label("Scale"),
        v3s("p", [0.5, 0.5, 0.0]).label("Pivot"),
        m("basis", 5, &NOISE_BASIS).label("Noise Type"),
        s("locationattrib", "P").label("Location Attribute"),
        f("elementsize", 1.0).label("Element Size"),
        s("elementsizetype", XFORM_MENU),
        v3s("elementscale", [1.0; 3]).label("Element Scale"),
        f("offset", 0.0).label("Offset"),
        s("offsettype", XFORM_MENU),
        v3s("offsetv", [0.0; 3]).label("Offset"),
        b("usenoiseexpression").label("Use Noise Expression"),
        s("noiseexpression", NOISE_EXPR).label("Noise Expression"),
        s("vex_cwdpath", ".").label("Evaluation Node Path"),
        b("animated").label("Animated"),
        f("pulseduration", 1.0).label("Pulse Duration"),
        m(
            "fractal",
            1,
            &[("none", "None"), ("fBm", "Standard (fBm)"), ("mfT", "Terrain (mfT)"), ("hmfT", "Hybrid (hmfT)")],
        )
        .label("Fractal Type"),
        f("oct", 3.0).label("Max Octaves"),
        f("lac", 2.01234).label("Lacunarity"),
        f("rough", 0.5).label("Roughness"),
        f("disp", 0.0).label("Displacement"),
        f("dispesize", 1.0).label("Displacement Element Size"),
        f("gflow", 0.0).label("Global Flow"),
        f("flowrot", 0.0).label("Flow Rotation"),
        b("accuml").label("Accumulate Local"),
        b("accumg").label("Accumulate Global"),
    ]);
    v
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
fn pattern_specs() -> Vec<ParamSpec> {
    let mut v = vec![
        m("randmethod", 0, &[("ppnum", "Element Number"), ("custom", "Custom Attribute")]).label("Seed Method"),
        s("randattrib", "id").label("Seed Attribute"),
        // template default 0 (Houdini's OnCreated script then picks a per-session seed, saved in the .hip)
        ParamSpec::new("randomseed", ParamValue::I(0)).label("Seed"),
    ];
    v.extend([
        v3s("startpos", [0.0; 3]).label("Start Position"),
        v3s("endpos", [5.0, 0.0, 0.0]).label("End Position"),
        m("bounddir", 0, &[("x", "X"), ("y", "Y"), ("z", "Z")]).label("Direction"),
        b("doboundmin").label("Bound Min"),
        f("boundmin", 0.0).label("Bound Min"),
        b("doboundmax").label("Bound Max"),
        f("boundmax", 1.0).label("Bound Max"),
        s("remapattrib", "mask").label("Remap Attribute"),
        // Houdini 2-tuple `remapattrib_rangemin` / `remapattrib_rangemax`
        v3s("remapattrib_range", [0.0, 1.0, 0.0]).label("Range"),
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
            // Houdini tuple `<name>Nc` = `<name>Ncr` / `cg` / `cb`
            v.push(v3s(&format!("{name}{n}c"), val));
        } else {
            v.push(f(&format!("{name}{n}value"), val[0]));
        }
        v.push(m(&format!("{name}{n}interp"), 1, &BASIS));
    }
    v
}

fn scalar_spec(is_int: bool) -> Vec<ParamSpec> {
    let (attrib, maxv) = if is_int { ("variant", 9.0) } else { ("pscale", 1.0) };
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
        ParamSpec::new("outlierseed", ParamValue::I(0)).label("Outlier Seed"),
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
        v.push(ip(&format!("folder3_{i}"), 0));
    }
    v.extend(pattern_specs());
    v.extend([
        b("enable_postprocess").label("Post-Process"),
        b("doclampmin").label("Clamp Min"),
        ParamSpec::new("clampminvalue", num(0.0, is_int)).label("Clamp Min"),
        b("doclampmax").label("Clamp Max"),
        ParamSpec::new("clampmaxvalue", num(1.0, is_int)).label("Clamp Max"),
        b("dodefault").label("Set Default"),
        ParamSpec::new("default", num(0.0, is_int)).label("Default"),
    ]);
    if is_int {
        v.extend(ignored_specs(&INT_FOLDERS, "sepparm", false, 0.0));
    } else {
        // unit conversion (any `unitsettings` but none -> Unsupported)
        v.extend([
            s("attribunit", "0").label("Attribute Unit"),
            s("parmunit", "1").label("Parameter Unit"),
            f("unitscale", 1.0).label("Unit Scale"),
            f("unitoffset", 0.0).label("Unit Offset"),
        ]);
        v.extend(ignored_specs(&FLOAT_FOLDERS, "sepparm2", false, 0.5));
    }
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
        v3s("initvalue", [1.0; 3]).label("Initial Value"),
        s("initcolorattrib", "Cd"),
        s("componentscope_initcolor", "111"),
        s("initfloatattrib", "pscale"),
        s("componentscope_initfloat", "1"),
        ParamSpec::new("adjustvalue", ParamValue::B(true)).label("Adjust Value"),
        m("operation", 1, &COLOR_OPS).label("Operation"),
        m("valuetype", 0, &VALUETYPES).label("Value"),
        m("rangemethod", 1, &COLOR_RANGES).label("Range"),
        v3s("singlevalue", [1.0; 3]).label("Value"),
        m("randomcolormodel", 0, &[("rgb", "RGB"), ("hsv", "HSV")]).label("Color Model"),
        b("dobasecolor").label("Base Color"),
        v3s("basecolor", [1.0; 3]).label("Base Color"),
    ]);
    // Houdini 2-tuples `<n>min` / `<n>max`
    for (n, lo, hi) in [
        ("randr", 0.0, 1.0),
        ("randg", 0.0, 1.0),
        ("randb", 0.0, 1.0),
        ("randhue", -180.0, 180.0),
        ("randsat", 0.0, 1.0),
        ("randval", 0.0, 1.0),
    ] {
        v.push(v3s(n, [lo, hi, 0.0]).label(n));
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
    v.push(s("colorpalette", "none").label("Color Palette"));
    v.push(ParamSpec::new("nvalues", ParamValue::I(1)).label("Number of Values"));
    // specific colours (`rangemethod` specific -> Unsupported): declared, never read
    for i in 0..MAX_VALUES {
        v.extend([
            ip(&format!("folder3_{i}_2"), 0),
            v3s(&format!("color{i}"), [1.0; 3]).label("Color"),
            m(&format!("colorscheme{i}"), 0, &SCHEMES).label("Color Scheme"),
            f(&format!("weight{i}"), 1.0).label("Weight"),
            f(&format!("twoanalogangle{i}"), 30.0),
            f(&format!("fouranalogangle{i}"), 30.0),
            f(&format!("triadicangle{i}"), 120.0),
            f(&format!("tetradicangle{i}"), 45.0),
            f(&format!("fiveshadesdist{i}"), 1.0),
            f(&format!("fivetintsdist{i}"), 1.0),
        ]);
    }
    v.extend([
        s("adjustmentattrib", "").label("Adjustment Attribute"),
        s("componentscope_adjustmentattrib", "111"),
        f("adjustmentattribgain", 1.0).label("Gain"),
        f("adjustmentattribbrightness", 0.0).label("Brightness"),
    ]);
    v.extend(pattern_specs());
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
        // UI preset menu that sets `gamma` (its callback); never read
        m("gammapresets", 0, &[("1", "1"), ("2.2", "2.2"), ("0.454545", "1/2.2")]).label("Gamma Presets"),
        b("deleteallccattrib"),
        b("enable_postprocess").label("Post-Process"),
        // UI action strip of the blend attribute (not wired in the HDA)
        s("deleteblendattrib", "SOP_delete"),
        b("doclampmin").label("Clamp Min"),
        v3s("clampminvalue", [0.0; 3]).label("Clamp Min"),
        b("doclampmax").label("Clamp Max"),
        v3s("clampmaxvalue", [1.0; 3]).label("Clamp Max"),
        b("docomplement").label("Complement"),
        b("dodefault").label("Set Default"),
        v3s("default", [1.0; 3]).label("Default"),
        b("dotypeinfo").label("Set Type Info"),
        m("typeinfo", 1, &[("none", "None"), ("color", "Color")]).label("Type Info"),
    ]);
    v.extend(ignored_specs(&COLOR_FOLDERS, "sepparm", true, 0.5));
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

/// Strict parm ranges of the HDAs (Houdini clamps the evaluated value; hython `minIsStrict` / `maxIsStrict`).
fn strict(n: &str) -> (f64, f64) {
    const INF: f64 = f64::INFINITY;
    match n {
        "blendweight" => (0.0, 1.0),
        "outlierpercent" => (0.0, 100.0),
        "stepsize"
        | "outlierrange"
        | "randomseed"
        | "outlierseed"
        | "adjustmentattribgain"
        | "adjustmentattribbrightness"
        | "gain"
        | "contrast"
        | "brightness"
        | "val"
        | "sat" => (0.0, INF),
        _ if n.starts_with("weight") => (0.0, INF),
        _ => (-INF, INF),
    }
}
/// A float tuple parm (V3; 2-tuples use slots 0 and 1), strict ranges per component (colour `singlevalue` /
/// `initvalue`: minimum 0).
fn c3(p: &ParamSet, n: &str) -> Result<[f32; 3], CookError> {
    let lo = if matches!(n, "singlevalue" | "initvalue") { 0.0 } else { f64::NEG_INFINITY };
    Ok(v3(p, n)?.map(|x| x.max(lo) as f32))
}
fn ff(p: &ParamSet, n: &str) -> Result<f32, CookError> {
    let (lo, hi) = strict(n);
    Ok(float(p, n)?.clamp(lo, hi) as f32)
}
/// Int or float parm as f64 (int SOP parms are ints, float SOP parms floats), strict ranges applied.
fn numf(p: &ParamSet, n: &str) -> Result<f64, CookError> {
    let (lo, hi) = strict(n);
    match p.get(n) {
        Some(ParamValue::I(v)) => Ok(f64::from(*v).clamp(lo, hi)),
        _ => Ok(float(p, n)?.clamp(lo, hi)),
    }
}
/// Seed parms (strict minimum 0).
fn seed(p: &ParamSet, n: &str) -> Result<i32, CookError> {
    Ok(int(p, n)?.max(0))
}
/// SYSrint: `floor(x + 0.5)`.
fn rint(x: f32) -> i32 {
    (x + 0.5).floor() as i32
}
/// VEX float `%` (floor modulo for a positive divisor).
fn fmod1(x: f32) -> f32 {
    x - x.floor()
}
/// SYSsafepow: a negative base takes the rounded exponent, `0^y` = 1 only for `y == 0`.
fn safepow(x: f32, y: f32) -> f32 {
    if x > 0.0 {
        x.powf(y)
    } else if x < 0.0 {
        x.powi(rint(y))
    } else if y == 0.0 {
        1.0
    } else {
        0.0
    }
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
            let val = if color { c3(p, &format!("{name}{k}c"))? } else { [ff(p, &format!("{name}{k}value"))?; 3] };
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
        Some(AttrData::FN { size: 3, data }) => Some(data.as_chunks::<3>().0.to_vec()),
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
            seed: seed(p, "randomseed")? as f32,
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
                let r = c3(p, "remapattrib_range")?;
                (pt.lo, pt.hi) = (r[0], r[1]);
                pt.cycle = menu(p, "remapattrib_rangemode")? == 1;
            }
            VT_LINE | VT_RADIAL => {
                pt.pos = positions(op, src, class, vt)?;
                let (s0, e0) = (c3(p, "startpos")?, c3(p, "endpos")?);
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
                if self.cycle { fmod1(efit(v, self.lo, self.hi, 0.0, 1.0)) } else { fit(v, self.lo, self.hi, 0.0, 1.0) }
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
    fn read(op: &str, p: &ParamSet, is_int: bool) -> Result<Range, CookError> {
        // the integer SOP's outlier range has a strict minimum of 1
        let omin = if is_int { 1.0 } else { 0.0 };
        let distrib = menu(p, "distribinrange")?;
        Ok(Range {
            distrib,
            ramp: if distrib == 1 { Ramp::read(op, p, "remapramp", false)? } else { Ramp { keys: Vec::new() } },
            pct: ff(p, "outlierpercent")?,
            orange: ff(p, "outlierrange")?.max(omin),
            omult: menu(p, "outlieroperation")? == 1,
            side: menu(p, "outlierside")?,
            oseed: seed(p, "outlierseed")?,
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
        if self.is_int { "attribadjustinteger" } else { "attribadjustfloat" }
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
        let range = Range::read(op, p, self.is_int)?;
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
        // outlier attribute: written by the range branch only (a pattern value type, not specific values)
        let outlier_name = if needs_pattern && spec_vals.is_none() && range.distrib == 2 && toggle(p, "outlierviz")? {
            let on = attr_name(p, "outlierattrib")?;
            if on == name {
                return Err(unsup(op, "outlierattrib", format!("{on} (same as the adjusted attribute)")));
            }
            Some(on)
        } else {
            None
        };
        let mut flags: Vec<i32> = match outlier_name.as_deref().map(|on| src.attr(class, on).map(|a| &a.data)) {
            None | Some(None) => vec![0; n],
            Some(Some(AttrData::I32(v))) => v.clone(),
            Some(Some(AttrData::IN { size: 1, data })) => data.clone(),
            Some(Some(_)) => {
                return Err(unsup(op, "outlierattrib", "existing attribute of another type / size"));
            }
        };
        flags.resize(n, 0);
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
                                    None => {
                                        let (v, o) = float_range(&range, d, pt.separation(i), lo, hi);
                                        if let Some(fl) = flags.get_mut(i) {
                                            *fl = i32::from(o);
                                        }
                                        v
                                    }
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
                let seed = seed(p, "randomseed")? as f32;
                for (i, x) in cur.iter_mut().enumerate() {
                    if !sel.get(i).copied().unwrap_or(false) {
                        continue;
                    }
                    let orig = *x;
                    let mut out = orig;
                    if adjust {
                        let value = match (vt, &pattern) {
                            (VT_ATTRIB, _) => rint(adj.get(i).copied().unwrap_or(0.0) * ascale + aoff),
                            (_, Some(pt)) => {
                                let l_id = match &pt.sep {
                                    Some(v) => v.get(i).copied().unwrap_or(0.0) as i32,
                                    None => i as i32,
                                };
                                let random = vt == VT_RAND;
                                let irand = if random { fhash(&[l_id as f32, seed]) as i32 } else { 0 };
                                let frand = if random {
                                    if spec_vals.is_some() { rand1(&[irand as f32]) } else { 0.0 }
                                } else {
                                    pt.distrib(i)
                                };
                                match &spec_vals {
                                    Some((vals, ws)) => {
                                        vals.get(sample_discrete(ws, frand)).copied().unwrap_or(0.0) as i32
                                    }
                                    None => {
                                        let r = IntRand { use_flt: !random, irand, frand, l_id };
                                        let (v, o) = int_range(&range, &r, min_val, width);
                                        if let Some(fl) = flags.get_mut(i) {
                                            *fl = i32::from(o);
                                        }
                                        v
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
                        out = orig.wrapping_add(rint(weight(i) * dlt));
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
        if let Some(on) = outlier_name {
            let type_info = src.attr(class, &on).map_or(TypeInfo::None, |a| a.type_info);
            out.add_attr(Attribute { name: on, class, data: AttrData::I32(flags), type_info });
        }
        Ok(Arc::new(out))
    }
}

/// aa_getRangeValue (float): outlier band, remap ramp, stepping, fit into [lo, hi]; also whether the element is an outlier.
fn float_range(r: &Range, d: f32, sep: f32, lo: f32, hi: f32) -> (f32, bool) {
    let (mut lo, mut hi, mut dd) = (lo, hi, d);
    let outlier = r.distrib == 2 && rand1(&[sep, r.oseed.wrapping_add(1) as f32]) < r.pct * 0.01;
    if outlier {
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
    let v = match r.step {
        Some(s) => stepping(dd, lo, hi, s as f32),
        None => fit01(dd, lo, hi),
    };
    (v, outlier)
}

struct IntRand {
    use_flt: bool,
    irand: i32,
    frand: f32,
    l_id: i32,
}
/// aa_getRangeValue (int): `irand mod width` (or a float distribution scaled), outliers, ramp, stepping; also whether
/// the element is an outlier.
fn int_range(r: &Range, x: &IntRand, min_val: i32, range_width: i32) -> (i32, bool) {
    let mut new_min = min_val;
    let mut width = range_width.wrapping_add(1);
    let mut outlier = false;
    if r.distrib == 2 {
        let [ox, oy] = vrand::<2>(&[r.oseed as f32, x.l_id as f32]);
        if ox < r.pct * 0.01 {
            outlier = true;
            width = if r.omult { rint(r.orange * range_width as f32) } else { rint(r.orange) };
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
        rint(r.ramp.eval(rv)[0] * span)
    } else if x.use_flt {
        rint(x.frand * span)
    } else {
        modw(x.irand)
    };
    if let Some(s) = r.step {
        let s = s as i32;
        val = val.checked_div(s).unwrap_or(0).wrapping_mul(s);
    }
    (new_min.wrapping_add(val), outlier)
}

// ---------------------------------------------------------------- color

struct Color {
    specs: Vec<ParamSpec>,
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
            c = c.map(|x| safepow(x, e));
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
            [lo[k], hi[k], _] = c3(p, n)?;
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
        let r = vrand::<3>(&[sep, seed]).map(|x| fmod1(x + GOLDEN));
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
                // the wrangle's (unwired) float scope is "100": red only
                Some(v.into_iter().map(|x| [x, 0.0, 0.0]).collect())
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
    fn parm_table() -> Value {
        serde_json::from_str(&std::fs::read_to_string(format!("{FX}parms.json")).expect("parms.json")).expect("json")
    }
    /// Houdini component parm name (`startposx`) -> (tuple name, component index), from the hython parm table.
    fn component(ty: &str, k: &str) -> Option<(String, usize)> {
        static TABLE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        let t = TABLE.get_or_init(parm_table);
        t["types"][ty].as_array()?.iter().find_map(|r| {
            let comps = r["comps"].as_array()?;
            let i = comps.iter().position(|c| c == k)?;
            (comps.len() > 1).then(|| (r["name"].as_str().unwrap_or_default().to_string(), i))
        })
    }
    /// Houdini parm values (menus as tokens, tuple components by their own names) -> hexe params by the spec's kind.
    fn params(n: &SopNode, parms: &serde_json::Map<String, Value>) -> ParamSet {
        let mut p = defaults(n);
        for (k, v) in parms {
            if let Some((tuple, i)) = component(n.type_name(), k) {
                let mut c = p.vec3(&tuple).unwrap_or_else(|| panic!("no hexe V3 {tuple} for {k}"));
                c[i] = v.as_f64().expect("num") as f32;
                p = p.with(&tuple, ParamValue::V3(c));
                continue;
            }
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
            if let Some(o) = c.get("outlier") {
                let on = o["name"].as_str().expect("outlier name");
                match (out.attr(cl, on).map(|a| &a.data), o["missing"] == true) {
                    (None, true) => {}
                    (Some(AttrData::I32(got)), false) => {
                        let want: Vec<i32> = o["values"]
                            .as_array()
                            .expect("values")
                            .iter()
                            .map(|x| x.as_i64().expect("int") as i32)
                            .collect();
                        if *got != want {
                            fails.push(format!("{name}: outlier {on} hexe {got:?} houdini {want:?}"));
                        }
                    }
                    (d, m) => fails.push(format!("{name}: outlier {on}: hexe {d:?}, houdini missing={m}")),
                }
            }
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
    /// bbox / remap-attribute values, every operation, blend, clamp, int cast, wrong type, defaults, all four classes,
    /// outlier attribute (`outlierviz`: range branch only, existing attribute kept outside the group).
    #[test]
    fn float_cases_match_houdini() {
        assert!(check_cases("attribadjustfloat") >= 41);
    }
    /// `i_*`: integer random (`random_fhash mod width`, min/max, min+range, mid+-range rounding), ramp, outliers, stepping,
    /// line, attribute (rint), specific values, blend, clamp, primitive class, wrong type, new attribute in a group.
    #[test]
    fn integer_cases_match_houdini() {
        assert!(check_cases("attribadjustinteger") >= 22);
    }
    /// `c_*`: constant / random RGB / HSV (with base colour) / ramp (random, line) / attribute values, all 11 operations,
    /// colour correction, clamp + complement, pre-process initial value, HSV round trip, new attribute default + type.
    #[test]
    fn color_cases_match_houdini() {
        assert!(check_cases("attribadjustcolor") >= 27);
    }

    #[test]
    fn vex_rand_matches_recorded_values() {
        // hython (float bit patterns): rand(0), rand(1), rand(0, 40), rand(0.1), rand(3); vector rand(3); random_fhash
        assert_eq!(rand1(&[0.0]).to_bits(), 1_054_187_368);
        assert_eq!(rand1(&[1.0]).to_bits(), 994_030_592);
        assert_eq!(rand1(&[0.0, 40.0]).to_bits(), 1_058_704_576);
        assert_eq!(rand1(&[0.1]).to_bits(), 1_063_334_920);
        let v = vrand::<3>(&[3.0]);
        assert_eq!(v[0].to_bits(), 1_048_991_420);
        assert!((v[1] - 0.736_549_6).abs() < 1e-7 && (v[2] - 0.116_141_44).abs() < 1e-7, "{v:?}");
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

    /// `parms.json` (hython 20.5 dump of every parm tuple, multiparm / ramp at 8 instances): each Houdini tuple is a
    /// hexe parm of the matching kind and default, and hexe declares nothing Houdini does not have, so importing a `.hip`
    /// never reports "parms without hexe twin" for these nodes.
    #[test]
    fn specs_declare_every_houdini_parm() {
        let t = parm_table();
        assert_eq!(t["max_instances"].as_u64(), Some(MAX_KEYS as u64));
        assert_eq!(MAX_KEYS, MAX_VALUES);
        let mut fails = Vec::new();
        for ty in ["attribadjustfloat", "attribadjustinteger", "attribadjustcolor"] {
            let n = node(ty);
            let rows = t["types"][ty].as_array().expect("rows");
            let houdini: Vec<&str> = rows.iter().map(|r| r["name"].as_str().expect("name")).collect();
            for spec in n.params() {
                if !houdini.contains(&spec.name.as_str()) {
                    fails.push(format!("{ty}: hexe-only parm {}", spec.name));
                }
            }
            for r in rows {
                let name = r["name"].as_str().expect("name");
                let Some(spec) = n.params().iter().find(|s| s.name == name) else {
                    fails.push(format!("{ty}: Houdini parm {name} not declared"));
                    continue;
                };
                let comps = r["comps"].as_array().map_or(0, Vec::len);
                let kind = r["kind"].as_str().expect("kind");
                let d = &r["default"];
                let num = |i: usize| d.get(i).and_then(Value::as_f64);
                // ramp key instances carry synthetic values (the ramp default is checked through its key count)
                let ramp_key = ["remapramp", "colorramp"]
                    .iter()
                    .any(|p| name.strip_prefix(p).is_some_and(|x| x.starts_with(|c: char| c.is_ascii_digit())));
                let ok = match (kind, &spec.default) {
                    ("Float", ParamValue::V3(v)) if (2..=3).contains(&comps) => {
                        ramp_key || (0..comps).all(|i| num(i).is_some_and(|x| (x as f32 - v[i]).abs() < 1e-6))
                    }
                    ("Float", ParamValue::F(v)) if comps == 1 => ramp_key || num(0).is_some_and(|x| x as f32 == *v),
                    ("Int", ParamValue::I(v)) if comps == 1 => num(0).is_some_and(|x| x as i32 == *v),
                    ("Toggle", ParamValue::B(v)) => d.as_bool() == Some(*v),
                    ("String", ParamValue::S(v)) => d.get(0).and_then(Value::as_str) == Some(v.as_str()),
                    ("Menu", ParamValue::Menu(v)) => {
                        let toks: Vec<&str> =
                            r["menu"].as_array().expect("menu").iter().map(|x| x.as_str().expect("tok")).collect();
                        let ours: Vec<&str> = spec.menu.iter().map(|x| x.0.as_str()).collect();
                        toks == ours && (ramp_key || d.as_u64() == Some(u64::from(*v)))
                    }
                    // string-valued menus (token toggles, script-generated items) are kept as their saved string
                    ("Menu", ParamValue::S(_)) => r["menu_type"] != "Normal" || r["menu_script"] == true,
                    ("Ramp" | "Folder", ParamValue::I(v)) => d.as_i64() == Some(i64::from(*v)),
                    ("FolderSet" | "Button", ParamValue::I(0)) => true,
                    ("Separator", ParamValue::S(v)) => v.is_empty(),
                    _ => false,
                };
                if !ok {
                    fails.push(format!("{ty}.{name}: Houdini {kind} x{comps} default {d} vs hexe {:?}", spec.default));
                }
            }
        }
        assert!(fails.is_empty(), "{} mismatches:\n{}", fails.len(), fails.join("\n"));
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

    /// Hostile values in the tuple / ignored / outlier parms: errors or `Unsupported`, never a panic or a silent guess.
    #[test]
    fn hostile_tuple_ignored_and_outlier_parms() {
        let (fl, it, co) = (float_new(), int_new(), color_new());
        let nan3 = ParamValue::V3([f32::NAN, 0.0, f32::INFINITY]);
        for n in [&fl, &it, &co] {
            let n = &**n;
            // non-finite tuples where they are read -> BadParam
            for (vt, parm) in [(VT_LINE, "startpos"), (VT_RADIAL, "endpos"), (VT_REMAP, "remapattrib_range")] {
                let r = run(n, grid(), &[("valuetype", ParamValue::Menu(vt)), (parm, nan3.clone())]);
                assert!(matches!(r, Err(CookError::BadParam(_)) | Err(CookError::Unsupported(_))), "{parm}: {r:?}");
            }
            // ignored parms with garbage values change nothing while their mode is off
            let junk = [
                ("oct", ParamValue::F(f32::NAN)),
                ("basis", ParamValue::Menu(999)),
                ("cmap", ParamValue::S("\0/../..".into())),
                ("t", nan3.clone()),
                ("elementscale", nan3.clone()),
                ("folder3", ParamValue::I(-7)),
                ("bbox_computerange", ParamValue::I(i32::MAX)),
                ("noiseexpression", ParamValue::S("@P = 1/0;".into())),
            ];
            let base = run(n, grid(), &[]).expect("default cook");
            assert_eq!(run(n, grid(), &junk).expect("ignored parms"), base);
            // ... and their modes stay Unsupported
            for vt in [VT_NOISE, VT_CMAP] {
                let r = run(n, grid(), &[("valuetype", ParamValue::Menu(vt)), ("basis", ParamValue::Menu(2))]);
                assert!(matches!(r, Err(CookError::Unsupported(_))), "{r:?}");
            }
        }
        // float units: any unit mode is Unsupported whatever the unit parms hold
        let units = [("unitsettings", ParamValue::Menu(4)), ("unitscale", ParamValue::F(f32::NAN))];
        assert!(matches!(run(&*fl, grid(), &units), Err(CookError::Unsupported(_))));
        // colour specific values / garbage schemes: Unsupported
        let spec = [
            ("valuetype", ParamValue::Menu(VT_RAND)),
            ("rangemethod", ParamValue::Menu(2)),
            ("colorscheme0", ParamValue::Menu(99)),
            ("color0", nan3.clone()),
        ];
        assert!(matches!(run(&*co, grid(), &spec), Err(CookError::Unsupported(_))));
        assert!(run(&*co, grid(), &[("randhue", nan3.clone())]).is_ok(), "unused while not random");
        let rnd = [("valuetype", ParamValue::Menu(VT_RAND)), ("rangemethod", ParamValue::Menu(0))];
        assert!(run(&*co, grid(), &[rnd[0].clone(), rnd[1].clone(), ("randr", nan3.clone())]).is_err());
        assert!(run(&*co, grid(), &[("colorramp1c", nan3.clone()), ("valuetype", ParamValue::Menu(VT_RAND))]).is_err());
        // outlier attribute: bad name, same as the target, existing of another type
        let out = |extra: (&'static str, ParamValue)| {
            [
                ("valuetype", ParamValue::Menu(VT_RAND)),
                ("distribinrange", ParamValue::Menu(2)),
                ("outlierpercent", ParamValue::F(1e9)),
                ("outlierviz", ParamValue::B(true)),
                extra,
            ]
        };
        for n in [&fl, &it] {
            let n = &**n;
            assert!(matches!(
                run(n, grid(), &out(("outlierattrib", ParamValue::S("a b".into())))),
                Err(CookError::BadParam(_))
            ));
            let same = n.params().iter().find(|s| s.name == "attrib").map(|s| s.default.clone()).expect("attrib");
            assert!(matches!(run(n, grid(), &out(("outlierattrib", same))), Err(CookError::Unsupported(_))));
            let mut d = grid();
            d.add_attr(Attribute {
                name: "outlier".into(),
                class: AttrClass::Point,
                data: AttrData::F32(vec![0.5; 9]),
                type_info: TypeInfo::None,
            });
            assert!(matches!(run(n, d, &out(("group", ParamValue::S(String::new())))), Err(CookError::Unsupported(_))));
            // percent clamped to 100: every element is an outlier
            let o = run(n, grid(), &out(("outlierseed", ParamValue::I(-5)))).expect("cooks");
            assert_eq!(o.attr(AttrClass::Point, "outlier").map(|a| &a.data), Some(&AttrData::I32(vec![1; 9])));
            // outlierviz off / not the outlier distribution: no attribute
            let o = run(n, grid(), &out(("distribinrange", ParamValue::Menu(0)))).expect("cooks");
            assert!(o.attr(AttrClass::Point, "outlier").is_none());
        }
    }
}
