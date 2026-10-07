//! Network pane: nodes as boxes (top-down by topological depth, x by sibling index), typed-input wires.
//! Click / shift-click = select, drag body = move, drag output dot -> input dot = connect, drag a wire off an
//! input = disconnect, right-click or D = display flag (independent of selection), Tab = add node, Del = delete.
use crate::Scene;
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use hexe_graph::NodeId;
use std::collections::{BTreeSet, HashMap};

const BOX: egui::Vec2 = vec2(104.0, 34.0);
const STEP: egui::Vec2 = vec2(120.0, 64.0);

/// Longest-path depth of every node; edges are `(src, dst, port)`. Bounded passes, so a hostile cycle terminates.
pub fn depths(n: usize, edges: &[(usize, usize, usize)]) -> Vec<usize> {
    let mut d = vec![0usize; n];
    for _ in 0..n {
        let mut changed = false;
        for &(s, t, _) in edges {
            let want = d.get(s).copied().unwrap_or(0).saturating_add(1).min(n);
            if let Some(slot) = d.get_mut(t)
                && *slot < want
            {
                *slot = want;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    d
}

/// Canvas position per node: y = depth, x = sibling index within the depth row.
pub fn layout(n: usize, edges: &[(usize, usize, usize)]) -> Vec<Pos2> {
    let mut row_fill: Vec<usize> = Vec::new();
    depths(n, edges)
        .into_iter()
        .map(|d| {
            if row_fill.len() <= d {
                row_fill.resize(d + 1, 0);
            }
            let col = row_fill.get(d).copied().unwrap_or(0);
            if let Some(c) = row_fill.get_mut(d) {
                *c += 1;
            }
            pos2(col as f32 * STEP.x, d as f32 * STEP.y)
        })
        .collect()
}

/// What the user did in the network pane this frame (the app applies it; node moves are applied in place).
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// Click / shift-click on a node.
    Select {
        id: NodeId,
        add: bool,
    },
    /// Click on empty canvas.
    Clear,
    Connect {
        src: NodeId,
        dst: NodeId,
        port: usize,
    },
    Disconnect {
        dst: NodeId,
        port: usize,
    },
    ToggleDisplay(NodeId),
    /// Delete/Backspace: remove the selected nodes.
    Delete,
    /// Tab: open the node-type popup at `(canvas, screen)` position.
    OpenAdd {
        canvas: Pos2,
        screen: Pos2,
    },
}

/// A wire being dragged: from a node output, or from an unconnected input that wants a source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WireFrom {
    Out(NodeId),
    In(NodeId, usize),
}

/// Everything the pane needs to know about the app, besides the scene.
pub struct Ctx<'a> {
    pub pos: &'a mut HashMap<NodeId, Pos2>,
    pub selected: &'a BTreeSet<NodeId>,
    pub active: Option<NodeId>,
    pub wire: &'a mut Option<WireFrom>,
}

const DOT: f32 = 5.0;
const GRAB: f32 = 7.0;
const DROP: f32 = 14.0;

/// Input dot `port` of `ports` on a node rect (top edge); output dot is `out_dot`.
fn in_dot(r: Rect, ports: usize, port: usize) -> Pos2 {
    pos2(r.left() + r.width() * (port as f32 + 1.0) / (ports as f32 + 1.0), r.top())
}
fn out_dot(r: Rect) -> Pos2 {
    r.center_bottom()
}
fn wire_shape(a: Pos2, b: Pos2) -> egui::Shape {
    let k = ((b.y - a.y).abs() * 0.5).max(18.0);
    let (c1, c2) = (a + vec2(0.0, k), b - vec2(0.0, k));
    egui::Shape::CubicBezier(egui::epaint::CubicBezierShape::from_points_stroke(
        [a, c1, c2, b],
        false,
        Color32::TRANSPARENT,
        Stroke::new(1.5, Color32::from_rgb(120, 160, 200)),
    ))
}

/// Draw the network and collect the user's actions. Keys (Tab / Delete / Backspace / D) act while the pointer is
/// over the pane and no text field has focus.
pub fn show(ui: &mut Ui, scene: &Scene, c: Ctx) -> Vec<Act> {
    let Ctx { pos, selected, active, wire } = c;
    let mut acts = Vec::new();
    let (bg, painter) = ui.allocate_painter(ui.available_size(), Sense::click());
    painter.rect_filled(bg.rect, 2.0, Color32::from_gray(28));
    let origin = bg.rect.min + vec2(12.0, 12.0);
    let rect_of = |id: NodeId| pos.get(&id).map(|p| Rect::from_min_size(origin + p.to_vec2(), BOX));
    let ports_of = |id: NodeId| scene.graph.input_sources(id).len();
    for &(s, t, port) in &scene.edges {
        let (Some(sn), Some(tn)) = (scene.nodes.get(s), scene.nodes.get(t)) else { continue };
        let (Some(rs), Some(rt)) = (rect_of(sn.id), rect_of(tn.id)) else { continue };
        let b = in_dot(rt, ports_of(tn.id), port);
        painter.add(wire_shape(out_dot(rs), b));
        painter.text(
            b + vec2(3.0, -9.0),
            egui::Align2::LEFT_CENTER,
            port.to_string(),
            FontId::proportional(9.0),
            Color32::GRAY,
        );
    }
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let mut moved: Option<(NodeId, egui::Vec2)> = None;
    for node in &scene.nodes {
        let Some(r) = rect_of(node.id) else { continue };
        let resp = ui.interact(r, ui.id().with(("node", node.id)), Sense::click_and_drag());
        let shift = ui.input(|i| i.modifiers.shift);
        if resp.clicked() || (resp.drag_started() && !selected.contains(&node.id)) {
            acts.push(Act::Select { id: node.id, add: shift });
        }
        if resp.dragged() {
            moved = Some((node.id, resp.drag_delta()));
        }
        if resp.secondary_clicked() {
            acts.push(Act::ToggleDisplay(node.id));
        }
        let sel = selected.contains(&node.id);
        let fill = if sel { Color32::from_rgb(70, 90, 130) } else { Color32::from_gray(55) };
        let edge = if sel { Color32::from_rgb(255, 200, 80) } else { Color32::from_gray(140) };
        painter.rect(r, 4.0, fill, Stroke::new(if sel { 2.0 } else { 1.0 }, edge), egui::StrokeKind::Inside);
        let name_col = if active == Some(node.id) { Color32::from_rgb(255, 230, 150) } else { Color32::WHITE };
        painter.text(
            r.center_top() + vec2(0.0, 5.0),
            egui::Align2::CENTER_TOP,
            &node.name,
            FontId::proportional(12.0),
            name_col,
        );
        painter.text(
            r.center_bottom() - vec2(0.0, 4.0),
            egui::Align2::CENTER_BOTTOM,
            node.ty,
            FontId::proportional(9.0),
            Color32::GRAY,
        );
        if scene.graph.display() == Some(node.id) {
            painter.circle_filled(r.right_top() + vec2(-6.0, 6.0), 3.5, Color32::from_rgb(80, 200, 255));
        }
    }
    // ports sit above the node bodies (interacted later = on top)
    let srcs = |id: NodeId| scene.graph.input_sources(id);
    for node in &scene.nodes {
        let Some(r) = rect_of(node.id) else { continue };
        let ports = ports_of(node.id);
        for port in 0..ports {
            let p = in_dot(r, ports, port);
            let hit = Rect::from_center_size(p, vec2(GRAB * 2.0, GRAB * 2.0));
            let resp = ui.interact(hit, ui.id().with(("in", node.id, port)), Sense::drag());
            let col = if resp.hovered() { Color32::WHITE } else { Color32::from_gray(170) };
            painter.circle_filled(p, DOT * 0.7, col);
            let from = srcs(node.id).get(port).copied().flatten();
            let named = from.filter(|s| scene.graph.name(*s).is_some());
            if resp.drag_started() {
                match named {
                    // picking up a wired input detaches it and carries the old source
                    Some(s) => {
                        acts.push(Act::Disconnect { dst: node.id, port });
                        *wire = Some(WireFrom::Out(s));
                    }
                    None => *wire = Some(WireFrom::In(node.id, port)),
                }
            }
            if resp.drag_stopped() {
                resolve(wire.take(), pointer, scene, &rect_of, &mut acts);
            }
        }
        let p = out_dot(r);
        let resp = ui.interact(
            Rect::from_center_size(p, vec2(GRAB * 2.0, GRAB * 2.0)),
            ui.id().with(("out", node.id)),
            Sense::drag(),
        );
        painter.circle_filled(p, DOT, if resp.hovered() { Color32::WHITE } else { Color32::from_rgb(120, 160, 200) });
        if resp.drag_started() {
            *wire = Some(WireFrom::Out(node.id));
        }
        if resp.drag_stopped() {
            resolve(wire.take(), pointer, scene, &rect_of, &mut acts);
        }
    }
    if let (Some(w), Some(ptr)) = (*wire, pointer) {
        let anchor = match w {
            WireFrom::Out(s) => rect_of(s).map(out_dot),
            WireFrom::In(d, port) => rect_of(d).map(|r| in_dot(r, ports_of(d), port)),
        };
        if let Some(a) = anchor {
            painter.add(if matches!(w, WireFrom::Out(_)) { wire_shape(a, ptr) } else { wire_shape(ptr, a) });
        }
    }
    if let Some((id, d)) = moved {
        let group: Vec<NodeId> = if selected.contains(&id) { selected.iter().copied().collect() } else { vec![id] };
        for n in group {
            if let Some(p) = pos.get_mut(&n) {
                *p += d;
            }
        }
    }
    if bg.clicked() {
        acts.push(Act::Clear);
    }
    if ui.rect_contains_pointer(bg.rect) && !ui.ctx().egui_wants_keyboard_input() {
        if let Some(ptr) = pointer
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab))
        {
            acts.push(Act::OpenAdd { canvas: (ptr - origin).to_pos2(), screen: ptr });
        }
        if ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
            acts.push(Act::Delete);
        }
        if let Some(a) = active
            && ui.input(|i| i.key_pressed(egui::Key::D) && !i.modifiers.command)
        {
            acts.push(Act::ToggleDisplay(a));
        }
    }
    acts
}

/// Wire dropped at `pointer`: connect to the nearest port (or a node body) if it makes sense.
fn resolve(
    wire: Option<WireFrom>,
    pointer: Option<Pos2>,
    scene: &Scene,
    rect_of: &dyn Fn(NodeId) -> Option<Rect>,
    acts: &mut Vec<Act>,
) {
    let (Some(w), Some(p)) = (wire, pointer) else { return };
    match w {
        WireFrom::Out(src) => {
            let mut best: Option<(f32, NodeId, usize)> = None;
            for n in scene.nodes.iter().filter(|n| n.id != src) {
                let Some(r) = rect_of(n.id) else { continue };
                let ports = scene.graph.input_sources(n.id);
                for port in 0..ports.len() {
                    let d = in_dot(r, ports.len(), port).distance(p);
                    if d < DROP && best.is_none_or(|b| d < b.0) {
                        best = Some((d, n.id, port));
                    }
                }
                // dropped on the body: first free input, else input 0
                if best.is_none() && r.contains(p) && !ports.is_empty() {
                    let port = ports.iter().position(|s| s.is_none_or(|s| scene.graph.name(s).is_none())).unwrap_or(0);
                    best = Some((DROP, n.id, port));
                }
            }
            if let Some((_, dst, port)) = best {
                acts.push(Act::Connect { src, dst, port });
            }
        }
        WireFrom::In(dst, port) => {
            let hit = scene
                .nodes
                .iter()
                .filter(|n| n.id != dst)
                .find(|n| rect_of(n.id).is_some_and(|r| r.contains(p) || out_dot(r).distance(p) < DROP));
            if let Some(n) = hit {
                acts.push(Act::Connect { src: n.id, dst, port });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copytopoints_depths() {
        let s = Scene::parse(include_str!("../../../examples/copytopoints.json")).expect("scene");
        let d = depths(s.nodes.len(), &s.edges);
        let of = |name: &str| s.nodes.iter().position(|n| n.name == name).map(|i| d[i]);
        assert_eq!(of("box1"), Some(0));
        assert_eq!(of("grid1"), Some(0));
        assert_eq!(of("copy1"), Some(1));
        assert_eq!(s.nodes.len(), 3, "hidden empty source must not be listed");
    }

    #[test]
    fn layout_siblings_and_cycle() {
        let p = layout(3, &[(0, 2, 0), (1, 2, 1)]);
        assert_eq!((p[0].x, p[1].x, p[2].x), (0.0, STEP.x, 0.0));
        assert_eq!((p[0].y, p[2].y), (0.0, STEP.y));
        assert_eq!(depths(2, &[(0, 1, 0), (1, 0, 0)]), vec![2, 2], "cycle terminates, bounded by n");
    }

    /// Synthetic-pointer harness: real egui input handling (press / drag / release), no window.
    struct Rig {
        ctx: egui::Context,
        t: f64,
        scene: Scene,
        pos: HashMap<NodeId, Pos2>,
        sel: BTreeSet<NodeId>,
        wire: Option<WireFrom>,
        origin: Pos2,
    }
    impl Rig {
        fn new(scene: Scene, pos: HashMap<NodeId, Pos2>) -> Rig {
            let mut r = Rig {
                ctx: egui::Context::default(),
                t: 0.0,
                scene,
                pos,
                sel: BTreeSet::new(),
                wire: None,
                origin: Pos2::ZERO,
            };
            r.frame(vec![], vec![]);
            r
        }
        fn frame(&mut self, events: Vec<egui::Event>, keys_acts: Vec<Act>) -> Vec<Act> {
            self.t += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 700.0))),
                time: Some(self.t),
                events,
                ..Default::default()
            };
            let mut acts = keys_acts;
            let Rig { ctx, scene, pos, sel, wire, origin, .. } = self;
            let mut out = ctx.run_ui(raw, |ui| {
                *origin = ui.cursor().min + vec2(12.0, 12.0);
                let c = Ctx { pos, selected: sel, active: None, wire };
                acts.extend(show(ui, scene, c));
            });
            out.textures_delta.clear();
            acts
        }
        fn at(&self, name: &str, f: impl Fn(Rect) -> Pos2) -> Pos2 {
            let id = self.scene.id(name).expect("node");
            f(Rect::from_min_size(self.origin + self.pos[&id].to_vec2(), BOX))
        }
        /// press at `from`, drag in steps to `to`, release; returns every action seen.
        fn drag(&mut self, from: Pos2, to: Pos2) -> Vec<Act> {
            let ptr = |p| egui::Event::PointerMoved(p);
            let btn = |p, pressed| egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let mut acts = self.frame(vec![ptr(from)], vec![]);
            acts.extend(self.frame(vec![btn(from, true)], vec![]));
            for i in 1..=6 {
                let p = from + (to - from) * (i as f32 / 6.0);
                acts.extend(self.frame(vec![ptr(p)], vec![]));
            }
            acts.extend(self.frame(vec![btn(to, false)], vec![]));
            acts.extend(self.frame(vec![], vec![]));
            acts
        }
    }
    fn rig() -> Rig {
        let mut s = Scene::parse(include_str!("../../../examples/polyextrude.json")).expect("scene");
        let b2 = s.add("box", Some("box2")).expect("adds");
        let pos = s
            .nodes
            .iter()
            .map(|n| {
                (n.id, if n.id == b2 { pos2(240.0, 0.0) } else { pos2(0.0, if n.name == "pe1" { 140.0 } else { 0.0 }) })
            })
            .collect();
        Rig::new(s, pos)
    }

    #[test]
    fn drag_output_dot_onto_input_dot_connects() {
        let mut r = rig();
        let (from, to) = (r.at("box2", out_dot), r.at("pe1", |rc| in_dot(rc, 1, 0)));
        let acts = r.drag(from, to);
        let (b2, pe) = (r.scene.id("box2").expect("b2"), r.scene.id("pe1").expect("pe"));
        assert!(acts.contains(&Act::Connect { src: b2, dst: pe, port: 0 }), "{acts:?}");
        assert_eq!(r.wire, None, "wire state cleared on release");
        assert!(!acts.iter().any(|a| matches!(a, Act::Select { .. })), "dot drag does not select");
    }

    #[test]
    fn drag_wire_off_input_disconnects_and_reroutes() {
        let mut r = rig();
        let (b1, pe) = (r.scene.id("box1").expect("b1"), r.scene.id("pe1").expect("pe"));
        let dot = r.at("pe1", |rc| in_dot(rc, 1, 0));
        let acts = r.drag(dot, dot + vec2(150.0, 150.0));
        assert!(acts.contains(&Act::Disconnect { dst: pe, port: 0 }), "{acts:?}");
        assert!(!acts.iter().any(|a| matches!(a, Act::Connect { .. })), "dropped on empty canvas");
        // same pick-up, dropped on box2... has no inputs; drop it back on pe1's own port instead
        let acts = r.drag(dot, r.at("pe1", |rc| in_dot(rc, 1, 0)) + vec2(2.0, 1.0));
        assert!(acts.contains(&Act::Disconnect { dst: pe, port: 0 }), "{acts:?}");
        assert!(acts.contains(&Act::Connect { src: b1, dst: pe, port: 0 }), "re-dropped: wire carried the old source");
    }

    #[test]
    fn node_body_click_selects_and_drag_moves() {
        let mut r = rig();
        let (c, b2) = (r.at("box2", |rc| rc.center()), r.scene.id("box2").expect("b2"));
        let acts = r.drag(c, c + vec2(30.0, 40.0));
        assert!(acts.contains(&Act::Select { id: b2, add: false }), "{acts:?}");
        let moved = r.pos[&b2];
        assert!((moved.x - 270.0).abs() < 8.0 && (moved.y - 40.0).abs() < 8.0, "moved by the drag: {moved:?}");
        assert!(!acts.iter().any(|a| matches!(a, Act::Connect { .. } | Act::Disconnect { .. })));
    }
}
