"""Houdini Null OBJ world transforms for synthetic parm sets (hython; oracle tooling, portable).
Run: hython null_xform_cases.py <out.json>. Writes [{parms, m}] with m = hou.Matrix4.asTuple() (row-major, row vectors)."""
import hou, json, random, sys

random.seed(7)
n = hou.node('/obj').createNode('null')
cases = [
    {},
    {'t': (1, 2, 3)},
    {'r': (30, 0, 0)}, {'r': (0, 45, 0)}, {'r': (0, 0, 60)},
    {'r': (10, 20, 30)},
    {'s': (2, 3, 4), 'scale': 0.5},
    {'t': (1, -2, 0.5), 'r': (15, -40, 70), 's': (1, 2, 0.5), 'p': (0.3, -1, 2)},
    {'t': (1, -2, 0.5), 'r': (15, -40, 70), 's': (1, 2, 0.5), 'p': (0.3, -1, 2), 'pr': (20, 10, -35)},
]
for xo in range(6):
    for ro in range(6):
        cases.append({'xOrd': xo, 'rOrd': ro, 't': (0.5, -1, 2), 'r': (25, -50, 80), 's': (1.5, 0.5, 2), 'scale': 1.25,
                      'p': (0.2, 0.4, -0.6), 'pr': (-15, 30, 45)})
for _ in range(10):
    cases.append({'xOrd': random.randrange(6), 'rOrd': random.randrange(6),
                  't': tuple(round(random.uniform(-5, 5), 3) for _ in range(3)),
                  'r': tuple(round(random.uniform(-180, 180), 2) for _ in range(3)),
                  's': tuple(round(random.uniform(0.2, 3), 3) for _ in range(3)),
                  'scale': round(random.uniform(0.3, 2), 3),
                  'p': tuple(round(random.uniform(-2, 2), 3) for _ in range(3)),
                  'pr': tuple(round(random.uniform(-90, 90), 2) for _ in range(3))})
out = []
for c in cases:
    for p in n.parms():
        if p.name() in ('xOrd', 'rOrd', 'tx', 'ty', 'tz', 'rx', 'ry', 'rz', 'sx', 'sy', 'sz', 'px', 'py', 'pz', 'prx', 'pry', 'prz', 'scale'):
            p.revertToDefaults()
    for k, v in c.items():
        if isinstance(v, tuple):
            n.parmTuple(k).set(v)
        else:
            n.parm(k).set(v)
    out.append({'parms': c, 'm': list(n.worldTransform().asTuple())})
with open(sys.argv[1], 'w', encoding='utf-8', newline='\n') as f:
    json.dump(out, f, indent=1)
print(len(out), 'cases')
