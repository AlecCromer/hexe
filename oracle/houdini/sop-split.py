from _common import *
SOP = "sop-split"
NOTE = 'split out0 of box prims 0 2 (selected prims, all 8 points used)'
SCENE = {"nodes": [{"name": "src", "type": "box", "params": {}}, {"name": "op1", "type": "split", "params": {'group': '0 2', 'grouptype': 4, 'negate': False, 'output': 0}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    b = mk(net, "box", "src")
    s = mk(net, "split", "sp", group='0 2', grouptype=4, negate=0)
    s.setInput(0, b)
    k = mk(net, "null", "op1")
    k.setInput(0, s, 0)
    return k
finish(SOP, SCENE, build, NOTE)
