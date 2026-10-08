//! hexe Tauri app (L7, UI lives in `../ui`): Tauri commands over `model::Session`, which wraps the hexe-graph engine.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
pub mod model;

use model::{CookResult, GraphView, NodeTypeInfo, Session};
use serde_json::Value;
use std::sync::{Mutex, MutexGuard};
use tauri::State;

/// Shared engine session (one graph per app instance).
pub struct App(Mutex<Session>);

impl App {
    /// A poisoned lock only means an earlier command panicked mid-edit; the graph is still structurally valid.
    fn lock(&self) -> MutexGuard<'_, Session> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

type Res<T> = Result<T, String>;

#[tauri::command]
fn list_node_types(app: State<App>) -> Vec<NodeTypeInfo> {
    app.lock().list_node_types()
}
#[tauri::command]
fn graph_new(app: State<App>) {
    app.lock().graph_new();
}
#[tauri::command]
fn graph_load(app: State<App>, path: String) -> Res<GraphView> {
    let mut s = app.lock();
    s.graph_load(&path)?;
    Ok(s.graph_get())
}
#[tauri::command]
fn graph_save(app: State<App>, path: String) -> Res<()> {
    app.lock().graph_save(&path)
}
#[tauri::command]
fn graph_get(app: State<App>) -> GraphView {
    app.lock().graph_get()
}
#[tauri::command]
fn node_add(app: State<App>, r#type: String, x: f32, y: f32) -> Res<u32> {
    app.lock().node_add(&r#type, x, y)
}
#[tauri::command]
fn node_remove(app: State<App>, id: u32) -> Res<()> {
    app.lock().node_remove(id)
}
#[tauri::command]
fn node_move(app: State<App>, id: u32, x: f32, y: f32) -> Res<()> {
    app.lock().node_move(id, x, y)
}
#[tauri::command]
fn node_set_param(app: State<App>, id: u32, name: String, value: Value) -> Res<()> {
    app.lock().node_set_param(id, &name, &value)
}
#[tauri::command]
fn edge_connect(app: State<App>, from: u32, to: u32, input: usize) -> Res<()> {
    app.lock().edge_connect(from, to, input)
}
#[tauri::command]
fn edge_disconnect(app: State<App>, to: u32, input: usize) -> Res<()> {
    app.lock().edge_disconnect(to, input)
}
#[tauri::command]
fn set_display(app: State<App>, id: u32) -> Res<()> {
    app.lock().set_display(id)
}
#[tauri::command]
fn cook(app: State<App>) -> CookResult {
    app.lock().cook()
}
/// Scene path to open at startup (`HEXE_OPEN`), used by the proof scripts; `None` = start empty.
#[tauri::command]
fn startup_scene() -> Option<String> {
    std::env::var("HEXE_OPEN").ok().filter(|p| !p.is_empty())
}

/// Start the app. A failure to start the webview is reported on stderr (exit 1), never a panic.
pub fn run() {
    let built = tauri::Builder::default().manage(App(Mutex::new(Session::new()))).invoke_handler(tauri::generate_handler![
        list_node_types,
        graph_new,
        graph_load,
        graph_save,
        graph_get,
        node_add,
        node_remove,
        node_move,
        node_set_param,
        edge_connect,
        edge_disconnect,
        set_display,
        cook,
        startup_scene,
    ]);
    if let Err(e) = built.run(tauri::generate_context!()) {
        eprintln!("hexe: {e}");
        std::process::exit(1);
    }
}
