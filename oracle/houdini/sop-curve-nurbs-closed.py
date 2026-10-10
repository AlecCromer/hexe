from _common import *
SOP = "sop-curve-nurbs-closed"
NOTE = "5 CVs closed NURBS order 3 (periodic); twin = curve::2.0 savedstashgeo closed NURBSCurve"
SCENE = {"nodes": [{"name": "curve1", "type": "curve", "params": {"coords": "0,0,0 2,0,0 3,2,0 1,3,0 -1,2,0", "type": 2, "order": 3, "close": True}}], "edges": [], "display": "curve1"}
PTS = [(0,0,0),(2,0,0),(3,2,0),(1,3,0),(-1,2,0)]
def build(net):
    # real curve::2.0 twin: its only scriptable authoring parm is the Geometry data parm `savedstashgeo` (parmTuples: stashgeo, savedstashgeo, parmpoints = Data/Geometry; addpts = opaque string; outputtype menu polygon|nurbs|bezier; order int). No coords/points string parm exists.
    geo = hou.Geometry()
    geo.createNURBSCurve(len(PTS), True, 3)
    for q, pt in zip(PTS, geo.points()):
        pt.setPosition(q)
    n = net.createNode("curve::2.0", "curve1")
    n.parm("outputtype").set(1)
    n.parm("order").set(3)
    n.parm("savedstashgeo").set(geo)
    return n
finish(SOP, SCENE, build, NOTE)
