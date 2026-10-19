"""Synthetic Remesh SOP (remesh::2.0) cases recorded from real Houdini (hython; oracle tooling, portable: run as
`hython sop_remesh_cases.py <out-dir>`). Remesh is iterative isotropic remeshing, so the evidence is STATISTICAL, not
point-for-point: for every case this writes <input>_in.geo (the input, built from Houdini's own grid / sphere / box /
tube SOPs or a python SOP) and one line of `stats.json` with Houdini's output counts, edge-length mean / stddev /
min / max, boundary + non-manifold edge counts, the farthest output point from the input surface, and areas.
Inputs:
  grid:    default Grid SOP (10x10, 10x10 points, quads)
  sphere4: polygon Sphere SOP, frequency 4 (radius 1)
  sphere8: polygon Sphere SOP, frequency 8 (radius 1)
  box:     default Box SOP (unit, 6 quads)
  tube:    open polygon Tube SOP (no caps, 12 columns, 2 rows)
  fold:    10x10 grid of quads folded 40 degrees along row 4, the fold row carried in edge group `crease`
"""
import hou, json, math, os, re, sys

out = sys.argv[1]
os.makedirs(out, exist_ok=True)


def save(g, path):
    """saveToFile, then blank the machine / user fields Houdini stamps into `info` (fixtures must be portable)."""
    g.saveToFile(path)
    with open(path, encoding='utf-8') as f:
        text = f.read()
    for key in ('hostname', 'artist', 'date'):
        text = re.sub(r'"%s":"[^"]*"' % key, '"%s":""' % key, text)
    with open(path, 'w', encoding='utf-8', newline='\n') as f:
        f.write(text)


geo = hou.node('/obj').createNode('geo')

FOLD = (
    "import math\n"
    "node=hou.pwd();g=node.geometry()\n"
    "n=10\n"
    "zc=-5.0+4*10.0/9\n"
    "pts=[]\n"
    "for r in range(n):\n"
    "    for c in range(n):\n"
    "        x=-5.0+c*10.0/9; z=-5.0+r*10.0/9\n"
    "        y=max(0.0,z-zc)*math.tan(math.radians(40.0))\n"
    "        p=g.createPoint(); p.setPosition((x,y,z)); pts.append(p)\n"
    "for r in range(n-1):\n"
    "    for c in range(n-1):\n"
    "        a=r*n+c\n"
    "        pr=g.createPolygon()\n"
    "        for i in (a,a+n,a+n+1,a+1): pr.addVertex(pts[i])\n"
    "eg=g.createEdgeGroup('crease')\n"
    "for c in range(n-1):\n"
    "    eg.add(g.findEdge(pts[4*n+c],pts[4*n+c+1]))\n"
)


def inputs():
    grid = geo.createNode('grid')
    s4 = geo.createNode('sphere'); s4.parm('type').set(2); s4.parm('freq').set(4)
    s8 = geo.createNode('sphere'); s8.parm('type').set(2); s8.parm('freq').set(8)
    box = geo.createNode('box')
    tube = geo.createNode('tube'); tube.parm('type').set(1); tube.parm('cap').set(0)
    fold = geo.createNode('python'); fold.parm('python').set(FOLD)
    return {'grid': grid, 'sphere4': s4, 'sphere8': s8, 'box': box, 'tube': tube, 'fold': fold}


def edges_of(g):
    edges = {}
    for p in g.prims():
        vs = [v.point().number() for v in p.vertices()]
        for k in range(len(vs)):
            a, b = vs[k], vs[(k + 1) % len(vs)]
            e = (min(a, b), max(a, b))
            edges[e] = edges.get(e, 0) + 1
    return edges


def area(g):
    return sum(p.intrinsicValue('measuredarea') for p in g.prims())


def stats(g, src):
    edges = edges_of(g)
    P = [pt.position() for pt in g.points()]
    L = [(P[a] - P[b]).length() for (a, b) in edges]
    m = sum(L) / len(L) if L else 0.0
    sd = math.sqrt(sum((x - m) ** 2 for x in L) / len(L)) if L else 0.0
    dmax = 0.0
    for p in P:
        hit = src.nearestPrim(p)
        if hit is not None:
            dmax = max(dmax, hit[3])
    return {
        'points': len(P),
        'prims': len(g.prims()),
        'all_tris': all(len(p.vertices()) == 3 for p in g.prims()),
        'edges': len(edges),
        'edge_mean': round(m, 5),
        'edge_sd': round(sd, 5),
        'edge_min': round(min(L), 5) if L else 0.0,
        'edge_max': round(max(L), 5) if L else 0.0,
        'boundary_edges': sum(1 for c in edges.values() if c == 1),
        'nonmanifold_edges': sum(1 for c in edges.values() if c > 2),
        'max_dist': round(dmax, 6),
        'area_in': round(area(src), 5),
        'area_out': round(area(g), 5),
        'edge_groups': {e.name(): len(e.edges()) for e in g.edgeGroups()},
    }


CASES = [
    ('grid_t05', 'grid', {'targetsize': 0.5}),
    ('grid_t02', 'grid', {'targetsize': 0.2}),
    ('grid_it0', 'grid', {'targetsize': 0.5, 'iterations': 0}),
    ('grid_it10', 'grid', {'targetsize': 0.5, 'iterations': 10}),
    ('grid_inputptsonly', 'grid', {'targetsize': 0.5, 'inputptsonly': 1}),
    ('sphere4_t02', 'sphere4', {'targetsize': 0.2}),
    ('box_t025', 'box', {'targetsize': 0.25}),
    ('tube_t015', 'tube', {'targetsize': 0.15}),
    ('fold_hard', 'fold', {'targetsize': 0.3, 'hard_edges': 'crease', 'useouthardedgesgroup': 1}),
    ('sphere8_adaptive_d1', 'sphere8', {'sizing': 1, 'density': 1.0}),
    ('sphere8_adaptive_d2', 'sphere8', {'sizing': 1, 'density': 2.0}),
    ('sphere8_adaptive_d4', 'sphere8', {'sizing': 1, 'density': 4.0}),
    ('sphere8_adaptive_max03', 'sphere8', {'sizing': 1, 'density': 2.0, 'usemaxsize': 1, 'maxsize': 0.3}),
    ('box_adaptive_d2', 'box', {'sizing': 1, 'density': 2.0}),
    ('grid_adaptive_max05', 'grid', {'sizing': 1, 'usemaxsize': 1, 'maxsize': 0.5}),
]

srcs = inputs()
for name, node in srcs.items():
    save(node.geometry(), os.path.join(out, name + '_in.geo'))
rows = []
for case, src, parms in CASES:
    r = geo.createNode('remesh')
    r.setInput(0, srcs[src])
    for k, v in parms.items():
        r.parm(k).set(v)
    st = stats(r.geometry(), srcs[src].geometry())
    if src == 'fold':
        # the crease row (z = zc, y = 0): output edges lying on it and their total length
        zc = -5.0 + 4 * 10.0 / 9
        g = r.geometry()
        P = [pt.position() for pt in g.points()]
        on = [(a, b) for (a, b) in edges_of(g) if abs(P[a][2] - zc) < 1e-4 and abs(P[b][2] - zc) < 1e-4]
        st['crease_edges'] = len(on)
        st['crease_length'] = round(sum((P[a] - P[b]).length() for (a, b) in on), 5)
    rows.append({'case': case, 'input': src + '_in.geo', 'parms': parms, 'houdini': st})
    print(case, st, r.errors())
with open(os.path.join(out, 'stats.json'), 'w', encoding='utf-8', newline='\n') as f:
    f.write(json.dumps(rows, indent=1, sort_keys=True) + '\n')
