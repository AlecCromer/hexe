from _common import *
SOP = "sop-groupcombine-pts"
NOTE = "box -> point groups pa=0-3 pb=2-5 -> groupcombine (guess) r = !pa or pb... = NOT pa XOR pb -> blast points keep r"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "ga", "type": "groupcreate", "params": {"groupname": "pa", "grouptype": 1, "basegroup": "0-3"}}, {"name": "gb", "type": "groupcreate", "params": {"groupname": "pb", "grouptype": 1, "basegroup": "2-5"}}, {"name": "comb", "type": "groupcombine", "params": {"group1": "r", "negate_a1": 1, "group_a1": "pa", "op_ab1": 3, "group_b1": "pb"}}, {"name": "bl", "type": "blast", "params": {"group": "r", "grouptype": 0, "negate": True}}], "edges": [["box1", 0, "ga", 0], ["ga", 0, "gb", 0], ["gb", 0, "comb", 0], ["comb", 0, "bl", 0]], "display": "bl"}
def build(net):
    b = mk(net, "box", "box1")
    ga = mk(net, "groupcreate", "ga", groupname="pa", grouptype="point", basegroup="0-3"); gb = mk(net, "groupcreate", "gb", groupname="pb", grouptype="point", basegroup="2-5")
    c = mk(net, "groupcombine", "comb", group1="r", negate_a1="on", group_a1="pa", op_ab1="xor", group_b1="pb")
    d = mk(net, "blast", "bl", group="r", grouptype="points", negate=1)
    ga.setInput(0, b); gb.setInput(0, ga); c.setInput(0, gb); d.setInput(0, c)
    return d
finish(SOP, SCENE, build, NOTE)
