from _common import *
SOP = "sop-curve"
NOTE = "closed poly through (0,1,0) (1,1,0) (1,2,0); twin = REAL curve::2.0 with the polygon injected via savedstashgeo (Data/Geometry parm); Curve SOP passes the stash through (data model + prim kind + P, interpretation of coords is hexe-side only)"
SCENE = {"nodes": [{"name": "curve1", "type": "curve", "params": {"coords": "0,1,0 1,1,0 1,2,0", "close": True}}], "edges": [], "display": "curve1"}
PTS = [(0,1,0),(1,1,0),(1,2,0)]
def build(net):
    # real curve::2.0 twin: its only scriptable authoring parm is the Geometry data parm `savedstashgeo` (parmTuples: stashgeo, savedstashgeo, parmpoints = Data/Geometry; addpts = opaque string; outputtype menu polygon|nurbs|bezier; order int). No coords/points string parm exists.
    geo = hou.Geometry()
    pr = geo.createPolygon(is_closed=True)
    for q in PTS:
        pt = geo.createPoint(); pt.setPosition(q); pr.addVertex(pt)
    n = net.createNode("curve::2.0", "curve1")
    n.parm("outputtype").set(0)
    n.parm("order").set(4)
    n.parm("savedstashgeo").set(geo)
    return n
finish(SOP, SCENE, build, NOTE)
