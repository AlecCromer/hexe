from _common import *
SOP = "sop-attribswap"
NOTE = "3x3 grid -> wrangle (Cd=(ptnum,0,1), f@h=P.x, i@k=ptnum) -> attribswap h <-> k (float <-> int storage swap); attribute values compared via houdini_values"
WR = "@Cd = set(@ptnum, 0, 1); f@h = @P.x; i@k = @ptnum;"
SCENE = {"nodes": [{"name": "g", "type": "grid", "params": {"rows": 3, "cols": 3}}, {"name": "w", "type": "wrangle", "params": {"snippet": WR}}, {"name": "sw", "type": "attribswap", "params": {"srcattribs1": "h", "dstattribs1": "k"}}], "edges": [["g", 0, "w", 0], ["w", 0, "sw", 0]], "display": "sw"}
def build(net):
    g = mk(net, "grid", "g", rows=3, cols=3)
    w = mk(net, "attribwrangle", "w", **{"class": "point", "snippet": WR}); w.setInput(0, g)
    s = mk(net, "attribswap", "sw", srcattribs1="h", dstattribs1="k"); s.setInput(0, w)
    return s
finish(SOP, SCENE, build, NOTE)
