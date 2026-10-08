//! hexe desktop app (egui via eframe; wgpu viewport as an egui-wgpu paint callback, see `gpu.rs`).
//! `hexe [scene.json] [--set node.param=value]... [--script ops]`: network (left) · viewport (centre) · params (right).
//! Editable: Tab add node · drag dots to wire · Del delete · D/right-click display flag · Ctrl/Cmd+S save · Ctrl/Cmd+O reload.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
mod gpu;
mod network;
mod params;
mod script;
mod viewport;
use egui::Pos2;
use hexe_geo::Detail;
use hexe_graph::{Graph, NodeId, ParamSet, ParamValue};
use hexe_sop::Registry;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

/// Largest scene file the Open button reads (hostile input is the default).
const MAX_SCENE_BYTES: u64 = 64 << 20;
/// Frames a script waits after each op so the window has painted before a `shot`.
const SCRIPT_SETTLE_FRAMES: u32 = 6;

/// One user-visible node (the loader's hidden empty source is not listed).
pub struct NodeInfo {
    pub id: NodeId,
    pub name: String,
    pub ty: &'static str,
}

/// A loaded scene: graph + visible (named) nodes + typed-input wires `(src, dst, dst port)` as indices into `nodes`.
/// The display flag lives in the graph (`graph.display()`), independent of selection.
pub struct Scene {
    pub graph: Graph<Detail>,
    pub nodes: Vec<NodeInfo>,
    pub edges: Vec<(usize, usize, usize)>,
    /// `ui.pos` read from the file (node name -> canvas position).
    pub pos0: BTreeMap<String, [f32; 2]>,
    reg: Registry,
}

impl Scene {
    pub fn parse(json: &str) -> Result<Scene, String> {
        let reg = hexe_sop::registry();
        let l = hexe_sop::load_scene_with(json, &reg).map_err(|e| e.to_string())?;
        let mut s = Scene { graph: l.graph, nodes: Vec::new(), edges: Vec::new(), pos0: l.pos, reg };
        s.rebuild();
        Ok(s)
    }
    /// Re-derive `nodes` / `edges` from the graph (named nodes only; wires from the hidden source are not shown).
    fn rebuild(&mut self) {
        self.nodes = self
            .graph
            .node_ids()
            .into_iter()
            .filter_map(|id| {
                Some(NodeInfo { id, name: self.graph.name(id)?.to_string(), ty: self.graph.node_type(id)? })
            })
            .collect();
        let index: HashMap<NodeId, usize> = self.nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
        self.edges.clear();
        for (dst, n) in self.nodes.iter().enumerate() {
            for (port, src) in self.graph.input_sources(n.id).into_iter().enumerate() {
                if let Some(&s) = src.and_then(|s| index.get(&s)) {
                    self.edges.push((s, dst, port));
                }
            }
        }
    }
    /// After any structural edit: unwired inputs get the hidden empty source again, lists are rebuilt.
    pub fn refresh(&mut self) -> Result<(), String> {
        hexe_sop::fill_unwired(&mut self.graph).map_err(|e| e.to_string())?;
        self.rebuild();
        Ok(())
    }
    /// Registered SOP type names (the Tab popup list).
    pub fn types(&self) -> Vec<&'static str> {
        self.reg.type_names()
    }
    pub fn id(&self, name: &str) -> Option<NodeId> {
        self.graph.node_by_name(name)
    }
    /// New node of registry type `ty`, named `name` or auto `<ty>1..n`.
    pub fn add(&mut self, ty: &str, name: Option<&str>) -> Result<NodeId, String> {
        let node = self.reg.create(ty).ok_or(format!("unknown node type `{ty}`"))?;
        let name = name.map_or_else(|| self.graph.unique_name(ty), str::to_string);
        if self.graph.node_by_name(&name).is_some() {
            return Err(format!("node name `{name}` is taken"));
        }
        let id = self.graph.add_boxed(node, ParamSet::new()).map_err(|e| e.to_string())?;
        if let Err(e) = self.graph.rename(id, &name) {
            let _ = self.graph.remove_node(id);
            return Err(e.to_string());
        }
        self.refresh()?;
        Ok(id)
    }
    pub fn remove(&mut self, id: NodeId) -> Result<(), String> {
        self.graph.remove_node(id).map_err(|e| e.to_string())?;
        self.refresh()
    }
    pub fn connect(&mut self, src: NodeId, dst: NodeId, port: usize) -> Result<(), String> {
        self.graph.connect(src, 0, dst, port).map_err(|e| e.to_string())?;
        self.refresh()
    }
    pub fn disconnect(&mut self, dst: NodeId, port: usize) -> Result<(), String> {
        self.graph.disconnect(dst, port).map_err(|e| e.to_string())?;
        self.refresh()
    }
    pub fn toggle_display(&mut self, id: NodeId) -> Result<(), String> {
        let next = (self.graph.display() != Some(id)).then_some(id);
        self.graph.set_display(next).map_err(|e| e.to_string())
    }
    /// Scene JSON (same schema `hexe-cli` reads) with `ui.pos` from the live canvas positions.
    pub fn to_json(&self, pos: &HashMap<NodeId, Pos2>) -> Result<String, String> {
        let by_name: BTreeMap<String, [f32; 2]> =
            self.nodes.iter().filter_map(|n| pos.get(&n.id).map(|p| (n.name.clone(), [p.x, p.y]))).collect();
        hexe_sop::save_scene(&self.graph, &by_name).map_err(|e| e.to_string())
    }
    /// `node.param=value` typed by the param's declared default (proof/test hook behind `--set`).
    pub fn set_arg(&mut self, arg: &str) -> Result<(), String> {
        let (key, val) = arg.split_once('=').ok_or(format!("--set {arg}: want node.param=value"))?;
        let (node, param) = key.split_once('.').ok_or(format!("--set {arg}: want node.param=value"))?;
        let id = self.id(node).ok_or(format!("--set: no node `{node}`"))?;
        let spec = self
            .graph
            .param_specs(id)
            .into_iter()
            .find(|s| s.name == param)
            .ok_or(format!("--set: no param `{param}`"))?;
        let bad = || format!("--set {arg}: bad value");
        let v = match spec.default {
            ParamValue::F(_) => ParamValue::F(val.parse().map_err(|_| bad())?),
            ParamValue::I(_) => ParamValue::I(val.parse().map_err(|_| bad())?),
            ParamValue::Menu(_) => ParamValue::Menu(val.parse().map_err(|_| bad())?),
            ParamValue::B(_) => ParamValue::B(matches!(val, "1" | "true")),
            ParamValue::S(_) => ParamValue::S(val.to_string()),
            ParamValue::V3(_) => {
                let c: Vec<f32> = val.split(',').filter_map(|c| c.trim().parse().ok()).collect();
                ParamValue::V3(<[f32; 3]>::try_from(c).map_err(|_| bad())?)
            }
        };
        self.graph.set_param(id, param, v).map_err(|e| e.to_string())
    }
}

/// Node-type popup (Tab): search text + where the node will be created.
struct AddMenu {
    canvas: Pos2,
    screen: Pos2,
    query: String,
    fresh: bool,
}

struct App {
    path: String,
    scene: Option<Scene>,
    /// Selection (shift-click multi) and the node whose params are shown. Not the display flag.
    selected: BTreeSet<NodeId>,
    active: Option<NodeId>,
    pos: HashMap<NodeId, Pos2>,
    wire: Option<network::WireFrom>,
    add_menu: Option<AddMenu>,
    view: viewport::Opts,
    detail: Option<Arc<Detail>>,
    mesh: Option<Arc<gpu::Mesh>>,
    style: viewport::Style,
    cam: viewport::Camera,
    error: Option<String>,
    note: Option<String>,
    cook_ms: f32,
    fit: bool,
    script: VecDeque<script::Op>,
    settle: u32,
}

impl App {
    fn new(path: String) -> App {
        App {
            path,
            scene: None,
            selected: BTreeSet::new(),
            active: None,
            pos: HashMap::new(),
            wire: None,
            add_menu: None,
            view: viewport::Opts::default(),
            detail: None,
            mesh: None,
            style: viewport::Style::default(),
            cam: viewport::Camera::default(),
            error: None,
            note: None,
            cook_ms: 0.0,
            fit: true,
            script: VecDeque::new(),
            settle: 0,
        }
    }
    fn open(&mut self, sets: &[String]) {
        let r = self.read(sets);
        if let Err(e) = r {
            self.error = Some(e);
        }
    }
    fn read(&mut self, sets: &[String]) -> Result<(), String> {
        let meta = std::fs::metadata(&self.path).map_err(|e| format!("{}: {e}", self.path))?;
        if meta.len() > MAX_SCENE_BYTES {
            return Err(format!("{}: larger than {MAX_SCENE_BYTES} bytes", self.path));
        }
        let json = std::fs::read_to_string(&self.path).map_err(|e| format!("{}: {e}", self.path))?;
        let mut scene = catch_unwind(|| Scene::parse(&json)).map_err(|_| "scene load panicked".to_string())??;
        for s in sets {
            scene.set_arg(s)?;
        }
        let auto = network::layout(scene.nodes.len(), &scene.edges);
        self.pos = scene
            .nodes
            .iter()
            .zip(auto)
            .map(|(n, a)| (n.id, scene.pos0.get(&n.name).map_or(a, |p| egui::pos2(p[0], p[1]))))
            .collect();
        let shown = scene.graph.display();
        self.selected = shown.into_iter().collect();
        self.active = shown;
        self.scene = Some(scene);
        self.fit = true;
        self.note = None;
        self.recook();
        Ok(())
    }
    /// Give nodes without a canvas position one (right of everything, top row).
    fn ensure_pos(&mut self) {
        let Some(scene) = &self.scene else { return };
        let ids: BTreeSet<NodeId> = scene.nodes.iter().map(|n| n.id).collect();
        self.pos.retain(|id, _| ids.contains(id));
        let mut right = self.pos.values().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
        for n in &scene.nodes {
            self.pos.entry(n.id).or_insert_with(|| {
                right = if right.is_finite() { right + 120.0 } else { 0.0 };
                egui::pos2(right, 0.0)
            });
        }
    }
    /// Cook the display node. Errors and panics land in the status bar, never abort.
    /// Rebuild the GPU draw data from the current `detail` (once per cook, not per frame).
    fn sync_mesh(&mut self) {
        self.mesh = self.detail.as_ref().map(|d| Arc::new(gpu::Mesh::build(d, &self.style)));
    }
    /// Home / frame-all (`H`, `F`): fit the display geometry, or the default camera when nothing is shown.
    fn home(&mut self) {
        match &self.detail {
            Some(d) => {
                let (lo, hi) = d.bbox();
                self.cam.fit(lo, hi);
            }
            None => self.cam = viewport::Camera::default(),
        }
        self.fit = false;
    }
    fn recook(&mut self) {
        self.ensure_pos();
        let Some(scene) = self.scene.as_mut() else { return };
        let Some(id) = scene.graph.display() else {
            (self.detail, self.error) = (None, None);
            self.sync_mesh();
            return;
        };
        let t = Instant::now();
        let r = catch_unwind(AssertUnwindSafe(|| scene.graph.cook(id, 0.0).map_err(|e| e.to_string())));
        self.cook_ms = t.elapsed().as_secs_f32() * 1000.0;
        match r {
            Ok(Ok(d)) => {
                self.detail = Some(d);
                self.error = None;
            }
            Ok(Err(e)) => (self.detail, self.error) = (None, Some(e)),
            Err(_) => (self.detail, self.error) = (None, Some("cook panicked (caught)".into())),
        }
        self.sync_mesh();
        if self.fit
            && let Some(d) = &self.detail
        {
            let (lo, hi) = d.bbox();
            self.cam.fit(lo, hi);
            self.fit = false;
        }
    }
    /// Run a structural edit, then fix up unwired inputs, positions and the cook. A failed edit shows in the status bar.
    fn edit(&mut self, f: impl FnOnce(&mut Scene) -> Result<(), String>) {
        let Some(scene) = self.scene.as_mut() else { return };
        let r = catch_unwind(AssertUnwindSafe(|| f(scene))).unwrap_or_else(|_| Err("edit panicked (caught)".into()));
        self.recook();
        if let Err(e) = r {
            self.error = Some(e);
        }
    }
    fn apply(&mut self, act: network::Act) {
        use network::Act;
        match act {
            Act::Select { id, add } => {
                if add && !self.selected.remove(&id) || !add {
                    if !add {
                        self.selected.clear();
                    }
                    self.selected.insert(id);
                    self.active = Some(id);
                } else {
                    self.active = self.selected.iter().next_back().copied();
                }
            }
            Act::Clear => (self.selected, self.active) = (BTreeSet::new(), None),
            Act::Connect { src, dst, port } => self.edit(|s| s.connect(src, dst, port)),
            Act::Disconnect { dst, port } => self.edit(|s| s.disconnect(dst, port)),
            Act::ToggleDisplay(id) => self.edit(|s| s.toggle_display(id)),
            Act::Delete => {
                let ids: Vec<NodeId> = std::mem::take(&mut self.selected).into_iter().collect();
                self.active = None;
                self.edit(|s| ids.into_iter().try_for_each(|id| s.remove(id)));
            }
            Act::OpenAdd { canvas, screen } => {
                self.add_menu = Some(AddMenu { canvas, screen, query: String::new(), fresh: true });
            }
        }
    }
    /// Create a node of `ty` at canvas position `at`; it becomes the selection.
    fn add_node(&mut self, ty: &str, name: Option<&str>, at: Option<Pos2>) {
        let mut made = None;
        self.edit(|s| s.add(ty, name).map(|id| made = Some(id)));
        if let Some(id) = made {
            if let Some(at) = at {
                self.pos.insert(id, at);
            }
            self.ensure_pos();
            (self.selected, self.active) = (BTreeSet::from([id]), Some(id));
        }
    }
    fn save(&mut self, to: Option<&str>) {
        let path = to.map_or_else(|| self.path.clone(), str::to_string);
        let Some(scene) = &self.scene else { return };
        let r = scene.to_json(&self.pos).and_then(|j| std::fs::write(&path, j).map_err(|e| format!("{path}: {e}")));
        match r {
            Ok(()) => (self.note, self.error) = (Some(format!("saved {path}")), None),
            Err(e) => self.error = Some(format!("save: {e}")),
        }
    }
    fn status(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let target = self.scene.as_ref().and_then(|s| s.graph.display().and_then(|d| s.graph.name(d)));
            ui.label(format!("display: {}", target.unwrap_or("none")));
            ui.separator();
            match (&self.error, &self.detail) {
                (Some(e), _) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("error: {e}"));
                }
                (None, Some(d)) => {
                    ui.label(format!(
                        "points {} · prims {} · verts {} · cook {:.2} ms",
                        d.point_count,
                        d.prim_count(),
                        d.vertex_count(),
                        self.cook_ms
                    ));
                }
                _ => {
                    ui.label(if self.scene.is_some() {
                        "no display node (right-click a node or press D)"
                    } else {
                        "no scene"
                    });
                }
            }
            if let Some(n) = &self.note {
                ui.separator();
                ui.colored_label(egui::Color32::LIGHT_GREEN, n);
            }
            ui.separator();
            ui.label(self.view.label());
        });
    }
    /// One script op (called once per settled frame). Failures are reported on stderr and in the status bar.
    fn run_op(&mut self, op: script::Op, ctx: &egui::Context) {
        use script::Op;
        let missing = |n: &str| format!("script: no node `{n}`");
        let find = |app: &App, n: &str| app.scene.as_ref().and_then(|s| s.id(n)).ok_or_else(|| missing(n));
        let r: Result<(), String> = match op {
            Op::Add { ty, name } => {
                self.add_node(&ty, name.as_deref(), None);
                Ok(())
            }
            Op::Connect { dst, port, src } => find(self, &dst).and_then(|d| {
                let s = find(self, &src)?;
                self.edit(|sc| sc.connect(s, d, port));
                Ok(())
            }),
            Op::Disconnect { dst, port } => find(self, &dst).map(|d| self.edit(|sc| sc.disconnect(d, port))),
            Op::Remove(n) => find(self, &n).map(|id| {
                self.selected.remove(&id);
                self.edit(|sc| sc.remove(id));
            }),
            Op::Display(n) => {
                find(self, &n).map(|id| self.edit(|sc| sc.graph.set_display(Some(id)).map_err(|e| e.to_string())))
            }
            Op::Select(n) => find(self, &n).map(|id| (self.selected, self.active) = (BTreeSet::from([id]), Some(id))),
            Op::Set(kv) => {
                self.edit(|sc| sc.set_arg(&kv));
                Ok(())
            }
            Op::View(k) => {
                match k {
                    'w' => self.view.wire ^= true,
                    'p' => self.view.point_nums ^= true,
                    _ => self.view.prim_nums ^= true,
                }
                Ok(())
            }
            Op::Save(p) => {
                self.save(Some(&p));
                self.error.clone().map_or(Ok(()), Err)
            }
            Op::Shot(p) => shot(&p),
            Op::Quit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                Ok(())
            }
        };
        if let Err(e) = r {
            eprintln!("{e}");
            self.error = Some(e);
        }
    }
}

/// Screenshot our own window: `kyty-winid <pid>` (override: `HEXE_WINID`) -> window id -> `screencapture -l`.
fn shot(path: &str) -> Result<(), String> {
    let winid = std::env::var("HEXE_WINID").unwrap_or_else(|_| "kyty-winid".into());
    let out = std::process::Command::new(&winid)
        .arg(std::process::id().to_string())
        .output()
        .map_err(|e| format!("shot: {winid}: {e}"))?;
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("shot: no window id from {winid} (got `{id}`)"));
    }
    let st = std::process::Command::new("screencapture")
        .args(["-x", "-o", &format!("-l{id}"), path])
        .status()
        .map_err(|e| format!("shot: screencapture: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("shot: screencapture exited {st}")) }
}

struct Shell {
    app: App,
    /// `--set node.param=value` args: re-applied on EVERY (re)load, not just the first (Cmd+O / Open).
    sets: Vec<String>,
}

impl Shell {
    fn new(app: App, sets: Vec<String>) -> Shell {
        Shell { app, sets }
    }
    /// Re-read the scene file (Cmd+O / Open button / Enter in the path field) keeping the `--set` overrides.
    fn reload(&mut self) {
        self.app.open(&self.sets);
    }
    /// Tab popup: search field + registry type list. Returns the type picked (Enter = first match, click = that one).
    fn add_popup(&mut self, ui: &mut egui::Ui) {
        let types = self.app.scene.as_ref().map(Scene::types).unwrap_or_default();
        let Some(menu) = self.app.add_menu.as_mut() else { return };
        let (mut pick, mut close) = (None::<&'static str>, false);
        egui::Area::new(egui::Id::new("add-node")).order(egui::Order::Foreground).fixed_pos(menu.screen).show(
            ui.ctx(),
            |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(180.0);
                    let r = ui.add(egui::TextEdit::singleline(&mut menu.query).hint_text("node type…"));
                    if std::mem::take(&mut menu.fresh) {
                        r.request_focus();
                    }
                    let q = menu.query.to_lowercase();
                    let hits: Vec<&'static str> = types.iter().copied().filter(|t| t.contains(&q)).collect();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        pick = hits.first().copied();
                        close = true;
                    }
                    egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                        for t in &hits {
                            if ui.selectable_label(false, *t).clicked() {
                                pick = Some(t);
                            }
                        }
                    });
                    close |= ui.input(|i| i.key_pressed(egui::Key::Escape)) || pick.is_some();
                });
            },
        );
        if close {
            let canvas = menu.canvas;
            self.app.add_menu = None;
            if let Some(t) = pick {
                self.app.add_node(t, None, Some(canvas));
            }
        }
    }
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut reload = false;
        let Shell { app, .. } = self;
        let (save, open) = ui.input(|i| {
            let cmd = i.modifiers.command || i.modifiers.ctrl;
            (cmd && i.key_pressed(egui::Key::S), cmd && i.key_pressed(egui::Key::O))
        });
        if save {
            app.save(None);
        }
        reload |= open && !app.path.is_empty();
        egui::Panel::top("open").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("File: Open");
                let r = ui.add(egui::TextEdit::singleline(&mut app.path).desired_width(360.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                reload |= ui.button("Open").clicked() || enter;
                if ui.button("Save").clicked() {
                    app.save(None);
                }
            });
        });
        if reload {
            self.reload();
        }
        let Shell { app, .. } = self;
        egui::Panel::bottom("status").show(ui, |ui| app.status(ui));
        let mut acts = Vec::new();
        egui::Panel::left("network").default_size(300.0).show(ui, |ui| {
            ui.heading("network");
            if let Some(scene) = &app.scene {
                let c = network::Ctx {
                    pos: &mut app.pos,
                    selected: &app.selected,
                    active: app.active,
                    wire: &mut app.wire,
                };
                acts = network::show(ui, scene, c);
            }
        });
        let mut recook = false;
        egui::Panel::right("params").default_size(260.0).show(ui, |ui| {
            ui.heading("params");
            if let Some(scene) = app.scene.as_mut() {
                match params::show(ui, scene, app.active) {
                    Ok(true) => recook = true,
                    Ok(false) => {}
                    Err(e) => app.error = Some(e),
                }
            }
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if ui.input(|i| i.key_pressed(egui::Key::F) || i.key_pressed(egui::Key::H))
                && !ui.ctx().egui_wants_keyboard_input()
            {
                app.home();
            }
            viewport::show(ui, &mut app.cam, app.detail.as_deref(), app.mesh.as_ref(), &mut app.view, &app.style);
        });
        for a in acts {
            app.apply(a);
        }
        if recook {
            app.recook();
        }
        self.add_popup(ui);
        let app = &mut self.app;
        if app.settle > 0 {
            app.settle -= 1;
            ui.ctx().request_repaint();
        } else if let Some(op) = app.script.pop_front() {
            app.run_op(op, ui.ctx());
            app.settle = SCRIPT_SETTLE_FRAMES;
            ui.ctx().request_repaint();
        }
    }
}

/// Default `--render` image size.
const DEFAULT_RENDER_SIZE: [u32; 2] = [640, 480];
/// `WxH`, both in 1..=MAX_TARGET.
fn parse_size(s: &str) -> Option<[u32; 2]> {
    let (w, h) = s.split_once('x')?;
    let ok = |v: u32| (1..=gpu::MAX_TARGET).contains(&v);
    let (w, h) = (w.parse().ok()?, h.parse().ok()?);
    (ok(w) && ok(h)).then_some([w, h])
}
/// Headless: render the display node through the same wgpu scene as the window and write a PNG.
fn render_to(app: &App, out: &str, size: [u32; 2]) -> Result<(), String> {
    if let Some(e) = &app.error {
        return Err(e.clone());
    }
    let img = gpu::render_offscreen(app.mesh.as_deref(), &app.cam, &app.view, &app.style, size)?;
    img.write_png(out)
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") {
        println!("hexe {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let (mut path, mut sets, mut script_arg, mut it) = (String::new(), Vec::new(), None, args.into_iter());
    let (mut render, mut size, mut app_wire) = (None, DEFAULT_RENDER_SIZE, false);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--set" => sets.extend(it.next()),
            "--script" => script_arg = it.next(),
            "--render" => render = it.next(),
            "--size" => size = it.next().and_then(|s| parse_size(&s)).unwrap_or(size),
            "--wire" => app_wire = true,
            _ => path = a,
        }
    }
    let mut app = App::new(path);
    if !app.path.is_empty() {
        app.open(&sets);
    }
    app.view.wire = app_wire;
    if let Some(out) = render {
        return match render_to(&app, &out, size) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };
    }
    if let Some(s) = script_arg {
        match script::parse(&s) {
            Ok(ops) => app.script = ops.into(),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        }
        app.settle = SCRIPT_SETTLE_FRAMES * 4;
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 700.0]),
        ..Default::default()
    };
    let style = app.style.clone();
    eframe::run_native(
        "hexe",
        opts,
        Box::new(move |cc| {
            if let Some(rs) = cc.wgpu_render_state.as_ref() {
                let r3 = gpu::Renderer3d::new(&rs.device, rs.target_format, style);
                rs.renderer.write().callback_resources.insert(r3);
            }
            Ok(Box::new(Shell::new(app, sets)))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> String {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/normal.json").to_string()
    }
    fn cusp(app: &App) -> Option<ParamValue> {
        let s = app.scene.as_ref()?;
        s.graph.param(s.id("normal1")?, "cusp_angle").cloned()
    }

    fn lit(img: &gpu::Image, bg: [u8; 3], cx: u32, cy: u32, r: u32) -> usize {
        let mut n = 0;
        for y in cy - r..=cy + r {
            for x in cx - r..=cx + r {
                let p = img.px(x, y);
                n += (p[..3].iter().zip(bg).map(|(a, b)| i32::from(*a).abs_diff(i32::from(b))).sum::<u32>() > 24) as usize;
            }
        }
        n
    }

    /// box + sphere (examples/viewport.json) through the real wgpu scene: lit centre, background corners.
    #[test]
    fn offscreen_box_and_sphere_has_lit_centre_and_background_corners() {
        let mut app = App::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/viewport.json").to_string());
        app.open(&[]);
        assert!(app.error.is_none(), "{:?}", app.error);
        // no grid in the test so corners are guaranteed background
        app.style = viewport::Style { grid_half: 0, ..viewport::Style::default() };
        app.sync_mesh();
        let m = app.mesh.as_ref().expect("cooked mesh");
        assert!(m.tris.len() > 36 && m.points.len() > 8, "box + sphere: {} tri verts", m.tris.len());
        let size = [320, 240];
        let render = |opts: &viewport::Opts| gpu::render_offscreen(app.mesh.as_deref(), &app.cam, opts, &app.style, size);
        let shaded = match render(&app.view) {
            Ok(i) => i,
            Err(e) if e.starts_with("no wgpu adapter") => return eprintln!("SKIP (no GPU): {e}"),
            Err(e) => panic!("{e}"),
        };
        let bg = app.style.background.map(|c| (c * 255.0).round() as u8);
        for (x, y) in [(0, 0), (319, 0), (0, 239), (319, 239)] {
            let p = shaded.px(x, y);
            assert_eq!(p[..3], bg, "corner ({x},{y}) is background");
        }
        assert!(lit(&shaded, bg, 160, 120, 8) > 0, "centre is lit");
        let c = shaded.px(160, 120);
        assert!(c[2] > c[0], "surface is blue-tinted: {c:?}");
        let wire = render(&viewport::Opts { wire: true, ..app.view }).expect("wire render");
        let (a, b) = (lit(&shaded, bg, 160, 120, 60), lit(&wire, bg, 160, 120, 60));
        assert!(b > 0 && b < a, "wireframe covers fewer pixels than shaded: {b} vs {a}");
        if let Ok(path) = std::env::var("HEXE_OFFSCREEN_PNG") {
            let big = gpu::render_offscreen(app.mesh.as_deref(), &app.cam, &app.view, &viewport::Style::default(), [640, 480]);
            big.expect("png render").write_png(&path).expect("write png");
        }
    }

    #[test]
    fn home_frames_the_display_node_and_size_arg_is_bounded() {
        let mut app = App::new(example());
        app.open(&[]);
        app.cam = viewport::Camera { dist: 1e5, ..viewport::Camera::default() };
        let mut shell = Shell::new(app, vec![]);
        shell.app.home();
        assert!(shell.app.cam.dist < 100.0, "H refits: {}", shell.app.cam.dist);
        shell.app.detail = None;
        shell.app.home();
        assert_eq!(shell.app.cam, viewport::Camera::default(), "nothing displayed -> default camera");
        assert_eq!(parse_size("640x480"), Some([640, 480]));
        for bad in ["0x5", "5x0", "99999x5", "x", "5", "axb"] {
            assert_eq!(parse_size(bad), None, "{bad}");
        }
    }
    #[test]
    fn set_overrides_survive_a_cmd_o_reload() {
        let sets = vec!["normal1.cusp_angle=99".to_string()];
        let mut app = App::new(example());
        app.open(&sets);
        assert_eq!(cusp(&app), Some(ParamValue::F(99.0)), "applied on first load");
        let mut shell = Shell::new(app, sets);
        // user fiddles, then Cmd+O re-reads the file: file value is 60, the --set must win again
        shell.reload();
        assert_eq!(cusp(&shell.app), Some(ParamValue::F(99.0)), "dropped by reload");
        shell.reload();
        assert_eq!(cusp(&shell.app), Some(ParamValue::F(99.0)));
        assert!(shell.app.error.is_none(), "{:?}", shell.app.error);
    }
}
