"""Sphere SOP type poly (geodesic) reference values recorded from real Houdini (hython; oracle tooling, portable: run as
`hython sphere_poly_ref.py`). Prints the samples hard-coded in crates/sop/src/builtins/sphere_sop.rs tests:
freq 1 / 2 prim lists, freq 5 samples for orient x / y / z, freq 2 with rad / scale / rotate / translate (orient y, x),
freq 101 counts + samples (Houdini has no upper bound on `freq`; min 1).
"""
import hou

geo = hou.node('/obj').createNode('geo')
s = geo.createNode('sphere')
s.parm('type').set('poly')
t = s.parmTemplateGroup().find('freq')
print('freq range', t.minValue(), t.minIsStrict(), t.maxValue(), t.maxIsStrict())


def dump(label, pts, prims):
    g = s.geometry()
    print(label, 'points', len(g.points()), 'prims', len(g.prims()))
    for i in pts:
        print('  P', i, list(g.points()[i].position()))
    for i in prims:
        print('  prim', i, [v.point().number() for v in g.prims()[i].vertices()])


for f in (1, 2):
    s.parm('freq').set(f)
    g = s.geometry()
    print('freq', f, 'prims', [[v.point().number() for v in pr.vertices()] for pr in g.prims()])
    print('freq', f, 'P', [list(p.position()) for p in g.points()])
for o in ('y', 'x', 'z'):
    s.parm('orient').set(o)
    s.parm('freq').set(5)
    dump('freq 5 orient ' + o, (4, 7, 8, 100, 251), (0, 24, 25, 100, 499))
s.parm('freq').set(2)
s.parmTuple('rad').set((1, 2, 3))
s.parm('scale').set(1.5)
s.parmTuple('r').set((10, 20, 30))
s.parmTuple('t').set((0.5, -1, 2))
for o in ('y', 'x'):
    s.parm('orient').set(o)
    dump('freq 2 xform orient ' + o, (0, 13, 41), ())
for p in s.parms():
    p.revertToDefaults()
s.parm('type').set('poly')
s.parm('freq').set(101)
dump('freq 101', (0, 1, 11, 12, 13, 100, 1000, 12345, 50000, 102011), (0, 1, 2, 5000, 204019))
