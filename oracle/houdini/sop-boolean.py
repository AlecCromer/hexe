from _common import *
SOP = "sop-boolean"
NOTE = "box minus box translated (0.5,0.5,0.5): subtract, solid (examples/boolean.json)"
SCENE = {"nodes": [{"name": "a", "type": "box"}, {"name": "b", "type": "box"}, {"name": "xf", "type": "xform", "params": {"t": [0.5, 0.5, 0.5]}}, {"name": "bool1", "type": "boolean", "params": {"operation": 1}}], "edges": [["b", 0, "xf", 0], ["a", 0, "bool1", 0], ["xf", 0, "bool1", 1]], "display": "bool1"}
def build(net):
    a = mk(net, "box", "a"); b = mk(net, "box", "b"); x = mk(net, "xform", "xf", t=(0.5, 0.5, 0.5))
    o = mk(net, "boolean::2.0", "bool1", booleanop="subtract", subtractchoices="aminusb", asurface="solid", bsurface="solid")
    x.setInput(0, b); o.setInput(0, a); o.setInput(1, x)
    return o
finish(SOP, SCENE, build, NOTE)
