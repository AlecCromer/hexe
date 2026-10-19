"""Edge Divide `group` syntax recorded from real Houdini 20.5 (hython; oracle tooling, portable: run as
`hython sop_edgedivide_groups.py <out-dir>`). Synthetic inputs only, each carrying edge groups made by Group Create:
  grid3:   3x3 point grid (4 quads); edge groups zeta / mid / alpha / b2 / empty (created in that, non-alphabetical,
           order), prim group qg, point group pg;
  box:     default box (6 quads, closed); edge groups ring (prim 0's edges), pair (2 edges), prim group side;
  openbox: box without its top face (a boundary loop); edge group rim = the boundary, edge group spoke.
Writes <input>_in.geo for each input and cases.json: one record per (input, group string, numdivs, sharedpoints) with
Houdini's result (error flag, or point count + every prim's point list + positions of the new points + the output's
edge groups).
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
geo = hou.node('/obj').createNode('geo')


def group(inp, name, pat, gt=2):
    """Group Create: gt 0 prim / 1 point / 2 edge; pat None = an empty group (base group off)."""
    n = geo.createNode('groupcreate')
    n.setInput(0, inp)
    n.parm('groupname').set(name)
    n.parm('grouptype').set(gt)
    if pat is None:
        n.parm('groupbase').set(0)
    else:
        n.parm('basegroup').set(pat)
    return n


grid = geo.createNode('grid')
grid.parm('rows').set(3)
grid.parm('cols').set(3)
g = group(grid, 'zeta', 'p4-3 p7-4')
g = group(g, 'mid', 'p2-1-0')
g = group(g, 'qg', '1', 0)
g = group(g, 'alpha', 'p8-7')
g = group(g, 'b2', 'p5-8')
g = group(g, 'empty', None)
grid3 = group(g, 'pg', '1 4', 1)

box = geo.createNode('box')
b = group(box, 'ring', '0')
b = group(b, 'pair', '3e1 4e2')
boxin = group(b, 'side', '1', 0)

blast = geo.createNode('blast')
blast.setInput(0, box)
blast.parm('group').set('5')
blast.parm('grouptype').set(4)  # primitives
bg = blast.geometry()
# boundary = edges used by exactly one prim, written as `pA-B` tokens in prim / corner order
uses = {}
order = []
for pr in bg.prims():
    pts = [v.point().number() for v in pr.vertices()]
    for k in range(len(pts)):
        e = (pts[k], pts[(k + 1) % len(pts)])
        key = tuple(sorted(e))
        if key not in uses:
            order.append(e)
        uses[key] = uses.get(key, 0) + 1
rim = ' '.join('p%d-%d' % e for e in order if uses[tuple(sorted(e))] == 1)
o = group(blast, 'rim', rim)
openbox = group(o, 'spoke', 'p0-1 p1-2')

inputs = {'grid3': grid3, 'box': boxin, 'openbox': openbox}

cases = {
    'grid3': [
        # named edge groups, globs, several names (Houdini emits them sorted by name, then the ad-hoc part)
        'mid', 'zeta', 'alpha', 'empty', 'm*', '*s', 'zeta mid alpha b2', 'b2 alpha', 'mid zeta',
        # names mixed with ad-hoc tokens / prim groups
        'p1-4 mid', 'mid p1-4', 'p4-1 mid', 'p1-4 mid p3-4', 'p1-4 qg', 'qg p1-4', 'qg mid', '0e1 mid', 'p1-4 m*',
        '* 0', '0 *', '* p1-4', '* alpha', '* *', 'p1-4 *',
        # ad-hoc only
        '*', 'p1-4', 'p4-1', 'p2-1-0', 'p1-4-7', 'p4', 'p8', 'p4-3-4', 'p1-4 p3-4', 'p3-4 p1-4', 'p4-1 p1-4',
        '0e0', '0e1', '1', '1 0', '0-1', '1-1', '0-0', 'p5-4 0', 'qg',
        # selections that divide nothing (valid group, input passes through)
        'p0-4', 'p1-1', 'p0-4-8', 'p0-4 p1-4',
        # complement / subtract
        '!mid', '!mid !zeta', 'p1-4 !mid', '!mid p1-4', '0e1 !zeta', '!p1-4', '!qg', '!empty', 'p1-4 !mid ^zeta',
        '!p1-4 ^zeta', '!mid ^p1-4', '0 ^p1-4', '0-3 ^p0-1', 'mid ^p0-1', 'mid ^qg', 'zeta ^p4-3', 'qg ^zeta',
        'p1-4 ^zeta', 'mid ^mid', 'empty ^mid', 'mid ^m*', 'mid ^p0-1 0', 'mid ^p1-0 ^p2-1', 'p3-0 mid ^p2-1',
        'p3-0 ^zeta', 'mid 0 ^0', 'mid 5', '5 mid',
        # errors: "not a valid group"
        'nonexist', 'pg', '5', 'p9', 'p9-10', '0e9', '1-0', '0v1', '1:2', '0,1', '@P.x>0', 'e1', 'p', 'p-1', '-1',
        '^mid', '^p0-1', '^', 'p4-3 ^zeta', 'p0-1 ^p0-1', 'p0-4 ^p0-4', 'p3-0 ^0', 'p1-4 5', 'p1-4 0:1',
        # tokens Houdini ignores when the rest selects something
        'p1-4 nonexist', 'p1-4 pg', 'p1-4 @P.x>0',
    ],
    'box': ['ring', 'pair', 'ring pair', 'pair p0-1', 'side', 'side ring', '*', '!ring', 'ring ^pair', 'p0-1',
            '0e0 ring', 'p0-1-2-3', 'nonexist'],
    'openbox': ['rim', 'spoke', 'rim spoke', '!rim', 'p0-1 rim', 'rim ^spoke', '*', '0', 'r*'],
}
extra = [  # (input, group, numdivs, sharedpoints)
    ('grid3', 'mid', 2, 0), ('grid3', 'mid', 4, 0), ('grid3', 'p1-4 mid', 3, 1), ('grid3', '*', 3, 1),
    ('grid3', '!mid', 2, 1), ('box', 'ring pair', 3, 1), ('openbox', 'rim', 2, 1), ('openbox', '!rim', 3, 1),
]
runs = [(i, s, 3, 0) for i, ss in cases.items() for s in ss] + extra

records = []
for name, node in inputs.items():
    save(node.geometry(), os.path.join(out, name + '_in.geo'))
for inp, s, nd, sh in runs:
    ed = geo.createNode('edgedivide')
    ed.setInput(0, inputs[inp])
    ed.parm('group').set(s)
    ed.parm('numdivs').set(nd)
    ed.parm('sharedpoints').set(sh)
    try:
        g = ed.geometry()
    except hou.Error:
        g = None
    rec = {'input': inp, 'group': s, 'numdivs': nd, 'sharedpoints': bool(sh)}
    if g is None or ed.errors():
        rec['error'] = True
    else:
        n_in = len(inputs[inp].geometry().points())
        rec['points'] = len(g.points())
        rec['prims'] = [[v.point().number() for v in p.vertices()] for p in g.prims()]
        rec['new_P'] = [[round(c, 6) for c in p.position()] for p in g.points()[n_in:]]
        rec['edge_groups'] = {eg.name(): [[q.number() for q in e.points()] for e in eg.edges()] for eg in g.edgeGroups()}
    records.append(rec)
    ed.destroy()
with open(os.path.join(out, 'cases.json'), 'w', encoding='utf-8', newline='\n') as f:
    json.dump(records, f, indent=0, sort_keys=True)
    f.write('\n')
print(len(records), 'cases;', sum(1 for r in records if r.get('error')), 'errors')
