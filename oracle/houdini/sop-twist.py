from _common import *
SOP = "sop-twist"
NOTE = "box -> twist 90 deg over the box height (origin y=-0.5, length 1) about +Y (twist.json). BEST GUESS: Houdini twist SOP op=twist, paxis=y, pivot (0,-0.5,0), strength=90 (deg per unit), no capture length/clamp"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "twist1", "type": "twist", "params": {"origin": [0, -0.5, 0], "length": 1, "angle": 90}}], "edges": [["box1", 0, "twist1", 0]], "display": "twist1"}
def build(net):
    b = mk(net, "box", "box1")
    t = mk(net, "twist", "twist1", op=0, paxis=1, p=(0, -0.5, 0), strength=90)
    t.setInput(0, b)
    return t
finish(SOP, SCENE, build, NOTE)
