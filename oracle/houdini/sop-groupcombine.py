from _common import *
SOP = "sop-groupcombine"
NOTE = "box -> groupcreate a=0-2 b=2-4 c=4-5 -> groupcombine r = a sub b or c (left to right) -> blast keep r"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "ga", "type": "groupcreate", "params": {"groupname": "a", "basegroup": "0-2"}}, {"name": "gb", "type": "groupcreate", "params": {"groupname": "b", "basegroup": "2-4"}}, {"name": "gcc", "type": "groupcreate", "params": {"groupname": "c", "basegroup": "4-5"}}, {"name": "comb", "type": "groupcombine", "params": {"group1": "r", "group_a1": "a", "op_ab1": 4, "group_b1": "b", "op_bc1": 1, "group_c1": "c"}}, {"name": "bl", "type": "blast", "params": {"group": "r", "grouptype": 1, "negate": True}}], "edges": [["box1", 0, "ga", 0], ["ga", 0, "gb", 0], ["gb", 0, "gcc", 0], ["gcc", 0, "comb", 0], ["comb", 0, "bl", 0]], "display": "bl"}
def build(net):
    b = mk(net, "box", "box1")
    prev = b
    for nm, pat in (("a", "0-2"), ("b", "2-4"), ("c", "4-5")):
        g = mk(net, "groupcreate", "g" + nm, groupname=nm, grouptype="primitive", basegroup=pat); g.setInput(0, prev); prev = g
    c = mk(net, "groupcombine", "comb", group1="r", group_a1="a", op_ab1="sub", group_b1="b", op_bc1="or", group_c1="c")
    d = mk(net, "blast", "bl", group="r", grouptype="prims", negate=1)
    c.setInput(0, prev); d.setInput(0, c)
    return d
finish(SOP, SCENE, build, NOTE)
