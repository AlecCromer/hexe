//! hexe sop (L2): SOP node `Registry` + graph-JSON scene loader. Payload = `hexe_geo::Detail` (`SopNode = dyn Node<Detail>`).
//! Adding a SOP = one file + two lines in `builtins.rs` (see spec/harvest/HOWTO-SOP-LANE.md). Hostile input is the
//! default (AGENTS.md §4): bad scenes / params / edges are `Err`, never a panic.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
mod builtins;
mod registry;
mod scene;
pub use registry::{CtorFn, Registry, SopNode, SpecFn};
pub use scene::{
    Loaded, SceneError, fill_unwired, load_scene, load_scene_in, load_scene_with, load_scene_with_base, save_scene,
};

/// Registry of every built-in SOP (`builtins.rs`).
pub fn registry() -> Registry {
    let mut r = Registry::new();
    builtins::builtins(&mut r);
    r
}
