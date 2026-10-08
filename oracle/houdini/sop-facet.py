from _common import *
SOP = "sop-facet"
NOTE = "box -> facet unique_points + post_nml"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "fc", "type": "facet", "params": {"unique_points": True, "post_nml": True}}], "edges": [["box1", 0, "fc", 0]], "display": "fc"}
def build(net):
    b = mk(net, "box", "box1"); f = mk(net, "facet", "fc", unique=1, postnml=1)
    f.setInput(0, b)
    return f
finish(SOP, SCENE, build, NOTE)
