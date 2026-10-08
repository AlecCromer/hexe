from _common import *
SOP = "sop-knife"
NOTE = "box cut at y=0 plane, keep both (knife.json). Group creation off both sides (hexe seam/above/below names blanked, Houdini newg 0) to compare geometry only"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "knife1", "type": "knife", "params": {"origin": [0, 0, 0], "dir": [0, 1, 0], "keep": 2, "seamgroup": "", "abovegroup": "", "belowgroup": ""}}], "edges": [["box1", 0, "knife1", 0]], "display": "knife1"}
def build(net):
    b = mk(net, "box", "box1")
    k = mk(net, "knife", "knife1", origin=(0, 0, 0), dir=(0, 1, 0), dist=0, knifeop=2, newg=0)
    k.setInput(0, b)
    return k
finish(SOP, SCENE, build, NOTE)
