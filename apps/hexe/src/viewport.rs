//! Viewport pane: painter-based 3D (no wgpu). Orbit camera, perspective projection, depth-sorted faces by centroid.
use egui::{Color32, Pos2, Sense, Shape, Stroke, Ui, pos2, vec2};
use hexe_geo::{Detail, PrimKind};

type V3 = [f32; 3];
const FOV: f32 = 0.9;
const NEAR: f32 = 1e-3;
/// Draw caps (never let a huge cook freeze the UI).
const MAX_FACES: usize = 100_000;
const MAX_POINTS: usize = 200_000;

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l > 1e-12 { [a[0] / l, a[1] / l, a[2] / l] } else { [0.0, 0.0, 1.0] }
}

/// Orbit camera around `target`.
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub target: V3,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { yaw: 0.6, pitch: 0.45, dist: 6.0, target: [0.0; 3] }
    }
}

/// Per-frame projection state.
struct View {
    eye: V3,
    right: V3,
    up: V3,
    fwd: V3,
    focal: f32,
    center: Pos2,
}

impl View {
    /// Screen position + view depth; `None` behind the near plane.
    fn project(&self, p: V3) -> Option<(Pos2, f32)> {
        let d = sub(p, self.eye);
        let z = dot(d, self.fwd);
        (z > NEAR).then(|| (pos2(self.center.x + self.focal * dot(d, self.right) / z, self.center.y - self.focal * dot(d, self.up) / z), z))
    }
}

impl Camera {
    fn view(&self, rect: egui::Rect) -> View {
        let (sy, cy, sp, cp) = (self.yaw.sin(), self.yaw.cos(), self.pitch.sin(), self.pitch.cos());
        let off = [cp * sy, sp, cp * cy];
        let eye = [self.target[0] + self.dist * off[0], self.target[1] + self.dist * off[1], self.target[2] + self.dist * off[2]];
        let fwd = norm(sub(self.target, eye));
        let right = norm(cross(fwd, [0.0, 1.0, 0.0]));
        let up = cross(right, fwd);
        View { eye, right, up, fwd, focal: rect.height() * 0.5 / (FOV * 0.5).tan(), center: rect.center() }
    }

    /// Frame the bounding box (F key, and on load).
    pub fn fit(&mut self, lo: V3, hi: V3) {
        self.target = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
        let r = (dot(sub(hi, lo), sub(hi, lo)).sqrt() * 0.5).max(1e-3);
        self.dist = r / (FOV * 0.5).sin() * 1.15;
    }
}

struct Face {
    depth: f32,
    start: usize,
    len: usize,
    closed: bool,
    shade: f32,
}

/// Orbit/pan/dolly input, then draw `detail` (points + polygon edges) and the axis gizmo.
pub fn show(ui: &mut Ui, cam: &mut Camera, detail: Option<&Detail>) {
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = resp.rect;
    painter.rect_filled(rect, 0.0, Color32::from_gray(38));
    if resp.dragged() {
        let (d, shift) = (resp.drag_delta(), ui.input(|i| i.modifiers.shift));
        if shift {
            let v = cam.view(rect);
            let k = cam.dist * 2.0 * (FOV * 0.5).tan() / rect.height().max(1.0);
            for i in 0..3 {
                cam.target[i] += (-v.right[i] * d.x + v.up[i] * d.y) * k;
            }
        } else {
            cam.yaw -= d.x * 0.01;
            cam.pitch = (cam.pitch + d.y * 0.01).clamp(-1.5, 1.5);
        }
    }
    if resp.hovered() {
        let s = ui.input(|i| i.smooth_scroll_delta.y);
        cam.dist = (cam.dist * (-s * 0.005).exp()).clamp(1e-3, 1e6);
    }
    let v = cam.view(rect);
    if let Some(d) = detail {
        draw_detail(&painter, &v, d);
    }
    gizmo(&painter, &v, rect);
}

fn draw_detail(painter: &egui::Painter, v: &View, d: &Detail) {
    let pts: Vec<Option<(Pos2, f32)>> = d.p().iter().map(|&p| v.project(p)).collect();
    let (mut flat, mut faces): (Vec<Pos2>, Vec<Face>) = (Vec::new(), Vec::new());
    for prim in d.prims.iter().take(MAX_FACES) {
        let closed = match prim.kind {
            PrimKind::Poly { closed } => closed,
            PrimKind::Polyline => false,
            _ => continue,
        };
        let (a, b) = (prim.verts.start as usize, prim.verts.end as usize);
        let ids = d.vertex_point.get(a..b).unwrap_or(&[]);
        let (start, mut depth, mut ok) = (flat.len(), 0.0, ids.len() >= 2);
        let mut world: Vec<V3> = Vec::with_capacity(ids.len());
        for &pid in ids {
            match (pts.get(pid as usize).copied().flatten(), d.p().get(pid as usize)) {
                (Some((s, z)), Some(&w)) => {
                    flat.push(s);
                    world.push(w);
                    depth += z / ids.len() as f32;
                }
                _ => ok = false,
            }
        }
        if !ok {
            flat.truncate(start);
            continue;
        }
        // Newell normal -> |n . view dir| as the face shade.
        let mut n = [0.0f32; 3];
        for (i, p) in world.iter().enumerate() {
            let q = world.get((i + 1) % world.len()).copied().unwrap_or(*p);
            n[0] += (p[1] - q[1]) * (p[2] + q[2]);
            n[1] += (p[2] - q[2]) * (p[0] + q[0]);
            n[2] += (p[0] - q[0]) * (p[1] + q[1]);
        }
        let shade = dot(norm(n), v.fwd).abs();
        faces.push(Face { depth, start, len: flat.len() - start, closed, shade });
    }
    faces.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    let edge = Stroke::new(1.0, Color32::from_rgb(235, 235, 245));
    for f in &faces {
        let Some(poly) = flat.get(f.start..f.start + f.len) else { continue };
        if f.closed && f.len >= 3 {
            let g = 55.0 + 150.0 * f.shade;
            painter.add(Shape::convex_polygon(poly.to_vec(), Color32::from_rgb((g * 0.6) as u8, (g * 0.8) as u8, g as u8), edge));
        } else {
            painter.add(Shape::line(poly.to_vec(), edge));
        }
    }
    for (s, _) in pts.iter().flatten().take(MAX_POINTS) {
        painter.circle_filled(*s, 2.2, Color32::from_rgb(255, 170, 60));
    }
}

fn gizmo(painter: &egui::Painter, v: &View, rect: egui::Rect) {
    let o = rect.left_bottom() + vec2(44.0, -44.0);
    for (axis, col, name) in [([1.0, 0.0, 0.0], Color32::from_rgb(230, 70, 70), "X"), ([0.0, 1.0, 0.0], Color32::from_rgb(90, 210, 90), "Y"), ([0.0, 0.0, 1.0], Color32::from_rgb(80, 130, 240), "Z")] {
        let tip = o + vec2(dot(axis, v.right), -dot(axis, v.up)) * 30.0;
        painter.line_segment([o, tip], Stroke::new(2.0, col));
        painter.text(tip, egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(12.0), col);
    }
}
