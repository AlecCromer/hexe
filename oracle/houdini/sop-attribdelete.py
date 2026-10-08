from _common import *
SOP = "sop-attribdelete"
NOTE = "grid 3x3 -> wrangle f@v f@w -> attribdelete point 'w'"
SCENE = {"nodes": [{"name": "g", "type": "grid", "params": {"rows": 3, "cols": 3}}, {"name": "w", "type": "wrangle", "params": {"snippet": "f@v = @ptnum; f@w = 2.0;"}}, {"name": "ad", "type": "attribdelete", "params": {"ptdel": "w"}}], "edges": [["g", 0, "w", 0], ["w", 0, "ad", 0]], "display": "ad"}
def build(net):
    g = mk(net, "grid", "g", rows=3, cols=3); w = mk(net, "attribwrangle", "w", **{"class": "point", "snippet": "f@v = @ptnum; f@w = 2.0;"})
    a = mk(net, "attribdelete", "ad", ptdel="w")
    w.setInput(0, g); a.setInput(0, w)
    return a
finish(SOP, SCENE, build, NOTE)
