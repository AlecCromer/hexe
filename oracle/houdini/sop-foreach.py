from _common import *
SOP = "sop-foreach"
NOTE = "add 6 points -> wrangle s@name a/b/c (ptnum%3) -> for-each piece (class point, attrib name) { xform t(0,1,0) } merge (examples/foreach.json)"
SNIP = "if(@ptnum % 3 == 0) s@name = 'a'; else if(@ptnum % 3 == 1) s@name = 'b'; else s@name = 'c';"
SCENE = {"nodes": [{"name": "src", "type": "add", "params": {"points": "0,0,0 1,0,0 2,0,0 3,0,0 4,0,0 5,0,0"}}, {"name": "nm", "type": "wrangle", "params": {"snippet": SNIP}}, {"name": "b", "type": "block_begin", "params": {"method": 0, "class": 0}}, {"name": "x", "type": "xform", "params": {"t": [0, 1, 0]}}, {"name": "e", "type": "block_end"}], "edges": [["src", 0, "nm", 0], ["nm", 0, "b", 0], ["b", 0, "x", 0], ["x", 0, "e", 0]], "display": "e"}
def build(net):
    s = net.createNode("add", "src")
    s.parm("points").set(6)
    for i in range(6):
        s.parm("usept%d" % i).set(1); s.parmTuple("pt%d" % i).set((i, 0, 0))
    w = mk(net, "attribwrangle", "nm", **{"class": "point", "snippet": SNIP})
    b = net.createNode("block_begin", "b"); e = net.createNode("block_end", "e")
    b.parm("method").set("piece"); b.parm("blockpath").set("../e")
    e.parm("itermethod").set("pieces"); e.parm("method").set("merge"); e.parm("class").set("point"); e.parm("attrib").set("name"); e.parm("blockpath").set("../b"); e.parm("templatepath").set("../b")
    x = mk(net, "xform", "x", t=(0, 1, 0))
    w.setInput(0, s); b.setInput(0, w); x.setInput(0, b); e.setInput(0, x); e.setInput(1, w)
    return e
finish(SOP, SCENE, build, NOTE)
