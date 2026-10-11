import json, numpy as np
D=json.load(open('probes.json'))
def adj(g):
    n=len(g['src']); A=np.zeros((n,n))
    for pr in g['prims']:
        for k in range(len(pr)):
            a,b=pr[k],pr[(k+1)%len(pr)]
            if a!=b: A[a,b]=A[b,a]=1
    return A
def bnd(g):
    from collections import Counter
    c=Counter()
    for pr in g['prims']:
        for k in range(len(pr)):
            a,b=pr[k],pr[(k+1)%len(pr)]; c[(min(a,b),max(a,b))]+=1
    b=np.zeros(len(g['src']),bool)
    for (a,bb),v in c.items():
        if v==1: b[a]=b[bb]=True
    return b
import sys
for name,g in D.items():
    A=adj(g); P=np.array(g['src']); n=len(P); d=A.sum(1); L=np.diag(d)-A
    B=bnd(g)
    print(name,n,'bnd',B.sum())
    for r in g['runs']:
        if r['cb']!=0 or r['q']!=1: continue
        s=r['s']; H=np.array(r['P'])
        M=np.eye(n)+s*np.diag(1/d)@L
        X=np.linalg.solve(M,P)
        M2=np.eye(n)+s*L/ max(d)  # sym
        print('  s',s,'maxdP D^-1L solve',abs(X-H).max(),'  |H-P|',abs(H-P).max())
