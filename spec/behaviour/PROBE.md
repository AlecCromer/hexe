# PROBE recipe (run when hython is licensed)
Blocker: `hython` → `No licenses could be found to run this application.` (sesinetd pid up on localhost:1715; `hserver -S` shows 0 licenses used; `/Library/Preferences/sesi/licenses` present but not accepted). Fix = box/user-side, NOT this lane.

Bin: `/Applications/Houdini/Current/Frameworks/Houdini.framework/Versions/Current/Resources/bin/hython` (Houdini 20.5.487).

Per SOP S in {box grid sphere xform merge scatter copytopoints attribwrangle attribnoise}:
1. `geo=hou.node('/obj').createNode('geo'); n=geo.createNode(S)`
2. dump per parm: `p.name(), p.parmTemplate().type(), p.eval(), defaultValue(), menuItems()/menuLabels()`, min/max (`p.parmTemplate().minValue()`) → table.
3. `n.cook(force=True); g=n.geometry()`: counts + `g.pointAttribs()/primAttribs()/vertexAttribs()/globalAttribs()/primGroups()/pointGroups()`.
4. 2 edge cases each (sizes ≤0, 0-point input), record `n.errors()/warnings()`.
5. `g.saveToFile('fixtures/<sop>-default/expected.geo')` + MANIFEST row (sha256).
6. cook-model: `n.needsToCook()` before/after cook, after parm set (same/different value), `n.isTimeDependent()` with `$F` expr and keyframe.
Write results into the `[R]`/`[?]` cells of `sop-*.md`; flip tag to `[O]`.
Binary names verified: Houdini `nodes.zip` internals: box grid sphere xform merge scatter copytopoints attribwrangle attribnoise::2.0 (+mountain).
Offline help source: `houdini/help/nodes.zip` → `sop/<internal>.txt`.
Not committed: scratch copy in `.scratch/` of the extracted docs.
