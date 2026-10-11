from _common import *
SOP = "sop-split-rest"
NOTE = 'split out1 (rest) of box prim 5: 5 prims, all 8 points'
SCENE = {"nodes": [{"name": "src", "type": "box", "params": {}}, {"name": "op1", "type": "split", "params": {'group': '5', 'grouptype': 4, 'negate': False, 'output': 1}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    b = mk(net, "box", "src")
    s = mk(net, "split", "sp", group='5', grouptype=4, negate=0)
    s.setInput(0, b)
    k = mk(net, "null", "op1")
    k.setInput(0, s, 1)
    return k
finish(SOP, SCENE, build, NOTE)
