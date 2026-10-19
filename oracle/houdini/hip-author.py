"""H40 oracle tooling (hython 20.5): AUTHOR the committed .hip fixtures (our own scenes, rights clean) + their Houdini twin .geo.
usage: hython hip-author.py <fixtures/hip dir> [name ...]   (names = only re-record those fixtures)
Writes authored-chain.hip (box -> xform, grid, merge; display merge1), authored-unsup.hip (box -> point[legacy SOP, no hexe twin]),
authored-dots.hip (network dots: box -> __dot1 -> xform1, __dot1 -> __dot2 -> merge1 input 0 (fan-out + chain), pinned
unwired-output __dot3), <name>.houdini.geo (display SOP cooked at the saved frame) and <name>.json (what the scene contains).
No author identity in committed fixtures: run with USER=hexe USERNAME=hexe JOB=C:/hexe (or /hexe), HOUDINI_USER_PREF_DIR
and the output dir outside your home; the machine name (from the OS, not settable) is overwritten after save with a
same-length placeholder (lengths kept, so container offsets stay valid; Houdini re-opens the file)."""
import json, os, socket, sys
import hou
out = sys.argv[1]
only = set(sys.argv[2:])
def fresh():
    hou.hipFile.clear(suppress_save_prompt=True)
    return hou.node("/obj").createNode("geo", "geo1")
def scrub(path):
    host = socket.gethostname().encode()
    with open(path, "rb") as f:
        b = f.read()
    with open(path, "wb") as f:
        f.write(b.replace(host, (b"hexe-fixture" + b"-" * len(host))[: len(host)]))
def save(name, disp):
    if only and name not in only:
        return
    disp.setDisplayFlag(True); disp.setRenderFlag(True)
    disp.cook(force=True)
    path = os.path.join(out, name + ".hip")
    hou.hipFile.save(path)
    geo = disp.geometry()
    geo.saveToFile(os.path.join(out, name + ".houdini.geo"))
    scrub(path)
    scrub(os.path.join(out, name + ".houdini.geo"))
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
g = fresh()
for c in g.children():
    c.destroy()
b = g.createNode("box", "box1"); b.setPosition((0, 0))
x = g.createNode("xform", "xform1"); x.setPosition((0, -4)); x.parmTuple("t").set((0, 2, 0))
m = g.createNode("merge", "merge1"); m.setPosition((3, -6))
d1 = g.createNetworkDot(); d1.setPosition((0.5, -2)); d1.setInput(b, 0)
x.setInput(0, d1)
d2 = g.createNetworkDot(); d2.setPosition((2, -3)); d2.setInput(d1, 0)
m.setInput(0, d2); m.setInput(1, x)
d3 = g.createNetworkDot(); d3.setPosition((4, -1)); d3.setInput(b, 0); d3.setPinned(True)
save("authored-dots", m)
