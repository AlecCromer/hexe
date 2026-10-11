from _common import *
SOP = "sop-smooth"
NOTE = "box -> smooth uniform strength 0.5 filterquality 2 no boundary pin. MATCH since H35: hexe implements the fitted Houdini smooth::2.0 model (I + s*diag(M^q)^-1 M^q) x = P, M = I - D^-1 A, q = min(filterquality,5); probes in parity/ref/smooth-probes/"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "sm", "type": "smooth", "params": {"strength": 0.5, "filterquality": 2, "constrainboundary": False}}], "edges": [["box1", 0, "sm", 0]], "display": "sm"}
def build(net):
    b = mk(net, "box", "box1"); s = mk(net, "smooth::2.0", "sm", strength=0.5, filterquality=2, contrainedboundary=0)
    s.setInput(0, b)
    return s
finish(SOP, SCENE, build, NOTE)
