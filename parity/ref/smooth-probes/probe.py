import hou, json, random
root=hou.node('/obj'); g=root.createNode('geo','g')
def src(name):
    if name=='box':
        return g.createNode('box','src')
    if name=='grid5':
        n=g.createNode('grid','src'); n.parm('rows').set(5); n.parm('cols').set(5); return n
    if name=='sphere':
        n=g.createNode('sphere','src'); n.parm('type').set(1); n.parm('freq').set(2); return n
    if name in('fan','grid5n'):
        n=g.createNode('python','src')
        if name=='fan':
            code='''
import math
geo=hou.pwd().geometry()
c=geo.createPoint(); c.setPosition((0,0,0))
ps=[]
for i in range(8):
    p=geo.createPoint(); a=2*math.pi*i/8; p.setPosition((math.cos(a),0,math.sin(a))); ps.append(p)
for i in range(8):
    pr=geo.createPolygon(); [pr.addVertex(x) for x in (c,ps[i],ps[(i+1)%8])]
'''
        else:
            code='''
import random
random.seed(3)
geo=hou.pwd().geometry()
N=5
pts=[]
for j in range(N):
    for i in range(N):
        p=geo.createPoint(); p.setPosition((i+random.uniform(-.3,.3),random.uniform(-.2,.2),j+random.uniform(-.3,.3))); pts.append(p)
for j in range(N-1):
    for i in range(N-1):
        pr=geo.createPolygon()
        for q in (pts[j*N+i],pts[j*N+i+1],pts[(j+1)*N+i+1],pts[(j+1)*N+i]): pr.addVertex(q)
'''
        n.parm('python').set(code); return n
out={}
for geoname in ['box','grid5','sphere','fan','grid5n']:
    s0=src(geoname)
    s0.cook(force=True)
    out[geoname]={'src':[list(p.position()) for p in s0.geometry().points()],'prims':[[v.point().number() for v in pr.vertices()] for pr in s0.geometry().prims()],'runs':[]}
    sm=g.createNode('smooth::2.0','sm'); sm.setInput(0,s0)
    for cb in (0,1):
      for meth in (0,):
        for q in (1,2,3,5,10):
            for st in (0.25,0.5,1.0,2.0):
                sm.parm('contrainedboundary').set(cb); sm.parm('method').set(meth); sm.parm('filterquality').set(q); sm.parm('strength').set(st)
                sm.cook(force=True)
                out[geoname]['runs'].append({'cb':cb,'method':meth,'q':q,'s':st,'P':[list(p.position()) for p in sm.geometry().points()]})
    sm.destroy(); s0.destroy()
json.dump(out,open('probes.json','w'))
print('ok',{k:len(v['runs']) for k,v in out.items()})
