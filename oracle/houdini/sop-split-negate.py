from _common import *
SOP = "sop-split-negate"
NOTE = 'split negate on: out0 = everything but prim 5 (5 prims, 8 points)'
SCENE = {"nodes": [{"name": "src", "type": "box", "params": {}}, {"name": "op1", "type": "split", "params": {'group': '5', 'grouptype': 4, 'negate': True, 'output': 0}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    b = mk(net, "box", "src")
    s = mk(net, "split", "sp", group='5', grouptype=4, negate=1)
    s.setInput(0, b)
    k = mk(net, "null", "op1")
    k.setInput(0, s, 0)
    return k
finish(SOP, SCENE, build, NOTE)
