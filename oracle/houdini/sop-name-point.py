from _common import *
SOP = "sop-name-point"
NOTE = "box -> name class point attribname tag, points 0-2 = p, 6 = q (values: oracle/houdini/check-name-values.py)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "nm", "type": "name", "params": {"attribname": "tag", "class": 1, "numnames": 2, "group1": "0-2", "name1": "p", "group2": "6", "name2": "q"}}], "edges": [["box1", 0, "nm", 0]], "display": "nm"}
def build(net):
    b = mk(net, "box", "box1")
    n = mk(net, "name", "nm", attribname="tag")
    n.parm("class").set("point")
    n.parm("numnames").set(2)
    for k, v in dict(group1="0-2", name1="p", group2="6", name2="q").items():
        setp(n, k, v)
    n.setInput(0, b)
    return n
finish(SOP, SCENE, build, NOTE)
