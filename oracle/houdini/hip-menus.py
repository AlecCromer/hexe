"""H40 oracle tooling (hython 20.5): dump Houdini menu tokens of every hexe SOP type -> crates/sop/src/hip_menus.json.
Used by the .hip importer to turn a saved menu token (`"poly"`) into the hexe `Menu(i)` index (= Houdini menu position).
usage: hython oracle/houdini/hip-menus.py <types.txt> <out.json>   (types.txt = hexe-cli nodes names, one per line)"""
import json, sys
import hou
ALIAS = {"wrangle": "attribwrangle"}
cat = hou.sopNodeTypeCategory()
out = {}
for line in open(sys.argv[1]):
    t = line.strip()
    if not t:
        continue
    nt = hou.nodeType(cat, ALIAS.get(t, t))
    if nt is None:
        continue
    m = {}
    for pt in nt.parmTemplates():
        stack = [pt]
        while stack:
            p = stack.pop()
            if p.type() == hou.parmTemplateType.Folder or p.type() == hou.parmTemplateType.FolderSet:
                stack.extend(p.parmTemplates()) if hasattr(p, "parmTemplates") else None
                continue
            if p.type() == hou.parmTemplateType.Menu:
                m[p.name()] = list(p.menuItems())
    if m:
        out[t] = m
json.dump(out, open(sys.argv[2], "w"), indent=0, sort_keys=True)
print(len(out), "types with menus")
