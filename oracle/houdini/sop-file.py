from _common import *
SOP = "sop-file"
NOTE = "file read houdini-ship joint_link_n.bgeo (hexe 6/24/8 per ledger)"
BGEO = os.path.join(FIX, "houdini-ship", "joint_link_n", "joint_link_n.bgeo")
SCENE = {"nodes": [{"name": "f", "type": "file", "params": {"file": "../houdini-ship/joint_link_n/joint_link_n.bgeo"}}], "edges": [], "display": "f"}
def build(net):
    return mk(net, "file", "f", file=BGEO)
finish(SOP, SCENE, build, NOTE)
