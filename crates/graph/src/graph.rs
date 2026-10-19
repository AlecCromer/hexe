//! `Graph<T>`: nodes + edges + cache. Cook = lazy (only what the target needs), cached (until dirty / time
//! change), dirty-propagated (`set_param`/`connect` clear every downstream cache), iterative (explicit stack).
use crate::node::{BlockRole, CookCtx, GraphError, Node, NodeId, Payload};
use crate::param::{ParamSet, ParamSpec, ParamValue};
use std::collections::HashMap;
use std::sync::Arc;

/// Node-count cap (capped allocations, AGENTS.md §4). `NodeId` is `u32`.
pub const MAX_NODES: usize = 1 << 20;
/// Frame rate handed to `CookCtx` unless `set_fps`.
pub const DEFAULT_FPS: f32 = 24.0;
/// Most iterations one For-Each block may run (capped allocations, AGENTS.md §4).
pub const MAX_BLOCK_ITERATIONS: usize = 1 << 16;
/// Deepest nesting of For-Each blocks cooked at once (bounded recursion, AGENTS.md §4).
pub const MAX_BLOCK_DEPTH: usize = 8;
mod block;
/// Longest node name accepted.
pub const MAX_NAME: usize = 256;

/// Cached output of one node. `sensitive` = depends on time (itself or any upstream).
struct Cached<T> {
    out: Arc<T>,
    time: f32,
    sensitive: bool,
}

struct Slot<T> {
    node: Box<dyn Node<T>>,
    params: ParamSet,
    /// One entry per input port of `node`.
    inputs: Vec<Option<NodeId>>,
    /// Downstream edges `(dst, dst input)`. Mirror of the `inputs` of other slots.
    outs: Vec<(NodeId, usize)>,
    /// `None` == dirty.
    cache: Option<Cached<T>>,
    /// Tombstone (`remove_node`): ids stay stable, the slot is invisible to every accessor.
    removed: bool,
    /// Bypass flag: the node does not run; its output is input 0 unchanged (other inputs are not needed / cooked).
    bypass: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    Active,
    Done,
}

pub struct Graph<T: Payload> {
    slots: Vec<Slot<T>>,
    /// Optional unique display names (unnamed = hidden/internal node).
    names: HashMap<NodeId, String>,
    display: Option<NodeId>,
    fps: f32,
    log: Arc<dyn Fn(&str) + Send + Sync>,
    /// Nesting level of the For-Each blocks currently being cooked (`block.rs`).
    block_depth: usize,
}

impl<T: Payload> Default for Graph<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Payload> Graph<T> {
    pub fn new() -> Self {
        Graph {
            slots: Vec::new(),
            names: HashMap::new(),
            display: None,
            fps: DEFAULT_FPS,
            log: Arc::new(|_| {}),
            block_depth: 0,
        }
    }

    /// Live (not removed) nodes.
    pub fn node_count(&self) -> usize {
        self.slots.iter().filter(|s| !s.removed).count()
    }
    pub fn fps(&self) -> f32 {
        self.fps
    }
    /// Non-finite or non-positive fps is ignored. Changes `CookCtx::fps`, so everything is dirtied.
    pub fn set_fps(&mut self, fps: f32) {
        if fps.is_finite() && fps > 0.0 && fps != self.fps {
            self.fps = fps;
            for s in &mut self.slots {
                s.cache = None;
            }
        }
    }
    pub fn set_logger(&mut self, log: impl Fn(&str) + Send + Sync + 'static) {
        self.log = Arc::new(log);
    }
    pub fn display(&self) -> Option<NodeId> {
        self.display
    }
    pub fn set_display(&mut self, id: Option<NodeId>) -> Result<(), GraphError> {
        if let Some(id) = id {
            self.slot(id)?;
        }
        self.display = id;
        Ok(())
    }

    /// Add a node; `overrides` replace spec defaults (validated: known name, same type, acceptable value).
    pub fn add_node<N: Node<T> + 'static>(&mut self, node: N, overrides: ParamSet) -> Result<NodeId, GraphError> {
        self.add_boxed(Box::new(node), overrides)
    }

    pub fn add_boxed(&mut self, node: Box<dyn Node<T>>, overrides: ParamSet) -> Result<NodeId, GraphError> {
        if self.slots.len() >= MAX_NODES {
            return Err(GraphError::TooManyNodes { max: MAX_NODES });
        }
        let id = NodeId(u32::try_from(self.slots.len()).map_err(|_| GraphError::TooManyNodes { max: MAX_NODES })?);
        let mut params = ParamSet::new();
        for spec in node.params() {
            params.set(&spec.name, spec.default.clone());
        }
        for (name, value) in overrides.iter() {
            check_param(id, node.params(), name, value)?;
            params.set(name, value.clone());
        }
        let inputs = vec![None; node.inputs().len()];
        self.slots.push(Slot { node, params, inputs, outs: Vec::new(), cache: None, removed: false, bypass: false });
        Ok(id)
    }

    /// Wire `src` output `out` into `dst` input `input` (replaces whatever was there). Rejects edges that
    /// would close a cycle. Dirties `dst` and everything downstream.
    pub fn connect(&mut self, src: NodeId, out: usize, dst: NodeId, input: usize) -> Result<(), GraphError> {
        self.link(src, out, dst, input, true)
    }

    /// Same as `connect` minus the cycle check. Houdini's network editor lets you wire a loop (the cook flags the
    /// loop's nodes as errors) while `hou` rejects it at connect, so the editor + scene files use this and scripts use
    /// `connect`. Safe: cook (`GraphError::Cycle`), dirty propagation and every walk terminate on cycles.
    pub fn connect_unchecked(&mut self, src: NodeId, out: usize, dst: NodeId, input: usize) -> Result<(), GraphError> {
        self.link(src, out, dst, input, false)
    }

    fn link(
        &mut self,
        src: NodeId,
        out: usize,
        dst: NodeId,
        input: usize,
        check_cycle: bool,
    ) -> Result<(), GraphError> {
        self.slot(src)?;
        let dst_inputs = self.slot(dst)?.inputs.len();
        // single output per node for now (`Node::cook` returns one payload)
        if out != 0 {
            return Err(GraphError::NoSuchOutput { node: src, output: out });
        }
        if input >= dst_inputs {
            return Err(GraphError::NoSuchInput { node: dst, input });
        }
        if check_cycle && (src == dst || self.reaches(dst, src)) {
            return Err(GraphError::Cycle(dst));
        }
        self.unlink(dst, input);
        if let Some(s) = self.live_mut(dst)
            && let Some(cell) = s.inputs.get_mut(input)
        {
            *cell = Some(src);
        }
        if let Some(s) = self.live_mut(src) {
            s.outs.push((dst, input));
        }
        self.invalidate(dst);
        Ok(())
    }

    /// Remove the edge into `dst` input `input` (no-op if unconnected). Dirties `dst` and downstream.
    pub fn disconnect(&mut self, dst: NodeId, input: usize) -> Result<(), GraphError> {
        let n = self.slot(dst)?.inputs.len();
        if input >= n {
            return Err(GraphError::NoSuchInput { node: dst, input });
        }
        self.unlink(dst, input);
        self.invalidate(dst);
        Ok(())
    }

    fn unlink(&mut self, dst: NodeId, input: usize) {
        let old = self.live_mut(dst).and_then(|s| s.inputs.get_mut(input)).and_then(Option::take);
        if let Some(old) = old
            && let Some(s) = self.live_mut(old)
        {
            s.outs.retain(|&e| e != (dst, input));
        }
    }

    /// Set a parameter; marks the node and everything downstream dirty. Same value → no-op (cache kept).
    pub fn set_param(&mut self, id: NodeId, name: &str, value: ParamValue) -> Result<(), GraphError> {
        let slot = self.slot(id)?;
        check_param(id, slot.node.params(), name, &value)?;
        if slot.params.get(name) == Some(&value) {
            return Ok(());
        }
        if let Some(s) = self.live_mut(id) {
            s.params.set(name, value);
        }
        self.invalidate(id);
        Ok(())
    }

    /// Bypass flag (Houdini semantics): a bypassed node outputs its input 0 unchanged and does not run.
    pub fn set_bypass(&mut self, id: NodeId, on: bool) -> Result<(), GraphError> {
        if self.slot(id)?.bypass == on {
            return Ok(());
        }
        if let Some(s) = self.live_mut(id) {
            s.bypass = on;
        }
        self.invalidate(id);
        Ok(())
    }

    pub fn bypass(&self, id: NodeId) -> bool {
        self.live(id).is_some_and(|s| s.bypass)
    }

    pub fn param(&self, id: NodeId, name: &str) -> Option<&ParamValue> {
        self.live(id)?.params.get(name)
    }

    /// Every node id in creation order (UI introspection).
    pub fn node_ids(&self) -> Vec<NodeId> {
        (0..self.slots.len())
            .filter_map(|i| u32::try_from(i).ok().map(NodeId))
            .filter(|&id| self.live(id).is_some())
            .collect()
    }
    /// Node type name, `None` for unknown ids.
    pub fn node_type(&self, id: NodeId) -> Option<&'static str> {
        self.live(id).map(|s| s.node.type_name())
    }
    /// Source node of each input port (`None` = unconnected).
    pub fn input_sources(&self, id: NodeId) -> Vec<Option<NodeId>> {
        self.live(id).map(|s| s.inputs.clone()).unwrap_or_default()
    }
    /// Parameter declarations of the node type (defaults, ranges).
    pub fn param_specs(&self, id: NodeId) -> Vec<ParamSpec> {
        self.live(id).map(|s| s.node.params().to_vec()).unwrap_or_default()
    }
    /// `true` when `cook(id, _)` would have to run the node (no cache). `false` for unknown ids.
    pub fn is_dirty(&self, id: NodeId) -> bool {
        self.live(id).is_some_and(|s| s.cache.is_none())
    }

    /// Houdini `needsToCook()` at `time`: no cache, or the cache is time-sensitive (the node or anything upstream is time dependent)
    /// and was cooked at another time. `false` for unknown ids.
    pub fn needs_to_cook(&self, id: NodeId, time: f32) -> bool {
        self.live(id)
            .is_some_and(|s| s.cache.as_ref().is_none_or(|c| c.sensitive && c.time.to_bits() != time.to_bits()))
    }

    /// Houdini `isTimeDependent()`: the node or any node upstream of it is time dependent (known before the first cook).
    /// Iterative, terminates on cycles. `false` for unknown ids.
    pub fn is_time_dependent(&self, id: NodeId) -> bool {
        let mut seen = vec![false; self.slots.len()];
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            let Some(flag) = seen.get_mut(n.slot()) else { continue };
            if std::mem::replace(flag, true) {
                continue;
            }
            let Some(s) = self.live(n) else { continue };
            if s.node.time_dependent() {
                return true;
            }
            stack.extend(s.inputs.iter().flatten().copied());
        }
        false
    }

    fn live(&self, id: NodeId) -> Option<&Slot<T>> {
        self.slots.get(id.slot()).filter(|s| !s.removed)
    }
    fn live_mut(&mut self, id: NodeId) -> Option<&mut Slot<T>> {
        self.slots.get_mut(id.slot()).filter(|s| !s.removed)
    }
    /// Remove a node: its input edges and every edge it feeds are cut (dependents are dirtied and left with an
    /// unconnected input; nothing is reconnected), its name and display flag are dropped. Other ids stay stable.
    pub fn remove_node(&mut self, id: NodeId) -> Result<(), GraphError> {
        let (ins, outs) = {
            let s = self.slot(id)?;
            (s.inputs.len(), s.outs.clone())
        };
        for i in 0..ins {
            self.unlink(id, i);
        }
        for (dst, input) in outs {
            self.unlink(dst, input);
            self.invalidate(dst);
        }
        if let Some(s) = self.slots.get_mut(id.slot()) {
            s.removed = true;
            s.cache = None;
            s.outs.clear();
        }
        self.names.remove(&id);
        if self.display == Some(id) {
            self.display = None;
        }
        Ok(())
    }
    /// Display name of a node (`None` = unnamed/hidden or unknown).
    pub fn name(&self, id: NodeId) -> Option<&str> {
        self.live(id).and(self.names.get(&id)).map(String::as_str)
    }
    /// Live node carrying exactly this name.
    pub fn node_by_name(&self, name: &str) -> Option<NodeId> {
        self.names.iter().find(|(id, n)| n.as_str() == name && self.live(**id).is_some()).map(|(id, _)| *id)
    }
    /// Name a node. Rejects empty / over-long names and names another live node already has.
    pub fn rename(&mut self, id: NodeId, name: &str) -> Result<(), GraphError> {
        self.slot(id)?;
        if name.is_empty() || name.len() > MAX_NAME {
            return Err(GraphError::BadName(name.chars().take(32).collect()));
        }
        if self.node_by_name(name).is_some_and(|other| other != id) {
            return Err(GraphError::NameTaken(name.to_string()));
        }
        self.names.insert(id, name.to_string());
        Ok(())
    }
    /// First free `<base>1`, `<base>2`, ... (Houdini-style auto name).
    pub fn unique_name(&self, base: &str) -> String {
        (1u32..).map(|n| format!("{base}{n}")).find(|c| self.node_by_name(c).is_none()).unwrap_or_default()
    }
    fn slot(&self, id: NodeId) -> Result<&Slot<T>, GraphError> {
        self.live(id).ok_or(GraphError::NoSuchNode(id))
    }

    /// Is `to` reachable from `from` following downstream edges? Iterative.
    fn reaches(&self, from: NodeId, to: NodeId) -> bool {
        let mut seen = vec![false; self.slots.len()];
        let mut stack = vec![from];
        while let Some(n) = stack.pop() {
            if n == to {
                return true;
            }
            let Some(flag) = seen.get_mut(n.slot()) else { continue };
            if std::mem::replace(flag, true) {
                continue;
            }
            if let Some(s) = self.live(n) {
                stack.extend(s.outs.iter().map(|&(d, _)| d));
            }
        }
        false
    }

    /// Clear the cache of `start` and every node downstream of it. Iterative, terminates on cycles.
    fn invalidate(&mut self, start: NodeId) {
        let mut seen = vec![false; self.slots.len()];
        let mut stack = vec![start];
        while let Some(n) = stack.pop() {
            let Some(flag) = seen.get_mut(n.slot()) else { continue };
            if std::mem::replace(flag, true) {
                continue;
            }
            if let Some(s) = self.live_mut(n) {
                s.cache = None;
                stack.extend(s.outs.iter().map(|&(d, _)| d));
            }
        }
    }

    fn cached(&self, id: NodeId, time: f32) -> Option<&Cached<T>> {
        let c = self.live(id)?.cache.as_ref()?;
        (!c.sensitive || c.time.to_bits() == time.to_bits()).then_some(c)
    }

    /// Cook `id` at `time`: returns the cached output unless dirty (or time-sensitive and `time` changed).
    /// Only the upstream cone that is actually dirty runs; shared (diamond) nodes cook once.
    pub fn cook(&mut self, id: NodeId, time: f32) -> Result<Arc<T>, GraphError> {
        self.slot(id)?;
        if let Some(c) = self.cached(id, time) {
            return Ok(Arc::clone(&c.out));
        }
        let mut visit: HashMap<NodeId, Visit> = HashMap::new();
        // (node, children already scheduled?)
        let mut stack: Vec<(NodeId, bool)> = vec![(id, false)];
        while let Some((cur, expanded)) = stack.pop() {
            if !expanded {
                match visit.get(&cur) {
                    Some(Visit::Done) => continue,
                    // popped while its own subtree is still being cooked → reachable from itself
                    Some(Visit::Active) => return Err(GraphError::Cycle(cur)),
                    None => {}
                }
                if self.cached(cur, time).is_some() {
                    visit.insert(cur, Visit::Done);
                    continue;
                }
                let slot = self.slot(cur)?;
                if slot.bypass {
                    // bypassed: only input 0 is cooked (it becomes the output)
                    let src = slot
                        .inputs
                        .first()
                        .copied()
                        .flatten()
                        .ok_or(GraphError::MissingInput { node: cur, input: 0 })?;
                    visit.insert(cur, Visit::Active);
                    stack.push((cur, true));
                    stack.push((src, false));
                    continue;
                }
                if slot.node.block_role() == BlockRole::End {
                    // For-Each: the loop is not an edge. The engine cooks the Begin..End sub-network once per
                    // iteration itself (`block.rs`), so the End node is a leaf here: no cycle, no input walk.
                    self.cook_block_end(cur, time)?;
                    visit.insert(cur, Visit::Done);
                    continue;
                }
                visit.insert(cur, Visit::Active);
                stack.push((cur, true));
                for (i, src) in slot.inputs.iter().enumerate() {
                    match src {
                        Some(src) => stack.push((*src, false)),
                        None => return Err(GraphError::MissingInput { node: cur, input: i }),
                    }
                }
            } else {
                self.cook_one(cur, time)?;
                visit.insert(cur, Visit::Done);
            }
        }
        self.cached(id, time).map(|c| Arc::clone(&c.out)).ok_or(GraphError::Internal("target has no cache after cook"))
    }

    /// Run one node whose inputs are all cached; store the result.
    fn cook_one(&mut self, id: NodeId, time: f32) -> Result<(), GraphError> {
        let slot = self.slot(id)?;
        if slot.bypass {
            let src = slot.inputs.first().copied().flatten().ok_or(GraphError::MissingInput { node: id, input: 0 })?;
            let c = self.cached(src, time).ok_or(GraphError::Internal("bypass input cache missing at cook"))?;
            let (out, sensitive) = (Arc::clone(&c.out), c.sensitive);
            if let Some(s) = self.live_mut(id) {
                s.cache = Some(Cached { out, time, sensitive });
            }
            return Ok(());
        }
        let mut ins: Vec<Arc<T>> = Vec::with_capacity(slot.inputs.len());
        let mut sensitive = slot.node.time_dependent();
        for src in &slot.inputs {
            let src = src.ok_or(GraphError::Internal("unconnected input reached cook"))?;
            let c = self.cached(src, time).ok_or(GraphError::Internal("input cache missing at cook"))?;
            sensitive |= c.sensitive;
            ins.push(Arc::clone(&c.out));
        }
        let ctx = CookCtx { time, fps: self.fps, params: &slot.params, log: &*self.log };
        let out = slot.node.cook(&ctx, &ins).map_err(|source| GraphError::Node { node: id, source })?;
        if let Some(s) = self.live_mut(id) {
            s.cache = Some(Cached { out, time, sensitive });
        }
        Ok(())
    }
}

fn check_param(
    node: NodeId,
    specs: &[crate::param::ParamSpec],
    name: &str,
    value: &ParamValue,
) -> Result<(), GraphError> {
    let spec = specs
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| GraphError::NoSuchParam { node, name: name.to_string() })?;
    if !spec.default.same_kind(value) {
        return Err(GraphError::ParamType { node, name: name.to_string() });
    }
    if !value.is_acceptable() {
        return Err(GraphError::BadParamValue { node, name: name.to_string() });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{CookError, InputSpec};
    use crate::param::ParamSpec;
    use std::sync::atomic::{AtomicUsize, Ordering};

    type Counter = Arc<AtomicUsize>;
    fn counter() -> Counter {
        Arc::new(AtomicUsize::new(0))
    }
    fn count(c: &Counter) -> usize {
        c.load(Ordering::SeqCst)
    }

    /// Dummy payload node: outputs param `value`.
    struct Const {
        n: Counter,
        specs: Vec<ParamSpec>,
    }
    fn konst(n: &Counter) -> Const {
        Const { n: n.clone(), specs: vec![ParamSpec::new("value", ParamValue::I(0))] }
    }
    impl Node<i32> for Const {
        fn type_name(&self) -> &'static str {
            "const"
        }
        fn inputs(&self) -> &[InputSpec] {
            &[]
        }
        fn params(&self) -> &[ParamSpec] {
            &self.specs
        }
        fn cook(&self, ctx: &CookCtx, _: &[Arc<i32>]) -> Result<Arc<i32>, CookError> {
            self.n.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(ctx.params.int("value").unwrap_or_default()))
        }
    }

    /// Dummy: in0 + in1.
    struct Add(Counter);
    impl Node<i32> for Add {
        fn type_name(&self) -> &'static str {
            "add"
        }
        fn inputs(&self) -> &[InputSpec] {
            &[InputSpec { name: "a" }, InputSpec { name: "b" }]
        }
        fn params(&self) -> &[ParamSpec] {
            &[]
        }
        fn cook(&self, _: &CookCtx, i: &[Arc<i32>]) -> Result<Arc<i32>, CookError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            match i {
                [a, b] => a.checked_add(**b).map(Arc::new).ok_or_else(|| CookError::Failed("overflow".into())),
                _ => Err(CookError::Failed("arity".into())),
            }
        }
    }

    /// Dummy: in0 + param `by`.
    struct Inc {
        n: Counter,
        specs: Vec<ParamSpec>,
    }
    fn inc(n: &Counter) -> Inc {
        Inc { n: n.clone(), specs: vec![ParamSpec::new("by", ParamValue::I(1))] }
    }
    impl Node<i32> for Inc {
        fn type_name(&self) -> &'static str {
            "inc"
        }
        fn inputs(&self) -> &[InputSpec] {
            &[InputSpec { name: "in" }]
        }
        fn params(&self) -> &[ParamSpec] {
            &self.specs
        }
        fn cook(&self, ctx: &CookCtx, i: &[Arc<i32>]) -> Result<Arc<i32>, CookError> {
            self.n.fetch_add(1, Ordering::SeqCst);
            let by = ctx.params.int("by").ok_or_else(|| CookError::BadParam("by".into()))?;
            i.first()
                .and_then(|v| v.checked_add(by))
                .map(Arc::new)
                .ok_or_else(|| CookError::Failed("overflow or arity".into()))
        }
    }

    /// Dummy: outputs `ctx.time` as whole frames; time-dependent.
    struct Clock(Counter);
    impl Node<i32> for Clock {
        fn type_name(&self) -> &'static str {
            "clock"
        }
        fn inputs(&self) -> &[InputSpec] {
            &[]
        }
        fn params(&self) -> &[ParamSpec] {
            &[]
        }
        fn time_dependent(&self) -> bool {
            true
        }
        fn cook(&self, ctx: &CookCtx, _: &[Arc<i32>]) -> Result<Arc<i32>, CookError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(ctx.time as i32))
        }
    }

    fn val(v: i32) -> ParamSet {
        ParamSet::new().with("value", ParamValue::I(v))
    }

    #[test]
    fn bypass_passes_input_zero_through() {
        let (ca, cb, cadd) = (counter(), counter(), counter());
        let mut g: Graph<i32> = Graph::new();
        let a = g.add_node(konst(&ca), val(5)).unwrap();
        let b = g.add_node(inc(&cb), ParamSet::new().with("by", ParamValue::I(10))).unwrap();
        g.connect(a, 0, b, 0).unwrap();
        assert_eq!(*g.cook(b, 0.0).unwrap(), 15);
        g.set_bypass(b, true).unwrap();
        assert!(g.bypass(b) && g.is_dirty(b), "toggling bypass dirties the node");
        let out = g.cook(b, 0.0).unwrap();
        assert_eq!(*out, 5, "bypassed: input 0 unchanged");
        assert!(Arc::ptr_eq(&out, &g.cook(a, 0.0).unwrap()), "pass-through shares the input Arc");
        assert_eq!(count(&cb), 1, "a bypassed node never runs");
        // only input 0 is needed: a bypassed two-input node with input 1 unconnected still cooks
        let add = g.add_node(Add(cadd.clone()), ParamSet::new()).unwrap();
        g.connect(a, 0, add, 0).unwrap();
        assert!(g.cook(add, 0.0).is_err(), "not bypassed: input 1 missing");
        g.set_bypass(add, true).unwrap();
        assert_eq!(*g.cook(add, 0.0).unwrap(), 5);
        assert_eq!(count(&cadd), 0);
        // nothing on input 0: an error, never a panic
        let lone = g.add_node(inc(&counter()), ParamSet::new()).unwrap();
        g.set_bypass(lone, true).unwrap();
        assert!(matches!(g.cook(lone, 0.0), Err(GraphError::MissingInput { input: 0, .. })));
        // un-bypass recooks
        g.set_bypass(b, false).unwrap();
        assert_eq!(*g.cook(b, 0.0).unwrap(), 15);
        assert!(g.set_bypass(NodeId(9999), true).is_err());
    }
    #[test]
    fn cook_once_then_cached_and_lazy() {
        let (ca, cb, cside) = (counter(), counter(), counter());
        let mut g: Graph<i32> = Graph::new();
        let a = g.add_node(konst(&ca), val(5)).unwrap();
        let b = g.add_node(inc(&cb), ParamSet::new().with("by", ParamValue::I(10))).unwrap();
        let side = g.add_node(konst(&cside), val(99)).unwrap();
        g.connect(a, 0, b, 0).unwrap();
        assert!(g.is_dirty(b));
        let first = g.cook(b, 0.0).unwrap();
        assert_eq!(*first, 15);
        assert!(!g.is_dirty(b));
        let second = g.cook(b, 0.0).unwrap();
        assert!(Arc::ptr_eq(&first, &second), "cached output is the same Arc (COW share)");
        assert_eq!((count(&ca), count(&cb)), (1, 1));
        // lazy: a node nobody asked for never cooks
        assert_eq!(count(&cside), 0);
        assert!(g.is_dirty(side));
        // asking for the upstream node alone is also a cache hit
        g.cook(a, 0.0).unwrap();
        assert_eq!(count(&ca), 1);
    }

    #[test]
    fn set_param_recooks_only_downstream() {
        let (ca, cb, cadd, cinc, cbinc) = (counter(), counter(), counter(), counter(), counter());
        let mut g: Graph<i32> = Graph::new();
        let a = g.add_node(konst(&ca), val(1)).unwrap();
        let b = g.add_node(konst(&cb), val(2)).unwrap();
        let add = g.add_node(Add(cadd.clone()), ParamSet::new()).unwrap();
        let after = g.add_node(inc(&cinc), ParamSet::new()).unwrap(); // downstream of a+b
        let b_only = g.add_node(inc(&cbinc), ParamSet::new()).unwrap(); // downstream of b only
        g.connect(a, 0, add, 0).unwrap();
        g.connect(b, 0, add, 1).unwrap();
        g.connect(add, 0, after, 0).unwrap();
        g.connect(b, 0, b_only, 0).unwrap();
        assert_eq!(*g.cook(after, 0.0).unwrap(), 4);
        assert_eq!(*g.cook(b_only, 0.0).unwrap(), 3);
        let before = (count(&ca), count(&cb), count(&cadd), count(&cinc), count(&cbinc));
        assert_eq!(before, (1, 1, 1, 1, 1));

        g.set_param(a, "value", ParamValue::I(10)).unwrap();
        assert!(g.is_dirty(a) && g.is_dirty(add) && g.is_dirty(after));
        assert!(!g.is_dirty(b) && !g.is_dirty(b_only), "siblings/other branches stay clean");
        assert_eq!(*g.cook(after, 0.0).unwrap(), 13);
        assert_eq!(*g.cook(b_only, 0.0).unwrap(), 3);
        let after_counts = (count(&ca), count(&cb), count(&cadd), count(&cinc), count(&cbinc));
        assert_eq!(after_counts, (2, 1, 2, 2, 1), "only a, add, after re-cooked");

        // same value → nothing dirtied
        g.set_param(a, "value", ParamValue::I(10)).unwrap();
        assert!(!g.is_dirty(after));
    }

    #[test]
    fn connect_and_disconnect_dirty_downstream() {
        let (ca, cb, cinc) = (counter(), counter(), counter());
        let mut g: Graph<i32> = Graph::new();
        let a = g.add_node(konst(&ca), val(1)).unwrap();
        let b = g.add_node(konst(&cb), val(100)).unwrap();
        let i = g.add_node(inc(&cinc), ParamSet::new()).unwrap();
        g.connect(a, 0, i, 0).unwrap();
        assert_eq!(*g.cook(i, 0.0).unwrap(), 2);
        g.connect(b, 0, i, 0).unwrap(); // rewire
        assert!(g.is_dirty(i));
        assert_eq!(*g.cook(i, 0.0).unwrap(), 101);
        g.set_param(a, "value", ParamValue::I(7)).unwrap(); // a no longer feeds i
        assert!(!g.is_dirty(i));
        g.disconnect(i, 0).unwrap();
        assert!(g.is_dirty(i));
        assert!(matches!(g.cook(i, 0.0), Err(GraphError::MissingInput { .. })));
    }

    #[test]
    fn diamond_shared_node_cooks_once() {
        let (croot, cl, cr, cadd) = (counter(), counter(), counter(), counter());
        let mut g: Graph<i32> = Graph::new();
        let root = g.add_node(konst(&croot), val(3)).unwrap();
        let l = g.add_node(inc(&cl), ParamSet::new().with("by", ParamValue::I(1))).unwrap();
        let r = g.add_node(inc(&cr), ParamSet::new().with("by", ParamValue::I(2))).unwrap();
        let add = g.add_node(Add(cadd.clone()), ParamSet::new()).unwrap();
        g.connect(root, 0, l, 0).unwrap();
        g.connect(root, 0, r, 0).unwrap();
        g.connect(l, 0, add, 0).unwrap();
        g.connect(r, 0, add, 1).unwrap();
        assert_eq!(*g.cook(add, 0.0).unwrap(), 4 + 5);
        assert_eq!((count(&croot), count(&cl), count(&cr), count(&cadd)), (1, 1, 1, 1));
        // dirtying the shared root re-cooks the whole diamond exactly once each
        g.set_param(root, "value", ParamValue::I(10)).unwrap();
        assert_eq!(*g.cook(add, 0.0).unwrap(), 11 + 12);
        assert_eq!((count(&croot), count(&cl), count(&cr), count(&cadd)), (2, 2, 2, 2));
    }

    #[test]
    fn cycle_is_error_not_panic() {
        let mut g: Graph<i32> = Graph::new();
        let (c0, c1) = (counter(), counter());
        let a = g.add_node(inc(&c0), ParamSet::new()).unwrap();
        let b = g.add_node(inc(&c1), ParamSet::new()).unwrap();
        // connect-time: self loop and 2-cycle are rejected, graph stays usable
        assert!(matches!(g.connect(a, 0, a, 0), Err(GraphError::Cycle(_))));
        g.connect(a, 0, b, 0).unwrap();
        assert!(matches!(g.connect(b, 0, a, 0), Err(GraphError::Cycle(_))));
        // cook-time (graph that arrived already cyclic): Err, no hang, no panic, nothing cooked
        g.connect_unchecked(b, 0, a, 0).unwrap();
        assert!(matches!(g.cook(b, 0.0), Err(GraphError::Cycle(_))));
        assert!(matches!(g.cook(a, 0.0), Err(GraphError::Cycle(_))));
        assert_eq!((count(&c0), count(&c1)), (0, 0));
        // dirty propagation terminates on the cycle
        g.set_param(a, "by", ParamValue::I(5)).unwrap();
        // breaking the cycle makes it cookable again
        g.disconnect(a, 0).unwrap();
        assert!(matches!(g.cook(b, 0.0), Err(GraphError::MissingInput { .. })));
    }

    #[test]
    fn missing_input_is_error() {
        let mut g: Graph<i32> = Graph::new();
        let ca = counter();
        let a = g.add_node(konst(&ca), val(1)).unwrap();
        let add = g.add_node(Add(counter()), ParamSet::new()).unwrap();
        g.connect(a, 0, add, 0).unwrap(); // input 1 left open
        assert_eq!(g.cook(add, 0.0), Err(GraphError::MissingInput { node: add, input: 1 }));
    }

    #[test]
    fn bad_ids_ports_and_params_are_errors() {
        let mut g: Graph<i32> = Graph::new();
        let a = g.add_node(konst(&counter()), ParamSet::new()).unwrap();
        let i = g.add_node(inc(&counter()), ParamSet::new()).unwrap();
        let ghost = NodeId(77);
        assert_eq!(g.cook(ghost, 0.0), Err(GraphError::NoSuchNode(ghost)));
        assert_eq!(g.connect(ghost, 0, i, 0), Err(GraphError::NoSuchNode(ghost)));
        assert_eq!(g.connect(a, 1, i, 0), Err(GraphError::NoSuchOutput { node: a, output: 1 }));
        assert_eq!(g.connect(a, 0, i, 3), Err(GraphError::NoSuchInput { node: i, input: 3 }));
        assert!(matches!(g.set_param(a, "nope", ParamValue::I(1)), Err(GraphError::NoSuchParam { .. })));
        assert!(matches!(g.set_param(a, "value", ParamValue::F(1.0)), Err(GraphError::ParamType { .. })));
        assert!(matches!(g.set_param(i, "by", ParamValue::S("x".into())), Err(GraphError::ParamType { .. })));
        assert!(g.add_node(konst(&counter()), ParamSet::new().with("zzz", ParamValue::I(1))).is_err());
        let nan = ParamSpec::new("f", ParamValue::F(0.0));
        assert!(nan.default.is_acceptable() && !ParamValue::F(f32::NAN).is_acceptable());
        assert!(!ParamValue::V3([0.0, f32::INFINITY, 0.0]).is_acceptable());
    }

    #[test]
    fn node_cook_failure_is_reported_and_not_cached() {
        let mut g: Graph<i32> = Graph::new();
        let a = g.add_node(konst(&counter()), val(i32::MAX)).unwrap();
        let i = g.add_node(inc(&counter()), ParamSet::new()).unwrap();
        g.connect(a, 0, i, 0).unwrap();
        assert!(matches!(g.cook(i, 0.0), Err(GraphError::Node { node, .. }) if node == i));
        assert!(g.is_dirty(i));
        g.set_param(a, "value", ParamValue::I(1)).unwrap();
        assert_eq!(*g.cook(i, 0.0).unwrap(), 2);
    }

    #[test]
    fn time_dependent_cooks_again_when_time_changes() {
        let (cclock, cinc) = (counter(), counter());
        let mut g: Graph<i32> = Graph::new();
        let clock = g.add_node(Clock(cclock.clone()), ParamSet::new()).unwrap();
        let i = g.add_node(inc(&cinc), ParamSet::new()).unwrap();
        g.connect(clock, 0, i, 0).unwrap();
        assert_eq!(*g.cook(i, 3.0).unwrap(), 4);
        assert_eq!(*g.cook(i, 3.0).unwrap(), 4);
        assert_eq!((count(&cclock), count(&cinc)), (1, 1), "same time → cached");
        assert_eq!(*g.cook(i, 9.0).unwrap(), 10);
        assert_eq!((count(&cclock), count(&cinc)), (2, 2), "time changed → time-sensitive cone re-cooks");
    }

    #[test]
    fn chain_1000_cooks_without_stack_overflow() {
        let mut g: Graph<i32> = Graph::new();
        let mut prev = g.add_node(konst(&counter()), val(0)).unwrap();
        for _ in 0..999 {
            let n = g.add_node(inc(&counter()), ParamSet::new()).unwrap();
            g.connect(prev, 0, n, 0).unwrap();
            prev = n;
        }
        assert_eq!(g.node_count(), 1000);
        assert_eq!(*g.cook(prev, 0.0).unwrap(), 999);
    }

    #[test]
    fn chain_100k_dirty_and_cook_stay_iterative() {
        let mut g: Graph<i32> = Graph::new();
        let head = g.add_node(konst(&counter()), val(0)).unwrap();
        let mut prev = head;
        for _ in 0..100_000 {
            let n = g.add_node(inc(&counter()), ParamSet::new()).unwrap();
            g.connect(prev, 0, n, 0).unwrap();
            prev = n;
        }
        assert_eq!(*g.cook(prev, 0.0).unwrap(), 100_000);
        g.set_param(head, "value", ParamValue::I(5)).unwrap(); // propagates 100k deep
        assert!(g.is_dirty(prev));
        assert_eq!(*g.cook(prev, 0.0).unwrap(), 100_005);
    }

    #[test]
    fn display_and_logger_and_fps() {
        let mut g: Graph<i32> = Graph::new();
        let a = g.add_node(konst(&counter()), ParamSet::new()).unwrap();
        assert_eq!(g.display(), None);
        g.set_display(Some(a)).unwrap();
        assert_eq!(g.display(), Some(a));
        assert!(g.set_display(Some(NodeId(5))).is_err());
        assert_eq!(g.fps(), DEFAULT_FPS);
        g.cook(a, 0.0).unwrap();
        g.set_fps(f32::NAN);
        assert!(!g.is_dirty(a), "hostile fps ignored");
        g.set_fps(30.0);
        assert!(g.is_dirty(a), "fps change dirties everything");
    }

    #[test]
    fn add_connect_disconnect_edit_cycle() {
        let (ca, ci) = (counter(), counter());
        let mut g: Graph<i32> = Graph::new();
        let a = g.add_node(konst(&ca), val(4)).unwrap();
        let i = g.add_node(inc(&ci), ParamSet::new()).unwrap();
        assert!(matches!(g.cook(i, 0.0), Err(GraphError::MissingInput { .. })));
        g.connect(a, 0, i, 0).unwrap();
        assert_eq!(*g.cook(i, 0.0).unwrap(), 5);
        g.disconnect(i, 0).unwrap();
        assert!(g.is_dirty(i) && g.input_sources(i) == vec![None]);
        assert!(matches!(g.cook(i, 0.0), Err(GraphError::MissingInput { .. })));
        assert!(g.disconnect(i, 3).is_err(), "no such input");
        g.connect(a, 0, i, 0).unwrap();
        let j = g.add_node(inc(&counter()), ParamSet::new()).unwrap();
        g.connect(i, 0, j, 0).unwrap();
        assert!(matches!(g.connect(j, 0, i, 0), Err(GraphError::Cycle(_))));
        assert_eq!(g.input_sources(i), vec![Some(a)], "rejected edge left no trace");
    }

    #[test]
    fn remove_node_cuts_edges_and_keeps_ids() {
        let (ca, ci, cj) = (counter(), counter(), counter());
        let mut g: Graph<i32> = Graph::new();
        let a = g.add_node(konst(&ca), val(1)).unwrap();
        let i = g.add_node(inc(&ci), ParamSet::new()).unwrap();
        let j = g.add_node(inc(&cj), ParamSet::new()).unwrap();
        g.connect(a, 0, i, 0).unwrap();
        g.connect(i, 0, j, 0).unwrap();
        g.rename(i, "mid").unwrap();
        g.set_display(Some(i)).unwrap();
        assert_eq!(*g.cook(j, 0.0).unwrap(), 3);
        g.remove_node(i).unwrap();
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.node_ids(), vec![a, j], "ids of survivors are stable");
        assert_eq!(g.display(), None);
        assert_eq!((g.name(i), g.node_type(i)), (None, None));
        assert!(g.node_by_name("mid").is_none());
        assert_eq!(g.input_sources(j), vec![None], "dependent left unconnected, nothing reconnected");
        assert!(g.is_dirty(j));
        assert!(matches!(g.cook(j, 0.0), Err(GraphError::MissingInput { .. })));
        assert!(matches!(g.cook(i, 0.0), Err(GraphError::NoSuchNode(_))));
        assert!(g.remove_node(i).is_err(), "double remove");
        assert!(g.connect(a, 0, i, 0).is_err() && g.set_param(i, "by", ParamValue::I(1)).is_err());
        let k = g.add_node(inc(&counter()), ParamSet::new()).unwrap();
        assert_ne!(k, i, "ids are never reused");
        g.connect(a, 0, j, 0).unwrap();
        assert_eq!(*g.cook(j, 0.0).unwrap(), 2);
    }

    #[test]
    fn names_unique_rename_and_auto() {
        let mut g: Graph<i32> = Graph::new();
        let a = g.add_node(konst(&counter()), ParamSet::new()).unwrap();
        let b = g.add_node(konst(&counter()), ParamSet::new()).unwrap();
        assert_eq!(g.unique_name("const"), "const1");
        g.rename(a, "const1").unwrap();
        assert_eq!(g.unique_name("const"), "const2");
        assert!(matches!(g.rename(b, "const1"), Err(GraphError::NameTaken(_))));
        assert!(matches!(g.rename(b, ""), Err(GraphError::BadName(_))));
        assert!(g.rename(b, &"x".repeat(MAX_NAME + 1)).is_err());
        g.rename(a, "const1").unwrap(); // own name again: fine
        g.rename(a, "renamed").unwrap();
        g.rename(b, "const1").unwrap(); // freed by the rename
        assert_eq!((g.node_by_name("renamed"), g.name(b)), (Some(a), Some("const1")));
        g.remove_node(a).unwrap();
        g.rename(b, "renamed").unwrap(); // removed node's name is free again
        assert!(g.rename(NodeId(77), "z").is_err());
    }
}
