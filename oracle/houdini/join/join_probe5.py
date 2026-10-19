import hou
geo = hou.node('/obj').createNode('geo')
def run(n, **parms):
    py = geo.createNode('python')
    py.parm('python').set("node=hou.pwd();g=node.geometry()\nfor i in range(%d):\n    p=g.createPolygon();p.setIsClosed(False)\n    for k in range(2):\n        pt=g.createPoint();pt.setPosition((i*2+k,0,0));p.addVertex(pt)\n" % n)
    j = geo.createNode('join'); j.setInput(0, py)
    for k,v in parms.items(): j.parm(k).set(v)
    g = j.geometry()
    print(n, parms, [[int(v.point().position()[0])//2 for v in pr.vertices()][::2] for pr in g.prims()])
    py.destroy(); j.destroy()
for n in (4,5,6,7,9):
  for inc in (2,3,4):
    run(n, blend=0, joinop='skip', inc=inc)
for n in (5,7):
  for inc in (2,3):
    run(n, blend=0, joinop='group', inc=inc)
run(5, blend=0, joinop='group', inc=0)
run(5, blend=0, joinop='skip', inc=0)
