from _common import *
SOP = "sop-curve-nurbs"
NOTE = "4 CVs open NURBS order 4; twin = curve::2.0 outputtype nurbs savedstashgeo NURBSCurve"
SCENE = {"nodes": [{"name": "curve1", "type": "curve", "params": {"coords": "0,0,0 1,2,0 3,2,0 4,0,0", "type": 2, "order": 4}}], "edges": [], "display": "curve1"}
PTS = [(0,0,0),(1,2,0),(3,2,0),(4,0,0)]
def build(net):
    # real curve::2.0 twin: its only scriptable authoring parm is the Geometry data parm `savedstashgeo` (parmTuples: stashgeo, savedstashgeo, parmpoints = Data/Geometry; addpts = opaque string; outputtype menu polygon|nurbs|bezier; order int). No coords/points string parm exists.
    geo = hou.Geometry()
    geo.createNURBSCurve(len(PTS), False, 4)
    for q, pt in zip(PTS, geo.points()):
        pt.setPosition(q)
    n = net.createNode("curve::2.0", "curve1")
    n.parm("outputtype").set(1)
    n.parm("order").set(4)
    n.parm("savedstashgeo").set(geo)
    return n
finish(SOP, SCENE, build, NOTE)
