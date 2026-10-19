"""Dump the parm names / types / defaults / menus of the Attribute Adjust SOPs (oracle tooling, run as
`hython dump_parms.py`). Prints one line per parm."""
import hou

geo = hou.node('/obj').createNode('geo')
for t in ('attribadjustfloat', 'attribadjustinteger', 'attribadjustcolor', 'attribadjustvector'):
    try:
        n = geo.createNode(t)
    except hou.OperationFailed as e:
        print('##', t, 'missing', e)
        continue
    print('##', t, n.type().definition().nodeTypeName() if n.type().definition() else 'builtin')
    for p in n.parms():
        pt = p.parmTemplate()
        extra = ''
        if pt.type() == hou.parmTemplateType.Menu:
            extra = ' menu=' + repr(list(zip(pt.menuItems(), pt.menuLabels())))
        try:
            v = p.eval()
        except Exception as e:  # noqa
            v = '<%s>' % e
        cond = ''
        try:
            cond = ' cond=' + repr({str(k): v2 for k, v2 in pt.conditionals().items()})
        except Exception:
            pass
        print(p.name(), pt.type(), repr(v), extra, cond)
