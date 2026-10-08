from _common import *
SOP = "sop-uvproject"
NOTE = "box -> uvproject cylindrical (hexe type 1) axis 1 (hexe y). Houdini: projtype cylin, inittype zx (approx axis mapping), fixseams on"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "uvp", "type": "uvproject", "params": {"type": 1, "axis": 1}}], "edges": [["box1", 0, "uvp", 0]], "display": "uvp"}
def build(net):
    b = mk(net, "box", "box1"); u = mk(net, "uvproject", "uvp", projtype="cylin", inittype="zx", fixseams=1)
    u.setInput(0, b)
    return u
finish(SOP, SCENE, build, NOTE)
