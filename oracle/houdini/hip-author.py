"""H40 oracle tooling (hython 20.5): AUTHOR the committed .hip fixtures (our own scenes, rights clean) + their Houdini twin .geo.
usage: hython hip-author.py <fixtures/hip dir>
Writes authored-chain.hip (box -> xform, grid, merge; display merge1), authored-unsup.hip (box -> point[legacy SOP, no hexe twin]),
<name>.houdini.geo (display SOP cooked at the saved frame) and <name>.json (what the scene contains)."""
import json, os, sys
import hou
out = sys.argv[1]
def fresh():
    hou.hipFile.clear(suppress_save_prompt=True)
    return hou.node("/obj").createNode("geo", "geo1")
def save(name, disp):
    disp.setDisplayFlag(True); disp.setRenderFlag(True)
    disp.cook(force=True)
    path = os.path.join(out, name + ".hip")
    hou.hipFile.save(path)
    geo = disp.geometry()
    geo.saveToFile(os.path.join(out, name + ".houdini.geo"))
    with open(os.path.join(out, name + ".json"), "w") as f:
        json.dump({"houdini": hou.applicationVersionString(), "frame": hou.frame(), "display": disp.name(), "points": len(geo.points()), "prims": len(geo.prims())}, f, indent=1)
    print(name, len(geo.points()), len(geo.prims()))
g = fresh()
for c in g.children():
    c.destroy()
b = g.createNode("box", "box1"); b.parmTuple("size").set((2, 1, 3))
x = g.createNode("xform", "xform1"); x.setInput(0, b); x.parmTuple("t").set((1, 2, 3)); x.parmTuple("s").set((1, 2, 1))
r = g.createNode("grid", "grid1"); r.parm("rows").set(4); r.parm("cols").set(3); r.parmTuple("size").set((6, 4))
m = g.createNode("merge", "merge1"); m.setInput(0, x); m.setInput(1, r)
save("authored-chain", m)
g = fresh()
for c in g.children():
    c.destroy()
b = g.createNode("box", "box1")
p = g.createNode("point", "point1"); p.setInput(0, b)
save("authored-unsup", p)
