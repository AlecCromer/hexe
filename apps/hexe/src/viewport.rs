//! Viewport pane: wgpu 3D (see `gpu.rs`) inside the egui window. Houdini-style camera: orbit / pan / dolly, `H` home.
//! Mouse: alt+LMB (or plain LMB) orbit · MMB pan (shift+LMB too) · RMB drag / scroll dolly. Egui paints the overlays
//! (W wireframe, P point numbers, N prim numbers, axis gizmo) on top of the GPU image.
use crate::gpu::{Frame, Mesh, ViewCallback};
use eframe::egui_wgpu;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use glam::{Mat4, Vec3, Vec4};
use hexe_geo::{Detail, PrimKind};
use std::sync::Arc;

/// Default vertical field of view (radians).
pub const DEFAULT_FOV: f32 = 0.9;
/// Frame-all leaves this much air around the bounding sphere.
const FIT_MARGIN: f32 = 1.15;
/// Smallest orbit distance / largest orbit distance.
const DIST_RANGE: (f32, f32) = (1e-3, 1e6);
/// Most prim/point labels (and faces scanned for labels) per frame.
const MAX_LABELS: usize = 500;
const MAX_LABEL_PRIMS: usize = 100_000;

/// Every tunable of the viewport (colours are 0..1 gamma-space RGB). `Default` = the shipped look.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub background: [f32; 3],
    pub surface: [f32; 3],
    pub edge: [f32; 3],
    pub point: [f32; 3],
    pub grid: [f32; 3],
    pub axis_x: [f32; 3],
    pub axis_y: [f32; 3],
    pub axis_z: [f32; 3],
    /// Headlight: colour = surface * (ambient + diffuse * |N.V|).
    pub ambient: f32,
    pub diffuse: f32,
    /// Point radius in physical pixels, and its depth pull toward the camera (clip-w fraction) so points sit on surfaces.
    pub point_px: f32,
    pub point_depth_bias: f32,
    /// Ground grid: lines each side of the origin, spacing in world units.
    pub grid_half: u32,
    pub grid_step: f32,
    /// Near/far plane as multiples of the orbit distance.
    pub near_ratio: f32,
    pub far_ratio: f32,
    /// Radians per dragged point (orbit) · dolly exp-rate per dragged point · exp-rate per scroll point.
    pub orbit_rate: f32,
    pub dolly_rate: f32,
    pub scroll_rate: f32,
    pub pitch_limit: f32,
}
impl Default for Style {
    fn default() -> Self {
        Style {
            background: [0.15, 0.15, 0.15],
            surface: [0.55, 0.72, 0.95],
            edge: [0.92, 0.92, 0.96],
            point: [1.0, 0.67, 0.24],
            grid: [0.27, 0.27, 0.27],
            axis_x: [0.80, 0.27, 0.27],
            axis_y: [0.35, 0.78, 0.35],
            axis_z: [0.31, 0.51, 0.94],
            ambient: 0.25,
            diffuse: 0.75,
            point_px: 2.5,
            point_depth_bias: 2e-4,
            grid_half: 10,
            grid_step: 1.0,
            near_ratio: 0.01,
            far_ratio: 1000.0,
            orbit_rate: 0.01,
            dolly_rate: 0.005,
            scroll_rate: 0.005,
            pitch_limit: 1.5,
        }
    }
}
fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::from_array(a)
}

/// Orbit camera around `target` (Y up, right-handed).
#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub target: [f32; 3],
    pub fov: f32,
}
impl Default for Camera {
    fn default() -> Self {
        Camera { yaw: 0.6, pitch: 0.45, dist: 6.0, target: [0.0; 3], fov: DEFAULT_FOV }
    }
}
/// What a mouse drag does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    Orbit,
    Pan,
    Dolly,
}
/// Houdini mapping: LMB (alt+LMB) orbit, shift+LMB or MMB pan, RMB dolly. Alt never changes the meaning.
pub fn drag_action(button: egui::PointerButton, shift: bool) -> Option<Drag> {
    use egui::PointerButton::{Middle, Primary, Secondary};
    match button {
        Primary if shift => Some(Drag::Pan),
        Primary => Some(Drag::Orbit),
        Middle => Some(Drag::Pan),
        Secondary => Some(Drag::Dolly),
        _ => None,
    }
}
impl Camera {
    pub fn eye(&self) -> Vec3 {
        let (sy, cy, sp, cp) = (self.yaw.sin(), self.yaw.cos(), self.pitch.sin(), self.pitch.cos());
        v3(self.target) + self.dist * Vec3::new(cp * sy, sp, cp * cy)
    }
    /// (right, up) in world space.
    pub fn basis(&self) -> (Vec3, Vec3) {
        let fwd = (v3(self.target) - self.eye()).normalize_or(Vec3::NEG_Z);
        let right = fwd.cross(Vec3::Y).normalize_or(Vec3::X);
        (right, right.cross(fwd))
    }
    /// (view-projection, eye). Depth range 0..1 (wgpu).
    pub fn matrices(&self, s: &Style, aspect: f32) -> (Mat4, Vec3) {
        let eye = self.eye();
        let view = Mat4::look_at_rh(eye, v3(self.target), Vec3::Y);
        let d = self.dist.max(DIST_RANGE.0);
        let proj = Mat4::perspective_rh(self.fov, aspect.max(1e-3), d * s.near_ratio, d * s.far_ratio);
        (proj * view, eye)
    }
    /// Frame the bounding box (`H` / `F`, and on load).
    pub fn fit(&mut self, lo: [f32; 3], hi: [f32; 3]) {
        let (lo, hi) = (v3(lo), v3(hi));
        self.target = ((lo + hi) * 0.5).to_array();
        let r = ((hi - lo).length() * 0.5).max(DIST_RANGE.0);
        self.dist = (r / (self.fov * 0.5).sin() * FIT_MARGIN).clamp(DIST_RANGE.0, DIST_RANGE.1);
    }
    /// Apply a drag of `d` points in a viewport `height` points tall.
    pub fn drag(&mut self, a: Drag, d: egui::Vec2, height: f32, s: &Style) {
        match a {
            Drag::Orbit => {
                self.yaw -= d.x * s.orbit_rate;
                self.pitch = (self.pitch + d.y * s.orbit_rate).clamp(-s.pitch_limit, s.pitch_limit);
            }
            Drag::Pan => {
                let (right, up) = self.basis();
                let k = self.dist * 2.0 * (self.fov * 0.5).tan() / height.max(1.0);
                self.target = (v3(self.target) + (-right * d.x + up * d.y) * k).to_array();
            }
            Drag::Dolly => self.zoom((d.x - d.y) * s.dolly_rate),
        }
    }
    /// Multiply the distance by `exp(-amount)`; positive = in.
    pub fn zoom(&mut self, amount: f32) {
        self.dist = (self.dist * (-amount).exp()).clamp(DIST_RANGE.0, DIST_RANGE.1);
    }
}

/// World -> screen for the egui overlays, same matrix as the GPU image.
struct Proj {
    vp: Mat4,
    rect: Rect,
}
impl Proj {
    /// Screen position + clip depth (w); `None` outside the depth range.
    fn project(&self, p: [f32; 3]) -> Option<(Pos2, f32)> {
        let c = self.vp * Vec4::new(p[0], p[1], p[2], 1.0);
        if c.w <= 1e-6 || c.z < 0.0 || c.z > c.w {
            return None;
        }
        Some((
            pos2(
                self.rect.min.x + (c.x / c.w * 0.5 + 0.5) * self.rect.width(),
                self.rect.min.y + (0.5 - c.y / c.w * 0.5) * self.rect.height(),
            ),
            c.w,
        ))
    }
}

/// Viewport display toggles: `W` wireframe/shaded, `P` point numbers, `N` primitive numbers.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Opts {
    pub wire: bool,
    pub point_nums: bool,
    pub prim_nums: bool,
}
impl Opts {
    /// Status-bar summary of the active toggles.
    pub fn label(&self) -> String {
        let mut s = String::from(if self.wire { "wireframe" } else { "shaded" });
        for (on, n) in [(self.point_nums, "P"), (self.prim_nums, "N")] {
            if on {
                s.push_str(" +");
                s.push_str(n);
            }
        }
        s
    }
}

/// Camera input, then the wgpu image (grid, axes, display geometry) and the egui overlays.
pub fn show(
    ui: &mut Ui,
    cam: &mut Camera,
    detail: Option<&Detail>,
    mesh: Option<&Arc<Mesh>>,
    opts: &mut Opts,
    style: &Style,
) {
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    if resp.dragged() {
        let (d, shift) = (resp.drag_delta(), ui.input(|i| i.modifiers.shift));
        for b in [egui::PointerButton::Primary, egui::PointerButton::Middle, egui::PointerButton::Secondary] {
            if resp.dragged_by(b)
                && let Some(a) = drag_action(b, shift)
            {
                cam.drag(a, d, rect.height(), style);
            }
        }
    }
    if resp.hovered() && !ui.ctx().egui_wants_keyboard_input() {
        ui.input(|i| {
            opts.wire ^= i.key_pressed(egui::Key::W);
            opts.point_nums ^= i.key_pressed(egui::Key::P);
            opts.prim_nums ^= i.key_pressed(egui::Key::N);
        });
    }
    if resp.hovered() {
        let s = ui.input(|i| i.smooth_scroll_delta.y);
        cam.zoom(s * style.scroll_rate);
    }
    let ppp = ui.ctx().pixels_per_point();
    let px = |v: f32| (v * ppp).round().max(1.0) as u32;
    let frame = Frame::new(cam, style, opts, [px(rect.width()), px(rect.height())]);
    let painter = ui.painter_at(rect);
    painter.add(egui_wgpu::Callback::new_paint_callback(rect, ViewCallback { mesh: mesh.cloned(), frame }));
    let proj = Proj { vp: frame.view_proj, rect };
    if let Some(d) = detail {
        overlays(&painter, &proj, d, opts);
    }
    gizmo(&painter, cam, rect);
}

fn overlays(painter: &egui::Painter, pr: &Proj, d: &Detail, opts: &Opts) {
    let label = |at: Pos2, text: String, col: Color32| {
        painter.text(at + vec2(4.0, -4.0), egui::Align2::LEFT_BOTTOM, text, egui::FontId::monospace(10.0), col);
    };
    if opts.point_nums {
        let mut pts: Vec<(f32, usize, Pos2)> =
            d.p().iter().enumerate().filter_map(|(i, &p)| pr.project(p).map(|(s, z)| (z, i, s))).collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, i, s) in pts.into_iter().take(MAX_LABELS) {
            label(s, i.to_string(), Color32::from_rgb(255, 200, 100));
        }
    }
    if opts.prim_nums {
        // nearest-first so the cap keeps the faces the user is looking at
        let mut faces: Vec<(f32, usize, Pos2)> = Vec::new();
        for (pi, prim) in d.prims.iter().enumerate().take(MAX_LABEL_PRIMS) {
            if !matches!(prim.kind, PrimKind::Poly { .. } | PrimKind::Polyline) {
                continue;
            }
            let ids = d.vertex_point.get(prim.verts.start as usize..prim.verts.end as usize).unwrap_or(&[]);
            let w: Vec<[f32; 3]> = ids.iter().filter_map(|&i| d.p().get(i as usize).copied()).collect();
            if w.is_empty() {
                continue;
            }
            let c = w.iter().fold(Vec3::ZERO, |a, &p| a + v3(p)) / w.len() as f32;
            if let Some((s, z)) = pr.project(c.to_array()) {
                faces.push((z, pi, s));
            }
        }
        faces.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, pi, s) in faces.into_iter().take(MAX_LABELS) {
            label(s, pi.to_string(), Color32::from_rgb(120, 230, 255));
        }
    }
}

fn gizmo(painter: &egui::Painter, cam: &Camera, rect: Rect) {
    let (right, up) = cam.basis();
    let o = rect.left_bottom() + vec2(44.0, -44.0);
    for (axis, col, name) in [
        (Vec3::X, Color32::from_rgb(230, 70, 70), "X"),
        (Vec3::Y, Color32::from_rgb(90, 210, 90), "Y"),
        (Vec3::Z, Color32::from_rgb(80, 130, 240), "Z"),
    ] {
        let tip = o + vec2(axis.dot(right), -axis.dot(up)) * 30.0;
        painter.line_segment([o, tip], Stroke::new(2.0, col));
        painter.text(tip, egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(12.0), col);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::PointerButton::{Extra1, Middle, Primary, Secondary};

    fn ndc(cam: &Camera, p: [f32; 3]) -> Vec3 {
        let (vp, _) = cam.matrices(&Style::default(), 1.5);
        let c = vp * Vec4::new(p[0], p[1], p[2], 1.0);
        c.truncate() / c.w
    }

    #[test]
    fn houdini_mouse_mapping() {
        assert_eq!(drag_action(Primary, false), Some(Drag::Orbit)); // alt+LMB and LMB
        assert_eq!(drag_action(Primary, true), Some(Drag::Pan));
        assert_eq!(drag_action(Middle, false), Some(Drag::Pan));
        assert_eq!(drag_action(Secondary, false), Some(Drag::Dolly));
        assert_eq!(drag_action(Extra1, false), None);
    }

    #[test]
    fn orbit_clamps_pitch_and_pan_moves_in_the_view_plane() {
        let (s, mut c) = (Style::default(), Camera::default());
        c.drag(Drag::Orbit, vec2(0.0, 1e6), 600.0, &s);
        assert!((c.pitch - s.pitch_limit).abs() < 1e-6);
        let mut c = Camera::default();
        let (e0, (right, up)) = (c.eye(), c.basis());
        c.drag(Drag::Pan, vec2(30.0, -20.0), 600.0, &s);
        let d = c.eye() - e0;
        let fwd = (v3(c.target) - c.eye()).normalize();
        assert!(d.dot(fwd).abs() < 1e-4, "pan stays in the view plane");
        assert!(d.dot(right) < 0.0 && d.dot(up) < 0.0, "scene follows the cursor: camera moves opposite");
    }

    #[test]
    fn dolly_and_scroll_zoom_in_and_clamp() {
        let (s, mut c) = (Style::default(), Camera::default());
        c.drag(Drag::Dolly, vec2(40.0, 0.0), 600.0, &s);
        assert!(c.dist < 6.0, "drag right = in");
        c.zoom(-1e9);
        assert_eq!(c.dist, DIST_RANGE.1);
        c.zoom(1e9);
        assert_eq!(c.dist, DIST_RANGE.0);
    }

    #[test]
    fn target_projects_to_screen_centre_and_home_frames_the_box() {
        let mut c = Camera::default();
        let t = ndc(&c, c.target);
        assert!(t.x.abs() < 1e-5 && t.y.abs() < 1e-5 && (0.0..1.0).contains(&t.z));
        let (lo, hi) = ([-2.0, 0.0, -1.0], [3.0, 4.0, 1.0]);
        c.fit(lo, hi);
        for i in 0..8 {
            let p = [if i & 1 == 0 { lo[0] } else { hi[0] }, if i & 2 == 0 { lo[1] } else { hi[1] }, if i & 4 == 0 { lo[2] } else { hi[2] }];
            let n = ndc(&c, p);
            assert!(n.x.abs() <= 1.0 && n.y.abs() <= 1.0 && (0.0..=1.0).contains(&n.z), "corner {p:?} -> {n:?}");
        }
    }

    #[test]
    fn overlay_projection_matches_the_gpu_matrix() {
        let c = Camera::default();
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 200.0));
        let (vp, _) = c.matrices(&Style::default(), 1.5);
        let (s, z) = (Proj { vp, rect }).project(c.target).expect("visible");
        assert!((s.x - 160.0).abs() < 1e-3 && (s.y - 120.0).abs() < 1e-3, "{s:?}");
        assert!((z - c.dist).abs() < 1e-3);
        assert!((Proj { vp, rect }).project(c.eye().to_array()).is_none(), "the eye itself is not drawable");
    }

    #[test]
    fn opts_label() {
        assert_eq!(Opts::default().label(), "shaded");
        assert_eq!(Opts { wire: true, point_nums: true, prim_nums: true }.label(), "wireframe +P +N");
    }
}
