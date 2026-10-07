//! Network pane: nodes as boxes (top-down by topological depth, x by sibling index), typed-input wires,
//! click = select (= display/cook target), drag = move (positions live in app state only).
use crate::Scene;
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};

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

/// Draw the network; returns the newly selected node index when the user clicks/drags a node.
pub fn show(ui: &mut Ui, scene: &Scene, pos: &mut [Pos2], selected: usize) -> Option<usize> {
    let (bg, painter) = ui.allocate_painter(ui.available_size(), Sense::hover());
    painter.rect_filled(bg.rect, 2.0, Color32::from_gray(28));
    let origin = bg.rect.min + vec2(12.0, 12.0);
    let rect_of = |p: Pos2| Rect::from_min_size(origin + p.to_vec2(), BOX);
    for &(s, t, port) in &scene.edges {
        let (Some(&ps), Some(&pt)) = (pos.get(s), pos.get(t)) else { continue };
        let ports = scene.graph.input_sources(scene.nodes.get(t).map(|n| n.id)?).len().max(1);
        let a = rect_of(ps).center_bottom();
        let r = rect_of(pt);
        let b = pos2(r.left() + r.width() * (port as f32 + 1.0) / (ports as f32 + 1.0), r.top());
        painter.line_segment([a, b], Stroke::new(1.5, Color32::from_rgb(120, 160, 200)));
        painter.text(
            b + vec2(3.0, -9.0),
            egui::Align2::LEFT_CENTER,
            port.to_string(),
            FontId::proportional(9.0),
            Color32::GRAY,
        );
    }
    let mut picked = None;
    for (i, node) in scene.nodes.iter().enumerate() {
        let Some(&p) = pos.get(i) else { continue };
        let r = rect_of(p);
        let resp = ui.interact(r, ui.id().with(("node", i)), Sense::click_and_drag());
        if resp.dragged()
            && let Some(slot) = pos.get_mut(i)
        {
            *slot += resp.drag_delta();
        }
        if resp.clicked() || resp.drag_started() {
            picked = Some(i);
        }
        let sel = i == selected;
        let fill = if sel { Color32::from_rgb(70, 90, 130) } else { Color32::from_gray(55) };
        let edge = if sel { Color32::from_rgb(255, 200, 80) } else { Color32::from_gray(140) };
        painter.rect(r, 4.0, fill, Stroke::new(if sel { 2.0 } else { 1.0 }, edge), egui::StrokeKind::Inside);
        painter.text(
            r.center_top() + vec2(0.0, 5.0),
            egui::Align2::CENTER_TOP,
            &node.name,
            FontId::proportional(12.0),
            Color32::WHITE,
        );
        painter.text(
            r.center_bottom() - vec2(0.0, 4.0),
            egui::Align2::CENTER_BOTTOM,
            node.ty,
            FontId::proportional(9.0),
            Color32::GRAY,
        );
        if sel {
            painter.circle_filled(r.right_top() + vec2(-6.0, 6.0), 3.5, Color32::from_rgb(80, 200, 255));
        }
    }
    picked.filter(|&i| i != selected)
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
}
