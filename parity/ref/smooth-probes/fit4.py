import json, numpy as np
exec(open('fit3.py').read().split("for name,g in D.items():")[0])
def solve_a(L,d,P,B,q,s):
    # K on full graph, boundary pinned (Dirichlet elimination)
    n=len(P); K=K_rowdiag(L,d,q); S=np.eye(n)+s*K
    I=~B; X=P.copy()
    rhs=P[I]-S[np.ix_(I,B)]@P[B]
    X[I]=np.linalg.solve(S[np.ix_(I,I)],rhs); return X
def solve_b(L,d,P,B,q,s):
    # boundary rows of L zeroed (Laplacian with pinned rows), then K from that
    n=len(P); L2=L.copy(); L2[B,:]=0
    M=np.diag(1/d)@L2; Mq=mp(M,q); dg=np.diag(Mq).copy(); dg[dg==0]=1
    K=np.diag(1/dg)@Mq; return np.linalg.solve(np.eye(n)+s*K,P)
def solve_c(L,d,P,B,q,s):
    # pinned: interior-only graph (boundary removed) with d unchanged
    n=len(P); I=~B; Lii=L[np.ix_(I,I)]; M=np.diag(1/d[I])@Lii; Mq=mp(M,q); K=np.diag(1/np.diag(Mq))@Mq
    # boundary couples through rhs? treat boundary as constant: shift
    X=P.copy(); X[I]=np.linalg.solve(np.eye(I.sum())+s*K,P[I]); return X
def solve_d(L,d,P,B,q,s):
    # K full graph; boundary rows replaced by 0 (identity row in S) , columns kept
    n=len(P); K=K_rowdiag(L,d,q); K[B,:]=0; return np.linalg.solve(np.eye(n)+s*K,P)
V={'a':solve_a,'b':solve_b,'c':solve_c,'d':solve_d}
for name in ('grid5','grid5n','fan'):
    g=D[name]; A=adj(g); P=np.array(g['src']); d=A.sum(1); L=np.diag(d)-A; B=bnd(g)
    res={k:[0]*5 for k in V}
    for r in g['runs']:
        if r['cb']!=1 or r['q']>5: continue
        q=r['q']; H=np.array(r['P'])
        for k,f in V.items():
            res[k][q-1]=max(res[k][q-1],abs(f(L,d,P,B,q,r['s'])-H).max())
    print(name,{k:['%.1e'%x for x in v] for k,v in res.items()})
