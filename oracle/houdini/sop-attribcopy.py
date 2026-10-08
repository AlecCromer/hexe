from _common import *
SOP = "sop-attribcopy"
NOTE = "target grid 3x3 (no attrs) <- source grid 3x3 with f@v: attribcopy point 'v' by index"
SCENE = {"nodes": [{"name": "g", "type": "grid", "params": {"rows": 3, "cols": 3}}, {"name": "s", "type": "grid", "params": {"rows": 3, "cols": 3}}, {"name": "w", "type": "wrangle", "params": {"snippet": "f@v = @ptnum;"}}, {"name": "ac", "type": "attribcopy", "params": {"class": 0, "attribs": "v"}}], "edges": [["s", 0, "w", 0], ["g", 0, "ac", 0], ["w", 0, "ac", 1]], "display": "ac"}
def build(net):
    g = mk(net, "grid", "g", rows=3, cols=3); s = mk(net, "grid", "s", rows=3, cols=3)
    w = mk(net, "attribwrangle", "w", **{"class": "point", "snippet": "f@v = @ptnum;"})
    a = mk(net, "attribcopy", "ac", srcgrouptype="points", destgrouptype="points", attrib="otherattrib", attribname="v", copyp=0)
    w.setInput(0, s); a.setInput(0, g); a.setInput(1, w)
    return a
finish(SOP, SCENE, build, NOTE)
