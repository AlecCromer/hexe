from _common import *
SOP = "sop-registry"
NOTE = "box -> null passthrough"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "nl", "type": "null"}], "edges": [["box1", 0, "nl", 0]], "display": "nl"}
def build(net):
    b = mk(net, "box", "box1"); n = mk(net, "null", "nl")
    n.setInput(0, b)
    return n
finish(SOP, SCENE, build, NOTE)
