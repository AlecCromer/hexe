"""H40 oracle tooling (hython 20.5): Houdini twin of `hexe-cli open`. NOT product code.
usage: hython hip-twin.py <list.tsv> <outdir>     list.tsv: `<id>\t<path-to-.hip>` per line
For every file: load it, and for every /obj container with SOP children cook its display SOP and save
<outdir>/<id>/<container>.geo (+ <id>/hip.json: saved frame, display node, counts, errors). hexe side: `hexe-sop` test
`hip_corpus_twin` (HEXE_HIP_TWIN_DIR=<outdir>) imports the same file and compares via the existing `houdini_twin`."""
import json, os, sys
import hou
lst, outdir = sys.argv[1], sys.argv[2]
for line in open(lst):
    line = line.rstrip("\n")
    if not line:
        continue
    fid, path = line.split("\t")
    d = os.path.join(outdir, fid)
    os.makedirs(d, exist_ok=True)
    rec = {"file": path, "houdini": hou.applicationVersionString(), "geos": {}}
    try:
        hou.hipFile.clear(suppress_save_prompt=True)
        hou.hipFile.load(path, suppress_save_prompt=True, ignore_load_warnings=True)
        rec["frame"] = hou.frame()
        rec["fps"] = hou.fps()
        for c in hou.node("/obj").children():
            kids = c.children()
            if c.type().name() != "geo" or not kids:
                continue
            g = {"nodes": len(kids)}
            try:
                dn = c.displayNode()
                g["display"] = dn.name() if dn else None
                if dn is not None:
                    dn.cook(force=True)
                    g["errors"] = list(dn.errors())
                    geo = dn.geometry()
                    if geo is not None:
                        geo.saveToFile(os.path.join(d, c.name() + ".geo"))
                        g["points"] = len(geo.points())
                        g["prims"] = len(geo.prims())
            except Exception as e:
                g["error"] = "%s: %s" % (type(e).__name__, e)
            rec["geos"][c.name()] = g
    except Exception as e:
        rec["error"] = "%s: %s" % (type(e).__name__, e)
    with open(os.path.join(d, "hip.json"), "w") as f:
        json.dump(rec, f, indent=1)
    print(fid, rec.get("error", "ok"), len(rec["geos"]), "geos")
