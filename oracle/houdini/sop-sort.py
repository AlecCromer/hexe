from _common import *
SOP = "sop-sort"
NOTE = "grid 3x3 -> sort points reverse (hexe ptsort 7=reverse)"
SCENE = {"nodes": [{"name": "g", "type": "grid", "params": {"rows": 3, "cols": 3}}, {"name": "so", "type": "sort", "params": {"ptsort": 7}}], "edges": [["g", 0, "so", 0]], "display": "so"}
def build(net):
    g = mk(net, "grid", "g", rows=3, cols=3); s = mk(net, "sort", "so", ptsort="rev")
    s.setInput(0, g)
    return s
finish(SOP, SCENE, build, NOTE)
