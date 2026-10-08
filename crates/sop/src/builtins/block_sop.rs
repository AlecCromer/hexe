//! For-Each block SOPs (`block_begin` / `block_end`, Houdini For-Each). The engine half lives in `hexe-graph`
//! (`graph/block.rs`): it cooks the sub-network between the two nodes once per iteration; these nodes slice and merge.
//!
//! # `block_begin` params
//! `method` menu `piece | count | points | prims` (default piece) · `class` menu `point | prim` (piece: which class carries
//! `attrib`, default prim) · `attrib` S (default `name`; string or int attribute) · `count` I (method `count`, default 1).
//! * piece: iteration i = the i-th distinct value of `attrib`, in FIRST-APPEARANCE order. prim class keeps the prims of
//!   that value + exactly the points they use; point class keeps the points + the prims whose every vertex is kept.
//!   Missing attribute on a non-empty class = `Failed`; empty class = 0 iterations.
//! * count: every iteration sees the whole input. points / prims: one element per iteration (a point + prims fully on it /
//!   a prim + its points). Points are compacted (ids renumbered), attributes + groups travel.
//! * feedback (End `mode`): the slice is the previous iteration's result (iteration 0: the input); the method only fixes N.
//! * every slice carries detail attributes `iteration` and `numiterations` (int); `block_end` strips them from its output.
//!   (No VEX `detail()` function exists in this port yet: wrangles read them as detail-class `@iteration`.)
//! * outside a block loop (`begin` cooked directly) it passes its input through with `iteration=0, numiterations=1`.
//!
//! # `block_end` params
//! `mode` menu `merge | feedback` (default merge) · `begin` S (name of the paired begin; empty = nearest upstream begin).
//! merge = `Detail::merge` of all iteration results in order (0 iterations = empty); feedback = result of the LAST
//! iteration (0 iterations = the begin input untouched).
//! The loop is NOT a graph edge, so graph cycle detection is untouched (see `hexe-graph` `graph/block.rs`).
//! UNVERIFIED vs Houdini (hython blocked): piece ordering, detail attribute names, metadata/fetch-input modes, per-iteration
//! stop condition, gather-method options, compile blocks.
use super::group_sop::{compact, failed, prim_verts};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{BlockIter, BlockRole, CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::collections::HashMap;
use std::sync::Arc;

const WHO: &str = "block_begin";
const M_PIECE: u32 = 0;
const M_COUNT: u32 = 1;
const M_POINTS: u32 = 2;
const M_PRIMS: u32 = 3;
const C_POINT: u32 = 0;
const C_PRIM: u32 = 1;
const MODE_FEEDBACK: u32 = 1;
/// Output point cap of a merged block (AGENTS.md §4).
const MAX_POINTS: u64 = 1 << 24;
const ITER: &str = "iteration";
const NUM_ITER: &str = "numiterations";
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];

pub fn begin_spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("method", ParamValue::Menu(M_PIECE)).label("Iterate Over"),
        ParamSpec::new("class", ParamValue::Menu(C_PRIM)).label("Piece Class"),
        ParamSpec::new("attrib", ParamValue::S("name".into())).label("Piece Attribute"),
        ParamSpec::new("count", ParamValue::I(1)).label("Iterations").range(0.0, 100.0),
    ]
}
pub fn begin_new() -> Box<SopNode> {
    Box::new(Begin { specs: begin_spec() })
}
pub fn end_spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("mode", ParamValue::Menu(0)).label("Gather (merge | feedback)"),
        ParamSpec::new("begin", ParamValue::S(String::new())).label("Block Begin"),
    ]
}
pub fn end_new() -> Box<SopNode> {
    Box::new(End { specs: end_spec() })
}

struct Begin {
    specs: Vec<ParamSpec>,
}
struct End {
    specs: Vec<ParamSpec>,
}

fn tag(d: &mut Detail, index: usize, count: usize) {
    for (name, v) in [(ITER, index), (NUM_ITER, count)] {
        d.add_attr(Attribute {
            name: name.into(),
            class: AttrClass::Detail,
            data: AttrData::I32(vec![i32::try_from(v).unwrap_or(i32::MAX)]),
            type_info: TypeInfo::None,
        });
    }
}
fn untag(d: &Arc<Detail>) -> Arc<Detail> {
    let tagged = |a: &Attribute| a.class == AttrClass::Detail && (a.name == ITER || a.name == NUM_ITER);
    if !d.attrs.iter().any(tagged) {
        return Arc::clone(d);
    }
    let mut o = (**d).clone();
    o.attrs.retain(|a| !tagged(a));
    Arc::new(o)
}

/// Piece key of every element of `class` + the distinct keys in first-appearance order.
fn pieces(d: &Detail, class: AttrClass, attrib: &str) -> Result<(Vec<String>, Vec<usize>), CookError> {
    let n = d.class_len(class);
    if n == 0 {
        return Ok((Vec::new(), Vec::new()));
    }
    let a = d.attr(class, attrib).ok_or_else(|| failed(WHO, format!("piece attribute `{attrib}` not found")))?;
    let key = |i: usize| -> Result<String, CookError> {
        match &a.data {
            AttrData::Str { idx, table } => Ok(idx
                .get(i)
                .and_then(|&t| usize::try_from(t).ok())
                .and_then(|t| table.get(t))
                .cloned()
                .unwrap_or_default()),
            AttrData::I32(v) => Ok(v.get(i).copied().unwrap_or(0).to_string()),
            _ => Err(CookError::Unsupported(format!("{WHO}: piece attribute `{attrib}` must be a string or int"))),
        }
    };
    let (mut order, mut seen, mut of) = (Vec::new(), HashMap::<String, usize>::new(), Vec::with_capacity(n));
    for i in 0..n {
        let k = key(i)?;
        let next = order.len();
        let id = *seen.entry(k.clone()).or_insert(next);
        if id == next {
            order.push(k);
        }
        of.push(id);
    }
    Ok((order, of))
}

/// Points used by the prims in `keep_prim`.
fn points_of(d: &Detail, keep_prim: &[bool]) -> Vec<bool> {
    let mut pts = vec![false; d.point_count as usize];
    for (p, _) in d.prims.iter().zip(keep_prim).filter(|(_, k)| **k) {
        for v in prim_verts(d, p) {
            if let Some(slot) = d.vertex_point.get(v).and_then(|&q| pts.get_mut(q as usize)) {
                *slot = true;
            }
        }
    }
    pts
}
/// Prims whose every vertex lies on a kept point (and that have at least one vertex).
fn prims_on(d: &Detail, keep_pt: &[bool]) -> Vec<bool> {
    d.prims
        .iter()
        .map(|p| {
            let r = prim_verts(d, p);
            !r.is_empty()
                && d.vertex_point.get(r).is_some_and(|vs| vs.iter().all(|&q| keep_pt.get(q as usize) == Some(&true)))
        })
        .collect()
}

fn class_of(m: u32) -> Result<AttrClass, CookError> {
    match m {
        C_POINT => Ok(AttrClass::Point),
        C_PRIM => Ok(AttrClass::Prim),
        _ => Err(CookError::BadParam(format!("{WHO}.class: menu index {m} out of range (point | prim)"))),
    }
}
fn bad_param(name: &str) -> CookError {
    CookError::BadParam(format!("{WHO}.{name}: missing or wrong type"))
}

impl Node<Detail> for Begin {
    fn type_name(&self) -> &'static str {
        "block_begin"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, _: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let mut d = inputs.first().map(|d| (**d).clone()).unwrap_or_default();
        tag(&mut d, 0, 1);
        Ok(Arc::new(d))
    }
    fn block_role(&self) -> BlockRole {
        BlockRole::Begin
    }
    fn block_count(&self, ctx: &CookCtx, source: &Arc<Detail>) -> Result<usize, CookError> {
        let p = ctx.params;
        match p.menu("method").ok_or_else(|| bad_param("method"))? {
            M_PIECE => {
                let class = class_of(p.menu("class").ok_or_else(|| bad_param("class"))?)?;
                let attrib = p.string("attrib").ok_or_else(|| bad_param("attrib"))?;
                Ok(pieces(source, class, attrib)?.0.len())
            }
            M_COUNT => usize::try_from(p.int("count").ok_or_else(|| bad_param("count"))?)
                .map_err(|_| CookError::BadParam(format!("{WHO}.count: must not be negative"))),
            M_POINTS => Ok(source.point_count as usize),
            M_PRIMS => Ok(source.prims.len()),
            m => Err(CookError::BadParam(format!("{WHO}.method: menu index {m} out of range 0..=3"))),
        }
    }
    fn cook_block(&self, ctx: &CookCtx, inputs: &[Arc<Detail>], it: BlockIter) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let d = inputs.first().cloned().unwrap_or_default();
        let method = p.menu("method").ok_or_else(|| bad_param("method"))?;
        let (npt, npr) = (d.point_count as usize, d.prims.len());
        let mut out = if it.feedback || method == M_COUNT {
            (*d).clone()
        } else {
            let (keep_pt, keep_prim) = match method {
                M_PIECE => {
                    let class = class_of(p.menu("class").ok_or_else(|| bad_param("class"))?)?;
                    let attrib = p.string("attrib").ok_or_else(|| bad_param("attrib"))?;
                    let mask: Vec<bool> = pieces(&d, class, attrib)?.1.iter().map(|&k| k == it.index).collect();
                    if class == AttrClass::Prim {
                        (points_of(&d, &mask), mask)
                    } else {
                        let on = prims_on(&d, &mask);
                        (mask, on)
                    }
                }
                M_POINTS => {
                    let mask: Vec<bool> = (0..npt).map(|i| i == it.index).collect();
                    let on = prims_on(&d, &mask);
                    (mask, on)
                }
                M_PRIMS => {
                    let mask: Vec<bool> = (0..npr).map(|i| i == it.index).collect();
                    (points_of(&d, &mask), mask)
                }
                m => return Err(CookError::BadParam(format!("{WHO}.method: menu index {m} out of range 0..=3"))),
            };
            compact(&d, &keep_pt, &keep_prim, WHO)?
        };
        tag(&mut out, it.index, it.count);
        Ok(Arc::new(out))
    }
}

impl Node<Detail> for End {
    fn type_name(&self) -> &'static str {
        "block_end"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    /// Only reached if a caller bypasses the engine: a single-iteration merge of the input.
    fn cook(&self, _: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        Ok(inputs.first().map(untag).unwrap_or_default())
    }
    fn block_role(&self) -> BlockRole {
        BlockRole::End
    }
    fn block_feedback(&self, ctx: &CookCtx) -> bool {
        ctx.params.menu("mode") == Some(MODE_FEEDBACK)
    }
    fn block_merge(&self, ctx: &CookCtx, results: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let mode = ctx.params.menu("mode").ok_or_else(|| bad_param("mode"))?;
        if mode > MODE_FEEDBACK {
            return Err(CookError::BadParam("block_end.mode: menu index out of range 0..=1".into()));
        }
        if mode == MODE_FEEDBACK {
            return Ok(results.last().map(untag).unwrap_or_default());
        }
        let total: u64 = results.iter().map(|d| u64::from(d.point_count)).sum();
        if total > MAX_POINTS {
            return Err(failed("block_end", format!("{total} points exceeds the {MAX_POINTS} point cap")));
        }
        let mut it = results.iter().map(untag);
        let Some(first) = it.next() else { return Ok(Arc::default()) };
        let mut rest = it.peekable();
        if rest.peek().is_none() {
            return Ok(first);
        }
        let mut out = (*first).clone();
        for d in rest {
            out.merge(&d);
        }
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;

    fn cook(json: &str) -> Result<Arc<Detail>, String> {
        let (mut g, id) = load_scene(json).map_err(|e| e.to_string())?;
        g.cook(id, 0.0).map_err(|e| e.to_string())
    }
    const SIX: &str = "0,0,0 1,0,0 2,0,0 3,0,0 4,0,0 5,0,0";
    /// 6 points x=0..5, `s@name` = a b c a b c.
    fn named_src() -> String {
        format!(
            r#"{{"name":"src","type":"add","params":{{"points":"{SIX}"}}}},
            {{"name":"nm","type":"wrangle","params":{{"snippet":"if(@ptnum % 3 == 0) s@name = 'a'; else if(@ptnum % 3 == 1) s@name = 'b'; else s@name = 'c';"}}}}"#
        )
    }
    fn scene(body: &str, edges: &str) -> String {
        format!(r#"{{"nodes":[{body}],"edges":{edges},"display":"e"}}"#)
    }
    fn ys(d: &Detail) -> Vec<f32> {
        d.p().iter().map(|p| p[1]).collect()
    }

    /// Oracle: 3 pieces by `name` (point class) through xform ty=1 -> pieces a,b,c in first-appearance order, every P.y + 1.
    #[test]
    fn three_pieces_by_name_through_xform_ty1_have_exact_p() {
        let body = format!(
            r#"{},
            {{"name":"b","type":"block_begin","params":{{"method":0,"class":0}}}},
            {{"name":"x","type":"xform","params":{{"t":[0,1,0]}}}},
            {{"name":"e","type":"block_end"}}"#,
            named_src()
        );
        let d = cook(&scene(&body, r#"[["src",0,"nm",0],["nm",0,"b",0],["b",0,"x",0],["x",0,"e",0]]"#)).expect("cooks");
        let want =
            [[0.0, 1.0, 0.0], [3.0, 1.0, 0.0], [1.0, 1.0, 0.0], [4.0, 1.0, 0.0], [2.0, 1.0, 0.0], [5.0, 1.0, 0.0]];
        assert_eq!(d.p(), &want);
        assert_eq!(d.point_count, 6);
        // name attribute travels, helper detail attributes do not leak out of the block
        assert!(d.attr(AttrClass::Point, "name").is_some());
        assert!(d.attr(AttrClass::Detail, ITER).is_none() && d.attr(AttrClass::Detail, NUM_ITER).is_none());
    }

    /// Oracle: count=4 feedback with `add 1 point` (+ xform ty=1 to make the order observable) -> exactly 4 points, y = 4,3,2,1.
    #[test]
    fn count4_feedback_add_one_point_gives_four_points() {
        let body = r#"{"name":"b","type":"block_begin","params":{"method":1,"count":4}},
            {"name":"a","type":"add","params":{"points":"0,0,0"}},
            {"name":"x","type":"xform","params":{"t":[0,1,0]}},
            {"name":"e","type":"block_end","params":{"mode":1}}"#;
        let d = cook(&scene(body, r#"[["b",0,"a",0],["a",0,"x",0],["x",0,"e",0]]"#)).expect("cooks");
        assert_eq!(d.point_count, 4);
        assert_eq!(ys(&d), vec![4.0, 3.0, 2.0, 1.0]);
        // same loop without feedback: every iteration starts from the (empty) input -> 4 merged single points at y=1
        let merged = body.replace(r#","params":{"mode":1}"#, "");
        let m = cook(&scene(&merged, r#"[["b",0,"a",0],["a",0,"x",0],["x",0,"e",0]]"#)).expect("cooks");
        assert_eq!(ys(&m), vec![1.0; 4]);
    }

    #[test]
    fn prim_pieces_points_and_prims_methods() {
        // grid 3x3 = 4 quads, 9 pts. prim pieces: quads 0,1 -> 'x', 2,3 -> 'y'
        let pre = r#"{"name":"g","type":"grid","params":{"rows":3,"cols":3}},
            {"name":"nm","type":"wrangle","params":{"class":"prim","snippet":"if(@primnum < 2) s@name = 'x'; else s@name = 'y';"}},
            {"name":"b","type":"block_begin","params":{"METHOD"}},
            {"name":"x","type":"xform","params":{"t":[0,0,1]}},
            {"name":"e","type":"block_end"}"#;
        let edges = r#"[["g",0,"nm",0],["nm",0,"b",0],["b",0,"x",0],["x",0,"e",0]]"#;
        let run = |m: &str| cook(&scene(&pre.replace("\"METHOD\"", m), edges)).expect(m);
        let piece = run(r#""method":0"#);
        // piece x = quads 0,1 = 6 pts, piece y = 6 pts (two pts shared by the halves are duplicated): 12 pts, 4 quads, z+1
        assert_eq!((piece.point_count, piece.prim_count()), (12, 4));
        let orig =
            cook(r#"{"nodes":[{"name":"e","type":"grid","params":{"rows":3,"cols":3}}],"display":"e"}"#).expect("grid");
        let on_grid = |d: &Detail| d.p().iter().all(|p| orig.p().contains(&[p[0], p[1], p[2] - 1.0]));
        assert!(on_grid(&piece), "every P is an original grid point + (0,0,1)");
        let prims = run(r#""method":3"#);
        assert_eq!((prims.point_count, prims.prim_count()), (16, 4), "one quad (4 pts) per iteration");
        let points = run(r#""method":2"#);
        assert_eq!((points.point_count, points.prim_count()), (9, 0), "a lone point carries no whole quad");
        assert!(on_grid(&points));
    }

    #[test]
    fn iteration_detail_attrs_are_on_every_slice() {
        let begin = crate::registry().create("block_begin").expect("registered");
        let ps = begin.params().iter().fold(hexe_graph::ParamSet::new(), |p, s| p.with(&s.name, s.default.clone()));
        let ps = ps.with("method", ParamValue::Menu(M_COUNT)).with("count", ParamValue::I(5));
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &ps, log: &|_| {} };
        let src = Arc::new(Detail::default());
        assert_eq!(begin.block_count(&ctx, &src), Ok(5));
        let s = begin.cook_block(&ctx, &[src], BlockIter { index: 3, count: 5, feedback: false }).expect("slice");
        let get = |n: &str| match s.attr(AttrClass::Detail, n).map(|a| &a.data) {
            Some(AttrData::I32(v)) => v.clone(),
            _ => Vec::new(),
        };
        assert_eq!((get(ITER), get(NUM_ITER)), (vec![3], vec![5]));
    }

    #[test]
    fn zero_iterations_merge_is_empty_and_feedback_passes_input() {
        let mk = |mode: u32| {
            scene(
                &format!(
                    r#"{{"name":"s","type":"add","params":{{"points":"1,2,3"}}}},
            {{"name":"b","type":"block_begin","params":{{"method":1,"count":0}}}},
            {{"name":"e","type":"block_end","params":{{"mode":{mode}}}}}"#
                ),
                r#"[["s",0,"b",0],["b",0,"e",0]]"#,
            )
        };
        assert_eq!(cook(&mk(0)).expect("merge").point_count, 0);
        assert_eq!(cook(&mk(1)).expect("feedback").p(), &[[1.0, 2.0, 3.0]]);
    }

    #[test]
    fn nested_blocks_pair_automatically() {
        // outer count 2 (merge) around inner count 3 (merge) around add-1-point: 2*3 = 6 points
        let body = r#"{"name":"ob","type":"block_begin","params":{"method":1,"count":2}},
            {"name":"ib","type":"block_begin","params":{"method":1,"count":3}},
            {"name":"a","type":"add","params":{"points":"0,0,0"}},
            {"name":"ie","type":"block_end"},
            {"name":"e","type":"block_end"}"#;
        let d = cook(&scene(body, r#"[["ob",0,"ib",0],["ib",0,"a",0],["a",0,"ie",0],["ie",0,"e",0]]"#)).expect("cooks");
        assert_eq!(d.point_count, 6);
    }

    #[test]
    fn hostile_scenes_are_errors() {
        let two = |b: &str, e: &str| {
            scene(
                &format!(
                    r#"{{"name":"s","type":"add","params":{{"points":"0,0,0"}}}},{{"name":"b","type":"block_begin","params":{b}}},
            {{"name":"e","type":"block_end","params":{e}}}"#
                ),
                r#"[["s",0,"b",0],["b",0,"e",0]]"#,
            )
        };
        assert!(cook(&two(r#"{"method":1,"count":-1}"#, "{}")).is_err(), "negative count");
        assert!(cook(&two(r#"{"method":1,"count":2000000000}"#, "{}")).is_err(), "over the iteration cap");
        let lines = scene(
            r#"{"name":"s","type":"add","params":{"points":"0,0,0 1,0,0","prims":true}},
            {"name":"b","type":"block_begin"},{"name":"e","type":"block_end"}"#,
            r#"[["s",0,"b",0],["b",0,"e",0]]"#,
        );
        assert!(cook(&lines).is_err(), "missing piece attribute on a non-empty class");
        assert_eq!(cook(&two(r#"{"method":0}"#, "{}")).expect("no prims = no pieces").point_count, 0);
        assert!(cook(&two(r#"{"method":9}"#, "{}")).is_err(), "bad method");
        assert!(cook(&two(r#"{}"#, r#"{"mode":7}"#)).is_err(), "bad merge mode");
        assert!(cook(&two(r#"{}"#, r#"{"begin":"s"}"#)).is_err(), "begin names a non-begin node");
        // a real wire from the end back into the begin is a cycle, rejected at load
        let cyc = scene(
            r#"{"name":"b","type":"block_begin"},{"name":"e","type":"block_end"}"#,
            r#"[["b",0,"e",0],["e",0,"b",0]]"#,
        );
        assert!(load_scene(&cyc).is_err());
    }

    #[test]
    fn edit_inside_the_block_recooks_and_second_cook_is_cached() {
        let body = r#"{"name":"b","type":"block_begin","params":{"method":1,"count":2}},
            {"name":"a","type":"add","params":{"points":"0,0,0"}},
            {"name":"e","type":"block_end"}"#;
        let (mut g, id) = load_scene(&scene(body, r#"[["b",0,"a",0],["a",0,"e",0]]"#)).expect("loads");
        let first = g.cook(id, 0.0).expect("cooks");
        assert_eq!(first.point_count, 2);
        assert!(Arc::ptr_eq(&first, &g.cook(id, 0.0).expect("cached")));
        let a = g.node_by_name("a").expect("a");
        g.set_param(a, "points", ParamValue::S("0,0,0 1,1,1".into())).expect("set");
        assert_eq!(g.cook(id, 0.0).expect("recooked").point_count, 4);
    }
}
