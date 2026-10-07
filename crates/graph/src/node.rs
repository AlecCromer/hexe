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
}

impl fmt::Display for CookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CookError::Failed(m) => write!(f, "cook failed: {m}"),
            CookError::BadParam(m) => write!(f, "bad parameter: {m}"),
        }
    }
}
impl std::error::Error for CookError {}

/// A node type. `inputs.len()` of the slice handed to `cook` == `self.inputs().len()`, in port order.
pub trait Node<T: Payload>: Send + Sync {
    fn type_name(&self) -> &'static str;
    fn inputs(&self) -> &[InputSpec];
    fn params(&self) -> &[ParamSpec];
    /// `true` → output depends on `ctx.time`; cache is keyed on time (and so is everything downstream).
    fn time_dependent(&self) -> bool {
        false
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<T>]) -> Result<Arc<T>, CookError>;
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
