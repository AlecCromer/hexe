import hou
geo = hou.node('/obj').createNode('geo')
def run(curves, extra="", **parms):
    py = geo.createNode('python')
    code = "node=hou.pwd();g=node.geometry()\n"
    code += "g.addAttrib(hou.attribType.Point,'Cd',(1.0,1.0,1.0))\ng.addAttrib(hou.attribType.Prim,'id',0)\ng.addAttrib(hou.attribType.Vertex,'uv',(0.0,0.0,0.0))\n"
    code += "for i,(c,closed) in enumerate(%r):\n" % (curves,)
    code += "    p=g.createPolygon(); p.setIsClosed(closed); p.setAttribValue('id',i+10)\n"
    code += "    for k,x in enumerate(c):\n        pt=g.createPoint(); pt.setPosition(x); pt.setAttribValue('Cd',(float(i),float(k),0.0)); v=p.addVertex(pt); v.setAttribValue('uv',(float(k),float(i),0.0))\n"
    code += extra
    py.parm('python').set(code)
    j = geo.createNode('join'); j.setInput(0, py)
    for k,v in parms.items(): j.parm(k).set(v)
    g = j.geometry()
    print(parms, 'npts', len(g.points()))
    for p in g.points(): print('   pt', p.number(), tuple(round(c,4) for c in p.position()), 'Cd', tuple(round(c,4) for c in p.attribValue('Cd')))
    for pr in g.prims():
        print('   prim', pr.number(), pr.isClosed(), 'id', pr.attribValue('id'), [v.point().number() for v in pr.vertices()], 'uv', [tuple(round(c,3) for c in v.attribValue('uv'))[:2] for v in pr.vertices()], [g.name() for g in pr.groups()] if hasattr(pr,'groups') else '')
    py.destroy(); j.destroy()
A = [(0,0,0),(1,0,0),(2,0,0)]
B = [(3,0,0),(4,0,0),(5,1,0)]
C = [(6,0,0),(7,0,0)]
D = [(8,0,0),(9,0,0)]
E = [(10,0,0),(11,0,0)]
print('### attrs blend .5'); run([(A,False),(B,False)], blend=1, tolerance=0.5, bias=0.25)
print('### attrs blend 1'); run([(A,False),(B,False)], blend=1, tolerance=1, bias=0.25)
print('### blend+flip'); run([(list(reversed(A)),False),(list(reversed(B)),False)], blend=1, tolerance=0.5, proximity=1)
print('### blend+flipfirst'); run([(A,False),([(-1,0,0),(-2,1,0),(-3,1,0)],False)], blend=1, tolerance=0.5, proximity=1)
for jo in ('group','skip'):
  for inc in (1,2,3):
    print('### joinop',jo,inc); run([(A,False),(B,False),(C,False),(D,False),(E,False)], blend=0, joinop=jo, inc=inc)
print('### group parm'); run([(A,False),(B,False),(C,False),(D,False)], extra="gr=g.createPrimGroup('sel')\nfor i in (0,2,3): gr.add(g.prims()[i])\n", blend=0, group='sel')
print('### onlyconn'); run([(A,False),(B,False),(C,False)], extra="", blend=0, onlyconnected=1)
