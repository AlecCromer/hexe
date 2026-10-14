# hython probe (oracle tooling): groupexpand prim/edge modes, edgedivide edge-group variants, trail variants. Not product code.
import hou
net = hou.node('/obj').createNode('geo', 'p')
def dump(tag, n, full=True):
    n.cook(force=True)
    print('--', tag, 'err', n.errors(), 'warn', n.warnings())
    g = n.geometry()
    print('  pts', len(g.points()), 'prims', len(g.prims()), [str(x).split('.')[-1] for x in sorted(set(str(p.type()) for p in g.prims()))])
    if full:
        print('  P', [tuple(round(c, 3) for c in p.position()) for p in g.points()][:40])
        print('  prims', [[v.point().number() for v in pr.vertices()] for pr in g.prims()][:40])
    print('  pattribs', [a.name() for a in g.pointAttribs()], 'primattribs', [a.name() for a in g.primAttribs()])
    print('  pgroups', {x.name(): [p.number() for p in x.points()] for x in g.pointGroups()})
    print('  primgroups', {x.name(): [p.number() for p in x.prims()] for x in g.primGroups()})
    print('  edgegroups', {x.name(): [(e.points()[0].number(), e.points()[1].number()) for e in x.edges()] for x in g.edgeGroups()})
grid = net.createNode('grid', 'grid1'); grid.parm('rows').set(5); grid.parm('cols').set(5)
gp = net.createNode('groupcreate', 'gp'); gp.parm('groupname').set('s'); gp.parm('grouptype').set('primitive'); gp.parm('basegroup').set('5'); gp.setInput(0, grid)
def ge(name, src, kw):
    n = net.createNode('groupexpand', name); n.parm('outputgroup').set('o')
    for k, v in kw.items():
        n.parm(k).set(v)
    n.setInput(0, src)
    dump(name, n, False)
ge('exp_prim_pt1', gp, dict(group='s', grouptype=4))
ge('exp_prim_edge1', gp, dict(group='s', grouptype=4, primshareedge=1))
ge('exp_prim_pt2', gp, dict(group='s', grouptype=4, numsteps=2))
ge('exp_prim_edge2', gp, dict(group='s', grouptype=4, primshareedge=1, numsteps=2))
ge('exp_prim_auto', gp, dict(group='s', grouptype=0))
ge('exp_prim_inplace', gp, dict(group='s', grouptype=4, outputgroup='s'))
ge('exp_prim_steps0', gp, dict(group='s', grouptype=4, numsteps=0))
ge('exp_prim_missing', gp, dict(group='zz', grouptype=4))
ge('exp_prim_empty', gp, dict(group='', grouptype=4))
gq = net.createNode('groupcreate', 'gq'); gq.parm('groupname').set('s'); gq.parm('grouptype').set('point'); gq.parm('basegroup').set('7'); gq.setInput(0, grid)
ge('exp_pt_asprim', gq, dict(group='s', grouptype=4))
ge('exp_pt_pt', gq, dict(group='s', grouptype=3))
ge('exp_pt_edge', gq, dict(group='s', grouptype=2))
# edgedivide on grid 3x3 pts interior edge
g3 = net.createNode('grid', 'g3'); g3.parm('rows').set(3); g3.parm('cols').set(3)
print('g3 pts', [(p.number(), tuple(round(c, 2) for c in p.position())) for p in g3.geometry().points()])
def gen(name, pat, kw):
    e = net.createNode('groupcreate', name + '_g'); e.parm('groupname').set('e'); e.parm('grouptype').set('edge'); e.parm('basegroup').set(pat); e.setInput(0, g3)
    n = net.createNode('edgedivide', name); n.parm('group').set('e')
    for k, v in kw.items():
        n.parm(k).set(v)
    n.setInput(0, e); dump(name, n)
gen('ed_int', 'p1-4', dict(applytoall=0))
gen('ed_int_all', 'p1-4', dict(applytoall=1))
gen('ed_int3', 'p1-4', dict(applytoall=0, numdivs=3))
gen('ed_int_conn', 'p1-4', dict(applytoall=0, connectpoints=1))
gen('ed_int_shared', 'p1-4', dict(applytoall=0, sharedpoints=1))
gen('ed_int_close', 'p1-4', dict(applytoall=0, closepath=1))
gen('ed_two', 'p1-4 p3-4', dict(applytoall=0))
gen('ed_rev', 'p4-1', dict(applytoall=0))
gen('ed_bound', 'p0-1', dict(applytoall=0))
# trail variants
box = net.createNode('box', 'box1')
for nm, kw in (('tr_len1', dict(length=1)), ('tr_len3', dict(length=3)), ('tr_len3_inc2', dict(length=3, inc=2.0)), ('tr_mesh_len3', dict(length=3, result=1)), ('tr_poly_len3', dict(length=3, result=2)), ('tr_mesh_cols', dict(length=2, result=1, surftype=1)), ('tr_mesh_quads', dict(length=2, result=1, surftype=4)), ('tr_mesh_noclose', dict(length=2, result=1, close=0)), ('tr_vel', dict(result=3))):
    n = net.createNode('trail', nm)
    for k, v in kw.items():
        n.parm(k).set(v)
    n.setInput(0, box)
    dump(nm, n)
    g = n.geometry()
    if kw.get('result') == 1:
        pr = g.prims()[0]
        print('  mesh type', pr.type(), getattr(pr, 'numRows', lambda: None)(), getattr(pr, 'numCols', lambda: None)(), 'wrapu', getattr(pr, 'isClosed', lambda: None)())
    if kw.get('result') == 3:
        print('  v0', g.points()[0].attribValue('v'), g.findPointAttrib('v').size())
# trail at frame 5 with moving input
hou.setFrame(5)
xf = net.createNode('xform', 'mv'); xf.parm('tx').setExpression('$F'); xf.setInput(0, box)
n = net.createNode('trail', 'tr_move'); n.parm('length').set(3); n.setInput(0, xf)
dump('tr_move_f5', n)
n2 = net.createNode('trail', 'tr_vel_move'); n2.parm('result').set(3); n2.setInput(0, xf)
n2.cook(force=True); g = n2.geometry(); print('tr_vel_move v', [g.points()[i].attribValue('v') for i in (0, 1)])
hou.setFrame(1)
n2.cook(force=True); g = n2.geometry(); print('tr_vel_move f1 v', [g.points()[i].attribValue('v') for i in (0, 1)])
n3 = net.createNode('trail', 'tr_pres_f1'); n3.parm('length').set(3); n3.setInput(0, xf); dump('tr_move_f1', n3)
