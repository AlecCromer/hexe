"""Synthetic Join SOP cases recorded from real Houdini (hython; oracle tooling, portable: run as
`hython sop_join_cases.py <out-dir>`). Writes <case>_in.geo (the input) and <case>_houdini.geo (Join's output) for:
  arcs2: two 127-point open arcs whose vertex order runs against point order, ends 0.5 apart at BOTH sides (a near tie
         for Connect Closest Ends: the trig endpoints carry ~1e-8 noise), blend tolerance 0.727;
  arcs7: seven 19-point arcs drawn right to left along x, blend tolerance 1.
"""
import hou, math, os, re, sys

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


def arcs(curves):
    """curves: list of (point list, reverse vertex order?) -> python SOP code."""
    return (
        "node=hou.pwd();g=node.geometry()\n"
        "for pts,rev in %r:\n"
        "    made=[g.createPoint() for _ in pts]\n"
        "    for p,x in zip(made,pts): p.setPosition(x)\n"
        "    pr=g.createPolygon(); pr.setIsClosed(False)\n"
        "    for p in (reversed(made) if rev else made): pr.addVertex(p)\n" % (curves,)
    )


def arc(cx, r, y0, n, start, end):
    return [(cx + r * math.cos(start + (end - start) * i / (n - 1)), y0 + r * math.sin(start + (end - start) * i / (n - 1)), 0.0) for i in range(n)]


cases = {
    # point order runs from the right end leftwards; vertex order reversed -> starts at the left end
    'arcs2': ([(arc(4.2, 3.3, 0.0, 127, 0.0, math.pi), True), (arc(4.2, 3.3, 0.5, 127, 0.0, math.pi), True)], 0.727),
    'arcs7': ([(arc(1.4 + 1.05 * k, 0.45 - 0.03 * k, 0.0, 19, 0.0, math.pi), False) for k in range(7)], 1.0),
}
for name, (curves, tol) in cases.items():
    py = geo.createNode('python')
    py.parm('python').set(arcs(curves))
    j = geo.createNode('join')
    j.setInput(0, py)
    j.parm('tolerance').set(tol)
    save(py.geometry(), os.path.join(out, name + '_in.geo'))
    save(j.geometry(), os.path.join(out, name + '_houdini.geo'))
    g = j.geometry()
    print(name, 'in', len(py.geometry().points()), 'out', len(g.points()), 'pts', len(g.prims()), 'prims')
