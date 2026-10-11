"""hython oracle/houdini/census.py -- h36 SOP census: dumps every hou.sopNodeTypeCategory() node type to oracle/houdini/census-houdini.json
({raw type key: [description, hidden, minInputs, maxInputs]}). Oracle tooling only; not picked up by run_all.py (no sop-/x- prefix)."""
import hou, json, os
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "census-houdini.json")
out = {}
for name, t in sorted(hou.sopNodeTypeCategory().nodeTypes().items()):
    out[name] = [t.description(), bool(t.hidden()), t.minNumInputs(), t.maxNumInputs()]
json.dump(out, open(OUT, "w"), indent=0, sort_keys=True)
print(len(out), "sop node types ->", OUT)
