"""Oracle probe (hython 20.5, oracle tooling): SOP cook model -> fixtures/core-cook-model/houdini.json.
Chain box -> xform -> null; observes needsToCook / cookCount / isTimeDependent through dirty, same-value set, real set,
force cook, $F expression + frame change, input reconnect. Run: hython oracle/houdini/probe-cook-model.py"""
import json, os
import hou
ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
D = os.path.join(ROOT, "fixtures", "core-cook-model")
os.makedirs(D, exist_ok=True)
obs = []


def snap(label, nodes):
    obs.append({"step": label, **{n.name(): {"needs": bool(n.needsToCook()), "count": int(n.cookCount()), "timedep": bool(n.isTimeDependent())} for n in nodes}})


hou.setFrame(1)
g = hou.node("/obj").createNode("geo", "g")
b = g.createNode("box", "b")
x = g.createNode("xform", "x")
n = g.createNode("null", "n")
x.setInput(0, b)
n.setInput(0, x)
N = [b, x, n]
snap("fresh", N)
n.cook()
snap("cook_tail", N)
n.cook()
snap("cook_tail_again", N)
x.parm("tx").set(0)
snap("set_same_value", N)
n.cook()
snap("cook_after_same_value", N)
x.parm("tx").set(2)
snap("set_new_value", N)
n.cook()
snap("cook_after_new_value", N)
n.cook(force=True)
snap("force_cook_tail", N)
hou.setFrame(5)
snap("frame_change_static_chain", N)
n.cook()
snap("cook_after_frame_change", N)
x.parm("tx").setExpression("$F")
snap("expr_F_set", N)
n.cook()
snap("cook_with_expr", N)
hou.setFrame(6)
snap("frame_change_timedep", N)
n.cook()
snap("cook_after_timedep_frame", N)
tx6 = n.geometry().points()[0].position()[0]
hou.setFrame(7)
n.cook()
tx7 = n.geometry().points()[0].position()[0]
obs.append({"step": "timedep_values", "x_frame6": round(tx6, 6), "x_frame7": round(tx7, 6)})
x.parm("tx").deleteAllKeyframes()
x.parm("tx").set(0)
n.cook()
b.parm("sizex").set(3)
snap("upstream_parm_set", N)
n.cook()
snap("cook_after_upstream", N)
x.setInput(0, None)
snap("disconnect_input", N)
x.setInput(0, b)
snap("reconnect_input", N)
n.cook()
snap("cook_after_reconnect", N)
cyc = None
try:
    b.setInput(0, n)
    n.cook()
    cyc = {"raised": False, "errors": list(n.errors()) + list(b.errors())}
except hou.Error as e:
    cyc = {"raised": True, "type": type(e).__name__}
obs.append({"step": "cycle_connect", **cyc})
with open(os.path.join(D, "houdini.json"), "w") as f:
    json.dump({"case": "core-cook-model", "houdini": hou.applicationVersionString(), "chain": "box b -> xform x (tx) -> null n", "observations": obs}, f, indent=1)
    f.write("\n")
print(json.dumps(obs, indent=0)[:6000])
