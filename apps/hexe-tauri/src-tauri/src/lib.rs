//! hexe Tauri app (L7, UI lives in `../ui`): Tauri commands over `model::Session`, which wraps the hexe-graph engine.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
pub mod cli;
pub mod model;
pub mod script;
pub mod sheet;
pub mod shell;

use model::{CookResult, GraphView, NodeTypeInfo, Session};
use serde_json::Value;
use sheet::Sheet;
use shell::Gate;
use std::sync::{Mutex, MutexGuard};
use tauri::State;

/// Shared engine session (one graph per app instance).
pub struct App {
    session: Mutex<Session>,
    cli: cli::Cli,
}

impl App {
    /// A poisoned lock only means an earlier command panicked mid-edit; the graph is still structurally valid.
    pub(crate) fn lock(&self) -> MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
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
fn set_display(app: State<App>, id: Option<u32>) -> Res<()> {
    app.lock().set_display(id)
}
/// Remove several nodes as one undo step.
#[tauri::command]
fn nodes_remove(app: State<App>, ids: Vec<u32>) -> Res<()> {
    app.lock().nodes_remove(&ids)
}
#[tauri::command]
fn node_rename(app: State<App>, id: u32, name: String) -> Res<()> {
    app.lock().node_rename(id, &name)
}
/// `true` = something was undone / redone (node ids may have changed: re-read the graph).
#[tauri::command]
fn undo(app: State<App>) -> Res<bool> {
    app.lock().undo()
}
#[tauri::command]
fn redo(app: State<App>) -> Res<bool> {
    app.lock().redo()
}
#[tauri::command]
fn copy_nodes(app: State<App>, ids: Vec<u32>) -> Res<usize> {
    app.lock().copy(&ids)
}
/// Pastes the clipboard; returns the new node ids.
#[tauri::command]
fn paste_nodes(app: State<App>) -> Res<Vec<u32>> {
    app.lock().paste()
}
/// `--script` handshake: the UI finished booting.
#[tauri::command]
fn script_ready(gate: State<Gate>) {
    let _ = gate.ready.send(());
}
/// `--script` handshake: the UI re-synced after an op.
#[tauri::command]
fn script_ack(gate: State<Gate>) {
    let _ = gate.ack.send(());
}
#[tauri::command]
fn cook(app: State<App>) -> CookResult {
    app.lock().cook()
}
/// One window of the geometry spreadsheet over the last cook (`class` = point|vertex|prim|detail).
#[tauri::command]
fn spreadsheet(app: State<App>, class: String, offset: usize, limit: usize) -> Res<Sheet> {
    app.lock().spreadsheet(&class, offset, limit)
}
/// Scene path to open at startup: the positional CLI argument, else `HEXE_OPEN`; `None` = start empty.
#[tauri::command]
fn startup_scene(app: State<App>) -> Option<String> {
    app.cli.scene.clone().or_else(|| std::env::var("HEXE_OPEN").ok().filter(|p| !p.is_empty()))
}

fn fail(msg: &str) -> ! {
    eprintln!("hexe: {msg}\n{}", cli::USAGE);
    std::process::exit(2)
}

/// `HEXE_WINDOW_SIZE=WxH` (logical px; layout proof shots at sizes the screen cannot give a user resize).
fn parse_size(v: &str) -> Option<(f64, f64)> {
    let (w, h) = v.split_once('x')?;
    let (w, h) = (w.trim().parse::<f64>().ok()?, h.trim().parse::<f64>().ok()?);
    (w >= 320.0 && h >= 240.0 && w <= 16384.0 && h <= 16384.0).then_some((w, h))
}

/// Start the app. A failure to start the webview is reported on stderr (exit 1), never a panic.
pub fn run() {
    let cli = match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Parsed::Version) => return println!("{}", cli::version()),
        Ok(cli::Parsed::Help) => return println!("{}", cli::USAGE),
        Ok(cli::Parsed::Run(c)) => c,
        Err(e) => fail(&e),
    };
    let ops = match cli.script.as_deref().map(script::parse).transpose() {
        Ok(o) => o,
        Err(e) => fail(&e),
    };
    let mut session = Session::new();
    if let Err(e) = session.set_overrides(cli.sets.clone()) {
        fail(&e);
    }
    // fail fast on a --set that cannot apply to the startup scene (the window would only show it in the log)
    if let Some(path) = &cli.scene {
        let mut probe = Session::new();
        if probe.set_overrides(cli.sets.clone()).is_ok()
            && probe.graph_load(path).is_ok()
            && !probe.set_errors.is_empty()
        {
            fail(&probe.set_errors.join("\n"));
        }
    }
    let (gate, ready, ack) = shell::gate();
    let built = tauri::Builder::default()
        .manage(App { session: Mutex::new(session), cli })
        .manage(gate)
        .setup(move |app| {
            use tauri::Manager;
            let size = std::env::var("HEXE_WINDOW_SIZE").ok().and_then(|v| parse_size(&v));
            if let (Some((w, h)), Some(win)) = (size, app.get_webview_window("main")) {
                let _ = win.set_size(tauri::LogicalSize::new(w, h));
            }
            if let Some(ops) = ops {
                shell::spawn(app.handle().clone(), ops, ready, ack);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_node_types,
            graph_new,
            graph_load,
            graph_save,
            graph_get,
            node_add,
            node_remove,
            nodes_remove,
            node_rename,
            node_move,
            node_set_param,
            edge_connect,
            edge_disconnect,
            set_display,
            undo,
            redo,
            copy_nodes,
            paste_nodes,
            script_ready,
            script_ack,
            cook,
            spreadsheet,
            startup_scene,
        ]);
    if let Err(e) = built.run(tauri::generate_context!()) {
        eprintln!("hexe: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod size_tests {
    use super::parse_size;

    #[test]
    fn window_size_env_parses_and_rejects_nonsense() {
        assert_eq!(parse_size("2400x1500"), Some((2400.0, 1500.0)));
        assert_eq!(parse_size(" 1600 x 1000 "), Some((1600.0, 1000.0)));
        for bad in ["", "x", "10x10", "99999x100", "axb", "1600"] {
            assert_eq!(parse_size(bad), None, "{bad}");
        }
    }
}
