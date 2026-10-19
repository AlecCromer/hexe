"""Synthetic Copy and Transform SOP cases recorded from real Houdini (hython; oracle tooling, portable: run as
`hython sop_copyxform_cases.py <out-dir>`). Every case starts from a default Box SOP decorated by wrangles with typed
attributes (point N/v/orient/rp(point)/Cd/pscale, vertex vn (normal), prim pn (normal)/dir (vector)/id/nm, groups) and
writes box_in.geo (the shared input) and <case>_houdini.geo (Copy and Transform's output):
  xform:   ncy 4, every transform parm set (t r s shear scale p pr, xOrd rts, rOrd zxy), copy-number attribute + groups;
  group:   ncy 3, prim group `top` only, negative x scale (det < 0: normals flip), custom copyattrib / group prefix;
  ptgroup: ncy 3, point group `low` (sourcegrouptype points), rotate + translate.
"""
import hou, os, re, sys

out = sys.argv[1]


def save(g, path):
    """saveToFile, then blank the machine / user fields Houdini stamps into `info` (fixtures must be portable)."""
    g.saveToFile(path)
    with open(path, encoding='utf-8') as f:
        text = f.read()
    for key in ('hostname', 'artist', 'date'):
        text = re.sub(r'"%s":"[^"]*"' % key, '"%s":""' % key, text)
    with open(path, 'w', encoding='utf-8', newline='\n') as f:
        f.write(text)


os.makedirs(out, exist_ok=True)
geo = hou.node('/obj').createNode('geo')
box = geo.createNode('box')
wp = geo.createNode('attribwrangle')
wp.setInput(0, box)
wp.parm('snippet').set(
    'v@N = normalize(@P) * (1 + @ptnum * 0.25); v@v = set(1, @ptnum * 0.1, 0); '
    'p@orient = quaternion(set(0.3, 0.1 * @ptnum, -0.2)); v@rp = @P * 2; setattribtypeinfo(0, "point", "rp", "point"); '
    'v@Cd = set(@ptnum / 8.0, 0.5, 1); f@pscale = 0.5 + @ptnum; '
    'if (@P.y < 0) setpointgroup(0, "low", @ptnum, 1);'
)
wv = geo.createNode('attribwrangle')
wv.setInput(0, wp)
wv.parm('class').set(3)
wv.parm('snippet').set('v@vn = set(0.2, 1, @vtxnum * 0.1); setattribtypeinfo(0, "vertex", "vn", "normal");')
wr = geo.createNode('attribwrangle')
wr.setInput(0, wv)
wr.parm('class').set(1)
wr.parm('snippet').set(
    'v@pn = set(1, 0.5, @primnum * 0.2); setattribtypeinfo(0, "prim", "pn", "normal"); '
    'v@dir = set(0, 1, @primnum * 0.3); setattribtypeinfo(0, "prim", "dir", "vector"); '
    'i@id = @primnum * 10; s@nm = sprintf("p%d", @primnum); '
    'if (@primnum == 1 || @primnum == 5) setprimgroup(0, "top", @primnum, 1);'
)

cases = {
    'xform': dict(ncy=4, t=(0.5, 0.25, -0.75), r=(15.0, 30.0, 45.0), s=(1.25, 0.8, 1.1), shear=(0.1, 0.2, 0.3),
                  scale=1.05, p=(0.3, -0.2, 0.1), pr=(10.0, 20.0, 30.0), xOrd=3, rOrd=4, docopyattrib=1, newgroups=1),
    'group': dict(ncy=3, sourcegroup='top', t=(0.0, 1.5, 0.0), r=(0.0, 0.0, 20.0), s=(-1.0, 1.0, 1.0),
                  docopyattrib=1, copyattrib='cnum', newgroups=1, newgroupprefix='grp'),
    'ptgroup': dict(ncy=3, sourcegroup='low', sourcegrouptype=2, t=(0.0, 0.0, 2.0), r=(0.0, 90.0, 0.0)),
}
save(wr.geometry(), os.path.join(out, 'box_in.geo'))
for name, parms in cases.items():
    c = geo.createNode('copyxform')
    c.setInput(0, wr)
    for k, v in parms.items():
        c.parmTuple(k).set(v if isinstance(v, tuple) else (v,))
    g = c.geometry()
    if c.warnings() or c.errors():
        raise SystemExit('%s: %s %s' % (name, c.warnings(), c.errors()))
    save(g, os.path.join(out, name + '_houdini.geo'))
    print(name, 'out', len(g.points()), 'pts', len(g.prims()), 'prims')
