//! hexe desktop app (egui via eframe, painter-based viewport, no custom wgpu pass).
//! `hexe [scene.json] [--set node.param=value]...`: network (left) · viewport (centre) · params (right).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
mod network;
mod params;
mod viewport;

use hexe_geo::Detail;
use hexe_graph::{Graph, NodeId, ParamValue};
use std::collections::{BTreeMap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

/// Largest scene file the Open button reads (hostile input is the default).
const MAX_SCENE_BYTES: u64 = 64 << 20;

/// One user-visible node (the loader's hidden empty source is not listed).
pub struct NodeInfo {
    pub id: NodeId,
    pub name: String,
    pub ty: &'static str,
}

/// A loaded scene: graph + visible nodes + typed-input wires `(src, dst, dst port)` as indices into `nodes`.
pub struct Scene {
    pub graph: Graph<Detail>,
    pub nodes: Vec<NodeInfo>,
    pub edges: Vec<(usize, usize, usize)>,
    pub display: usize,
}

impl Scene {
    pub fn parse(json: &str) -> Result<Scene, String> {
        let l = hexe_sop::load_scene_with(json, &hexe_sop::registry()).map_err(|e| e.to_string())?;
        let names: BTreeMap<NodeId, &String> = l.names.iter().map(|(n, id)| (*id, n)).collect();
        let mut nodes = Vec::new();
        for id in l.graph.node_ids() {
            if let (Some(name), Some(ty)) = (names.get(&id), l.graph.node_type(id)) {
                nodes.push(NodeInfo { id, name: (*name).clone(), ty });
            }
        }
        let index: HashMap<NodeId, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
        let mut edges = Vec::new();
        for (dst, n) in nodes.iter().enumerate() {
            for (port, src) in l.graph.input_sources(n.id).into_iter().enumerate() {
                if let Some(&s) = src.and_then(|s| index.get(&s)) {
                    edges.push((s, dst, port));
                }
            }
        }
        let display = index.get(&l.display).copied().unwrap_or(0);
        Ok(Scene { graph: l.graph, nodes, edges, display })
    }

    /// `node.param=value` typed by the param's declared default (proof/test hook behind `--set`).
    pub fn set_arg(&mut self, arg: &str) -> Result<(), String> {
        let (key, val) = arg.split_once('=').ok_or(format!("--set {arg}: want node.param=value"))?;
        let (node, param) = key.split_once('.').ok_or(format!("--set {arg}: want node.param=value"))?;
        let id = self.nodes.iter().find(|n| n.name == node).map(|n| n.id).ok_or(format!("--set: no node `{node}`"))?;
        let spec = self.graph.param_specs(id).into_iter().find(|s| s.name == param).ok_or(format!("--set: no param `{param}`"))?;
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

struct App {
    path: String,
    scene: Option<Scene>,
    selected: usize,
    pos: Vec<egui::Pos2>,
    detail: Option<Arc<Detail>>,
    cam: viewport::Camera,
    error: Option<String>,
    cook_ms: f32,
    fit: bool,
}

impl App {
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
        self.pos = network::layout(scene.nodes.len(), &scene.edges);
        self.selected = scene.display;
        self.scene = Some(scene);
        self.fit = true;
        self.recook();
        Ok(())
    }

    /// Cook the selected node (= display flag). Errors and panics land in the status bar, never abort.
    fn recook(&mut self) {
        let Some(scene) = self.scene.as_mut() else { return };
        let Some(id) = scene.nodes.get(self.selected).map(|n| n.id) else { return };
        let t = Instant::now();
        let r = catch_unwind(AssertUnwindSafe(|| {
            scene.graph.set_display(Some(id)).map_err(|e| e.to_string())?;
            scene.graph.cook(id, 0.0).map_err(|e| e.to_string())
        }));
        self.cook_ms = t.elapsed().as_secs_f32() * 1000.0;
        match r {
            Ok(Ok(d)) => {
                self.detail = Some(d);
                self.error = None;
            }
            Ok(Err(e)) => (self.detail, self.error) = (None, Some(e)),
            Err(_) => (self.detail, self.error) = (None, Some("cook panicked (caught)".into())),
        }
        if self.fit && let Some(d) = &self.detail {
            let (lo, hi) = d.bbox();
            self.cam.fit(lo, hi);
            self.fit = false;
        }
    }

    fn status(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| match (&self.error, &self.detail) {
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
                ui.label("no scene");
            }
        });
    }
}

struct Shell {
    app: App,
    sets: Vec<String>,
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let Shell { app, sets } = self;
        egui::Panel::top("open").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("File: Open");
                let r = ui.add(egui::TextEdit::singleline(&mut app.path).desired_width(360.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Open").clicked() || enter {
                    app.open(sets);
                }
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| app.status(ui));
        let mut recook = false;
        egui::Panel::left("network").default_size(280.0).show(ui, |ui| {
            ui.heading("network");
            if let Some(scene) = &app.scene
                && let Some(sel) = network::show(ui, scene, &mut app.pos, app.selected)
            {
                app.selected = sel;
                recook = true;
            }
        });
        egui::Panel::right("params").default_size(260.0).show(ui, |ui| {
            ui.heading("params");
            if let Some(scene) = app.scene.as_mut() {
                match params::show(ui, scene, app.selected) {
                    Ok(true) => recook = true,
                    Ok(false) => {}
                    Err(e) => app.error = Some(e),
                }
            }
        });
        if recook {
            app.recook();
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if ui.input(|i| i.key_pressed(egui::Key::F)) && !ui.ctx().egui_wants_keyboard_input() {
                app.fit = true;
                if let Some(d) = &app.detail {
                    let (lo, hi) = d.bbox();
                    app.cam.fit(lo, hi);
                    app.fit = false;
                }
            }
            viewport::show(ui, &mut app.cam, app.detail.as_deref());
        });
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") {
        println!("hexe {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let (mut path, mut sets, mut it) = (String::new(), Vec::new(), args.into_iter());
    while let Some(a) = it.next() {
        match a.as_str() {
            "--set" => sets.extend(it.next()),
            _ => path = a,
        }
    }
    let mut app = App {
        path,
        scene: None,
        selected: 0,
        pos: Vec::new(),
        detail: None,
        cam: viewport::Camera::default(),
        error: None,
        cook_ms: 0.0,
        fit: true,
    };
    if !app.path.is_empty() {
        app.open(&sets);
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native("hexe", opts, Box::new(|_cc| Ok(Box::new(Shell { app, sets: Vec::new() }))))
}
