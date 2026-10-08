from _common import *
SOP = "sop-resample"
NOTE = "circle(div 6, r 2x1) -> resample by segments 24 (curves.json)"
SCENE = {"nodes": [{"name": "circle1", "type": "circle", "params": {"divisions": 6, "radius": [2, 1, 0]}}, {"name": "rs", "type": "resample", "params": {"method": 1, "segments": 24}}], "edges": [["circle1", 0, "rs", 0]], "display": "rs"}
def build(net):
    c = mk(net, "circle", "circle1", type=1, divs=6, rad=(2, 1))
    r = mk(net, "resample", "rs", dolength=0, dosegs=1, segs=24)
    r.setInput(0, c)
    return r
finish(SOP, SCENE, build, NOTE)
