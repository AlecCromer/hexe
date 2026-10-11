"""H30 oracle helpers. Runs under hython (Houdini 20.5.487). NOT product code: oracle tooling only.
Each <sop>.py: SCENE (hexe scene dict) + build(net) -> hou.SopNode (the Houdini twin of SCENE) + finish(SOP, SCENE, build).
"""
import json, os, sys
import hou

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
FIX = os.path.join(ROOT, "fixtures")
CAP = 5000  # max points / prims serialised in full


def r6(x):
    return round(float(x), 6)


def summarize(geo):
    """Uniform summary of a hou.Geometry (used for BOTH the Houdini result and hexe's .geo loaded back into Houdini)."""
    pts = geo.points()
    prims = geo.prims()
    P = [[r6(c) for c in p.position()] for p in pts[:CAP]]
    attrs = {
        "point": sorted(a.name() for a in geo.pointAttribs()),
        "prim": sorted(a.name() for a in geo.primAttribs()),
        "vertex": sorted(a.name() for a in geo.vertexAttribs()),
        "detail": sorted(a.name() for a in geo.globalAttribs()),
    }
    atypes = {}
    for cls, lst in (("point", geo.pointAttribs()), ("prim", geo.primAttribs()), ("vertex", geo.vertexAttribs()), ("detail", geo.globalAttribs())):
        for a in lst:
            atypes[cls + ":" + a.name()] = [str(a.dataType()).split(".")[-1], a.size()]
    nvtx = sum(p.numVertices() for p in prims)
    bb = geo.boundingBox() if pts else None
    prim_pts = [[v.point().number() for v in pr.vertices()] for pr in prims[:CAP]]
    prim_types = sorted(set(str(pr.type()).split(".")[-1] for pr in prims))
    closed = [bool(getattr(pr, "isClosed", lambda: True)()) for pr in prims[:CAP]]
    return {
        "points": len(pts),
        "prims": len(prims),
        "vertices": nvtx,
        "attribs": attrs,
        "attrib_types": atypes,
        "prim_types": prim_types,
        "point_groups": sorted(g.name() for g in geo.pointGroups()),
        "prim_groups": sorted(g.name() for g in geo.primGroups()),
        "bbox_min": [r6(c) for c in bb.minvec()] if bb else None,
        "bbox_max": [r6(c) for c in bb.maxvec()] if bb else None,
        "P": P[:8],
        "P_all": P,
        "prim_pts": prim_pts,
        "prim_closed": closed,
    }


def scratch_net():
    obj = hou.node("/obj")
    for c in obj.children():
        c.destroy()
    return obj.createNode("geo", "oracle")


def mk(net, typ, name, **parms):
    """Create SOP; scalar parms by name, tuple parms by name given a tuple/list (fan-out to name+x/y/z or r/g/b)."""
    n = net.createNode(typ, name)
    for k, v in parms.items():
        setp(n, k, v)
    return n


def setp(n, k, v):
    p = n.parm(k)
    if p is not None and not isinstance(v, (list, tuple)):
        p.set(v)
        return
    t = n.parmTuple(k)
    if t is None:
        raise RuntimeError("%s has no parm %r" % (n.type().name(), k))
    t.set(tuple(v) if isinstance(v, (list, tuple)) else (v,))


def finish(sop, scene, build, note="", oracle=None, stat=None):
    """Write scene.json (hexe twin), cook the Houdini twin, save houdini.geo + houdini.json.
    oracle="stat": statistical oracle (parity/README.md): houdini.json gets `oracle`="stat" + `stat` (kind points|disp);
    kind=disp also saves base.geo = the first input of the twin (displacement magnitude is |P_out - P_base|)."""
    d = os.path.join(FIX, sop)
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, "scene.json"), "w") as f:
        json.dump(scene, f, indent=1)
        f.write("\n")
    out = {"sop": sop, "houdini": "%s" % hou.applicationVersionString(), "note": note}
    if oracle:
        out["oracle"] = oracle
        out["stat"] = stat or "points"
    try:
        net = scratch_net()
        node = build(net)
        node.cook(force=True)
        errs = node.errors()
        warns = node.warnings()
        if errs:
            out["error"] = "; ".join(errs)
        geo = node.geometry()
        if geo is None:
            out["error"] = out.get("error", "node.geometry() is None")
        else:
            geo.saveToFile(os.path.join(d, "houdini.geo"))
            out.update(summarize(geo))
            if oracle == "stat" and out["stat"] == "disp":
                bg = node.inputs()[0].geometry()
                bg.saveToFile(os.path.join(d, "base.geo"))
                out["base_P_all"] = [[r6(c) for c in p.position()] for p in bg.points()[:CAP]]
        if warns:
            out["warnings"] = list(warns)
    except Exception as e:  # recorded verbatim, run continues
        out["error"] = "%s: %s" % (type(e).__name__, e)
    with open(os.path.join(d, "houdini.json"), "w") as f:
        json.dump(out, f, indent=1)
        f.write("\n")
    return out
def poly_curve(net, name, pts, closed=False):
    """Houdini twin of the hexe `curve` SOP (polygon output): curve::2.0 whose stash geometry is one polygon through `pts`."""
    geo = hou.Geometry()
    pr = geo.createPolygon(is_closed=closed)
    for q in pts:
        pt = geo.createPoint(); pt.setPosition(q); pr.addVertex(pt)
    n = net.createNode("curve::2.0", name)
    n.parm("outputtype").set(0)
    n.parm("order").set(4)
    n.parm("savedstashgeo").set(geo)
    return n
