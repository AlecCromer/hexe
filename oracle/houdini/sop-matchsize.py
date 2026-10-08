from _common import *
SOP = "sop-matchsize"
NOTE = "box -> matchsize explicit target size (2,4,6) center 0: doscale on, nonuniform, translate on, justify center (matchsize.json)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "match1", "type": "matchsize", "params": {"targetsize": [2, 4, 6], "targetcenter": [0, 0, 0], "doscale": True, "uniformscale": False, "dotranslate": True}}], "edges": [["box1", 0, "match1", 0]], "display": "match1"}
def build(net):
    b = mk(net, "box", "box1")
    m = mk(net, "matchsize", "match1", justifytarget=2, size=(2, 4, 6), t=(0, 0, 0), doscale=1, uniformscale=0, dotranslate=1)
    m.setInput(0, b)
    return m
finish(SOP, SCENE, build, NOTE)
