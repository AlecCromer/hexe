from _common import *
SOP = "sop-ray"
NOTE = "grid 3x3 0.8x0.8 at y=3 projected along -Y onto a unit box (ray.json); Houdini method project, dirmethod vector"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "grid1", "type": "grid", "params": {"rows": 3, "cols": 3, "size": [0.8, 0.8, 0], "t": [0, 3, 0]}}, {"name": "ray1", "type": "ray", "params": {"method": 1, "dir": [0, -1, 0]}}], "edges": [["grid1", 0, "ray1", 0], ["box1", 0, "ray1", 1]], "display": "ray1"}
def build(net):
    b = mk(net, "box", "box1")
    g = mk(net, "grid", "grid1", rows=3, cols=3, size=(0.8, 0.8), t=(0, 3, 0))
    r = mk(net, "ray", "ray1", method=1, dirmethod=0)
    for nm, v in zip(("dirx", "diry", "dirz"), (0, -1, 0)):
        r.parm(nm).deleteAllKeyframes()  # default is expression @N.x etc.
        r.parm(nm).set(v)
    r.setInput(0, g); r.setInput(1, b)
    return r
finish(SOP, SCENE, build, NOTE)
