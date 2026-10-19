"""Group Create edge / vertex groups (+ bounding sphere) recorded from real Houdini (hython; oracle tooling, portable: run
as `hython groupcreate_edge_vertex.py <out-dir>`). Every input is synthetic geometry built here:
  box:     Box SOP defaults (8 points, 6 quads);
  boxpre:  box + prim group `a` (0-1), point group `pa` (0 1 2), edge group `ea` (p0-1 p1-2), vertex group `va` (0v0 1v1);
  grid:    Grid SOP rows 3 cols 4 (12 points, 6 quads);
  ellip:   Sphere SOP polymesh rows 6 cols 7, rad (1, 0.5, 2), rotate (20, 30, 40) (planar quads, varied normals);
  torus:   Torus SOP rows 5 cols 7, rotate (15, 0, 25).
Writes <input>_in.geo for each input, cases.json (hexe-ready params + Houdini's group: an ORDERED edge list of point
pairs, or element indices) and two full outputs (edge / vertex group .geo) to check the writers' format.
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
inputs = {}
inputs['box'] = geo.createNode('box')
n = inputs['box']
for nm, gt, pat in (('a', 'primitive', '0-1'), ('pa', 'point', '0 1 2'), ('ea', 'edge', 'p0-1 p1-2'), ('va', 'vertex', '0v0 1v1')):
    g = geo.createNode('groupcreate')
    g.setInput(0, n)
    g.parm('groupname').set(nm)
    g.parm('grouptype').set(gt)
    g.parm('basegroup').set(pat)
    n = g
inputs['boxpre'] = n
gr = geo.createNode('grid')
gr.parm('rows').set(3)
gr.parm('cols').set(4)
inputs['grid'] = gr
sp = geo.createNode('sphere')
sp.parm('type').set('polymesh')
sp.parm('rows').set(6)
sp.parm('cols').set(7)
sp.parmTuple('rad').set((1, 0.5, 2))
sp.parmTuple('r').set((20, 30, 40))
inputs['ellip'] = sp
to = geo.createNode('torus')
to.parm('rows').set(5)
to.parm('cols').set(7)
to.parmTuple('r').set((15, 0, 25))
inputs['torus'] = to
for k, node in inputs.items():
    save(node.geometry(), os.path.join(out, k + '_in.geo'))

GT = {'primitive': 0, 'point': 1, 'edge': 2, 'vertex': 3}
MERGE = {'replace': 0, 'union': 1, 'intersect': 2, 'subtract': 3}
BOUND = {'usebbox': 0, 'usebsphere': 1}
gc = geo.createNode('groupcreate')


def run(src, kind, name='group1', **parms):
    gc.setInput(0, inputs[src])
    for p in gc.parms():
        p.revertToDefaults()
    gc.parm('grouptype').set(kind)
    gc.parm('groupname').set(name)
    hexe = {'grouptype': GT[kind], 'groupname': name}
    for k, v in parms.items():
        (gc.parmTuple(k) if isinstance(v, tuple) else gc.parm(k)).set(v)
        if k == 'mergeop':
            hexe[k] = MERGE[v]
        elif k == 'boundtype':
            hexe[k] = BOUND[v]
        elif k in ('groupbase', 'groupbounding', 'groupnormal', 'includenotwhollycontained', 'oppositenormals'):
            hexe[k] = bool(v)
        elif isinstance(v, tuple):
            hexe[k] = [float(c) for c in v]
        elif k == 'angle':
            hexe[k] = float(v)
        else:
            hexe[k] = v
    g = gc.geometry()
    if kind == 'edge':
        res = [[p.number() for p in e.points()] for e in g.findEdgeGroup(name).edges()]
    elif kind == 'vertex':
        res = [v.linearNumber() for v in g.findVertexGroup(name).vertices()]
    elif kind == 'point':
        res = [p.number() for p in g.findPointGroup(name).points()]
    else:
        res = [p.number() for p in g.findPrimGroup(name).prims()]
    return {'input': src, 'params': hexe, 'result': res, 'errors': list(gc.errors())}


cases = []
pats = ['', '0', '0-2', '1 3', '!0', '*', '2-0', '0-100', '9', '0 9', '0 2-0', '0-5:2', '0 0-5:2', 'nope', '0 nope',
        'p0-1', 'p0-1-2', 'p1-0', 'p0-5', '0 p0-5', 'p0', 'p0-1-5-4-0', '0e1', '0e1 0e2', '0e9', '0 0e9', '1v2', '0v1',
        '0 0v1', '0v1-3', '1 0', 'p1-0 0', 'p4-5 0', '0 !1', 'p1-2 !0', '!0 1', '!0 !1', '!0 ^p0-1', 'p1-2 !0 ^3',
        '0-1 ^0', '0-1 ^p0-1', '0-1 ^0e1', '0-1 ^0v1', '!*', '^0', '0v1 0v0', '1v0 0v0', '0v2-0', '0 ^1', '0 !1 ']
for kind in ('edge', 'vertex'):
    for src in ('box', 'grid'):
        for pat in pats:
            cases.append(run(src, kind, basegroup=pat))
        cases.append(run(src, kind, groupbase=0))
    for pat in ('a', 'pa', 'ea', 'va', 'a*', '!a', '!ea', '!va', '0-2 ^1'):
        cases.append(run('boxpre', kind, basegroup=pat))
    for op in ('replace', 'union', 'intersect', 'subtract'):
        cases.append(run('boxpre', kind, name='ea' if kind == 'edge' else 'va', basegroup='0', mergeop=op))
    for src in ('box', 'grid', 'ellip'):
        for t, s in (((0.5, 0.0, 0.0), (1.2, 2.0, 1.2)), ((0.3, 0.1, -0.2), (0.7, 0.5, 1.1)), ((0.5, -0.2, 0.9), (0.9, 0.3, 0.6))):
            for bt in ('usebbox', 'usebsphere'):
                for part in (0, 1):
                    cases.append(run(src, kind, groupbounding=1, boundtype=bt, t=t, size=s, includenotwhollycontained=part))
        cases.append(run(src, kind, groupbase=0, groupbounding=1, t=(0.5, 0.0, 0.0), size=(1.2, 2.0, 1.2)))
    for src in ('box', 'ellip', 'torus'):
        for d, ang, opp in (((0, 1, 0), 30, 0), ((1, 0.3, -0.2), 50, 0), ((0.2, -1, 0.5), 70, 0), ((0, 0, 1), 89, 0),
                            ((0.5, 0.5, 0.5), 40, 0), ((-1, 0, 0.3), 60, 0), ((0, 1, 0), 45, 1), ((0, 1, 0), 90, 0), ((0, 0, 0), 10, 0)):
            cases.append(run(src, kind, groupnormal=1, dir=d, angle=ang, oppositenormals=opp))
    cases.append(run('box', kind, groupedges=1, unshared=1))
# bounding sphere on points / prims
for kind in ('point', 'primitive'):
    for src in ('box', 'ellip'):
        for t, s in (((0.5, 0.0, 0.0), (1.2, 2.0, 1.2)), ((0.3, 0.1, -0.2), (0.7, 0.5, 1.1)), ((0.0, 0.0, 0.0), (1.4, 1.0, 3.0))):
            for part in (0, 1):
                cases.append(run(src, kind, groupbounding=1, boundtype='usebsphere', t=t, size=s, includenotwhollycontained=part))
with open(os.path.join(out, 'cases.json'), 'w', encoding='utf-8', newline='\n') as f:
    json.dump(cases, f, indent=0)
    f.write('\n')
# full outputs: an ordered edge group after a union, a vertex group
gc.setInput(0, inputs['boxpre'])
for p in gc.parms():
    p.revertToDefaults()
gc.parm('groupname').set('ea')
gc.parm('grouptype').set('edge')
gc.parm('basegroup').set('1 p4-5')
gc.parm('mergeop').set('union')
save(gc.geometry(), os.path.join(out, 'boxpre_edge_union_houdini.geo'))
gc.parm('groupname').set('vg')
gc.parm('grouptype').set('vertex')
gc.parm('basegroup').set('1 3v2 5v0-1')
gc.parm('mergeop').set('replace')
save(gc.geometry(), os.path.join(out, 'boxpre_vertex_houdini.geo'))
print(len(cases), 'cases;', sum(1 for c in cases if c['errors']), 'with errors')
