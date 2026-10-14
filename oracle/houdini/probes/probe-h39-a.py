# hython probe (oracle tooling): behaviour of groupinvert / groupexpand / attribswap / edgedivide / trail on small inputs. Not product code.
import hou
net = hou.node('/obj').createNode('geo', 'p')
def menu(n, p):
    pt = n.parm(p).parmTemplate()
    return list(zip(pt.menuItems(), pt.menuLabels()))
def dump(tag, n):
    n.cook(force=True)
    print('--', tag, 'err', n.errors(), 'warn', n.warnings())
    g = n.geometry()
    if g is None:
        return
    print('  pts', len(g.points()), 'prims', len(g.prims()))
    print('  P', [tuple(round(c, 3) for c in p.position()) for p in g.points()][:40])
    print('  prims', [[v.point().number() for v in pr.vertices()] for pr in g.prims()][:40])
    print('  pattribs', [a.name() for a in g.pointAttribs()], 'primattribs', [a.name() for a in g.primAttribs()], 'vtx', [a.name() for a in g.vertexAttribs()])
    print('  pgroups', {x.name(): [p.number() for p in x.points()] for x in g.pointGroups()})
    print('  primgroups', {x.name(): [p.number() for p in x.prims()] for x in g.primGroups()})
    print('  edgegroups', {x.name(): [(e.points()[0].number(), e.points()[1].number()) for e in x.edges()] for x in g.edgeGroups()})
box = net.createNode('box', 'box1')
grid = net.createNode('grid', 'grid1'); grid.parm('rows').set(4); grid.parm('cols').set(4)
# groupinvert
print('MENU gc', menu(net.createNode('groupcreate','gc0'),'grouptype'))
g = net.createNode('groupcreate', 'ga'); g.parm('groupname').set('a'); g.parm('grouptype').set('primitive'); g.parm('basegroup').set('0-2'); g.setInput(0, box)
for nm, kw in (('inv-default', dict(group='a', grouptype=1)), ('inv-newname', dict(group='a', grouptype=1, newname='b')), ('inv-pointtype', dict(group='a', grouptype=0)), ('inv-empty', dict(group='', grouptype=1)), ('inv-missing', dict(group='zz', grouptype=1))):
    n = net.createNode('groupinvert', nm)
    for k, v in kw.items():
        n.parm(k).set(v)
    n.setInput(0, g)
    dump(nm, n)
# groupexpand
e0 = net.createNode('groupexpand', 'ge0')
for p in ('grouptype', 'colgrouptype'):
    print('MENU groupexpand', p, menu(e0, p))
gg = net.createNode('groupcreate', 'gg'); gg.parm('groupname').set('s'); gg.parm('grouptype').set('primitive'); gg.parm('basegroup').set('5'); gg.setInput(0, grid)
for nm, kw in (('exp_prim1', dict(group='s', grouptype=1)), ('exp_prim1_edge', dict(group='s', grouptype=1, primshareedge=1)), ('exp_prim2_edge', dict(group='s', grouptype=1, primshareedge=1, numsteps=2)), ('exp_prim2_pt', dict(group='s', grouptype=1, numsteps=2)), ('exp_outname', dict(group='s', grouptype=1, outputgroup='o'))):
    n = net.createNode('groupexpand', nm); n.parm('outputgroup').set('o')
    for k, v in kw.items():
        n.parm(k).set(v)
    n.setInput(0, gg)
    dump(nm, n)
# pt group expand
gp = net.createNode('groupcreate', 'gp'); gp.parm('groupname').set('s'); gp.parm('grouptype').set('point'); gp.parm('basegroup').set('6'); gp.setInput(0, grid)
for nm, kw in (('exp_pt1', dict(group='s', grouptype=0)), ('exp_pt2', dict(group='s', grouptype=0, numsteps=2))):
    n = net.createNode('groupexpand', nm); n.parm('outputgroup').set('o')
    for k, v in kw.items():
        n.parm(k).set(v)
    n.setInput(0, gp)
    dump(nm, n)
# attribswap
sw0 = net.createNode('attribswap', 'sw0')
for p in ('method1', 'class1', 'typeinfo1'):
    print('MENU attribswap', p, menu(sw0, p))
c = net.createNode('color', 'c'); c.parm('colorr').set(1); c.parm('colorg').set(0.5); c.parm('colorb').set(0.25); c.setInput(0, grid)
nm_ = net.createNode('normal', 'nrm'); nm_.parm('type').set(0); nm_.setInput(0, c)
for nm, kw in (('swp_default', dict(srcattribs1='Cd', dstattribs1='N')), ('swp_m1', dict(srcattribs1='Cd', dstattribs1='N', method1=1)), ('swp_m2', dict(srcattribs1='Cd', dstattribs1='N', method1=2)), ('swp_ti0', dict(srcattribs1='Cd', dstattribs1='N', typeinfo1=0))):
    n = net.createNode('attribswap', nm)
    for k, v in kw.items():
        n.parm(k).set(v)
    n.setInput(0, nm_)
    n.cook(force=True)
    g = n.geometry()
    print('--', nm, n.errors(), n.warnings(), [(a.name(), a.size(), str(a.type()).split('.')[-1]) for a in g.pointAttribs()], [(a.name(), a.size()) for a in g.primAttribs()])
    print('  pt0', {a.name(): g.points()[0].attribValue(a) for a in g.pointAttribs() if a.name() != 'P'})
# edgedivide
for nm, src, kw in (('ed_box', box, dict()), ('ed_box3', box, dict(numdivs=3)), ('ed_grid', grid, dict()), ('ed_box_connect', box, dict(connectpoints=1)), ('ed_box_shared', box, dict(sharedpoints=1)), ('ed_box_close', box, dict(closepath=1))):
    n = net.createNode('edgedivide', nm)
    for k, v in kw.items():
        n.parm(k).set(v)
    n.setInput(0, src)
    dump(nm, n)
# edgedivide with an edge group
ge = net.createNode('groupcreate', 'ge'); ge.parm('groupname').set('e'); ge.parm('grouptype').set('edge'); ge.parm('basegroup').set('p0-1'); ge.setInput(0, grid)
try:
    n = net.createNode('edgedivide', 'ed_grp'); n.parm('group').set('e'); n.parm('applytoall').set(0); n.setInput(0, ge); dump('ed_grp', n)
except Exception as ex:
    print('ed-grp EXC', ex)
# trail
tr0 = net.createNode('trail', 'tr0')
for p in ('result', 'surftype', 'velapproximation', 'cache'):
    print('MENU trail', p, menu(tr0, p))
for nm, kw in (('tr_default', dict()), ('tr_r1', dict(result=1)), ('tr_r2', dict(result=2)), ('tr_r3', dict(result=3)), ('tr_r4', dict(result=4)), ('tr_r5', dict(result=5))):
    n = net.createNode('trail', nm)
    for k, v in kw.items():
        n.parm(k).set(v)
    n.setInput(0, box)
    try:
        dump(nm, n)
    except Exception as ex:
        print(nm, 'EXC', ex)
