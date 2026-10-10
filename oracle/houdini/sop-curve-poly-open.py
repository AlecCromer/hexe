from _common import *
SOP = "sop-curve-poly-open"
NOTE = "open poly through 5 points; twin = curve::2.0 savedstashgeo polygon"
SCENE = {"nodes": [{"name": "curve1", "type": "curve", "params": {"coords": "0,0,0 1,2,0 3,2,0 4,0,0 5,1,1", "close": False}}], "edges": [], "display": "curve1"}
PTS = [(0,0,0),(1,2,0),(3,2,0),(4,0,0),(5,1,1)]
def build(net):
    # real curve::2.0 twin: its only scriptable authoring parm is the Geometry data parm `savedstashgeo` (parmTuples: stashgeo, savedstashgeo, parmpoints = Data/Geometry; addpts = opaque string; outputtype menu polygon|nurbs|bezier; order int). No coords/points string parm exists.
    geo = hou.Geometry()
    pr = geo.createPolygon(is_closed=False)
    for q in PTS:
        pt = geo.createPoint(); pt.setPosition(q); pr.addVertex(pt)
    n = net.createNode("curve::2.0", "curve1")
    n.parm("outputtype").set(0)
    n.parm("order").set(4)
    n.parm("savedstashgeo").set(geo)
    return n
finish(SOP, SCENE, build, NOTE)
