from _common import *
SOP = "sop-attribpromote"
NOTE = "grid 3x3 -> wrangle f@v=ptnum -> attribpromote v point->prim (hexe method 0 -> Houdini max? see hexe docs; Houdini twin uses method 'mean'), delete original"
SCENE = {"nodes": [{"name": "g", "type": "grid", "params": {"rows": 3, "cols": 3}}, {"name": "w", "type": "wrangle", "params": {"snippet": "f@v = @ptnum; f@w = 2.0;"}}, {"name": "ap", "type": "attribpromote", "params": {"inname": "v", "inclass": 0, "outclass": 2}}], "edges": [["g", 0, "w", 0], ["w", 0, "ap", 0]], "display": "ap"}
def build(net):
    g = mk(net, "grid", "g", rows=3, cols=3); w = mk(net, "attribwrangle", "w", **{"class": "point", "snippet": "f@v = @ptnum; f@w = 2.0;"})
    a = mk(net, "attribpromote", "ap", inname="v", inclass="point", outclass="primitive", method="mean", deletein=1)
    w.setInput(0, g); a.setInput(0, w)
    return a
finish(SOP, SCENE, build, NOTE)
