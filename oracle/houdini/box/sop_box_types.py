"""Synthetic Box SOP cases recorded from real Houdini (hython; oracle tooling, portable: run as
`hython sop_box_types.py <out-dir>`). For each case writes <case>_houdini.geo (the Box output) plus one cases.json with
every case's non-default parms, so the hexe test cooks the very same parm set:
  pm_*   : type Polygon Mesh (divrate = points per axis; 1 = a flat axis at the centre; dodivs ignored),
  pts_*  : type Points (polymesh surface points, or the dodivs lattice when dodivs is on),
  poly_* : type Polygon with rotation, and with dodivs (Houdini: divs = SEGMENTS, a full lattice of open polylines).
"""
import hou, json, os, re, sys

out = sys.argv[1]


def save(g, path):
    """saveToFile, then blank the machine / user fields Houdini stamps into `info` (fixtures must be portable)."""
    g.saveToFile(path)
    with open(path, encoding='utf-8') as f:
        text = f.read()
    for key in ('hostname', 'artist', 'date'):
        text = re.sub(r'"%s":"[^"]*"' % key, '"%s":""' % key, text)
    with open(path, 'w', encoding='utf-8', newline='\n') as f:
        f.write(text)


os.makedirs(out, exist_ok=True)
box = hou.node('/obj').createNode('geo').createNode('box')

# type menu: 0 poly, 1 polymesh, 2 mesh, 3 nurbs, 4 bezier, 5 points, 6 prim (Houdini 20.5 item order)
cases = {
    'pm_default': {'type': 1},
    'pm_div111': {'type': 1, 'divrate': [1, 1, 1], 'size': [2, 3, 4]},
    'pm_flat_y': {'type': 1, 'divrate': [3, 1, 4], 'size': [2, 1, 3], 't': [0.25, 1, -0.5]},
    'pm_div2': {'type': 1, 'divrate': [2, 2, 2]},
    'pm_nonuniform': {'type': 1, 'divrate': [3, 4, 5], 'size': [2, 3, 4], 't': [1, -2, 0.5]},
    'pm_rot_scale': {'type': 1, 'divrate': [2, 5, 3], 'size': [1, 2, 3], 't': [1, 2, 3], 'r': [30, 45, 60], 'scale': 2},
    'pm_neg_dodivs': {'type': 1, 'divrate': [4, 2, 3], 'size': [-1, 2, 0.5], 'r': [0, 90, 0], 'dodivs': 1, 'divs': [5, 5, 5]},
    'pm_dense': {'type': 1, 'divrate': [7, 6, 8], 'scale': 0.5, 't': [0, 0.5, 0]},
    'pts_divrate': {'type': 5, 'divrate': [3, 2, 4], 'size': [1, 2, 3]},
    'pts_dodivs': {'type': 5, 'dodivs': 1, 'divs': [2, 3, 1], 'size': [2, 2, 2]},
    'pts_dodivs_noconsol': {'type': 5, 'dodivs': 1, 'divs': [2, 3, 1], 'consolidatepts': 0},
    'poly_rot_scale': {'type': 0, 'size': [1, 2, 3], 't': [1, 2, 3], 'r': [30, 45, 60], 'scale': 2},
    'poly_dodivs333': {'type': 0, 'dodivs': 1, 'divs': [3, 3, 3], 'size': [2, 4, 6], 't': [1, -1, 0.5], 'divrate': [2, 2, 2]},
    'poly_dodivs_mixed': {'type': 0, 'dodivs': 1, 'divs': [1, 2, 4], 'r': [10, 20, 30], 'consolidatepts': 0},
}
for name, parms in cases.items():
    for p in box.parms():
        p.revertToDefaults()
    for k, v in parms.items():
        box.parmTuple(k).set(v if isinstance(v, list) else [v])
    g = box.geometry()
    save(g, os.path.join(out, name + '_houdini.geo'))
    print(name, len(g.points()), 'pts', len(g.prims()), 'prims')
with open(os.path.join(out, 'cases.json'), 'w', encoding='utf-8', newline='\n') as f:
    json.dump(cases, f, indent=1, sort_keys=True)
    f.write('\n')
