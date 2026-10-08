"""hython summarize_hexe.py <dir>  -- load every <dir>/<sop>.geo (written by hexe-cli) INTO HOUDINI and summarise it with the
same summarize() used for the Houdini side. A file Houdini cannot load is itself a finding (recorded as `load_error`)."""
import glob, json, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hou
from _common import summarize
d = sys.argv[1]
for f in sorted(glob.glob(os.path.join(d, "*.geo"))):
    out = {}
    try:
        g = hou.Geometry()
        g.loadFromFile(f)
        out = summarize(g)
    except Exception as e:
        out = {"load_error": "%s: %s" % (type(e).__name__, e)}
    with open(f[:-4] + ".json", "w") as h:
        json.dump(out, h)
