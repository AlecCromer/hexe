from _common import *
SOP = "sop-smooth"
NOTE = "box -> smooth uniform strength 0.5 no boundary pin. Houdini smooth::2.0 is a different model (strength/filterquality); P values expected to differ"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "sm", "type": "smooth", "params": {"strength": 0.5, "filterquality": 2, "constrainboundary": False}}], "edges": [["box1", 0, "sm", 0]], "display": "sm"}
def build(net):
    b = mk(net, "box", "box1"); s = mk(net, "smooth::2.0", "sm", strength=0.5, filterquality=2, contrainedboundary=0)
    s.setInput(0, b)
    return s
finish(SOP, SCENE, build, NOTE)
