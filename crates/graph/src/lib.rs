//! hexe graph (L1): node graph + lazy, cached, dirty-propagated cook engine (DESIGN.md `crates/graph contract`).
//! Payload-generic (`Graph<T>`): no dependency on `hexe-geo`. Hostile input is the default (AGENTS.md §4):
//! cycles / missing inputs / bad params are `Err`, cook is iterative (no recursion depth to blow).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod graph;
mod node;
mod param;

pub use graph::{DEFAULT_FPS, Graph, MAX_NODES};
pub use node::{CookCtx, CookError, GraphError, InputSpec, Node, NodeId, Payload};
pub use param::{MAX_PARAM_STRING, Param, ParamSet, ParamSpec, ParamValue};
