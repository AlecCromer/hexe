//! For-Each block engine (`block_begin` / `block_end`, Houdini For-Each). Payload-generic: the engine only schedules,
//! the Begin node slices and the End node merges (`Node::{block_count, cook_block, block_feedback, block_merge}`).
//!
//! # Cook
//! `Graph::cook` treats a node with `BlockRole::End` as a LEAF of the dependency walk and calls `cook_block_end`:
//! 1. resolve the paired Begin (End param `begin` = node name; empty = nearest upstream Begin, jumping over nested blocks);
//! 2. region = nodes downstream of Begin AND upstream of End's input 0 (the sub-network);
//! 3. cook Begin's input once (the source), ask Begin for the iteration count;
//! 4. per iteration: clear the region caches, seed Begin's cache with its slice (`cook_block`), cook End's input 0 through
//!    the ordinary lazy path, collect the result; feedback mode hands the result to Begin of the next iteration;
//! 5. clear the region again (Begin cooked standalone later must not see a stale slice), `block_merge`, cache on End.
//!
//! # Why the feedback loop is not a cycle
//! There is NO edge from End back to Begin. The loop is carried by the engine as a value (`prev` below), so the
//! graph stays a DAG: `connect`'s reachability check and the cook-time `Active` check never see a back edge. A real
//! wire from End (or anything downstream of Begin) into Begin's input is still `GraphError::Cycle`.
//!
//! # Bounds (AGENTS.md §4)
//! Iterations ≤ `MAX_BLOCK_ITERATIONS`, nesting ≤ `MAX_BLOCK_DEPTH` (recursion only per nested block; a source that
//! depends on its own End recurses until that cap and fails with `Node` instead of overflowing the stack).
use super::{Cached, Graph, MAX_BLOCK_DEPTH, MAX_BLOCK_ITERATIONS};
use crate::node::{BlockIter, BlockRole, CookCtx, CookError, GraphError, NodeId, Payload};
use std::sync::Arc;

fn fail(node: NodeId, msg: impl Into<String>) -> GraphError {
    GraphError::Node { node, source: CookError::Failed(msg.into()) }
}

impl<T: Payload> Graph<T> {
    fn role(&self, id: NodeId) -> BlockRole {
        self.live(id).map_or(BlockRole::None, |s| s.node.block_role())
    }

    /// The Begin paired with `end` (see module doc, step 1).
    fn resolve_begin(&self, end: NodeId, depth: usize) -> Result<NodeId, GraphError> {
        if depth > MAX_BLOCK_DEPTH {
            return Err(fail(end, "block nesting too deep"));
        }
        let slot = self.slot(end)?;
        let name = slot.params.string("begin").unwrap_or_default();
        if !name.is_empty() {
            let b = self.node_by_name(name).ok_or_else(|| fail(end, format!("begin node `{name}` not found")))?;
            return if self.role(b) == BlockRole::Begin {
                Ok(b)
            } else {
                Err(fail(end, format!("`{name}` is not a block begin")))
            };
        }
        let first = slot.inputs.first().copied().flatten().ok_or(GraphError::MissingInput { node: end, input: 0 })?;
        let mut seen = vec![false; self.slots.len()];
        let mut stack = vec![first];
        while let Some(n) = stack.pop() {
            let Some(flag) = seen.get_mut(n.slot()) else { continue };
            if std::mem::replace(flag, true) {
                continue;
            }
            let Some(s) = self.live(n) else { continue };
            match s.node.block_role() {
                BlockRole::Begin => return Ok(n),
                BlockRole::End => {
                    // nested block: skip over it, continue from the source of its Begin
                    let inner = self.resolve_begin(n, depth + 1)?;
                    if let Some(src) = self.live(inner).and_then(|b| b.inputs.first().copied().flatten()) {
                        stack.push(src);
                    }
                }
                BlockRole::None => stack.extend(s.inputs.iter().rev().flatten().copied()),
            }
        }
        Err(fail(end, "no block_begin upstream of this block_end"))
    }

    /// Nodes reachable from `from` (inclusive) following inputs (`up`) or output edges (`!up`).
    fn reach_set(&self, from: NodeId, up: bool) -> Vec<bool> {
        let mut seen = vec![false; self.slots.len()];
        let mut stack = vec![from];
        while let Some(n) = stack.pop() {
            let Some(flag) = seen.get_mut(n.slot()) else { continue };
            if std::mem::replace(flag, true) {
                continue;
            }
            if let Some(s) = self.live(n) {
                if up {
                    stack.extend(s.inputs.iter().flatten().copied());
                } else {
                    stack.extend(s.outs.iter().map(|&(d, _)| d));
                }
            }
        }
        seen
    }

    /// Cook one block `end` at `time` and cache its merged output. Entered from `Graph::cook` only.
    pub(super) fn cook_block_end(&mut self, end: NodeId, time: f32) -> Result<(), GraphError> {
        if self.block_depth >= MAX_BLOCK_DEPTH {
            return Err(fail(end, "block nesting too deep (or a block source depends on its own block_end)"));
        }
        let e_in = self
            .slot(end)?
            .inputs
            .first()
            .copied()
            .flatten()
            .ok_or(GraphError::MissingInput { node: end, input: 0 })?;
        let begin = self.resolve_begin(end, 0)?;
        let (up, down) = (self.reach_set(e_in, true), self.reach_set(begin, false));
        if !up.get(begin.slot()).copied().unwrap_or(false) {
            return Err(fail(end, "block_begin is not upstream of this block_end"));
        }
        let region: Vec<NodeId> = self
            .node_ids()
            .into_iter()
            .filter(|id| up.get(id.slot()).copied().unwrap_or(false) && down.get(id.slot()).copied().unwrap_or(false))
            .collect();
        let b_src = self
            .slot(begin)?
            .inputs
            .first()
            .copied()
            .flatten()
            .ok_or(GraphError::MissingInput { node: begin, input: 0 })?;
        if down.get(b_src.slot()).copied().unwrap_or(false) {
            return Err(GraphError::Cycle(begin));
        }
        self.block_depth += 1;
        let run = self.run_block(end, begin, (e_in, b_src), &region, time);
        self.block_depth -= 1;
        for id in &region {
            if let Some(s) = self.live_mut(*id) {
                s.cache = None;
            }
        }
        let (out, sensitive) = run?;
        if let Some(s) = self.live_mut(end) {
            s.cache = Some(Cached { out, time, sensitive });
        }
        Ok(())
    }

    fn run_block(
        &mut self,
        end: NodeId,
        begin: NodeId,
        (e_in, b_src): (NodeId, NodeId),
        region: &[NodeId],
        time: f32,
    ) -> Result<(Arc<T>, bool), GraphError> {
        let source = self.cook(b_src, time)?;
        let src_sensitive = self.cached(b_src, time).is_some_and(|c| c.sensitive);
        let (count, begin_td, feedback, end_td) = {
            let (b, e) = (self.slot(begin)?, self.slot(end)?);
            let bctx = CookCtx { time, fps: self.fps, params: &b.params, log: &*self.log };
            let ectx = CookCtx { time, fps: self.fps, params: &e.params, log: &*self.log };
            let n = b.node.block_count(&bctx, &source).map_err(|source| GraphError::Node { node: begin, source })?;
            (n, b.node.time_dependent(), e.node.block_feedback(&ectx), e.node.time_dependent())
        };
        if count > MAX_BLOCK_ITERATIONS {
            return Err(fail(begin, format!("{count} iterations exceeds the {MAX_BLOCK_ITERATIONS} cap")));
        }
        let mut sensitive = src_sensitive || begin_td || end_td;
        let mut results: Vec<Arc<T>> = Vec::new();
        let mut prev: Option<Arc<T>> = None;
        for index in 0..count {
            for id in region {
                if let Some(s) = self.live_mut(*id) {
                    s.cache = None;
                }
            }
            let input = match (&prev, feedback) {
                (Some(p), true) => Arc::clone(p),
                _ => Arc::clone(&source),
            };
            let slice = {
                let b = self.slot(begin)?;
                let ctx = CookCtx { time, fps: self.fps, params: &b.params, log: &*self.log };
                b.node
                    .cook_block(&ctx, &[input], BlockIter { index, count, feedback })
                    .map_err(|source| GraphError::Node { node: begin, source })?
            };
            if let Some(s) = self.live_mut(begin) {
                s.cache = Some(Cached { out: slice, time, sensitive: src_sensitive || begin_td });
            }
            let r = self.cook(e_in, time)?;
            sensitive |= self.cached(e_in, time).is_some_and(|c| c.sensitive);
            if feedback {
                prev = Some(r);
            } else {
                results.push(r);
            }
        }
        if feedback {
            results = vec![prev.unwrap_or(source)];
        }
        let e = self.slot(end)?;
        let ctx = CookCtx { time, fps: self.fps, params: &e.params, log: &*self.log };
        let out = e.node.block_merge(&ctx, &results).map_err(|source| GraphError::Node { node: end, source })?;
        Ok((out, sensitive))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{InputSpec, Node};
    use crate::param::{ParamSet, ParamSpec, ParamValue};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// `i32` payload. Begin: slice = input + index; count = param `n`.
    struct BeginI(Vec<ParamSpec>);
    impl Node<i32> for BeginI {
        fn type_name(&self) -> &'static str {
            "begin"
        }
        fn inputs(&self) -> &[InputSpec] {
            &[InputSpec { name: "in" }]
        }
        fn params(&self) -> &[ParamSpec] {
            &self.0
        }
        fn cook(&self, _: &CookCtx, i: &[Arc<i32>]) -> Result<Arc<i32>, CookError> {
            Ok(Arc::clone(&i[0]))
        }
        fn block_role(&self) -> BlockRole {
            BlockRole::Begin
        }
        fn block_count(&self, ctx: &CookCtx, _: &Arc<i32>) -> Result<usize, CookError> {
            usize::try_from(ctx.params.int("n").unwrap_or(0)).map_err(|_| CookError::BadParam("n".into()))
        }
        fn cook_block(&self, _: &CookCtx, i: &[Arc<i32>], it: BlockIter) -> Result<Arc<i32>, CookError> {
            Ok(Arc::new(*i[0] + i32::try_from(it.index).unwrap_or(0)))
        }
    }
    /// Inner node: +1, counts its cooks.
    struct Inc(Arc<AtomicUsize>);
    impl Node<i32> for Inc {
        fn type_name(&self) -> &'static str {
            "inc"
        }
        fn inputs(&self) -> &[InputSpec] {
            &[InputSpec { name: "in" }]
        }
        fn params(&self) -> &[ParamSpec] {
            &[]
        }
        fn cook(&self, _: &CookCtx, i: &[Arc<i32>]) -> Result<Arc<i32>, CookError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(*i[0] + 1))
        }
    }
    /// Source constant (param `value`).
    struct Src(Vec<ParamSpec>);
    impl Node<i32> for Src {
        fn type_name(&self) -> &'static str {
            "src"
        }
        fn inputs(&self) -> &[InputSpec] {
            &[]
        }
        fn params(&self) -> &[ParamSpec] {
            &self.0
        }
        fn cook(&self, c: &CookCtx, _: &[Arc<i32>]) -> Result<Arc<i32>, CookError> {
            Ok(Arc::new(c.params.int("value").unwrap_or(0)))
        }
    }
    /// End: merge = sum, feedback (param `fb`) = last.
    struct EndI(Vec<ParamSpec>);
    impl Node<i32> for EndI {
        fn type_name(&self) -> &'static str {
            "end"
        }
        fn inputs(&self) -> &[InputSpec] {
            &[InputSpec { name: "in" }]
        }
        fn params(&self) -> &[ParamSpec] {
            &self.0
        }
        fn cook(&self, _: &CookCtx, i: &[Arc<i32>]) -> Result<Arc<i32>, CookError> {
            Ok(Arc::clone(&i[0]))
        }
        fn block_role(&self) -> BlockRole {
            BlockRole::End
        }
        fn block_feedback(&self, ctx: &CookCtx) -> bool {
            ctx.params.toggle("fb") == Some(true)
        }
        fn block_merge(&self, ctx: &CookCtx, r: &[Arc<i32>]) -> Result<Arc<i32>, CookError> {
            Ok(Arc::new(if self.block_feedback(ctx) {
                r.last().map_or(0, |x| **x)
            } else {
                r.iter().map(|x| **x).sum()
            }))
        }
    }
    fn begin() -> BeginI {
        BeginI(vec![ParamSpec::new("n", ParamValue::I(3))])
    }
    fn src(v: i32) -> Src {
        Src(vec![ParamSpec::new("value", ParamValue::I(v))])
    }
    fn end() -> EndI {
        EndI(vec![ParamSpec::new("fb", ParamValue::B(false)), ParamSpec::new("begin", ParamValue::S(String::new()))])
    }
    /// src(10) -> begin(n=3) -> inc -> end. Returns (graph, end, inc counter, begin, src).
    fn rig() -> (Graph<i32>, NodeId, Arc<AtomicUsize>, NodeId, NodeId) {
        let n = Arc::new(AtomicUsize::new(0));
        let mut g = Graph::new();
        let s = g.add_node(src(10), ParamSet::new()).expect("src");
        let b = g.add_node(begin(), ParamSet::new()).expect("begin");
        let i = g.add_node(Inc(n.clone()), ParamSet::new()).expect("inc");
        let e = g.add_node(end(), ParamSet::new()).expect("end");
        g.connect(s, 0, b, 0).expect("s-b");
        g.connect(b, 0, i, 0).expect("b-i");
        g.connect(i, 0, e, 0).expect("i-e");
        (g, e, n, b, s)
    }

    #[test]
    fn subnetwork_runs_once_per_iteration_and_ends_merge_in_order() {
        let (mut g, e, n, _, _) = rig();
        // slices 10,11,12 -> inc -> 11,12,13 -> sum 36
        assert_eq!(*g.cook(e, 0.0).expect("cook"), 36);
        assert_eq!(n.load(Ordering::SeqCst), 3);
        assert_eq!(*g.cook(e, 0.0).expect("cached"), 36);
        assert_eq!(n.load(Ordering::SeqCst), 3, "second cook is a cache hit");
    }

    #[test]
    fn dirty_propagates_through_the_block_and_begin_is_not_left_stale() {
        let (mut g, e, n, b, s) = rig();
        g.cook(e, 0.0).expect("cook");
        g.set_param(s, "value", ParamValue::I(20)).expect("set");
        assert!(g.is_dirty(e));
        assert_eq!(*g.cook(e, 0.0).expect("recook"), 21 + 22 + 23);
        assert_eq!(n.load(Ordering::SeqCst), 6);
        // Begin cooked outside the loop is the plain pass-through, not the last slice
        assert_eq!(*g.cook(b, 0.0).expect("begin"), 20);
        g.set_param(b, "n", ParamValue::I(0)).expect("n");
        assert_eq!(*g.cook(e, 0.0).expect("zero iterations"), 0);
    }

    #[test]
    fn feedback_is_carried_by_the_engine_not_by_an_edge() {
        let (mut g, e, _, b, _) = rig();
        g.set_param(e, "fb", ParamValue::B(true)).expect("fb");
        // the loop is not a graph edge: the only way back is rejected like any other cycle
        assert!(matches!(g.connect(e, 0, b, 0), Err(GraphError::Cycle(_))));
        let n = Arc::new(AtomicUsize::new(0));
        let mut h = Graph::new();
        let s = h.add_node(src(10), ParamSet::new()).expect("s");
        let b2 = h.add_node(begin(), ParamSet::new()).expect("b");
        let probe = h.add_node(Inc(n.clone()), ParamSet::new()).expect("i");
        let e2 = h.add_node(end(), ParamSet::new()).expect("e");
        h.connect(s, 0, b2, 0).expect("w");
        h.connect(b2, 0, probe, 0).expect("w");
        h.connect(probe, 0, e2, 0).expect("w");
        h.set_param(e2, "fb", ParamValue::B(true)).expect("fb");
        // src 10: it0 slice 10 -> 11 ; it1 slice 11+1 -> 13 ; it2 slice 13+2 -> 16 (output of i feeds begin of i+1)
        assert_eq!(*h.cook(e2, 0.0).expect("feedback cooks without a cycle error"), 16);
        assert_eq!(n.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn missing_begin_and_over_cap_and_bad_name_are_errors_not_panics() {
        let mut g: Graph<i32> = Graph::new();
        let s = g.add_node(src(1), ParamSet::new()).expect("s");
        let e = g.add_node(end(), ParamSet::new()).expect("e");
        g.connect(s, 0, e, 0).expect("w");
        assert!(matches!(g.cook(e, 0.0), Err(GraphError::Node { .. })), "no begin upstream");
        g.set_param(e, "begin", ParamValue::S("ghost".into())).expect("name");
        assert!(matches!(g.cook(e, 0.0), Err(GraphError::Node { .. })), "unknown begin name");
        let (mut g, e, _, b, _) = rig();
        g.set_param(b, "n", ParamValue::I(i32::MAX)).expect("n");
        assert!(matches!(g.cook(e, 0.0), Err(GraphError::Node { node, .. }) if node == b));
    }

    #[test]
    fn explicit_begin_name_pairs_nested_blocks() {
        // src -> outer(n=2) -> inner(n=3) -> inc -> innerEnd -> outerEnd.  outer sums (inner sums (src+o+i+1)).
        let n = Arc::new(AtomicUsize::new(0));
        let mut g = Graph::new();
        let s = g.add_node(src(0), ParamSet::new()).expect("s");
        let ob = g.add_node(BeginI(vec![ParamSpec::new("n", ParamValue::I(2))]), ParamSet::new()).expect("ob");
        let ib = g.add_node(begin(), ParamSet::new()).expect("ib");
        let inc = g.add_node(Inc(n.clone()), ParamSet::new()).expect("inc");
        let ie = g.add_node(end(), ParamSet::new()).expect("ie");
        let oe = g.add_node(end(), ParamSet::new()).expect("oe");
        for (a, b) in [(s, ob), (ob, ib), (ib, inc), (inc, ie), (ie, oe)] {
            g.connect(a, 0, b, 0).expect("w");
        }
        // auto pairing (no `begin` set): inner end -> ib, outer end jumps over the inner block -> ob
        // outer o=0: ib src 0 -> slices 0,1,2 -> +1 -> 1,2,3 = 6 ; o=1: src 1 -> 2,3,4 = 9 ; total 15
        assert_eq!(*g.cook(oe, 0.0).expect("nested"), 15);
        assert_eq!(n.load(Ordering::SeqCst), 6);
    }
}
