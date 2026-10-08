"""hython run_all.py [sop-id ...]  -- runs every oracle/houdini/{sop,x}-*.py in ONE hython process (saves ~15 s startup each).
A script that raises is recorded verbatim in fixtures/<sop>/houdini.json (key `error`) and the run continues."""
import glob, json, os, runpy, sys, traceback
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import _common
want = sys.argv[1:]
files = sorted(glob.glob(os.path.join(HERE, "sop-*.py")) + glob.glob(os.path.join(HERE, "x-*.py")))
for f in files:
    sop = os.path.basename(f)[:-3]
    if want and sop not in want:
        continue
    try:
        runpy.run_path(f, run_name="__main__")
        print("ran", sop)
    except BaseException as e:
        traceback.print_exc()
        d = os.path.join(_common.FIX, sop)
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "houdini.json"), "w") as g:
            json.dump({"sop": sop, "error": "script %s: %s" % (type(e).__name__, e)}, g, indent=1)
        print("ERR", sop, e)
