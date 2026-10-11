import hou, json
root=hou.node('/obj'); g=root.createNode('geo','g')
out=[]
for N in (16,64):
  for kind in ('poly','line'):
    py=g.createNode('python','src')
    py.parm('python').set('''
import math
geo=hou.pwd().geometry()
N=%d
pts=[]
for i in range(N):
    p=geo.createPoint(); pts.append(p)
KS=[1,2,3,4,5,6,7,8]
for i,p in enumerate(pts):
    p.setPosition(tuple(math.cos(k*2*math.pi*i/N) for k in KS[:3]))
    
prim=geo.createPolygon()
for p in pts: prim.addVertex(p)
'''%N)
    # 3 coords per probe => do multiple probes per mode set by separate runs
    py.destroy()
# simpler: one mode per run via parm-less rewrite
for N in (16,64):
  for k in range(1,N//2+1):
    py=g.createNode('python','src')
    py.parm('python').set('''
import math
geo=hou.pwd().geometry()
N=%d;k=%d
pts=[geo.createPoint() for i in range(N)]
for i,p in enumerate(pts): p.setPosition((math.cos(2*math.pi*k*i/N),math.sin(2*math.pi*k*i/N),0))
prim=geo.createPolygon()
for p in pts: prim.addVertex(p)
'''%(N,k))
    sm=g.createNode('smooth::2.0','sm'); sm.setInput(0,py); sm.parm('contrainedboundary').set(0)
    for q in (1,2,3,4,5,10):
      for s in (0.5,1.0):
        sm.parm('filterquality').set(q); sm.parm('strength').set(s); sm.cook(force=True)
        P=sm.geometry().points()[0].position()
        out.append({'N':N,'k':k,'q':q,'s':s,'P0':list(P)})
    sm.destroy(); py.destroy()
json.dump(out,open('ring.json','w')); print(len(out))
