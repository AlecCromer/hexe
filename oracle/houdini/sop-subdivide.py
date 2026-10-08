from _common import *
SOP = "sop-subdivide"
NOTE = "box -> subdivide 1 iteration Catmull-Clark (Houdini osdcc default)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "sub1", "type": "subdivide", "params": {"iterations": 1, "algorithm": 0}}], "edges": [["box1", 0, "sub1", 0]], "display": "sub1"}
def build(net):
    b = mk(net, "box", "box1"); s = mk(net, "subdivide", "sub1")
    if s.parm("iterations") is not None:
        s.parm("iterations").set(1)
    s.setInput(0, b)
    return s
finish(SOP, SCENE, build, NOTE)
