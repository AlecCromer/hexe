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
#[cfg(test)]
mod cook_model_oracle;
mod hip_expr;
mod hip_import;
mod registry;
mod scene;
pub use builtins::dot_sop::{NAME_BASE as DOT_NAME_BASE, TYPE_NAME as DOT_TYPE};
pub use builtins::render_frames;
pub use hip_expr::channel_report as hip_channel_report;
pub use hip_import::{
    ContainerImport, GeoImport, HIP_PX, HipImport, ObjMeta, hexe_type, import_hip, import_scene, is_hip_path,
};
pub use registry::{CtorFn, Registry, SopNode, SpecFn};
pub use scene::{
    Loaded, MAX_UI_ITEMS, SceneError, UiBox, UiExtra, UiNote, fill_unwired, load_scene, load_scene_in, load_scene_with,
    load_scene_with_base, save_scene, save_scene_ext,
};

/// Registry of every built-in SOP (`builtins.rs`).
pub fn registry() -> Registry {
    let mut r = Registry::new();
    builtins::builtins(&mut r);
    r
}
