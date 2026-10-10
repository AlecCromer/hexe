//! Cook-model parity (ledger `core.cook-model`): the engine (`hexe_graph::Graph`) driven through the SAME event sequence a real
//! Houdini 20.5 (hython, `oracle/houdini/probe-cook-model.py`) was driven through, compared step by step with the recorded
//! `needsToCook()` / `isTimeDependent()` per node (`fixtures/core-cook-model/houdini.json`; chain box -> xform -> null).
//! hexe side: `Const` -> `Inc` -> `Pass` (payload i32); the `$F` expression is a node property here, so the time-dependent half
//! swaps `Inc` for `TimeInc`. Measured differences (NOT hidden, pinned below): Houdini dirties on a same-value parm set, hexe keeps
//! the cache; Houdini `cook(force=True)` has no hexe counterpart (cooks are demand-driven only).
use hexe_graph::{CookCtx, CookError, Graph, GraphError, InputSpec, Node, NodeId, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

struct Const(Vec<ParamSpec>);
impl Node<i32> for Const {
    fn type_name(&self) -> &'static str {
        "const"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.0
    }
    fn cook(&self, ctx: &CookCtx, _: &[Arc<i32>]) -> Result<Arc<i32>, CookError> {
        Ok(Arc::new(ctx.params.int("value").unwrap_or_default()))
    }
}
struct Inc(Vec<ParamSpec>, bool);
impl Node<i32> for Inc {
    fn type_name(&self) -> &'static str {
        "inc"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "in" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.0
    }
    fn time_dependent(&self) -> bool {
        self.1
    }
    fn cook(&self, ctx: &CookCtx, i: &[Arc<i32>]) -> Result<Arc<i32>, CookError> {
        let by = ctx.params.int("by").unwrap_or_default();
        let t = if self.1 { ctx.time as i32 } else { 0 };
        i.first().map(|v| Arc::new(**v + by + t)).ok_or_else(|| CookError::Failed("no input".into()))
    }
}
struct Pass;
impl Node<i32> for Pass {
    fn type_name(&self) -> &'static str {
        "pass"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "in" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &[]
    }
    fn cook(&self, _: &CookCtx, i: &[Arc<i32>]) -> Result<Arc<i32>, CookError> {
        i.first().cloned().ok_or_else(|| CookError::Failed("no input".into()))
    }
}

const FIXTURE: &str = include_str!("../../../fixtures/core-cook-model/houdini.json");

struct Rig {
    g: Graph<i32>,
    ids: [NodeId; 3],
}
fn rig(time_dependent: bool) -> Rig {
    let mut g: Graph<i32> = Graph::new();
    let b = g.add_node(Const(vec![ParamSpec::new("value", ParamValue::I(1))]), ParamSet::new()).expect("b");
    let x = g.add_node(Inc(vec![ParamSpec::new("by", ParamValue::I(0))], time_dependent), ParamSet::new()).expect("x");
    let n = g.add_node(Pass, ParamSet::new()).expect("n");
    g.connect(b, 0, x, 0).expect("b->x");
    g.connect(x, 0, n, 0).expect("x->n");
    Rig { g, ids: [b, x, n] }
}

fn step(fix: &serde_json::Value, name: &str) -> serde_json::Value {
    fix["observations"]
        .as_array()
        .and_then(|a| a.iter().find(|o| o["step"] == name))
        .cloned()
        .unwrap_or_else(|| panic!("fixture lacks step {name}"))
}
/// Compare `needs` / `timedep` of b, x, n at `time` with the recorded Houdini step.
fn same(fix: &serde_json::Value, name: &str, r: &Rig, time: f32) {
    let o = step(fix, name);
    for (id, key) in r.ids.iter().zip(["b", "x", "n"]) {
        assert_eq!(
            r.g.needs_to_cook(*id, time),
            o[key]["needs"].as_bool().expect("needs"),
            "{name}: needsToCook({key})"
        );
        assert_eq!(
            r.g.is_time_dependent(*id),
            o[key]["timedep"].as_bool().expect("timedep"),
            "{name}: isTimeDependent({key})"
        );
    }
}

#[test]
fn engine_follows_houdinis_recorded_cook_model_step_by_step() {
    let fix: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture parses");
    assert!(fix["houdini"].as_str().is_some_and(|v| v.contains("20.5")));
    let mut r = rig(false);
    let [b, x, n] = r.ids;
    // fresh nodes need a cook; cooking the tail cooks the whole chain (pull); a second cook is free
    same(&fix, "fresh", &r, 0.0);
    r.g.cook(n, 0.0).expect("cooks");
    same(&fix, "cook_tail", &r, 0.0);
    r.g.cook(n, 0.0).expect("cooks");
    same(&fix, "cook_tail_again", &r, 0.0);
    // DEVIATION (measured): Houdini dirties x and n when a parm is set to the value it already has; hexe keeps the cache
    let o = step(&fix, "set_same_value");
    assert!(
        o["x"]["needs"].as_bool().expect("x")
            && o["n"]["needs"].as_bool().expect("n")
            && !o["b"]["needs"].as_bool().expect("b")
    );
    r.g.set_param(x, "by", ParamValue::I(0)).expect("set");
    assert!(!r.g.needs_to_cook(x, 0.0) && !r.g.needs_to_cook(n, 0.0), "hexe: same value = no-op");
    // a real edit dirties the node and everything downstream, nothing upstream
    r.g.set_param(x, "by", ParamValue::I(2)).expect("set");
    same(&fix, "set_new_value", &r, 0.0);
    assert_eq!(*r.g.cook(n, 0.0).expect("cooks"), 3);
    same(&fix, "cook_after_new_value", &r, 0.0);
    // a frame change leaves a static chain alone
    same(&fix, "frame_change_static_chain", &r, 5.0);
    r.g.cook(n, 5.0).expect("cooks");
    same(&fix, "cook_after_frame_change", &r, 5.0);
    // upstream edit dirties the whole cone below it
    r.g.set_param(b, "value", ParamValue::I(7)).expect("set");
    same(&fix, "upstream_parm_set", &r, 5.0);
    r.g.cook(n, 5.0).expect("cooks");
    same(&fix, "cook_after_upstream", &r, 5.0);
    // rewiring dirties downstream
    r.g.disconnect(x, 0).expect("disconnect");
    same(&fix, "disconnect_input", &r, 5.0);
    r.g.connect(b, 0, x, 0).expect("reconnect");
    same(&fix, "reconnect_input", &r, 5.0);
    r.g.cook(n, 5.0).expect("cooks");
    same(&fix, "cook_after_reconnect", &r, 5.0);
    // a cycle is refused at connect time (Houdini raises OperationFailed)
    assert_eq!(step(&fix, "cycle_connect")["raised"], true);
    assert!(matches!(r.g.connect(n, 0, x, 0), Err(GraphError::Cycle(_))));
}

#[test]
fn time_dependence_propagates_downstream_and_frame_change_dirties_only_that_cone() {
    let fix: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture parses");
    let mut r = rig(true);
    let [b, _x, n] = r.ids;
    // Houdini's recorded state right after `$F` is put on x: b clean, x and n need a cook, x AND n are time dependent
    r.g.cook(b, 5.0).expect("cooks");
    same(&fix, "expr_F_set", &r, 5.0);
    r.g.cook(n, 5.0).expect("cooks");
    same(&fix, "cook_with_expr", &r, 5.0);
    same(&fix, "frame_change_timedep", &r, 6.0);
    let v6 = *r.g.cook(n, 6.0).expect("cooks");
    same(&fix, "cook_after_timedep_frame", &r, 6.0);
    let v7 = *r.g.cook(n, 7.0).expect("cooks");
    let tv = step(&fix, "timedep_values");
    let dh = tv["x_frame7"].as_f64().expect("f7") - tv["x_frame6"].as_f64().expect("f6");
    assert!(
        (f64::from(v7 - v6) - dh).abs() < 1e-9,
        "one frame later the output moved by the same step as in Houdini ({dh})"
    );
}

#[test]
fn needs_to_cook_and_is_time_dependent_hostile_ids() {
    let mut r = rig(false);
    let [_, x, n] = r.ids;
    r.g.remove_node(n).expect("removed");
    assert!(!r.g.needs_to_cook(n, 0.0) && !r.g.is_time_dependent(n), "removed id");
    assert!(r.g.needs_to_cook(x, f32::NAN), "never cooked");
    r.g.cook(x, f32::NAN).expect("cooks");
    assert!(!r.g.needs_to_cook(x, f32::NAN), "static chain ignores time, even NaN");
}
