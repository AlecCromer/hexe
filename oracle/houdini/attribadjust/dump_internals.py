"""Dump the internal network of an Attribute Adjust HDA (oracle tooling, run as `hython dump_internals.py <type> <out-file>`):
every child node with its type, inputs and any non-default string parm (VEX snippets, python code)."""
import hou, sys

t, out = sys.argv[1], sys.argv[2]
geo = hou.node('/obj').createNode('geo')
n = geo.createNode(t)
n.syncDelayedDefinition()  # HDA contents load lazily
lines = []


def walk(node, depth):
    for c in node.children():
        ins = [i.name() if i else None for i in c.inputs()]
        lines.append('%s%s [%s] in=%s' % ('  ' * depth, c.name(), c.type().name(), ins))
        for p in c.parms():
            pt = p.parmTemplate()
            if pt.type() == hou.parmTemplateType.String:
                try:
                    raw = p.unexpandedString()
                except hou.OperationFailed:
                    raw = p.expression() if p.keyframes() else ''
                if raw and len(raw) > 0 and not p.isAtDefault():
                    lines.append('%s  %s = <<<\n%s\n>>>' % ('  ' * depth, p.name(), raw))
            elif not p.isAtDefault():
                try:
                    lines.append('%s  %s := %s' % ('  ' * depth, p.name(), p.expression()))
                except hou.OperationFailed:
                    lines.append('%s  %s = %r' % ('  ' * depth, p.name(), p.eval()))
        walk(c, depth + 1)


walk(n, 0)
with open(out, 'w', encoding='utf-8') as f:
    f.write('\n'.join(lines))
