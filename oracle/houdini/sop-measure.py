from _common import *
SOP = "sop-measure"
NOTE = "box -> measure::2.0 area per prim into attrib 'area' (measure.json; hexe measure 0 = area)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "measure1", "type": "measure", "params": {"measure": 0, "attribname": "area"}}], "edges": [["box1", 0, "measure1", 0]], "display": "measure1"}
def build(net):
    b = mk(net, "box", "box1")
    m = mk(net, "measure::2.0", "measure1", grouptype=1, measure=1, attribname="area")
    m.setInput(0, b)
    return m
finish(SOP, SCENE, build, NOTE)
