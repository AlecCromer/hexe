import hou
geo = hou.node('/obj').createNode('geo')
def run(code, **parms):
    py = geo.createNode('python'); py.parm('python').set("node=hou.pwd();g=node.geometry()\n"+code)
    j = geo.createNode('join'); j.setInput(0, py)
    for k,v in parms.items(): j.parm(k).set(v)
    g = j.geometry()
    if j.errors(): print('ERR', j.errors())
    if j.warnings(): print('WARN', j.warnings())
    print(parms, 'npts', len(g.points()), [tuple(round(c,3) for c in p.position()) for p in g.points()])
    for pr in g.prims(): print('   prim', pr.number(), pr.type().name(), [v.point().number() for v in pr.vertices()])
    py.destroy(); j.destroy()
# shared-point curves: A=0,1,2 B=2,3,4 (shares pt2), C=5,6 separate, D = 4,7 shares with B end
code = """
P=[(0,0,0),(1,0,0),(2,0,0),(3,0,0),(4,0,0),(9,0,0),(10,0,0),(5,1,0)]
pts=[g.createPoint() for _ in P]
for p,x in zip(pts,P): p.setPosition(x)
for ids in ((0,1,2),(2,3,4),(5,6),(4,7)):
    pr=g.createPolygon(); pr.setIsClosed(False)
    for i in ids: pr.addVertex(pts[i])
"""
for oc in (0,1):
  for bl in (0,1):
    run(code, onlyconnected=oc, blend=bl, tolerance=0.5)
code2 = """
P=[(0,0,0),(1,0,0),(2,0,0),(3,0,0),(4,0,0)]
pts=[g.createPoint() for _ in P]
for p,x in zip(pts,P): p.setPosition(x)
for ids in ((2,1,0),(2,3,4)):
    pr=g.createPolygon(); pr.setIsClosed(False)
    for i in ids: pr.addVertex(pts[i])
"""
run(code2, onlyconnected=1, blend=0)
run(code2, onlyconnected=1, blend=0, proximity=0)
run(code2, onlyconnected=0, blend=0, proximity=1)
run(code2, onlyconnected=0, blend=1, tolerance=1)
