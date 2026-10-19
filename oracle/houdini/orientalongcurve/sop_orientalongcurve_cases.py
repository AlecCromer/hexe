"""Synthetic Orient Along Curve SOP cases recorded from real Houdini (hython; oracle tooling, portable: run as
`hython sop_orientalongcurve_cases.py <out-dir>`). Builds ONE input (`curves_in.geo`) from curves constructed here:
  helix (24 pts), planar zigzag (9), closed tilted circle (12), straight diagonal line (5), open curve with coincident
  points at the start / middle (3 in a row) / end (10), unrolled non-planar loop (last vertex reuses the first point),
  closed trefoil-like knot (16), closed square whose first and last points coincide, a single-point poly, a 2-point
  segment, two 3-point curves sharing their middle point (point-class averaging).
Prim attribute `target_up`, point attribute `roll` (0..1 along each curve) and prim group `sel` feed the attribute /
group cases. Writes `<case>_houdini.geo` per entry of CASES and `cases.json` (case -> parm overrides) for the test.
"""
import hou, json, math, os, re, sys

out = sys.argv[1]


def save(g, path):
    """saveToFile, then blank the machine / user fields Houdini stamps into `info` (fixtures must be portable).
    The detail `varmap` the Python SOP stamps (a multi-entry string tuple, irrelevant here) is dropped first."""
    g = g.freeze()
    vm = g.findGlobalAttrib('varmap')
    if vm is not None:
        vm.destroy()
    g.saveToFile(path)
    with open(path, encoding='utf-8') as f:
        text = f.read()
    for key in ('hostname', 'artist', 'date'):
        text = re.sub(r'"%s":"[^"]*"' % key, '"%s":""' % key, text)
    with open(path, 'w', encoding='utf-8', newline='\n') as f:
        f.write(text)


os.makedirs(out, exist_ok=True)
geo = hou.node('/obj').createNode('geo')


def r(x):
    return round(x, 6)


helix = [(r(math.cos(0.45 * i)), r(0.12 * i), r(math.sin(0.45 * i))) for i in range(24)]
zigzag = [(0.5 * i, 0.4 * (i % 2), 0.0) for i in range(9)]
tilt = math.radians(30)
circle = [(r(1.5 * math.cos(2 * math.pi * i / 12)), r(1.5 * math.sin(2 * math.pi * i / 12) * math.sin(tilt)),
           r(1.5 * math.sin(2 * math.pi * i / 12) * math.cos(tilt))) for i in range(12)]
line = [(0.3 * i, 0.2 * i, -0.1 * i) for i in range(5)]
coinc = [(0, 0, 0), (0, 0, 0), (1, 0, 0), (2, 0.5, 0), (2, 0.5, 0), (2, 0.5, 0), (3, 0.5, 1), (4, 0, 1), (4, 0, 1), (4, 0, 1)]
loop = [(r(math.cos(2 * math.pi * i / 9)), r(0.3 * math.sin(4 * math.pi * i / 9)), r(math.sin(2 * math.pi * i / 9))) for i in range(9)]
knot = []
for i in range(16):
    t = 2 * math.pi * i / 16
    knot.append((r(math.sin(t) + 2 * math.sin(2 * t)), r(math.cos(t) - 2 * math.cos(2 * t)), r(-math.sin(3 * t))))
square = [(0, 0, 5), (1, 0, 5), (1, 1, 5), (0, 1, 5), (0, 0, 5)]
single = [(7, 7, 7)]
seg = [(5, 0, 0), (5, 1, 0.5)]
# curves: (points, closed, extra vertex = first point (unrolled), in group `sel`, target_up)
curves = [
    (helix, False, False, True, (0, 1, 0)),
    (zigzag, False, False, False, (0, 0, 1)),
    (circle, True, False, True, (1, 1, 0)),
    (line, False, False, False, (0, 1, 0)),
    (coinc, False, False, True, (0, 0, 1)),
    (loop, False, True, False, (0, 1, 0)),
    (knot, True, False, True, (0.2, 0.3, 1)),
    (square, True, False, False, (0, 0, 1)),
    (single, False, False, True, (0, 1, 0)),
    (seg, False, False, False, (1, 0, 0)),
]
code = (
    "node=hou.pwd();g=node.geometry()\n"
    "g.addAttrib(hou.attribType.Prim,'target_up',(0.0,0.0,0.0))\n"
    "g.addAttrib(hou.attribType.Point,'roll',0.0)\n"
    "sel=g.createPrimGroup('sel')\n"
    "for pts,closed,unrolled,insel,up in %r:\n"
    "    made=[g.createPoint() for _ in pts]\n"
    "    n=len(made)\n"
    "    for k,(p,x) in enumerate(zip(made,pts)):\n"
    "        p.setPosition(x); p.setAttribValue('roll', k/max(n-1,1))\n"
    "    pr=g.createPolygon(); pr.setIsClosed(closed)\n"
    "    for p in made: pr.addVertex(p)\n"
    "    if unrolled: pr.addVertex(made[0])\n"
    "    pr.setAttribValue('target_up', tuple(float(c) for c in up))\n"
    "    if insel: sel.add(pr)\n"
    # two 3-point curves sharing their middle point
    "a=g.createPoint(); a.setPosition((8,0,0)); m=g.createPoint(); m.setPosition((9,0.5,0)); b=g.createPoint(); b.setPosition((10,0,0.5))\n"
    "c=g.createPoint(); c.setPosition((9,0.5,-1)); d=g.createPoint(); d.setPosition((9.5,1.5,0))\n"
    "for ids in ((a,m,b),(c,m,d)):\n"
    "    pr=g.createPolygon(); pr.setIsClosed(False)\n"
    "    for p in ids: pr.addVertex(p)\n"
    "    pr.setAttribValue('target_up',(0.0,1.0,0.0))\n"
    "for p in (a,m,b,c,d): p.setAttribValue('roll', 0.5)\n" % (curves,)
)
ALL_OUT = {'outputxaxis': 1, 'outputtranslation': 1, 'translationname': 'tr', 'outputquaternion': 1,
           'outputtransform3': 1, 'transform3name': 'xf3', 'outputtransform4': 1, 'transform4name': 'xf4'}
CASES = {
    'default': {},
    'diff': {'tangenttype': 1},
    'prev': {'tangenttype': 2},
    'next': {'tangenttype': 3},
    'none': {'tangenttype': 4, 'applyroll': 1, 'roll': 25.0},
    'extrap_avg': {'extrapolateendtangents': 1},
    'extrap_diff': {'tangenttype': 1, 'extrapolateendtangents': 1},
    'extrap_prev': {'tangenttype': 2, 'extrapolateendtangents': 1},
    'extrap_next': {'tangenttype': 3, 'extrapolateendtangents': 1},
    'normal_avg': {'upvectoratstart': 0},
    'x_avg': {'upvectortype': 1, 'upvectoratstart': 0},
    'z_start': {'upvectortype': 3},
    'custom_end': {'upvectortype': 5, 'upvectorx': 0.3, 'upvectory': 0.2, 'upvectorz': 1.0, 'useendupvector': 1,
                   'endupvectorx': 1.0, 'endupvectory': 0.0, 'endupvectorz': 0.0},
    'y_end': {'upvectortype': 2, 'useendupvector': 1},
    'normal_end': {'useendupvector': 1},
    'attrib_up': {'upvectortype': 4},
    'discontinuous': {'continuousclosed': 0},
    'roll_full': {'applyroll': 1, 'roll': 30.0, 'fulltwists': 1, 'incroll': 45.0},
    'roll_edge': {'applyroll': 1, 'rollper': 0, 'incroll': 10.0},
    'roll_distance_open': {'applyroll': 1, 'rollper': 1, 'incroll': 20.0, 'continuousclosed': 0},
    'euler_mix': {'rOrd': 5, 'applyroll': 1, 'roll': 5.0, 'rollper': 0, 'incroll': 10.0,
                  'applyyaw': 1, 'yaw': 15.0, 'yawper': 1, 'incyaw': 20.0,
                  'applypitch': 1, 'pitch': -20.0, 'pitchper': 3, 'incpitch': 90.0,
                  # per-full-curve-by-edges divides by the edge count: Houdini writes NaN on the 1-point prim 8
                  'group': '0-7 9-11'},
    'euler_xyz': {'applyyaw': 1, 'yaw': 33.0, 'applypitch': 1, 'pitch': 12.0, 'pitchper': 4, 'incpitch': 40.0},
    'roll_attrib': {'applyroll': 1, 'rollper': 2, 'incroll': 90.0},
    'scale_stretch': dict({'scale': 2.5, 'stretcharoundturns': 1, 'maxstretcharoundturns': 3.0}, **ALL_OUT),
    'outputs_point': dict(ALL_OUT),
    'outputs_vertex': dict({'class': 1}, **ALL_OUT),
    'group': {'group': 'sel'},
}

py = geo.createNode('python')
py.parm('python').set(code)
if py.geometry() is None:
    raise SystemExit('python SOP: %s' % (py.errors(),))
save(py.geometry(), os.path.join(out, 'curves_in.geo'))
for name, parms in CASES.items():
    n = geo.createNode('orientalongcurve')
    n.setInput(0, py)
    for k, v in parms.items():
        n.parm(k).set(v)
    g = n.geometry()
    if n.errors():
        raise SystemExit('%s: %s' % (name, n.errors()))
    save(g, os.path.join(out, name + '_houdini.geo'))
    print(name, 'warnings', n.warnings())
with open(os.path.join(out, 'cases.json'), 'w', encoding='utf-8', newline='\n') as f:
    json.dump(CASES, f, indent=1, sort_keys=True)
    f.write('\n')
print('points', len(py.geometry().points()), 'prims', len(py.geometry().prims()))
