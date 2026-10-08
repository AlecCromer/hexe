from _common import *
SOP = "sop-switch"
NOTE = "box -> null -> switch(input 0) (+ grid 3x3 on input 1) -> output (switch.json)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "grid1", "type": "grid", "params": {"rows": 3, "cols": 3}}, {"name": "null1", "type": "null"}, {"name": "switch1", "type": "switch", "params": {"input": 0}}, {"name": "out1", "type": "output"}], "edges": [["box1", 0, "null1", 0], ["null1", 0, "switch1", 0], ["grid1", 0, "switch1", 1], ["switch1", 0, "out1", 0]], "display": "out1"}
def build(net):
    b = mk(net, "box", "box1"); g = mk(net, "grid", "grid1", rows=3, cols=3)
    n = mk(net, "null", "null1"); n.setInput(0, b)
    s = mk(net, "switch", "switch1", input=0); s.setInput(0, n); s.setInput(1, g)
    o = mk(net, "output", "out1"); o.setInput(0, s)
    return o
finish(SOP, SCENE, build, NOTE)
