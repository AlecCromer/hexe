"""hython bgeo-twins.py -- H39: load each fixtures/bgeo-bjson/*.bgeo in Houdini 20.5.487, save the ASCII twin fixtures/bgeo-bjson/<n>.geo Oracle tooling only (AGENTS.md section 2 exception)."""
import glob, json, os
import hou
ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
D = os.path.join(ROOT, "fixtures", "bgeo-bjson")
for f in sorted(glob.glob(os.path.join(D, "*.bgeo"))):
    n = os.path.basename(f)[:-5]
    g = hou.Geometry()
    g.loadFromFile(f)
    g.saveToFile(os.path.join(D, n + ".geo"))
    print(n, len(g.points()), len(g.prims()), sum(len(p.vertices()) for p in g.prims()))
