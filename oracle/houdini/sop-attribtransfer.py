from _common import *
SOP = "sop-attribtransfer"
NOTE = "target grid 3x3, source grid 2x2 with f@v=ptnum -> attribtransfer point 'v' nearest within 10"
SCENE = {"nodes": [{"name": "g", "type": "grid", "params": {"rows": 3, "cols": 3}}, {"name": "s", "type": "grid", "params": {"rows": 2, "cols": 2}}, {"name": "w", "type": "wrangle", "params": {"snippet": "f@v = @ptnum;"}}, {"name": "at", "type": "attribtransfer", "params": {"pointattribs": "v", "distancethreshold": 10}}], "edges": [["s", 0, "w", 0], ["g", 0, "at", 0], ["w", 0, "at", 1]], "display": "at"}
def build(net):
    g = mk(net, "grid", "g", rows=3, cols=3); s = mk(net, "grid", "s", rows=2, cols=2)
    w = mk(net, "attribwrangle", "w", **{"class": "point", "snippet": "f@v = @ptnum;"})
    a = mk(net, "attribtransfer", "at", pointattribs=1, pointattriblist="v", primitiveattribs=0, thresholddist=10, kernelradius=10, maxsamplecount=1)
    w.setInput(0, s); a.setInput(0, g); a.setInput(1, w)
    return a
finish(SOP, SCENE, build, NOTE)
