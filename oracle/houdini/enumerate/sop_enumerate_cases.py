"""Synthetic Enumerate SOP cases recorded from real Houdini (hython; oracle tooling, portable: run as
`hython sop_enumerate_cases.py <out-dir>`). Builds ONE input (`in.geo`): 10 points on a line, 6 open polys,
prim string `name` (b a b c a b), point string `s` (natural-order words incl. empty and a non-ASCII one), point float `f`,
point int `pid`, vertex string `vn`, existing prim float `fidx` and string `sx`, prim group `sel`, point group `psel`.
Writes `<case>_houdini.geo` per entry of CASES and `cases.json` (case -> parm overrides) for the test.
"""
import hou, json, os, re, sys

out = sys.argv[1]


def save(g, path):
    """saveToFile, then blank the machine / user fields Houdini stamps into `info` (fixtures must be portable).
    The detail `varmap` the Python SOP stamps (a multi-entry string tuple, irrelevant here) is dropped first."""
    g = g.freeze()
    vm = g.findGlobalAttrib('varmap')
    if vm is not None:
        vm.destroy()
    g.saveToFile(path)
    with open(path, encoding='utf-8') as f:
        text = f.read()
    for key in ('hostname', 'artist', 'date'):
        text = re.sub(r'"%s":"[^"]*"' % key, '"%s":""' % key, text)
    with open(path, 'w', encoding='utf-8', newline='\n') as f:
        f.write(text)


os.makedirs(out, exist_ok=True)
geo = hou.node('/obj').createNode('geo')
code = r'''
node=hou.pwd();g=node.geometry()
pts=[g.createPoint() for i in range(10)]
for i,p in enumerate(pts): p.setPosition((float(i),0.0,0.0))
for ids in [(0,1,2),(2,3,4),(5,6),(6,7,0),(1,3),(8,9,2)]:
    pr=g.createPolygon(); pr.setIsClosed(False)
    for i in ids: pr.addVertex(pts[i])
g.addAttrib(hou.attribType.Prim,'name','')
for pr,n in zip(g.prims(),['b','a','b','c','a','b']): pr.setAttribValue('name',n)
g.addAttrib(hou.attribType.Point,'s','')
for p,n in zip(pts,['b9','B','b10','a','','b9','x2','x10','a01','Ã©']): p.setAttribValue('s',n)
g.addAttrib(hou.attribType.Point,'f',0.0)
for p,v in zip(pts,[-0.5,0.5,-1.5,1.5,-0.0,2.0,0.25,7.9,-7.9,0.0]): p.setAttribValue('f',v)
g.addAttrib(hou.attribType.Point,'pid',0)
for p,v in zip(pts,[5,3,5,-1,3,3,7,7,0,5]): p.setAttribValue('pid',v)
g.addAttrib(hou.attribType.Vertex,'vn','')
k=0
for pr in g.prims():
    for v in pr.vertices():
        v.setAttribValue('vn',['u','v','u','w'][k%4]); k+=1
g.addAttrib(hou.attribType.Prim,'fidx',2.5)
g.addAttrib(hou.attribType.Prim,'sx','zz')
sel=g.createPrimGroup('sel')
for i in (1,3,4): sel.add(g.prims()[i])
psel=g.createPointGroup('psel')
for i in (0,2,5,7,9): psel.add(pts[i])
'''
CASES = {
    'default': {},
    'points': {'grouptype': 1},
    'vertices': {'grouptype': 2},
    'prim_group': {'group': 'sel'},
    'point_group_string': {'grouptype': 1, 'group': 'psel', 'attribtype': 1, 'prefix': 'p_'},
    'prim_elements': {'usepieceattrib': 1},
    'prim_pieces_string': {'usepieceattrib': 1, 'piecemode': 1, 'attribtype': 1},
    'prim_pieces_group': {'usepieceattrib': 1, 'piecemode': 1, 'group': 'sel'},
    'point_natural_pieces': {'grouptype': 1, 'usepieceattrib': 1, 'pieceattrib': 's', 'piecemode': 1},
    'point_natural_elements': {'grouptype': 1, 'usepieceattrib': 1, 'pieceattrib': 's'},
    'point_float_pieces': {'grouptype': 1, 'usepieceattrib': 1, 'pieceattrib': 'f', 'piecemode': 1},
    'point_float_elements': {'grouptype': 1, 'usepieceattrib': 1, 'pieceattrib': 'f'},
    'point_int_pieces': {'grouptype': 1, 'usepieceattrib': 1, 'pieceattrib': 'pid', 'piecemode': 1, 'attribname': 'pc'},
    'vertex_pieces': {'grouptype': 2, 'usepieceattrib': 1, 'pieceattrib': 'vn', 'piecemode': 1},
    'vertex_elements_string': {'grouptype': 2, 'usepieceattrib': 1, 'pieceattrib': 'vn', 'attribtype': 1, 'prefix': ''},
    'existing_float': {'group': 'sel', 'attribname': 'fidx'},
    'existing_string_to_int': {'group': 'sel', 'attribname': 'sx'},
    'existing_string': {'group': 'sel', 'attribname': 'sx', 'attribtype': 1},
    'overwrite_piece': {'usepieceattrib': 1, 'attribname': 'name'},
}

py = geo.createNode('python')
py.parm('python').set(code)
if py.geometry() is None:
    raise SystemExit('python SOP: %s' % (py.errors(),))
save(py.geometry(), os.path.join(out, 'in.geo'))
for name, parms in CASES.items():
    n = geo.createNode('enumerate')
    n.setInput(0, py)
    for k, v in parms.items():
        n.parm(k).set(v)
    g = n.geometry()
    if g is None or n.errors():
        raise SystemExit('%s: %s' % (name, n.errors()))
    save(g, os.path.join(out, name + '_houdini.geo'))
    print(name, 'warnings', n.warnings())
with open(os.path.join(out, 'cases.json'), 'w', encoding='utf-8', newline='\n') as f:
    json.dump(CASES, f, indent=1, sort_keys=True)
    f.write('\n')
