from _common import *
SOP = "sop-split-pts-rest"
NOTE = 'split out1 of box point group 0 1: 6 points, 4 prims (two quads shortened to tris, two prims dropped), renumbered'
SCENE = {"nodes": [{"name": "src", "type": "box", "params": {}}, {"name": "op1", "type": "split", "params": {'group': '0 1', 'grouptype': 3, 'negate': False, 'output': 1}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    b = mk(net, "box", "src")
    s = mk(net, "split", "sp", group='0 1', grouptype=3, negate=0)
    s.setInput(0, b)
    k = mk(net, "null", "op1")
    k.setInput(0, s, 1)
    return k
finish(SOP, SCENE, build, NOTE)
