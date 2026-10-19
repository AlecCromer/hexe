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
pub mod files;
pub mod model;
pub mod render;
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
    /// `--render` job (headless mode), if any.
    render: Option<render::RenderReq>,
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
/// Network dot centred at (x, y), wired `from` -> dot and dot -> `to`'s `input` when given; one undo step.
#[tauri::command]
fn dot_add(app: State<App>, x: f32, y: f32, from: Option<u32>, to: Option<u32>, input: Option<usize>) -> Res<u32> {
    app.lock().dot_add(x, y, from, to.map(|t| (t, input.unwrap_or(0))))
}
/// TAB menu with a wire selected: a `type` node at (x, y) spliced into `from` -> `to`:`input` (`Session::node_add_wired`).
#[tauri::command]
fn node_add_wired(
    app: State<App>,
    r#type: String,
    x: f32,
    y: f32,
    from: Option<u32>,
    to: Option<u32>,
    input: Option<usize>,
) -> Res<u32> {
    app.lock().node_add_wired(&r#type, x, y, from, to.map(|t| (t, input.unwrap_or(0))))
}
#[tauri::command]
fn set_display(app: State<App>, id: Option<u32>) -> Res<()> {
    app.lock().set_display(id)
}
/// Bypass flag on / off (one undo step).
#[tauri::command]
fn set_bypass(app: State<App>, id: u32, on: bool) -> Res<()> {
    app.lock().set_bypass(id, on)
}
/// Remove several nodes as one undo step.
#[tauri::command]
fn nodes_remove(app: State<App>, ids: Vec<u32>) -> Res<()> {
    app.lock().nodes_remove(&ids)
}
/// Move several nodes (`[id, x, y]` each) as one undo step.
#[tauri::command]
fn nodes_move(app: State<App>, moves: Vec<(u32, f32, f32)>) -> Res<()> {
    app.lock().nodes_move(&moves)
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
fn script_ack(gate: State<Gate>, error: Option<String>) {
    let _ = gate.ack.send(error);
}
/// Timeline `(frame, fps)`: `$F` (1-based) and the rate; the display cook runs at `($F - 1) / fps`.
#[tauri::command]
fn timeline_get(app: State<App>) -> (i32, f32) {
    let s = app.lock();
    (s.frame(), s.fps())
}
/// Set `$F` (scrub). The UI re-cooks afterwards.
#[tauri::command]
fn timeline_set(app: State<App>, frame: i32) -> Res<()> {
    app.lock().set_frame(frame)
}
#[tauri::command]
fn cook(app: State<App>) -> CookResult {
    app.lock().cook()
}
/// Cook one node that is not the display node (template flag overlay); the spreadsheet keeps the display cook.
#[tauri::command]
fn cook_node(app: State<App>, id: u32) -> CookResult {
    app.lock().cook_node(id)
}
/// One window of the geometry spreadsheet over the last cook (`class` = point|vertex|prim|detail).
#[tauri::command]
fn spreadsheet(app: State<App>, class: String, offset: usize, limit: usize) -> Res<Sheet> {
    app.lock().spreadsheet(&class, offset, limit)
}
/// Viewport point selection (ptnums of the display geometry): pushed by the viewport on every pick change.
#[tauri::command]
fn selection_set(app: State<App>, ids: Vec<u32>) {
    app.lock().selection_set(&ids);
}
/// The selected ptnums, ascending.
#[tauri::command]
fn selection_get(app: State<App>) -> Vec<u32> {
    app.lock().selection_get()
}
/// Group from selection: new point `group` SOP (explicit ptnum pattern) under the display node, made display. Returns its id.
#[tauri::command]
fn group_from_selection(app: State<App>) -> Res<u32> {
    app.lock().group_from_selection()
}
/// `--render` job for the window (`None` = normal interactive run).
#[tauri::command]
fn render_request(app: State<App>) -> Option<render::RenderReq> {
    app.render.clone()
}
/// Headless render result: `data_url` = the canvas PNG. Writes the file and ENDS the app (exit 0, or 2 on any failure).
#[tauri::command]
fn render_save(handle: tauri::AppHandle, app: State<App>, data_url: String) {
    let Some(job) = app.render.clone() else { return };
    let done =
        render::decode_data_url(&data_url).and_then(|png| render::write_png(&job.out, &png, (job.width, job.height)));
    match done {
        Ok(()) => {
            println!(
                "hexe: rendered {} ({}x{}{})",
                job.out,
                job.width,
                job.height,
                if job.wire { ", wire" } else { "" }
            );
            handle.exit(0);
        }
        Err(e) => {
            eprintln!("hexe: {e}");
            handle.exit(2);
        }
    }
}
/// Headless render failed in the window (cook error, no display node, ...): report on stderr, exit 2.
#[tauri::command]
fn render_fail(handle: tauri::AppHandle, message: String) {
    eprintln!("hexe: render: {message}");
    handle.exit(2);
}
/// File > Open browser: entries of `dir` (`files::list`).
#[tauri::command]
fn fs_list(dir: String) -> Res<Vec<files::Entry>> {
    files::list(&dir)
}
/// File > Open browser: home / desktop / temp / drive roots.
#[tauri::command]
fn fs_places() -> files::Places {
    files::places()
}
/// File > Open browser New Folder: creates `New Folder` (numbered when taken) in `dir`, returns its name.
#[tauri::command]
fn fs_mkdir(dir: String) -> Res<String> {
    files::mkdir(&dir)
}
/// /obj: every Geometry object's name (sorted) + the current one (whose network the SOP commands edit).
#[tauri::command]
fn objects_list(app: State<App>) -> (Vec<String>, String) {
    let s = app.lock();
    (s.objects(), s.object_name().to_string())
}
/// Enter a Geometry object (double-click at /obj): its network becomes current; returns it.
#[tauri::command]
fn object_enter(app: State<App>, name: String) -> Res<GraphView> {
    let mut s = app.lock();
    s.object_enter(&name)?;
    Ok(s.graph_get())
}
/// New empty Geometry object (`name`, numbered up when taken); returns its final name.
#[tauri::command]
fn object_add(app: State<App>, name: String, kind: Option<String>, net: Option<String>) -> Res<String> {
    app.lock().object_add_kind(&name, kind.as_deref().unwrap_or("geo"), net.as_deref())
}
/// Every object: kind (geo / null / subnet), container, file position, flags, a container's sticky notes.
#[tauri::command]
fn objects_info(app: State<App>) -> model::ObjInfoList {
    let s = app.lock();
    (s.objects_info(), s.object_name().to_string())
}
/// Delete a Geometry object and its network.
#[tauri::command]
fn object_remove(app: State<App>, name: String) -> Res<()> {
    app.lock().object_remove(&name)
}
/// Rename a Geometry object (numbered up when taken); returns the final name.
#[tauri::command]
fn object_rename(app: State<App>, old: String, new: String) -> Res<String> {
    app.lock().object_rename(&old, &new)
}
/// Cook another object's display node (the viewport draws every visible object).
#[tauri::command]
fn cook_object(app: State<App>, name: String) -> Res<CookResult> {
    app.lock().cook_object(&name)
}
/// Scene path to open at startup: the positional CLI argument, else `HEXE_OPEN`; `None` = start empty.
#[tauri::command]
fn startup_scene(app: State<App>) -> Option<String> {
    app.cli.scene.clone().or_else(|| std::env::var("HEXE_OPEN").ok().filter(|p| !p.is_empty()))
}

/// Max wall time of a `--render` run.
const RENDER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
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

/// Debug-door port: `HEXE_DEBUG_PORT` (default 9353; 0/invalid = off). Drive the UI via `app-eval.js --port 9353`.
pub fn debug_port() -> u16 {
    tauri_debug_server::port_from_env("HEXE_DEBUG_PORT", 9353)
}

/// The app's tauri context; generic over the runtime so tests can build the real conf on `MockRuntime` (no display).
pub fn context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

/// Build the conf windows with the debug init script, then start the debug server (setup-time).
pub fn attach_debug<R: tauri::Runtime>(
    h: &tauri::AppHandle<R>,
    windows: &[tauri::utils::config::WindowConfig],
    port: u16,
) -> tauri::Result<()> {
    tauri_debug_server::build_windows(h, windows, port)?;
    tauri_debug_server::spawn(h.clone(), port);
    Ok(())
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
    if let Some(fps) = cli.fps {
        session.set_fps(fps).unwrap_or_else(|e| fail(&e));
    }
    if let Some(frame) = cli.frame {
        session.set_frame(frame).unwrap_or_else(|e| fail(&e));
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
    let render = cli::render_req(&cli);
    let (gate, ready, ack) = shell::gate();
    // debug door (:9353, HEXE_DEBUG_PORT): off for headless `--render` jobs. Conf windows are built in setup so they carry its init script.
    let debug_port = if render.is_some() { 0 } else { debug_port() };
    let mut ctx = context();
    let windows = tauri_debug_server::defer_windows(ctx.config_mut());
    let built = tauri::Builder::default()
        .manage(App { session: Mutex::new(session), cli, render: render.clone() })
        .manage(gate)
        .setup(move |app| {
            use tauri::Manager;
            attach_debug(app.handle(), &windows, debug_port)?;
            let size = std::env::var("HEXE_WINDOW_SIZE").ok().and_then(|v| parse_size(&v));
            if let (Some((w, h)), Some(win)) = (size, app.get_webview_window("main")) {
                let _ = win.set_size(tauri::LogicalSize::new(w, h));
            }
            shell::install_menu(app)?;
            if render.is_some() {
                // headless: no visible window (the canvas is drawn + read back at the requested size); a watchdog ends a stuck run
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(RENDER_TIMEOUT);
                    eprintln!("hexe: render: timed out after {}s", RENDER_TIMEOUT.as_secs());
                    handle.exit(3);
                });
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
            nodes_move,
            node_set_param,
            edge_connect,
            edge_disconnect,
            dot_add,
            node_add_wired,
            set_display,
            set_bypass,
            objects_info,
            undo,
            redo,
            copy_nodes,
            paste_nodes,
            script_ready,
            script_ack,
            timeline_get,
            timeline_set,
            cook,
            cook_node,
            spreadsheet,
            startup_scene,
            objects_list,
            object_enter,
            object_add,
            object_remove,
            object_rename,
            cook_object,
            fs_list,
            fs_places,
            fs_mkdir,
            selection_set,
            selection_get,
            group_from_selection,
            render_request,
            render_save,
            render_fail,
        ]);
    if let Err(e) = built.run(ctx) {
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
