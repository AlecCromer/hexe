# hython probe (oracle tooling): dump parm names/defaults of the h39 census-tail SOPs. Not product code.
import hou
net = hou.node('/obj').createNode('geo', 'p')
for t in ['trail', 'attribswap', 'groupexpand', 'groupinvert', 'edgedivide', 'align', 'bulge']:
    try:
        n = net.createNode(t, t + '1')
    except Exception as e:
        print('==', t, 'NOCREATE', e)
        continue
    print('==', t, n.type().name(), 'inputs', n.type().minNumInputs(), n.type().maxNumInputs())
    for p in n.parms():
        try:
            v = p.eval()
        except Exception as e:
            v = 'ERR'
        print('  ', p.name(), repr(v), p.parmTemplate().type().name())
