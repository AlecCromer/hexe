import json, numpy as np
exec(open('fit.py').read().split('import sys')[0])
from numpy.linalg import matrix_power as mp
variants={}
def K_rowdiag(L,d,q):
    M=np.diag(1/d)@L; Mq=mp(M,q); return np.diag(1/np.diag(Mq))@Mq
def K_coldiag(L,d,q):
    M=np.diag(1/d)@L; Mq=mp(M,q); return Mq@np.diag(1/np.diag(Mq))
def K_Lform(L,d,q):  # M=L D^-1
    M=L@np.diag(1/d); Mq=mp(M,q); return np.diag(1/np.diag(Mq))@Mq
def K_Lform_c(L,d,q):
    M=L@np.diag(1/d); Mq=mp(M,q); return Mq@np.diag(1/np.diag(Mq))
V={'row':K_rowdiag,'col':K_coldiag,'LDrow':K_Lform,'LDcol':K_Lform_c}
for name,g in D.items():
    A=adj(g); P=np.array(g['src']); n=len(P); d=A.sum(1); L=np.diag(d)-A
    res={k:0 for k in V}
    for r in g['runs']:
        if r['cb']!=0: continue
        q=min(r['q'],5); H=np.array(r['P'])
        for k,f in V.items():
            X=np.linalg.solve(np.eye(n)+r['s']*f(L,d,q),P)
            res[k]=max(res[k],abs(X-H).max())
    print(name,{k:'%.2e'%v for k,v in res.items()})
print('---detail')
for name in ('grid5','sphere'):
    g=D[name]; A=adj(g); P=np.array(g['src']); n=len(P); d=A.sum(1); L=np.diag(d)-A
    for r in g['runs']:
        if r['cb']!=0: continue
        q=min(r['q'],5); H=np.array(r['P'])
        X=np.linalg.solve(np.eye(n)+r['s']*K_rowdiag(L,d,q),P)
        e=abs(X-H); print(name,'q',r['q'],'s',r['s'],'err %.2e'%e.max(),'at',np.unravel_index(e.argmax(),e.shape))
print('---resid')
for name in ('grid5','grid5n','sphere'):
    g=D[name]; A=adj(g); P=np.array(g['src']); n=len(P); d=A.sum(1); L=np.diag(d)-A
    for r in g['runs']:
        if r['cb']!=0 or r['q']>5: continue
        q=r['q']; H=np.array(r['P']); Kq=K_rowdiag(L,d,q); S=np.eye(n)+r['s']*Kq
        res=S@H-P; 
        # symmetrised variants
        print(name,'q',q,'s',r['s'],'|res|inf %.2e'%abs(res).max(),'rel2 %.2e'%(np.linalg.norm(res)/np.linalg.norm(P)), 'colwise',['%.1e'%(np.linalg.norm(res[:,c])/max(np.linalg.norm(P[:,c]),1e-9)) for c in range(3)])
