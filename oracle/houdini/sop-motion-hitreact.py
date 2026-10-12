"""sop-motion-hitreact: hexe `motion` mode=hitreact (seed 12, 3 s @ 30 fps, frame 30) vs the gaia-motion HDA `gaia::motion::1.0` (scene=hit_reaction, same seed, frame 30) cooked in real Houdini.
Houdini side = the HDA (it spawns the SAME core CLI `gaia_motion.cli hitreact`), so this compares two consumers of one sim: hexe's .geo reader/skeleton builder vs the HDA's. Needs the gaia-motion checkout (GAIA_MOTION_ROOT, default ~/projects/gaia-motion (main)) + venv python (GAIA_MOTION_PYTHON, default ~/.gaia/cache/gaia-motion/venv/bin/python)."""
from _common import *
SOP = "sop-motion-hitreact"
MROOT = os.environ.get("GAIA_MOTION_ROOT") or os.path.expanduser("~/projects/gaia-motion")
PY = os.environ.get("GAIA_MOTION_PYTHON") or os.path.expanduser("~/.gaia/cache/gaia-motion/venv/bin/python")
SEED, FRAME, FRAMES = 12, 30, 90
NOTE = "gaia::motion::1.0 HDA (scene hit_reaction, seed 12, 90 ticks, mesh off, $F 30) vs hexe motion mode=hitreact (seed 12, duration 3 s, 30 fps, frame 30); same core CLI sim, 24 joints + 23 bone polylines + detail pelvis_z"
SCENE = {"nodes": [{"name": "motion1", "type": "motion", "params": {"mode": 3, "seed": SEED, "impulse_ns": 100.0, "duration_s": FRAMES / 30.0, "fps": 30.0, "frame": FRAME, "motion_root": "$GAIA_MOTION_ROOT"}}], "display": "motion1"}
def build(net):
    hou.hda.installFile(os.path.join(MROOT, "houdini", "otls", "gaia_motion.hda"))
    out = os.path.join(os.path.expanduser("~/.gaia/cache/hexe/scratch"), "oracle-motion-hitreact")
    os.environ["GAIA_MOTION_PYTHON"] = PY
    n = net.createNode("gaia::motion::1.0", "motion1")
    for k, v in dict(scene="hit_reaction", mesh=0, use_baked=0, run_on_cook=1, seed=SEED, frames=FRAMES, out_dir=out, core_root=MROOT).items():
        n.parm(k).set(v)
    hou.setFrame(FRAME)
    return n
finish(SOP, SCENE, build, NOTE)
