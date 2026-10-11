# smooth::2.0 probes (hython 20.5.487) -> fitted model
probe.py: box/grid5/sphere/fan/grid5n x cb{none,unsharededges} x q{1,2,3,5,10} x s{.25,.5,1,2} -> probes.json (full P). ring.py (cycle N=16/64, single Fourier mode, 1 pt read) -> ring.json. tor.py (periodic quad/tri torus 12x12, q 1..20) -> tor.json. fit*.py = numpy fits.
## Findings
- q clamps at 5 (q=5..20 bit-identical). Houdini parm `contrainedboundary` (sic): 0 none, 1 unsharededges, 2 groupboundary (default).
- Pure Fourier mode on regular meshes: r = 1/(1+nu), nu = s * c_q(mesh) * mu^q, mu = eig(D^-1 L). Power law exact (ring/quad/tri).
- c_q = 1/(M^q)_ii, M = D^-1 L: ring d=2 c=2^q/C(2q,q); c2 = d/(d+1) on triangle-free meshes. => K = diag(M^q)^-1 M^q.
- MODEL: (I + s K) x = P, M = I - D^-1 A (deg = distinct neighbours), pinned rows of M zeroed (D from full graph), K row-scaled so diag=1.
- Candidate table (max|dP| over all probes, cb=0, q=1..5): row-diag-scaled D^-1L 3.8e-5(grid5)/4.6e-5(grid5n)/8.9e-6(sphere)/0..1e-8(box,fan); column-scaled 0.24; L D^-1 forms 0.7. cb=1 (pinned rows zeroed): 8.9e-16 grid5, 4.3e-5..5.7e-5 grid5n; Dirichlet elimination variants 0.1-0.2; boundary-removed graph 1.5.
- Residual 1e-5-level = Houdini's own iterative solve (|(I+sK)H-P|/|P| < 1e-5 per column in its output), not model error (shrinks to 1e-8 on small-coordinate meshes).
- Not probed: groupboundary (cb=2), group parm, method scaledominant/curvaturedominant.
