"""Does a parm channel whose expression ends in a backslash evaluate (hython; oracle tooling)?
Houdini 20.5.522 prints `eval 6.0`: the trailing `\` is a line continuation (hexe_graph::hscript ignores it too)."""
import hou

n = hou.node('/obj').createNode('geo').createNode('null')
n.addSpareParmTuple(hou.FloatParmTemplate('w', 'w', 1, default_value=(10,)))
x = n.parent().createNode('xform')
x.parm('tx').setExpression('ch("../%s/w")/2 + 1\\' % n.name())
print('eval', x.parm('tx').eval())
