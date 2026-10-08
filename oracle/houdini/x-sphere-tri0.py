from _common import *
SOP = "x-sphere-tri0"
NOTE = "extra case: sphere polymesh rows 5 cols 4, triangularpoles OFF (degenerate pole quads)"
SCENE = {"nodes": [{"name": "sph", "type": "sphere", "params": {"type": 2, "rows": 5, "cols": 4, "triangularpoles": False}}], "edges": [], "display": "sph"}
def build(net):
    return mk(net, "sphere", "sph", type=2, rows=5, cols=4, triangularpoles=0)
finish(SOP, SCENE, build, NOTE)
