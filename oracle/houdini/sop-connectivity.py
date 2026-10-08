from _common import *
SOP = "sop-connectivity"
NOTE = "two boxes (2nd moved +3 X) merged -> connectivity per PRIM (hexe connecttype 0 = prim; Houdini connecttype 1 = prim, menu order is point,prim)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "box2", "type": "box"}, {"name": "move", "type": "xform", "params": {"t": [3, 0, 0]}}, {"name": "merge1", "type": "merge"}, {"name": "conn1", "type": "connectivity", "params": {"connecttype": 0}}], "edges": [["box2", 0, "move", 0], ["box1", 0, "merge1", 0], ["move", 0, "merge1", 1], ["merge1", 0, "conn1", 0]], "display": "conn1"}
def build(net):
    b1 = mk(net, "box", "box1"); b2 = mk(net, "box", "box2")
    mv = mk(net, "xform", "move", t=(3, 0, 0)); mv.setInput(0, b2)
    m = mk(net, "merge", "merge1"); m.setInput(0, b1); m.setInput(1, mv)
    c = mk(net, "connectivity", "conn1", connecttype=1, attribname="class")
    c.setInput(0, m)
    return c
finish(SOP, SCENE, build, NOTE)
