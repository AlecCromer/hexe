//! `Node` trait, `CookCtx`, ids, errors. Payload-generic: see `Payload`.
use crate::param::{ParamSet, ParamSpec};
use std::fmt;
use std::sync::Arc;

/// What a node cooks. DESIGN.md says `Arc<Detail>`; `hexe-geo` is built in parallel (wave 1), so the
/// engine is generic over any `Payload`. Blanket-implemented: wave 2 uses `Graph<hexe_geo::Detail>`
/// directly, no swap needed (only the `Cargo.toml` dep lives in the consumer crate).
pub trait Payload: Send + Sync + 'static {}
impl<T: Send + Sync + 'static> Payload for T {}

/// Stable handle of a node inside one `Graph`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub(crate) u32);

impl NodeId {
    pub fn index(self) -> u32 {
        self.0
    }
    pub(crate) fn slot(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "node#{}", self.0)
    }
}

/// Declaration of one input port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputSpec {
    pub name: &'static str,
}

/// Everything a node may read while cooking. Nodes never see the graph (no upstream mutation).
pub struct CookCtx<'a> {
    pub time: f32,
    pub fps: f32,
    pub params: &'a ParamSet,
    pub log: &'a dyn Fn(&str),
}

/// A node's own failure (bad param, bad input data, arithmetic overflow…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CookError {
    Failed(String),
    BadParam(String),
    /// A valid request for a feature this port does not implement yet (never a silent fallback).
    Unsupported(String),
}

impl fmt::Display for CookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CookError::Failed(m) => write!(f, "cook failed: {m}"),
            CookError::BadParam(m) => write!(f, "bad parameter: {m}"),
            CookError::Unsupported(m) => write!(f, "unsupported: {m}"),
        }
    }
}
impl std::error::Error for CookError {}

/// Role of a node in a For-Each block (`block_begin` / `block_end` pair). See `Graph::cook` (engine side, `graph/block.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlockRole {
    /// Ordinary node.
    #[default]
    None,
    /// Emits one slice per iteration (`Node::block_count` / `Node::cook_block`).
    Begin,
    /// Owns the loop: the engine cooks the sub-network between its Begin and this node once per iteration, then calls
    /// `Node::block_merge`. Convention: string param `begin` = name of the paired Begin node (empty = nearest upstream Begin).
    End,
}
/// Which iteration a Begin node is cooking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockIter {
    pub index: usize,
    pub count: usize,
    /// `true` = feedback loop: the Begin input is the previous iteration's result (iteration 0: the original source).
    pub feedback: bool,
}
/// A node type. `inputs.len()` of the slice handed to `cook` == `self.inputs().len()`, in port order.
pub trait Node<T: Payload>: Send + Sync {
    fn type_name(&self) -> &'static str;
    fn inputs(&self) -> &[InputSpec];
    /// Inputs `0..min_inputs()` must be wired for the node to mean anything (the editor draws their wires solid and marks
    /// an unwired one with a red arrow); the rest are optional (dashed wires). Default: every input is required.
    fn min_inputs(&self) -> usize {
        self.inputs().len()
    }
    fn params(&self) -> &[ParamSpec];
    /// `true` → output depends on `ctx.time`; cache is keyed on time (and so is everything downstream).
    fn time_dependent(&self) -> bool {
        false
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<T>]) -> Result<Arc<T>, CookError>;
    /// For-Each role (default: none). Only block nodes override the four `block_*` methods.
    fn block_role(&self) -> BlockRole {
        BlockRole::None
    }
    /// Begin: number of iterations for `source` (the cooked input 0).
    fn block_count(&self, _ctx: &CookCtx, _source: &Arc<T>) -> Result<usize, CookError> {
        Err(CookError::Unsupported(format!("{} is not a block begin", self.type_name())))
    }
    /// Begin: the slice for iteration `iter.index` (`inputs` = `[source]`, or `[previous result]` in feedback mode).
    fn cook_block(&self, _ctx: &CookCtx, _inputs: &[Arc<T>], _iter: BlockIter) -> Result<Arc<T>, CookError> {
        Err(CookError::Unsupported(format!("{} is not a block begin", self.type_name())))
    }
    /// End: `true` = feedback mode (iteration i output feeds Begin of i+1, only the last result is merged).
    fn block_feedback(&self, _ctx: &CookCtx) -> bool {
        false
    }
    /// End: combine the per-iteration results (feedback: `[last]`, or `[source]` when there were 0 iterations).
    fn block_merge(&self, _ctx: &CookCtx, _results: &[Arc<T>]) -> Result<Arc<T>, CookError> {
        Err(CookError::Unsupported(format!("{} is not a block end", self.type_name())))
    }
}

/// Structural / scheduling failure of a `Graph` operation. Never a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    NoSuchNode(NodeId),
    NoSuchInput {
        node: NodeId,
        input: usize,
    },
    NoSuchOutput {
        node: NodeId,
        output: usize,
    },
    NoSuchParam {
        node: NodeId,
        name: String,
    },
    ParamType {
        node: NodeId,
        name: String,
    },
    BadParamValue {
        node: NodeId,
        name: String,
    },
    /// Edge would close (or cook found) a cycle; carries a node on it.
    Cycle(NodeId),
    MissingInput {
        node: NodeId,
        input: usize,
    },
    /// Empty or over-long node name.
    BadName(String),
    /// Another live node already has this name.
    NameTaken(String),
    TooManyNodes {
        max: usize,
    },
    Node {
        node: NodeId,
        source: CookError,
    },
    /// Engine invariant broken (a cache vanished mid-cook). Reported, never panicked.
    Internal(&'static str),
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphError::NoSuchNode(n) => write!(f, "no such node {n}"),
            GraphError::NoSuchInput { node, input } => write!(f, "{node} has no input {input}"),
            GraphError::NoSuchOutput { node, output } => write!(f, "{node} has no output {output}"),
            GraphError::NoSuchParam { node, name } => write!(f, "{node} has no parameter `{name}`"),
            GraphError::ParamType { node, name } => write!(f, "{node} parameter `{name}`: wrong value type"),
            GraphError::BadParamValue { node, name } => write!(f, "{node} parameter `{name}`: value rejected"),
            GraphError::Cycle(n) => write!(f, "cycle through {n}"),
            GraphError::MissingInput { node, input } => write!(f, "{node} input {input} is not connected"),
            GraphError::BadName(n) => write!(f, "bad node name `{n}`"),
            GraphError::NameTaken(n) => write!(f, "node name `{n}` is taken"),
            GraphError::TooManyNodes { max } => write!(f, "graph is capped at {max} nodes"),
            GraphError::Node { node, source } => write!(f, "{node}: {source}"),
            GraphError::Internal(m) => write!(f, "internal engine error: {m}"),
        }
    }
}

impl std::error::Error for GraphError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GraphError::Node { source, .. } => Some(source),
            _ => None,
        }
    }
}
