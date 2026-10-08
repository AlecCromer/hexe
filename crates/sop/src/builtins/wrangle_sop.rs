//! Attribute Wrangle SOP (`wrangle`): runs a VEX-subset snippet (crate `hexe-vex`) once per element of `class`.
//! Clean-room semantics (UNVERIFIED vs Houdini: hython blocked; noise/rand streams are OUR hash):
//! - params: `snippet` (VEX text), `class` = point|prim|vertex|detail (default point). Input 0 is copied, then edited.
//! - `@name` writes create the attribute on the element class (type from prefix / builtin: P N Cd v = vector); writes to an
//!   existing attribute are converted to its type (int↔float only; anything else → `Failed`).
//! - reads: `@P` on prim = centroid of its points, on vertex = owning point's P; `@ptnum/@primnum/@vtxnum` = element id for
//!   the class (else -1); `@numpt/@numprim` = input counts; `@Frame` = time*fps+1, `@Time` = time.
//! - writes to `@P` on prim/vertex/detail = `Unsupported`. Writes to read-only builtins = `Failed`.
//! - `addpoint` appends immediately (returned ptnum = current count) → new points always sit after the originals;
//!   `removepoint` is deferred to the end of the run and also removes every primitive using the point (DEVIATION guess).
//! - `point(0, name, n)` reads the INPUT snapshot for n < input count (so results do not depend on iteration order),
//!   the output for added points. `setpointattrib` writes the output.
//! - parse error → `Failed("wrangle: line:col: msg")`; runtime type mismatch → `Failed`; step budget {MAX_STEPS} per cook.
//! - NOT modelled: other geometry handles than 0, vertex/prim/detail variants of point funcs, arrays, user functions.
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, PrimKind, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use hexe_vex::{ErrorKind, Host, HostError, Program, Ty, Value, parse, run};
use std::sync::Arc;

/// Interpreter step budget per cook (all elements together).
const MAX_STEPS: u64 = 50_000_000;
/// Cap on points a snippet may add (AGENTS.md §4).
const MAX_ADDED_POINTS: usize = 1_000_000;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("snippet", ParamValue::S(String::new())).label("VEXpression").language("vex"),
        ParamSpec::new("class", ParamValue::S("point".into()))
            .label("Run Over")
            .choices(&["point", "prim", "vertex", "detail"]),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(Wrangle { specs: spec() })
}

struct Wrangle {
    specs: Vec<ParamSpec>,
}

fn failed(m: impl std::fmt::Display) -> CookError {
    CookError::Failed(format!("wrangle: {m}"))
}

fn class_of(s: &str) -> Result<AttrClass, CookError> {
    match s.trim() {
        "point" => Ok(AttrClass::Point),
        "prim" | "primitive" => Ok(AttrClass::Prim),
        "vertex" => Ok(AttrClass::Vertex),
        "detail" | "global" => Ok(AttrClass::Detail),
        other => Err(CookError::BadParam(format!("wrangle: `class` `{other}`: expected point|prim|vertex|detail"))),
    }
}

// ------------------------------------------------------------ attribute <-> Value ----

fn data_ty(d: &AttrData) -> Option<Ty> {
    Some(match d {
        AttrData::F32(_) => Ty::Float,
        AttrData::I32(_) => Ty::Int,
        AttrData::V2(_) => Ty::Vec2,
        AttrData::V3(_) => Ty::Vec3,
        AttrData::V4(_) => Ty::Vec4,
        AttrData::Str { .. } => Ty::Str,
        AttrData::FN { .. } | AttrData::IN { .. } => return None,
    })
}

fn new_data(t: Ty) -> AttrData {
    match t {
        Ty::Float => AttrData::F32(Vec::new()),
        Ty::Int => AttrData::I32(Vec::new()),
        Ty::Vec2 => AttrData::V2(Vec::new()),
        Ty::Vec3 => AttrData::V3(Vec::new()),
        Ty::Vec4 => AttrData::V4(Vec::new()),
        Ty::Str => AttrData::Str { idx: Vec::new(), table: Vec::new() },
    }
}

fn type_info(name: &str) -> TypeInfo {
    match name {
        "P" => TypeInfo::Point,
        "N" => TypeInfo::Normal,
        "Cd" => TypeInfo::Color,
        "v" => TypeInfo::Vector,
        _ => TypeInfo::None,
    }
}

fn f32s<const N: usize>(c: &[f64; N]) -> [f32; N] {
    c.map(|x| x as f32)
}
fn f64s<const N: usize>(c: &[f32; N]) -> [f64; N] {
    c.map(f64::from)
}

fn read_elem(d: &AttrData, i: usize) -> Result<Value, HostError> {
    let oob = || HostError::Failed(format!("element {i} out of range"));
    Ok(match d {
        AttrData::F32(v) => Value::Float(f64::from(*v.get(i).ok_or_else(oob)?)),
        AttrData::I32(v) => Value::Int(i64::from(*v.get(i).ok_or_else(oob)?)),
        AttrData::V2(v) => Value::Vec2(f64s(v.get(i).ok_or_else(oob)?)),
        AttrData::V3(v) => Value::Vec3(f64s(v.get(i).ok_or_else(oob)?)),
        AttrData::V4(v) => Value::Vec4(f64s(v.get(i).ok_or_else(oob)?)),
        AttrData::Str { idx, table } => {
            let k = *idx.get(i).ok_or_else(oob)?;
            Value::Str(usize::try_from(k).ok().and_then(|k| table.get(k)).cloned().unwrap_or_default())
        }
        AttrData::FN { .. } | AttrData::IN { .. } => {
            return Err(HostError::Unsupported("tuple attributes with more than 4 components".into()));
        }
    })
}

fn write_elem(d: &mut AttrData, i: usize, v: &Value) -> Result<(), HostError> {
    fn put<T>(vec: &mut [T], i: usize, x: T) -> Result<(), HostError> {
        match vec.get_mut(i) {
            Some(s) => {
                *s = x;
                Ok(())
            }
            None => Err(HostError::Failed(format!("element {i} out of range"))),
        }
    }
    match (d, v) {
        (AttrData::F32(a), Value::Float(x)) => put(a, i, *x as f32),
        (AttrData::I32(a), Value::Int(x)) => {
            put(a, i, i32::try_from(*x).unwrap_or(if *x < 0 { i32::MIN } else { i32::MAX }))
        }
        (AttrData::V2(a), Value::Vec2(x)) => put(a, i, f32s(x)),
        (AttrData::V3(a), Value::Vec3(x)) => put(a, i, f32s(x)),
        (AttrData::V4(a), Value::Vec4(x)) => put(a, i, f32s(x)),
        (AttrData::Str { idx, table }, Value::Str(s)) => {
            let k = match table.iter().position(|t| t == s) {
                Some(k) => k,
                None => {
                    table.push(s.clone());
                    table.len() - 1
                }
            };
            put(idx, i, i32::try_from(k).unwrap_or(-1))
        }
        (AttrData::FN { .. } | AttrData::IN { .. }, _) => {
            Err(HostError::Unsupported("tuple attributes with more than 4 components".into()))
        }
        (d, v) => Err(HostError::Failed(format!(
            "cannot store {} into a {} attribute",
            v.ty().name(),
            data_ty(d).map_or("tuple", Ty::name)
        ))),
    }
}

/// Write `v` at element `i` of class attribute `name`, creating it (type of `v`, or `hint`) when absent.
fn write_attr(
    d: &mut Detail,
    class: AttrClass,
    name: &str,
    hint: Option<Ty>,
    i: usize,
    v: Value,
) -> Result<(), HostError> {
    if name.is_empty() {
        return Err(HostError::Failed("empty attribute name".into()));
    }
    if d.attr(class, name).is_none() {
        let t = hint.unwrap_or_else(|| if matches!(v, Value::Int(_)) { Ty::Float } else { v.ty() });
        d.add_attr(Attribute { name: name.into(), class, data: new_data(t), type_info: type_info(name) });
    }
    let a = d.attr_mut(class, name).ok_or_else(|| HostError::Failed(format!("attribute `{name}` vanished")))?;
    let Some(t) = data_ty(&a.data) else {
        return Err(HostError::Unsupported(format!("attribute `{name}` is a >4 component tuple")));
    };
    let v = v
        .coerce(t)
        .ok_or_else(|| HostError::Failed(format!("attribute `{name}` is {} but got {}", t.name(), v.ty().name())))?;
    write_elem(&mut a.data, i, &v)
}

// ------------------------------------------------------------------------ host ----

struct Geo<'a> {
    src: &'a Detail,
    out: Detail,
    class: AttrClass,
    idx: usize,
    n_pt: i64,
    n_prim: i64,
    vtx_prim: Vec<u32>,
    gone: Vec<bool>,
    time: f64,
    fps: f64,
    added: usize,
}

fn oor(what: &str, n: i64) -> HostError {
    HostError::Failed(format!("{what} {n} out of range"))
}

impl Geo<'_> {
    fn point_of_elem(&self) -> Option<usize> {
        match self.class {
            AttrClass::Point => Some(self.idx),
            AttrClass::Vertex => self.out.vertex_point.get(self.idx).map(|p| *p as usize),
            _ => None,
        }
    }
    fn prim_of_elem(&self) -> Option<usize> {
        match self.class {
            AttrClass::Prim => Some(self.idx),
            AttrClass::Vertex => self.vtx_prim.get(self.idx).map(|p| *p as usize),
            _ => None,
        }
    }
    fn point_p(&self, p: usize) -> [f64; 3] {
        self.out.p().get(p).map_or([0.0; 3], f64s)
    }
    fn centroid(&self) -> [f64; 3] {
        let Some(prim) = self.out.prims.get(self.idx) else { return [0.0; 3] };
        let mut sum = [0.0; 3];
        let mut n = 0.0;
        for v in prim.verts.clone() {
            if let Some(p) = self.out.vertex_point.get(v as usize) {
                let q = self.point_p(*p as usize);
                for k in 0..3 {
                    sum[k] += q[k];
                }
                n += 1.0;
            }
        }
        if n > 0.0 { sum.map(|s| s / n) } else { [0.0; 3] }
    }
    fn point_index(&self, n: i64, count: usize) -> Result<usize, HostError> {
        usize::try_from(n).ok().filter(|k| *k < count).ok_or_else(|| oor("point", n))
    }
}

impl Host for Geo<'_> {
    fn attr(&mut self, name: &str, _hint: Option<Ty>) -> Result<Option<Value>, HostError> {
        let id = |o: Option<usize>| Value::Int(o.map_or(-1, |k| k as i64));
        match name {
            "ptnum" => return Ok(Some(id(self.point_of_elem()))),
            "primnum" => return Ok(Some(id(self.prim_of_elem()))),
            "vtxnum" => return Ok(Some(id((self.class == AttrClass::Vertex).then_some(self.idx)))),
            "numpt" => return Ok(Some(Value::Int(self.n_pt))),
            "numprim" => return Ok(Some(Value::Int(self.n_prim))),
            "Frame" => return Ok(Some(Value::Float((self.time * self.fps).round() + 1.0))),
            "Time" => return Ok(Some(Value::Float(self.time))),
            "P" => match self.class {
                AttrClass::Prim => return Ok(Some(Value::Vec3(self.centroid()))),
                AttrClass::Vertex => {
                    return Ok(Some(Value::Vec3(self.point_of_elem().map_or([0.0; 3], |p| self.point_p(p)))));
                }
                AttrClass::Detail => return Ok(None),
                AttrClass::Point => {}
            },
            _ => {}
        }
        match self.out.attr(self.class, name) {
            Some(a) => read_elem(&a.data, self.idx).map(Some),
            None => Ok(None),
        }
    }

    fn set_attr(&mut self, name: &str, hint: Option<Ty>, v: Value) -> Result<(), HostError> {
        if name == "P" && self.class != AttrClass::Point {
            return Err(HostError::Unsupported(format!("writing @P on class {:?} (use a point wrangle)", self.class)));
        }
        write_attr(&mut self.out, self.class, name, hint, self.idx, v)
    }

    fn add_point(&mut self, p: [f64; 3]) -> Result<i64, HostError> {
        if self.added >= MAX_ADDED_POINTS {
            return Err(HostError::Failed(format!("addpoint: more than {MAX_ADDED_POINTS} points")));
        }
        self.added += 1;
        let r = self.out.add_points(1);
        let k = r.start as usize;
        write_attr(&mut self.out, AttrClass::Point, "P", Some(Ty::Vec3), k, Value::Vec3(p))?;
        Ok(i64::from(r.start))
    }

    fn remove_point(&mut self, n: i64) -> Result<(), HostError> {
        let k = self.point_index(n, self.out.point_count as usize)?;
        if self.gone.len() <= k {
            self.gone.resize(self.out.point_count as usize, false);
        }
        if let Some(s) = self.gone.get_mut(k) {
            *s = true;
        }
        Ok(())
    }

    fn add_prim(&mut self, kind: &str, pts: &[i64]) -> Result<i64, HostError> {
        let (closed, min) = match kind {
            "poly" => (true, 3),
            "polyline" => (false, 2),
            other => return Err(HostError::Failed(format!("addprim: unknown primitive `{other}` (poly|polyline)"))),
        };
        if pts.len() < min {
            return Err(HostError::Failed(format!("addprim: `{kind}` needs at least {min} points")));
        }
        let ids: Vec<u32> = pts
            .iter()
            .map(|n| self.point_index(*n, self.out.point_count as usize).map(|k| k as u32))
            .collect::<Result<_, _>>()?;
        let id = self.out.add_poly(&ids, closed);
        if !closed && let Some(p) = self.out.prims.get_mut(id as usize) {
            p.kind = PrimKind::Polyline;
        }
        Ok(i64::from(id))
    }

    fn set_point_attrib(&mut self, name: &str, n: i64, v: Value) -> Result<(), HostError> {
        let k = self.point_index(n, self.out.point_count as usize)?;
        let hint = (name == "P").then_some(Ty::Vec3);
        write_attr(&mut self.out, AttrClass::Point, name, hint, k, v)
    }

    fn point(&mut self, name: &str, n: i64) -> Result<Value, HostError> {
        let src_n = self.src.point_count as usize;
        let k = self.point_index(n, self.out.point_count as usize)?;
        let d = if k < src_n { self.src } else { &self.out };
        match d.attr(AttrClass::Point, name) {
            Some(a) => read_elem(&a.data, k),
            None => Err(HostError::Failed(format!("point: no point attribute `{name}`"))),
        }
    }

    fn npoints(&mut self) -> i64 {
        i64::from(self.out.point_count)
    }
}

// ------------------------------------------------------------------ removal ----

fn keep_only<T>(v: &mut Vec<T>, keep: &[bool]) {
    let mut i = 0;
    v.retain(|_| {
        let k = keep.get(i).copied().unwrap_or(true);
        i += 1;
        k
    });
}

fn keep_data(d: &mut AttrData, keep: &[bool]) {
    fn tuples<T: Copy>(v: &mut Vec<T>, size: usize, keep: &[bool]) {
        let mut out = Vec::with_capacity(v.len());
        for (i, ch) in v.chunks(size.max(1)).enumerate() {
            if keep.get(i).copied().unwrap_or(true) {
                out.extend_from_slice(ch);
            }
        }
        *v = out;
    }
    match d {
        AttrData::F32(v) => keep_only(v, keep),
        AttrData::I32(v) => keep_only(v, keep),
        AttrData::V2(v) => keep_only(v, keep),
        AttrData::V3(v) => keep_only(v, keep),
        AttrData::V4(v) => keep_only(v, keep),
        AttrData::Str { idx, .. } => keep_only(idx, keep),
        AttrData::FN { size, data } => tuples(data, *size as usize, keep),
        AttrData::IN { size, data } => tuples(data, *size as usize, keep),
    }
}

/// Remove points flagged in `gone` plus every primitive that uses one; renumber everything else.
fn remove_points(d: &mut Detail, gone: &[bool]) {
    let is_gone = |p: u32| gone.get(p as usize).copied().unwrap_or(false);
    let keep_pt: Vec<bool> = (0..d.point_count).map(|p| !is_gone(p)).collect();
    let mut new_id = vec![0u32; keep_pt.len()];
    let mut next = 0u32;
    for (i, k) in keep_pt.iter().enumerate() {
        if let Some(slot) = new_id.get_mut(i) {
            *slot = next;
        }
        next += u32::from(*k);
    }
    let vp = std::mem::take(&mut d.vertex_point);
    let keep_prim: Vec<bool> =
        d.prims.iter().map(|p| !p.verts.clone().any(|v| vp.get(v as usize).is_some_and(|q| is_gone(*q)))).collect();
    let mut keep_vtx: Vec<bool> = vp.iter().map(|q| !is_gone(*q)).collect();
    for (p, k) in d.prims.iter().zip(&keep_prim) {
        if !*k {
            for v in p.verts.clone() {
                if let Some(s) = keep_vtx.get_mut(v as usize) {
                    *s = false;
                }
            }
        }
    }
    let mut vnew = vec![0u32; vp.len()];
    let mut nv = 0u32;
    for (i, k) in keep_vtx.iter().enumerate() {
        if let Some(s) = vnew.get_mut(i) {
            *s = nv;
        }
        nv += u32::from(*k);
    }
    d.vertex_point = vp
        .iter()
        .zip(&keep_vtx)
        .filter(|(_, k)| **k)
        .map(|(q, _)| new_id.get(*q as usize).copied().unwrap_or(0))
        .collect();
    for (p, k) in d.prims.iter_mut().zip(&keep_prim) {
        if *k && !p.verts.is_empty() {
            let (a, b) = (p.verts.start as usize, p.verts.end as usize);
            p.verts = vnew.get(a).copied().unwrap_or(0)..vnew.get(b.saturating_sub(1)).map_or(0, |e| e + 1);
        }
    }
    keep_only(&mut d.prims, &keep_prim);
    for a in &mut d.attrs {
        match a.class {
            AttrClass::Point => keep_data(&mut a.data, &keep_pt),
            AttrClass::Vertex => keep_data(&mut a.data, &keep_vtx),
            AttrClass::Prim => keep_data(&mut a.data, &keep_prim),
            AttrClass::Detail => {}
        }
    }
    for g in &mut d.groups {
        match g.class {
            AttrClass::Point => keep_only(&mut g.mask, &keep_pt),
            AttrClass::Vertex => keep_only(&mut g.mask, &keep_vtx),
            AttrClass::Prim => keep_only(&mut g.mask, &keep_prim),
            AttrClass::Detail => {}
        }
    }
    d.point_count = next;
}

// --------------------------------------------------------------------- driver ----

struct Config {
    prog: Program,
    class: AttrClass,
}

impl Config {
    fn read(p: &hexe_graph::ParamSet) -> Result<Config, CookError> {
        let snippet =
            p.string("snippet").ok_or_else(|| CookError::BadParam("wrangle: missing or mistyped `snippet`".into()))?;
        let class = class_of(
            p.string("class").ok_or_else(|| CookError::BadParam("wrangle: missing or mistyped `class`".into()))?,
        )?;
        let prog = parse(snippet).map_err(failed)?;
        Ok(Config { prog, class })
    }
}

fn execute(input: &Detail, cfg: &Config, time: f32, fps: f32) -> Result<Detail, CookError> {
    let mut vtx_prim = Vec::new();
    if cfg.class == AttrClass::Vertex {
        vtx_prim = vec![0u32; input.vertex_point.len()];
        for (pi, p) in input.prims.iter().enumerate() {
            for v in p.verts.clone() {
                if let Some(s) = vtx_prim.get_mut(v as usize) {
                    *s = pi as u32;
                }
            }
        }
    }
    let mut g = Geo {
        src: input,
        out: input.clone(),
        class: cfg.class,
        idx: 0,
        n_pt: i64::from(input.point_count),
        n_prim: i64::from(input.prim_count()),
        vtx_prim,
        gone: Vec::new(),
        time: f64::from(time),
        fps: f64::from(fps),
        added: 0,
    };
    let mut steps = MAX_STEPS;
    for idx in 0..input.class_len(cfg.class) {
        g.idx = idx;
        if let Err(e) = run(&cfg.prog, &mut g, &mut steps) {
            let m = format!("element {idx}: {e}");
            return Err(match e.kind {
                ErrorKind::Unsupported => CookError::Unsupported(format!("wrangle: {m}")),
                _ => failed(m),
            });
        }
    }
    if g.gone.iter().any(|x| *x) {
        g.gone.resize(g.out.point_count as usize, false);
        let gone = std::mem::take(&mut g.gone);
        remove_points(&mut g.out, &gone);
    }
    Ok(g.out)
}

impl Node<Detail> for Wrangle {
    fn type_name(&self) -> &'static str {
        "wrangle"
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
        execute(inputs.first().map_or(&empty, |a| &**a), &cfg, ctx.time, ctx.fps).map(Arc::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_graph::ParamSet;

    fn params(snippet: &str, class: &str) -> ParamSet {
        ParamSet::new().with("snippet", ParamValue::S(snippet.into())).with("class", ParamValue::S(class.into()))
    }
    fn cook_class(d: &Detail, snippet: &str, class: &str) -> Result<Detail, CookError> {
        let p = params(snippet, class);
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        new().cook(&ctx, &[Arc::new(d.clone())]).map(|a| (*a).clone())
    }
    fn cook(d: &Detail, snippet: &str) -> Detail {
        cook_class(d, snippet, "point").unwrap_or_else(|e| panic!("{snippet}: {e}"))
    }
    /// 3x3 points, x,z in {-1,0,1}, y=0, x-major; 4 quad cells.
    fn grid3() -> Detail {
        let mut d = Detail::default();
        d.add_points(9);
        let mut p = Vec::new();
        for x in -1..=1 {
            for z in -1..=1 {
                p.push([x as f32, 0.0, z as f32]);
            }
        }
        *d.p_mut() = p;
        for x in 0..2u32 {
            for z in 0..2u32 {
                let a = x * 3 + z;
                d.add_poly(&[a, a + 3, a + 4, a + 1], true);
            }
        }
        d
    }
    fn v3(d: &Detail, name: &str) -> Vec<[f32; 3]> {
        match d.attr(AttrClass::Point, name).map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            _ => panic!("no V3 point attr {name}"),
        }
    }
    fn f1(d: &Detail, c: AttrClass, name: &str) -> Vec<f32> {
        match d.attr(c, name).map(|a| &a.data) {
            Some(AttrData::F32(v)) => v.clone(),
            _ => panic!("no F32 attr {name}"),
        }
    }
    fn i1(d: &Detail, c: AttrClass, name: &str) -> Vec<i32> {
        match d.attr(c, name).map(|a| &a.data) {
            Some(AttrData::I32(v)) => v.clone(),
            _ => panic!("no I32 attr {name}"),
        }
    }

    #[test]
    fn oracle_p_y_plus_one() {
        let (g, o) = (grid3(), cook(&grid3(), "@P.y += 1;"));
        for (a, b) in g.p().iter().zip(o.p()) {
            assert_eq!((b[0], b[1], b[2]), (a[0], a[1] + 1.0, a[2]));
        }
        assert_eq!((o.point_count, o.prim_count()), (9, 4));
    }
    #[test]
    fn oracle_cd_vector_created_on_all_points() {
        let o = cook(&grid3(), "@Cd = {1,0,0};");
        assert_eq!(v3(&o, "Cd"), vec![[1.0, 0.0, 0.0]; 9]);
        assert_eq!(o.attr(AttrClass::Point, "Cd").map(|a| a.type_info), Some(TypeInfo::Color));
    }
    #[test]
    fn oracle_float_attr_exact_per_point() {
        let (g, o) = (grid3(), cook(&grid3(), "f@h = @P.x * 2 + 1;"));
        let want: Vec<f32> = g.p().iter().map(|p| p[0] * 2.0 + 1.0).collect();
        assert_eq!(f1(&o, AttrClass::Point, "h"), want);
    }
    #[test]
    fn oracle_ptnum_ids() {
        let o = cook(&grid3(), "i@id = @ptnum; i@n = @numpt;");
        assert_eq!(i1(&o, AttrClass::Point, "id"), (0..9).collect::<Vec<i32>>());
        assert_eq!(i1(&o, AttrClass::Point, "n"), vec![9; 9]);
    }
    #[test]
    fn oracle_p_doubles() {
        let (g, o) = (grid3(), cook(&grid3(), "@P = @P * 2;"));
        let want: Vec<[f32; 3]> = g.p().iter().map(|p| p.map(|c| c * 2.0)).collect();
        assert_eq!(o.p(), want.as_slice());
    }
    #[test]
    fn oracle_removepoint_keeps_six_and_drops_touching_prims() {
        let o = cook(&grid3(), "if(@P.x > 0) removepoint(0,@ptnum);");
        assert_eq!(o.point_count, 6);
        assert!(o.p().iter().all(|p| p[0] <= 0.0));
        // both cells touching the x=1 column die; the two x∈[-1,0] cells survive, renumbered and valid
        assert_eq!(o.prim_count(), 2);
        assert!(o.vertex_point.iter().all(|p| *p < 6));
        assert_eq!(o.vertex_point.len(), 8);
        assert_eq!(o.prims[1].verts, 4..8);
    }
    #[test]
    fn oracle_addpoint_detail_class_and_ptnum_guard() {
        let o = cook_class(&grid3(), "int n = addpoint(0, {5,5,5}); i@n = n;", "detail").expect("cooks");
        assert_eq!(o.point_count, 10);
        assert_eq!(o.p()[9], [5.0, 5.0, 5.0]);
        assert_eq!(i1(&o, AttrClass::Detail, "n"), vec![9]);
        // point class, guarded: also exactly one new point, attrs of existing points untouched, new point defaulted
        let o = cook(&grid3(), "i@k = 1; if(@ptnum == 0) addpoint(0, {5,5,5});");
        assert_eq!(o.point_count, 10);
        assert_eq!(i1(&o, AttrClass::Point, "k"), [vec![1; 9], vec![0]].concat());
    }
    #[test]
    fn oracle_sphere_normals_unit_length() {
        let json = r#"{"nodes":[{"name":"s","type":"sphere","params":{"type":2}},
            {"name":"w","type":"wrangle","params":{"snippet":"@N = normalize(@P - {0,0,0});"}}],
            "edges":[["s",0,"w",0]],"display":"w"}"#;
        let (mut g, id) = load_scene(json).map_err(|e| e.to_string()).expect("loads");
        let d = g.cook(id, 0.0).map_err(|e| e.to_string()).expect("cooks");
        assert!(d.point_count > 50);
        let n = v3(&d, "N");
        for (p, n) in d.p().iter().zip(&n) {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((l - 1.0).abs() < 1e-5, "|N|={l}");
            assert!((n[0] - p[0]).abs() < 1e-5, "unit sphere: N == P");
        }
        assert_eq!(d.attr(AttrClass::Point, "N").map(|a| a.type_info), Some(TypeInfo::Normal));
    }
    #[test]
    fn oracle_for_loop_adds_three() {
        let (g, o) = (grid3(), cook(&grid3(), "for(int i=0;i<3;i++) @P.x += 1;"));
        for (a, b) in g.p().iter().zip(o.p()) {
            assert_eq!(b[0], a[0] + 3.0);
        }
    }
    #[test]
    fn oracle_rand_and_fit01() {
        let s = "f@r = rand(@ptnum); f@r2 = rand(@ptnum); f@c = fit01(0.5, 10, 20);";
        let (a, b) = (cook(&grid3(), s), cook(&grid3(), s));
        let r = f1(&a, AttrClass::Point, "r");
        assert_eq!(r, f1(&b, AttrClass::Point, "r"));
        assert_eq!(r, f1(&a, AttrClass::Point, "r2"));
        assert!(r.iter().all(|x| (0.0..1.0).contains(x)));
        assert!(r.windows(2).any(|w| w[0] != w[1]), "streams differ per point");
        assert_eq!(f1(&a, AttrClass::Point, "c"), vec![15.0; 9]);
    }
    #[test]
    fn oracle_detail_class_runs_once() {
        let o = cook_class(&grid3(), "i@runs = @runs + 1; f@t = @Time;", "detail").expect("cooks");
        // @runs unprefixed = float, prefix i@ on write converts: first run only → 1
        assert_eq!(i1(&o, AttrClass::Detail, "runs"), vec![1]);
        assert_eq!(f1(&o, AttrClass::Detail, "t"), vec![0.0]);
        assert_eq!(o.point_count, 9);
    }
    #[test]
    fn oracle_parse_error_is_failed_with_position() {
        let e = cook_class(&grid3(), "@P.y +=;", "point").expect_err("must fail");
        let CookError::Failed(m) = &e else { panic!("not Failed: {e:?}") };
        assert!(m.contains("1:8"), "{m}");
        let e = cook_class(&grid3(), "float a = 1;\n  a = (2;", "point").expect_err("must fail");
        assert!(matches!(&e, CookError::Failed(m) if m.contains("2:9")), "{e:?}");
    }
    #[test]
    fn oracle_div_by_zero_is_inf() {
        let o = cook(&grid3(), "f@a = 1/0; f@b = -1.0/0;");
        assert_eq!(f1(&o, AttrClass::Point, "a"), vec![f32::INFINITY; 9]);
        assert_eq!(f1(&o, AttrClass::Point, "b"), vec![f32::NEG_INFINITY; 9]);
    }
    #[test]
    fn oracle_ten_thousand_iterations() {
        let o = cook(&grid3(), "int s = 0; for(int i=0;i<10000;i++) s += 1; i@s = s;");
        assert_eq!(i1(&o, AttrClass::Point, "s"), vec![10_000; 9]);
    }
    #[test]
    fn runaway_loop_is_failed_not_a_hang() {
        let e = cook_class(&grid3(), "while(1) {}", "point").expect_err("budget");
        assert!(matches!(&e, CookError::Failed(m) if m.contains("budget")), "{e:?}");
    }
    #[test]
    fn runtime_type_mismatch_is_failed() {
        for s in ["v@a = 1.5;", "@P = 3;", "f@x = {1,2,3};", "@ptnum = 3;", "s@n = 4;", "@Cd = 'red';"] {
            let e = cook_class(&grid3(), s, "point").expect_err(s);
            assert!(matches!(&e, CookError::Failed(_)), "{s}: {e:?}");
        }
        // existing attribute keeps its type: float write into an int attribute converts, vector into it fails
        let o = cook(&cook(&grid3(), "i@k = 2;"), "f@k = 3.7;");
        assert_eq!(i1(&o, AttrClass::Point, "k"), vec![3; 9]);
        assert!(cook_class(&cook(&grid3(), "i@k = 2;"), "v@k = {1,2,3};", "point").is_err());
    }
    #[test]
    fn prim_class_centroid_read_and_p_write_unsupported() {
        let o = cook_class(&grid3(), "@Cd = @P; i@pn = @primnum;", "prim").expect("cooks");
        let centroids = match o.attr(AttrClass::Prim, "Cd").map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            _ => panic!("no prim Cd"),
        };
        assert_eq!(centroids, vec![[-0.5, 0.0, -0.5], [-0.5, 0.0, 0.5], [0.5, 0.0, -0.5], [0.5, 0.0, 0.5]]);
        assert_eq!(i1(&o, AttrClass::Prim, "pn"), vec![0, 1, 2, 3]);
        let e = cook_class(&grid3(), "@P.y = 1;", "prim").expect_err("unsupported");
        assert!(matches!(e, CookError::Unsupported(_)), "{e:?}");
        let e = cook_class(&grid3(), "@P = {0,0,0};", "vertex").expect_err("unsupported");
        assert!(matches!(e, CookError::Unsupported(_)), "{e:?}");
    }
    #[test]
    fn vertex_class_sees_owning_point_and_prim() {
        let o =
            cook_class(&grid3(), "i@pt = @ptnum; i@pr = @primnum; i@vt = @vtxnum; v@q = @P;", "vertex").expect("cooks");
        assert_eq!(i1(&o, AttrClass::Vertex, "pt"), o.vertex_point.iter().map(|p| *p as i32).collect::<Vec<_>>());
        assert_eq!(i1(&o, AttrClass::Vertex, "pr")[..8], [0, 0, 0, 0, 1, 1, 1, 1]);
        assert_eq!(i1(&o, AttrClass::Vertex, "vt"), (0..16).collect::<Vec<i32>>());
    }
    #[test]
    fn point_setpointattrib_addprim_npoints() {
        let o = cook(
            &grid3(),
            "if(@ptnum == 0) { setpointattrib(0, 'tag', 8, 5.0); int a = addpoint(0, {0,9,0}); int b = addpoint(0, {1,9,0}); \
             addprim(0, 'polyline', a, b); addprim(0, 'poly', 0, 1, a); } \
             f@nb = point(0, 'P', 8).x; i@np = npoints(0);",
        );
        assert_eq!(o.point_count, 11);
        assert_eq!(f1(&o, AttrClass::Point, "tag")[8], 5.0);
        assert_eq!(o.prim_count(), 6);
        assert_eq!(o.prims[4].kind, PrimKind::Polyline);
        assert_eq!(o.prims[5].kind, PrimKind::Poly { closed: true });
        // point() reads the INPUT snapshot: ptnum 0 sees x=-1... point 8 is (1,0,1) regardless of order
        assert_eq!(f1(&o, AttrClass::Point, "nb")[..9], [1.0; 9]);
        assert_eq!(i1(&o, AttrClass::Point, "np")[..2], [11, 11]);
        for s in [
            "addprim(0,'poly',0,1);",
            "addprim(0,'blob',0,1,2);",
            "addprim(0,'poly',0,1,99);",
            "removepoint(0, 99);",
            "f@a = point(0,'nope',0);",
        ] {
            assert!(cook_class(&grid3(), s, "detail").is_err(), "{s}");
        }
    }
    #[test]
    fn bad_class_param_and_empty_inputs_never_panic() {
        let e = cook_class(&grid3(), "", "edge").expect_err("bad class");
        assert!(matches!(e, CookError::BadParam(_)));
        for class in ["point", "prim", "vertex", "detail"] {
            let o = cook_class(&Detail::default(), "@P.y = 1; @Cd = {1,1,1}; i@x = @ptnum;", class);
            assert!(o.is_ok() || class == "detail", "{class}: {o:?}");
        }
        let p = params("@P.y = 1;", "point");
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        assert!(new().cook(&ctx, &[]).is_ok());
    }
    #[test]
    fn frame_follows_time() {
        let p = params("f@fr = @Frame;", "detail");
        let ctx = CookCtx { time: 1.0, fps: 24.0, params: &p, log: &|_| {} };
        let o = new().cook(&ctx, &[Arc::new(grid3())]).expect("cooks");
        assert_eq!(f1(&o, AttrClass::Detail, "fr"), vec![25.0]);
    }
    #[test]
    fn example_scene_cooks() {
        let json = include_str!("../../../../examples/wrangle.json");
        let (mut g, id) = load_scene(json).map_err(|e| e.to_string()).expect("loads");
        let d = g.cook(id, 0.0).map_err(|e| e.to_string()).expect("cooks");
        assert!(d.point_count > 0);
        let n = d.p().iter().filter(|p| (p[1] - (p[0].sin() * 0.5)).abs() < 1e-6).count();
        assert_eq!(n, d.point_count as usize);
        assert!(d.p().iter().any(|p| p[1].abs() > 0.1));
    }
}
