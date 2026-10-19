"""Dump every node of the given Houdini types in a scene with its non-default parms (hython; oracle tooling).
usage: hython hip_parms.py <scene.hip> <type,type,...>"""
import hou, sys
hou.hipFile.load(sys.argv[1], suppress_save_prompt=True, ignore_load_warnings=True)
types = set(sys.argv[2].split(','))
for n in hou.node('/obj').allSubChildren():
    t = n.type().name()
    if t not in types:
        continue
    nd = []
    for p in n.parms():
        try:
            if p.isAtDefault():
                continue
            v = p.evalAsString() if p.parmTemplate().type() in (hou.parmTemplateType.Menu, hou.parmTemplateType.String) else p.eval()
            nd.append('%s=%r%s' % (p.name(), v, ' [expr]' if p.keyframes() or (p.expression() if p.keyframes() else '') else ''))
        except hou.Error:
            pass
    ins = [i.path().split('/')[-1] if i else None for i in n.inputs()]
    print(n.path(), t, 'in', ins)
    print('   ', ' '.join(nd))
