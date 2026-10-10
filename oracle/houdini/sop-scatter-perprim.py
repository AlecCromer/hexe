from _common import *
SOP = "sop-scatter-perprim"
SNIP = "i@count = (@primnum % 4) * 10 + 10;"
NOTE = "STAT oracle: grid prims get int count = (primnum%4)*10+10 (wrangle) -> scatter generateby Count per Primitive (2025 pts total), seed 2, relax OFF"
SCENE = {"nodes": [{"name": "grid1", "type": "grid"}, {"name": "w", "type": "wrangle", "params": {"class": "primitive", "snippet": SNIP}}, {"name": "scatter1", "type": "scatter", "params": {"generateby": 2, "primcountattrib": "count", "seed": 2}}], "edges": [["grid1", 0, "w", 0], ["w", 0, "scatter1", 0]], "display": "scatter1"}
def build(net):
    g = mk(net, "grid", "grid1")
    w = mk(net, "attribwrangle", "w", **{"class": "primitive", "snippet": SNIP})
    w.setInput(0, g)
    s = mk(net, "scatter", "scatter1", generateby=1, primcountattrib="count", seed=2, relaxpoints=0)
    s.setInput(0, w)
    return s
finish(SOP, SCENE, build, NOTE, oracle="stat")
