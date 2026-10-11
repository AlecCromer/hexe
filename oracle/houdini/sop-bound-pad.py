from _common import *
SOP = "sop-bound-pad"
NOTE = "box(t 1,2,3 size 1,2,3) -> bound minpad (.1,.2,.3) maxpad (.4,.5,.6), rebar minsize (5,5,5)"
SCENE = {"nodes": [{"name": "box1", "type": "box", "params": {"t": [1, 2, 3], "size": [1, 2, 3]}}, {"name": "bd", "type": "bound", "params": {"minpad": [0.1, 0.2, 0.3], "maxpad": [0.4, 0.5, 0.6], "rebar": True, "minsize": [5, 5, 5]}}], "edges": [["box1", 0, "bd", 0]], "display": "bd"}
def build(net):
    b = mk(net, "box", "box1", t=(1, 2, 3), size=(1, 2, 3)); n = mk(net, "bound", "bd", minpad=(.1, .2, .3), maxpad=(.4, .5, .6), rebar=1, minsize=(5, 5, 5))
    n.setInput(0, b)
    return n
finish(SOP, SCENE, build, NOTE)
