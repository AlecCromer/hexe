"""Houdini side of `hexe-cli open <hip> --exprs`: for every `path<TAB>channel` line of argv[2], print Houdini's evaluated
parm value at the file's saved frame, then compare (oracle tooling; hython)."""
import hou, sys
hou.hipFile.load(sys.argv[1], suppress_save_prompt=True, ignore_load_warnings=True)
bad = ok = skipped = 0
for line in open(sys.argv[2], encoding='utf-8'):
    path, chan, val = line.rstrip('\n').split('\t')
    n = hou.node(path); p = n.parm(chan) if n else None
    if p is None:
        print('NOPARM', path, chan); bad += 1; continue
    try:
        hv = p.eval()
        if not isinstance(hv, (int, float, str)):
            print("NONSCALAR", path, chan, type(hv).__name__); continue
    except hou.Error as e:
        hv = 'ERR'
    if val.startswith('ERR'):
        skipped += 1
        print('SKIP', path, chan, '| houdini', repr(hv), '|', val)
        continue
    if isinstance(hv, str):
        same = val == '"%s"' % hv or val == hv
    else:
        try: same = abs(float(val) - float(hv)) <= 1e-6 * max(1.0, abs(float(hv)))
        except ValueError: same = False
    if same: ok += 1
    else:
        bad += 1; print('DIFF', path, chan, 'hexe', val, 'houdini', repr(hv))
print('match', ok, 'diff', bad, 'not-evaluated', skipped)
