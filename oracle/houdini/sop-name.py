from _common import *
SOP = "sop-name"
NOTE = "box -> groupcreate piece1=prims 0-1 -> name (donamefromgroup, rows 2-3 a / 5 b) prim 'name' (values: oracle/houdini/check-name-values.py)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "g1", "type": "groupcreate", "params": {"groupname": "piece1", "basegroup": "0-1"}}, {"name": "nm", "type": "name", "params": {"donamefromgroup": True, "numnames": 2, "group1": "2-3", "name1": "a", "group2": "5", "name2": "b"}}], "edges": [["box1", 0, "g1", 0], ["g1", 0, "nm", 0]], "display": "nm"}
def build(net):
    b = mk(net, "box", "box1"); g = mk(net, "groupcreate", "g1", groupname="piece1", grouptype="primitive", basegroup="0-1")
    n = mk(net, "name", "nm", donamefromgroup=1)
    n.parm("numnames").set(2)
    for k, v in dict(group1="2-3", name1="a", group2="5", name2="b").items():
        setp(n, k, v)
    g.setInput(0, b); n.setInput(0, g)
    return n
finish(SOP, SCENE, build, NOTE)
