import hou, json
root=hou.node('/obj'); g=root.createNode('geo','g')
out=[]
def run(label, code, lam_of_k, ks):
  for k in ks:
    py=g.createNode('python','src')
    py.parm('python').set(code%k)
    sm=g.createNode('smooth::2.0','sm'); sm.setInput(0,py); sm.parm('contrainedboundary').set(0)
    for q in (1,2,3,4,5,6,8,10,15,20):
        sm.parm('filterquality').set(q); sm.parm('strength').set(1.0); sm.cook(force=True)
        gm=sm.geometry(); P=gm.points()[0].position()
        out.append({'label':label,'k':k,'q':q,'P0':list(P),'np':len(gm.points())})
    sm.destroy(); py.destroy()
quad='''
import math
geo=hou.pwd().geometry()
N=12;k=%d
pts=[[geo.createPoint() for j in range(N)] for i in range(N)]
for i in range(N):
  for j in range(N): pts[i][j].setPosition((math.cos(2*math.pi*k*i/N),0,0))
for i in range(N):
  for j in range(N):
    pr=geo.createPolygon()
    for a,b in ((0,0),(1,0),(1,1),(0,1)): pr.addVertex(pts[(i+a)%%N][(j+b)%%N])
'''
tri='''
import math
geo=hou.pwd().geometry()
N=12;k=%d
pts=[[geo.createPoint() for j in range(N)] for i in range(N)]
for i in range(N):
  for j in range(N): pts[i][j].setPosition((math.cos(2*math.pi*k*i/N),0,0))
for i in range(N):
  for j in range(N):
    for tri in (((0,0),(1,0),(1,1)),((0,0),(1,1),(0,1))):
      pr=geo.createPolygon()
      for a,b in tri: pr.addVertex(pts[(i+a)%%N][(j+b)%%N])
'''
run('quad',quad,None,(1,2,3))
run('tri',tri,None,(1,2,3))
json.dump(out,open('tor.json','w')); print(len(out))
