//! Shared helpers of the `sort` / `delete` SOPs (no SOP token of its own): index-list rebuild of a `Detail` and per-element
//! VEX-subset expression evaluation (read-only host; the expression is wrapped as `f@hexe_result = (EXPR);`).
use super::group_sop::{bad, failed, prim_verts};
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, Group, Prim};
use hexe_graph::CookError;
use hexe_vex::{Host, HostError, Ty, Value, parse, run};

/// Interpreter step budget per cook (all elements together).
const MAX_STEPS: u64 = 50_000_000;
const RESULT: &str = "hexe_result";

/// `idx[k]`-th element of the attribute data; out-of-range -> default element. Total.
fn gather(d: &AttrData, idx: &[usize]) -> AttrData {
    fn pick<T: Copy>(v: &[T], idx: &[usize], def: T) -> Vec<T> {
        idx.iter().map(|&i| v.get(i).copied().unwrap_or(def)).collect()
    }
    fn pick_n<T: Copy>(v: &[T], size: u32, idx: &[usize], def: T) -> Vec<T> {
        let s = usize::try_from(size).unwrap_or(0);
        let mut o = Vec::with_capacity(idx.len().saturating_mul(s));
        for &i in idx {
            let chunk = i.checked_mul(s).and_then(|a| v.get(a..a.checked_add(s)?));
            match chunk {
                Some(c) => o.extend_from_slice(c),
                None => o.extend(std::iter::repeat_n(def, s)),
            }
        }
        o
    }
    match d {
        AttrData::F32(v) => AttrData::F32(pick(v, idx, 0.0)),
        AttrData::I32(v) => AttrData::I32(pick(v, idx, 0)),
        AttrData::V2(v) => AttrData::V2(pick(v, idx, [0.0; 2])),
        AttrData::V3(v) => AttrData::V3(pick(v, idx, [0.0; 3])),
        AttrData::V4(v) => AttrData::V4(pick(v, idx, [0.0; 4])),
        AttrData::Str { idx: si, table } => AttrData::Str { idx: pick(si, idx, -1), table: table.clone() },
        AttrData::FN { size, data } => AttrData::FN { size: *size, data: pick_n(data, *size, idx, 0.0) },
        AttrData::IN { size, data } => AttrData::IN { size: *size, data: pick_n(data, *size, idx, 0) },
    }
}

/// New `Detail` from index lists. `pidx[new] = old point`; `prims[new] = (old prim, old vertex ids it keeps, in order)`.
/// Vertex storage is rebuilt contiguously in new prim order, `vertex_point` is remapped through the point permutation,
/// every point/vertex/prim attribute and group is gathered along. A kept vertex whose point is not in `pidx` is `Failed`.
pub(super) fn rebuild(
    src: &Detail,
    pidx: &[usize],
    prims: &[(usize, Vec<usize>)],
    who: &str,
) -> Result<Detail, CookError> {
    let mut pmap: Vec<Option<u32>> = vec![None; src.point_count as usize];
    for (new, &old) in pidx.iter().enumerate() {
        if let Some(s) = pmap.get_mut(old) {
            *s = u32::try_from(new).ok();
        }
    }
    let (mut vidx, mut qidx, mut vertex_point, mut out_prims) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (q, verts) in prims {
        let pr = src.prims.get(*q).ok_or_else(|| failed(who, "primitive out of range"))?;
        let start = u32::try_from(vertex_point.len()).map_err(|_| failed(who, "vertex id overflow"))?;
        for &v in verts {
            let old = src.vertex_point.get(v).copied().ok_or_else(|| failed(who, "vertex out of range"))?;
            let new = pmap
                .get(old as usize)
                .copied()
                .flatten()
                .ok_or_else(|| failed(who, "kept vertex references a removed point"))?;
            vertex_point.push(new);
            vidx.push(v);
        }
        let end = u32::try_from(vertex_point.len()).map_err(|_| failed(who, "vertex id overflow"))?;
        out_prims.push(Prim { kind: pr.kind.clone(), verts: start..end });
        qidx.push(*q);
    }
    let pick = |c: AttrClass| -> &[usize] {
        match c {
            AttrClass::Point => pidx,
            AttrClass::Vertex => &vidx,
            AttrClass::Prim => &qidx,
            AttrClass::Detail => &[],
        }
    };
    let attrs = src
        .attrs
        .iter()
        .map(|a| Attribute {
            name: a.name.clone(),
            class: a.class,
            data: if a.class == AttrClass::Detail { a.data.clone() } else { gather(&a.data, pick(a.class)) },
            type_info: a.type_info,
        })
        .collect();
    let groups = src
        .groups
        .iter()
        .map(|g| Group {
            name: g.name.clone(),
            class: g.class,
            mask: pick(g.class).iter().map(|&i| g.mask.get(i).copied().unwrap_or(false)).collect(),
        })
        .collect();
    Ok(Detail {
        point_count: u32::try_from(pidx.len()).map_err(|_| failed(who, "point id overflow"))?,
        vertex_point,
        prims: out_prims,
        attrs,
        groups,
        skipped_attribs: src.skipped_attribs.clone(),
    })
}

/// Position of element `i` of `class` (point = P, prim = mean of its points); `[0;3]` when unknown.
pub(super) fn position(d: &Detail, class: AttrClass, i: usize) -> [f64; 3] {
    let p = d.p();
    let at = |k: usize| p.get(k).map_or([0.0f64; 3], |q| q.map(f64::from));
    match class {
        AttrClass::Point => at(i),
        AttrClass::Prim => {
            let Some(pr) = d.prims.get(i) else { return [0.0; 3] };
            let (mut sum, mut n) = ([0.0f64; 3], 0.0f64);
            for &q in d.vertex_point.get(prim_verts(d, pr)).unwrap_or(&[]) {
                let c = at(q as usize);
                for k in 0..3 {
                    sum[k] += c[k];
                }
                n += 1.0;
            }
            if n > 0.0 { sum.map(|s| s / n) } else { [0.0; 3] }
        }
        _ => [0.0; 3],
    }
}

fn read_elem(d: &AttrData, i: usize) -> Result<Value, HostError> {
    let oob = || HostError::Failed(format!("element {i} out of range"));
    Ok(match d {
        AttrData::F32(v) => Value::Float(f64::from(*v.get(i).ok_or_else(oob)?)),
        AttrData::I32(v) => Value::Int(i64::from(*v.get(i).ok_or_else(oob)?)),
        AttrData::V2(v) => Value::Vec2(v.get(i).ok_or_else(oob)?.map(f64::from)),
        AttrData::V3(v) => Value::Vec3(v.get(i).ok_or_else(oob)?.map(f64::from)),
        AttrData::V4(v) => Value::Vec4(v.get(i).ok_or_else(oob)?.map(f64::from)),
        AttrData::Str { idx, table } => {
            let k = *idx.get(i).ok_or_else(oob)?;
            Value::Str(usize::try_from(k).ok().and_then(|k| table.get(k)).cloned().unwrap_or_default())
        }
        AttrData::FN { .. } | AttrData::IN { .. } => {
            return Err(HostError::Unsupported("tuple attributes with more than 4 components".into()));
        }
    })
}

/// Read-only per-element host: `@ptnum @primnum @numpt @numprim @P` (prim = centroid) + any attribute of the class.
struct ElemHost<'a> {
    d: &'a Detail,
    class: AttrClass,
    idx: usize,
    out: Option<Value>,
}
fn ro() -> HostError {
    HostError::Unsupported(
        "expressions are read-only (no writes, addpoint, removepoint, addprim, setpointattrib)".into(),
    )
}
impl Host for ElemHost<'_> {
    fn attr(&mut self, name: &str, _hint: Option<Ty>) -> Result<Option<Value>, HostError> {
        let id = |on: bool| Value::Int(if on { self.idx as i64 } else { -1 });
        match name {
            "ptnum" => return Ok(Some(id(self.class == AttrClass::Point))),
            "primnum" => return Ok(Some(id(self.class == AttrClass::Prim))),
            "numpt" => return Ok(Some(Value::Int(i64::from(self.d.point_count)))),
            "numprim" => return Ok(Some(Value::Int(i64::from(self.d.prim_count())))),
            "P" if self.class == AttrClass::Prim => {
                return Ok(Some(Value::Vec3(position(self.d, AttrClass::Prim, self.idx))));
            }
            _ => {}
        }
        match self.d.attr(self.class, name) {
            Some(a) => read_elem(&a.data, self.idx).map(Some),
            None => Ok(None),
        }
    }
    fn set_attr(&mut self, name: &str, _hint: Option<Ty>, v: Value) -> Result<(), HostError> {
        if name == RESULT {
            self.out = Some(v);
            Ok(())
        } else {
            Err(ro())
        }
    }
    fn add_point(&mut self, _p: [f64; 3]) -> Result<i64, HostError> {
        Err(ro())
    }
    fn remove_point(&mut self, _n: i64) -> Result<(), HostError> {
        Err(ro())
    }
    fn add_prim(&mut self, _kind: &str, _pts: &[i64]) -> Result<i64, HostError> {
        Err(ro())
    }
    fn set_point_attrib(&mut self, _name: &str, _n: i64, _v: Value) -> Result<(), HostError> {
        Err(ro())
    }
    fn point(&mut self, name: &str, n: i64) -> Result<Value, HostError> {
        let k = usize::try_from(n).ok().filter(|k| *k < self.d.point_count as usize);
        let k = k.ok_or_else(|| HostError::Failed(format!("point {n} out of range")))?;
        match self.d.attr(AttrClass::Point, name) {
            Some(a) => read_elem(&a.data, k),
            None => Err(HostError::Failed(format!("point: no point attribute `{name}`"))),
        }
    }
    fn npoints(&mut self) -> i64 {
        i64::from(self.d.point_count)
    }
}

/// Evaluate the VEX-subset expression `src` (e.g. `@P.y > 0`, `@ptnum % 2 == 0`) once per element of `class`
/// (Point | Prim). Result is numeric (int/float; comparisons give 0/1). Empty / unparsable expression -> `BadParam`;
/// runtime failure -> `Failed`; non-numeric result -> `Failed`.
pub(super) fn eval_expr(d: &Detail, class: AttrClass, src: &str, who: &str) -> Result<Vec<f64>, CookError> {
    if src.trim().is_empty() {
        return Err(bad(who, "empty expression"));
    }
    let prog = parse(&format!("f@{RESULT} = ({src}\n);")).map_err(|e| bad(who, format!("expression: {e}")))?;
    let n = d.class_len(class);
    let mut host = ElemHost { d, class, idx: 0, out: None };
    let mut steps = MAX_STEPS;
    let mut res = Vec::with_capacity(n);
    for idx in 0..n {
        host.idx = idx;
        host.out = None;
        run(&prog, &mut host, &mut steps).map_err(|e| failed(who, format!("expression, element {idx}: {e}")))?;
        res.push(match host.out.take() {
            Some(Value::Float(x)) => x,
            Some(Value::Int(x)) => x as f64,
            _ => return Err(failed(who, format!("expression, element {idx}: result is not numeric"))),
        });
    }
    Ok(res)
}
