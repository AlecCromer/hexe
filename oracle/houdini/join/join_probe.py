import hou, sys
geo = hou.node('/obj').createNode('geo')
def run(curves, **parms):
    py = geo.createNode('python')
    code = "node=hou.pwd();g=node.geometry()\n"
    code += "for c,closed in %r:\n" % (curves,)
    code += "    p=g.createPolygon(); p.setIsClosed(closed)\n"
    code += "    for x in c:\n        pt=g.createPoint(); pt.setPosition(x); p.addVertex(pt)\n"
    py.parm('python').set(code)
    j = geo.createNode('join'); j.setInput(0, py)
    for k,v in parms.items(): j.parm(k).set(v)
    g = j.geometry()
    out = []
    for pr in g.prims():
        out.append((pr.type().name(), pr.isClosed() if hasattr(pr,'isClosed') else None, [tuple(round(c,4) for c in v.point().position()) for v in pr.vertices()], [v.point().number() for v in pr.vertices()]))
    print(parms, 'npts', len(g.points()), [tuple(round(c,4) for c in p.position()) for p in g.points()])
    for o in out: print('   ', o)
    py.destroy(); j.destroy()
A = [(0,0,0),(1,0,0),(2,0,0)]
B = [(3,0,0),(4,0,0),(5,1,0)]
C = [(5,3,0),(3,3,0)]
Br = list(reversed(B))
cases = [[(A,False),(B,False)], [(A,False),(Br,False)], [(Br,False),(A,False)], [(A,False),(B,False),(C,False)], [(A,False),(C,False),(B,False)]]
for cs in cases:
    print('=== curves', cs)
    for blend in (0,1):
      for prox in (0,1):
        run(cs, blend=blend, proximity=prox, tolerance=1.0, bias=0.5)
    run(cs, blend=1, proximity=1, tolerance=0.727, bias=0.5)
    run(cs, blend=1, proximity=1, tolerance=0.5, bias=0.0)
    run(cs, blend=1, proximity=0, tolerance=0.0, bias=0.5)
    run(cs, blend=0, proximity=0, loop=1)
    run(cs, blend=0, proximity=0, prim=1)
    run(cs, blend=0, proximity=0, onlyconnected=1)
