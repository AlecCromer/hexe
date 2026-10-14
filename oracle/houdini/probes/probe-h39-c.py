# hython probe (oracle tooling): attribswap corner cases, edgedivide attribute interpolation + edge direction. Not product code.
import hou
net = hou.node('/obj').createNode('geo', 'p')
def dump(tag, n):
    n.cook(force=True)
    g = n.geometry()
    print('--', tag, 'err', n.errors(), 'warn', n.warnings())
    if g is None:
        return
    print('  pts', len(g.points()), 'prims', len(g.prims()))
    print('  pattribs', [(a.name(), a.size(), str(a.dataType()).split('.')[-1], str(a.type()).split('.')[-1]) for a in g.pointAttribs()], 'prim', [(a.name(), a.size()) for a in g.primAttribs()])
    print('  prims', [[v.point().number() for v in pr.vertices()] for pr in g.prims()][:12])
    for a in g.pointAttribs():
        if a.name() != 'P':
            print('   ', a.name(), [tuple(round(c, 3) for c in (p.attribValue(a) if isinstance(p.attribValue(a), tuple) else (p.attribValue(a),))) for p in g.points()][:14])
    print('  P', [tuple(round(c, 3) for c in p.position()) for p in g.points()][:14])
grid = net.createNode('grid', 'grid1'); grid.parm('rows').set(3); grid.parm('cols').set(3)
c = net.createNode('color', 'c'); c.parm('class').set(2) if c.parm('class') else None
c.setInput(0, grid)
w = net.createNode('attribwrangle', 'w'); w.parm('class').set(2); w.parm('snippet').set('f@h = @P.x; i@k = @ptnum; @Cd = set(@ptnum, 0, 1);'); w.setInput(0, grid)
def sw(name, **kw):
    n = net.createNode('attribswap', name)
    for k, v in kw.items():
        n.parm(k).set(v)
    n.setInput(0, w)
    dump(name, n)
sw('sw_f_i', srcattribs1='h', dstattribs1='k')
sw('sw_missing_src', srcattribs1='nope', dstattribs1='Cd')
sw('sw_missing_dst', srcattribs1='Cd', dstattribs1='nope', method1=1)
sw('sw_missing_dst_swap', srcattribs1='Cd', dstattribs1='nope')
sw('sw_same', srcattribs1='Cd', dstattribs1='Cd')
sw('sw_empty')
sw('sw_P', srcattribs1='Cd', dstattribs1='P')
sw('sw_list', srcattribs1='h k', dstattribs1='k h')
sw('sw_glob', srcattribs1='h*', dstattribs1='k')
sw('sw_prim', srcattribs1='Cd', dstattribs1='Cd', **{'class1': 2})
# edgedivide attribute interpolation
w2 = net.createNode('attribwrangle', 'w2'); w2.parm('class').set(2); w2.parm('snippet').set('@Cd = set(@P.z/5, @P.x/5, 0); f@h = @ptnum; i@k = @ptnum;'); w2.setInput(0, grid)
def ge(name, pat, **kw):
    e = net.createNode('groupcreate', name + '_g'); e.parm('groupname').set('e'); e.parm('grouptype').set('edge'); e.parm('basegroup').set(pat); e.setInput(0, w2)
    n = net.createNode('edgedivide', name); n.parm('group').set('e')
    for k, v in kw.items():
        n.parm(k).set(v)
    n.setInput(0, e); dump(name, n)
ge('ed_attr', 'p1-4', numdivs=3)
ge('ed_rev3', 'p4-1', numdivs=3)
ge('ed_sh3', 'p1-4', numdivs=3, sharedpoints=1)
ge('ed_all_empty', '', applytoall=1)
ge('ed_two_chain', 'p1-4 p4-7', numdivs=2)
ge('ed_nd1', 'p1-4', numdivs=1)
