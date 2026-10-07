//! Params pane: selected node's params (registry defaults + scene overrides), F/I/S/V3/B/Menu widgets.
use crate::Scene;
use egui::Ui;
use hexe_graph::ParamValue;

/// Edit widget for one value; `true` when changed.
fn edit(ui: &mut Ui, v: &mut ParamValue, range: Option<(f32, f32)>) -> bool {
    let speed = range.map_or(0.01, |(lo, hi)| ((hi - lo) / 200.0).max(1e-4));
    match v {
        ParamValue::F(x) => ui.add(egui::DragValue::new(x).speed(speed)).changed(),
        ParamValue::I(x) => ui.add(egui::DragValue::new(x)).changed(),
        ParamValue::Menu(x) => ui.add(egui::DragValue::new(x)).changed(),
        ParamValue::B(x) => ui.checkbox(x, "").changed(),
        ParamValue::S(s) => ui.text_edit_singleline(s).changed(),
        ParamValue::V3(c) => ui
            .horizontal(|ui| c.iter_mut().fold(false, |ch, x| ui.add(egui::DragValue::new(x).speed(speed)).changed() || ch))
            .inner,
    }
}

/// `Ok(true)` = a param changed (caller re-cooks). `Err` = the graph rejected the value (shown in the status bar).
pub fn show(ui: &mut Ui, scene: &mut Scene, selected: usize) -> Result<bool, String> {
    let Some(node) = scene.nodes.get(selected) else { return Ok(false) };
    let (id, title) = (node.id, format!("{} ({})", node.name, node.ty));
    ui.label(title);
    ui.separator();
    let mut changed = false;
    for spec in scene.graph.param_specs(id) {
        let Some(cur) = scene.graph.param(id, &spec.name).cloned().or(Some(spec.default.clone())) else { continue };
        let mut v = cur.clone();
        let edited = ui
            .horizontal(|ui| {
                ui.label(&spec.label);
                edit(ui, &mut v, spec.range)
            })
            .inner;
        if edited && v != cur {
            scene.graph.set_param(id, &spec.name, v).map_err(|e| e.to_string())?;
            changed = true;
        }
    }
    Ok(changed)
}
