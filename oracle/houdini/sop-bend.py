from _common import *
SOP = "sop-bend"
NOTE = "line 5 pts y=0..4 -> bend angle 90, length 4, capture origin 0 dir +Y (bend.json). BEST GUESS parm map: Houdini bend (not bend::1) has bend/length/origin/dir + setcaptureregion; Houdini bends toward -X here, so twin hexe scene sets side=(-1,0,0) (hexe default +X)"
SCENE = {"nodes": [{"name": "line1", "type": "line", "params": {"origin": [0, 0, 0], "dir": [0, 1, 0], "dist": 4, "points": 5}}, {"name": "bend1", "type": "bend", "params": {"length": 4, "angle": 90, "side": [-1, 0, 0]}}], "edges": [["line1", 0, "bend1", 0]], "display": "bend1"}
def build(net):
    l = mk(net, "line", "line1", origin=(0, 0, 0), dir=(0, 1, 0), dist=4, points=5)
    b = mk(net, "bend", "bend1", bend=90, length=4, origin=(0, 0, 0), dir=(0, 1, 0))
    b.setInput(0, l)
    return b
finish(SOP, SCENE, build, NOTE)
