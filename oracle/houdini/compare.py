#!/usr/bin/env python3
"""Oracle comparator (oracle tooling, not product code).
  compare.py [--hexe-cli PATH] [--live-houdini] [--hython PATH] [--work DIR] [sop-id ...]
steps: (1) hexe-cli cook fixtures/<sop>/scene.json -> DIR/<sop>.geo  (2) hexe-cli oracle summary DIR/<sop>.geo -> DIR/<sop>.json (NO Houdini)  (3) compare vs the RECORDED corpus fixtures/<sop>/houdini.json
LAW: Houdini is a recorded oracle only. hython runs ONLY behind --live-houdini (step 2 = hython summarize_hexe.py, also checks the .geo loads in Houdini); if it is unavailable/fails the row says NO-ORACLE (never HEXE-ERR) and the run continues.
A sop with no recorded houdini.json (or none with points) is NO-ORACLE too (corpus gap).
-> oracle/houdini/results.json + docs/HOUDINI-ORACLE.md
Verdict priority: NO-ORACLE > HOUDINI-ERR > HEXE-ERR > COUNT-DIFF > ATTRIB-DIFF > TOPO-DIFF > WINDING-DIFF (same P order + prim cycles, direction reversed) > ORDER-DIFF (same P multiset, other order) > P-DIFF > MATCH.
Cyclic: prim point lists equal up to rotation of the start vertex (same winding) = MATCH, counted as res.cyclic_prims; reversal is never passed.
Strict: permutation is only CLASSIFIED (ORDER-DIFF), never passed as MATCH. P tolerance 1e-4.
STAT mode (houdini.json `oracle`="stat", see parity/README.md, metrics in statoracle.py): RNG/noise streams are not reproducible, so distributions are compared instead of P:
count exact, bbox min/max per axis +-2 % of the Houdini extent, mean nearest-neighbour distance +-5 %, P std per axis +-5 %, attribute names exact
(kind `disp`: count exact, mean + std of |P - base| +-5 %, attribute names exact). Verdict STAT-PASS | STAT-FAIL, never MATCH."""
import argparse, glob, json, os, subprocess, sys
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import statoracle
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
TOL = 1e-4
ap = argparse.ArgumentParser()
ap.add_argument("--hexe-cli", default=os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.expanduser("~/.gaia/cache/hexe/target-h30")), "debug", "hexe-cli"))
ap.add_argument("--hython", default="/Applications/Houdini/Houdini20.5.487/Frameworks/Houdini.framework/Versions/Current/Resources/bin/hython")
ap.add_argument("--work", default=os.path.expanduser("~/.gaia/cache/hexe/scratch/h31/hexe-out"))
ap.add_argument("--live-houdini", action="store_true", help="summarise hexe output with hython (loads the .geo back into Houdini); default = recorded corpus only, hexe-cli oracle summary")
ap.add_argument("--skip-cook", action="store_true")
ap.add_argument("sops", nargs="*")
a = ap.parse_args()
sops = a.sops or sorted(os.path.basename(p) for p in glob.glob(os.path.join(ROOT, "fixtures", "sop-*")) + glob.glob(os.path.join(ROOT, "fixtures", "x-*")))
os.makedirs(a.work, exist_ok=True)
cook = {}
if not a.skip_cook:
    for s in sops:
        out = os.path.join(a.work, s + ".geo")
        for p in (out, out[:-4] + ".json"):
            if os.path.exists(p):
                os.remove(p)
        r = subprocess.run([a.hexe_cli, "cook", os.path.join(ROOT, "fixtures", s, "scene.json"), "--out", out, "--time", "0"], capture_output=True, text=True, timeout=60)
        cook[s] = (r.returncode, (r.stderr or r.stdout).strip())
    if a.live_houdini:
        subprocess.run(["timeout", "120", a.hython, os.path.join(HERE, "summarize_hexe.py"), a.work], check=False, capture_output=True)
    else:
        for s in sops:
            geo = os.path.join(a.work, s + ".geo")
            if cook[s][0] == 0 and os.path.exists(geo):
                r = subprocess.run([a.hexe_cli, "oracle", "summary", geo], capture_output=True, text=True, timeout=60)
                if r.returncode == 0:
                    open(geo[:-4] + ".json", "w").write(r.stdout)


def close(p, q):
    return all(abs(x - y) <= TOL for x, y in zip(p, q))


def cmp_one(s):
    hj = os.path.join(ROOT, "fixtures", s, "houdini.json")
    H = json.load(open(hj)) if os.path.exists(hj) else {"error": "no houdini.json"}
    res = {"sop": s}
    if "error" in H or "points" not in H:
        if "error" not in H:
            res.update(verdict="NO-ORACLE", detail="no recorded corpus (fixtures/%s/houdini.json)" % s)
        else:
            res.update(verdict="HOUDINI-ERR", detail=H["error"])
        return res
    res["houdini"] = [H["points"], H["prims"], H["vertices"]]
    xj = os.path.join(a.work, s + ".json")
    rc, msg = cook.get(s, (0, ""))
    if rc == 0 and not os.path.exists(xj) and a.live_houdini:
        res.update(verdict="NO-ORACLE", detail="live hython produced no summary (unavailable/license?)")
        return res
    if rc != 0 or not os.path.exists(xj):
        res.update(verdict="HEXE-ERR", detail="cook rc=%s: %s" % (rc, msg[:300]))
        return res
    X = json.load(open(xj))
    if "load_error" in X:  # live-houdini only: Houdini could not load it -> no oracle verdict possible, not a hexe failure
        res.update(verdict="NO-ORACLE", detail="live Houdini could not load hexe .geo: " + X["load_error"])
        return res
    res["hexe"] = [X["points"], X["prims"], X["vertices"]]
    if H.get("oracle") == "stat":
        ok, met, bad = statoracle.compare(H, X)
        res.update(verdict="STAT-PASS" if ok else "STAT-FAIL", p="stat", flags=[], stat=met, detail=(statoracle.fmt(met) + (" | FAIL: " + "; ".join(bad) if bad else "")))
        return res
    flags = []
    if res["houdini"] != res["hexe"]:
        flags.append("COUNT-DIFF")
    an = {c: (sorted(H["attribs"][c]), sorted(X["attribs"][c])) for c in H["attribs"]}
    ad = {c: {"houdini_only": sorted(set(h) - set(x)), "hexe_only": sorted(set(x) - set(h))} for c, (h, x) in an.items() if set(h) != set(x)}
    for gk in ("point_groups", "prim_groups"):
        if sorted(H.get(gk, [])) != sorted(X.get(gk, [])):
            ad[gk] = {"houdini": H.get(gk, []), "hexe": X.get(gk, [])}
    if ad:
        flags.append("ATTRIB-DIFF")
        res["attrib_diff"] = ad
    hp, xp = H["P_all"], X["P_all"]
    pstate = "MATCH"
    if len(hp) != len(xp):
        pstate = "n/a"
    elif not all(close(p, q) for p, q in zip(hp, xp)):
        # permutation test: greedy nearest-unused matching (sorting is fragile under 1e-7 float noise)
        used = [False] * len(xp)
        worst = 0.0
        miss = 0
        for p in hp:
            best, bj = None, -1
            for j, q in enumerate(xp):
                if used[j]:
                    continue
                d = max(abs(x - y) for x, y in zip(p, q))
                if best is None or d < best:
                    best, bj = d, j
                    if d <= TOL:
                        break
            used[bj] = True
            worst = max(worst, best)
            miss += best > TOL
        if miss == 0:
            pstate = "ORDER-DIFF"
        else:
            pstate = "P-DIFF"
            res["p_unmatched_points"] = miss
            res["p_maxdev_nearest"] = round(worst, 6)
    res["p"] = pstate
    if pstate == "MATCH":
        # topology: per prim, same point-id cycle (rotation of the start vertex allowed = MATCH(cyclic), counted in cyclic_prims); a cycle with the same first point run the OTHER way is WINDING-DIFF
        # (hexe is counter-clockwise, Houdini clockwise, see docs/HOUDINI-ORACLE.md); anything else is TOPO-DIFF; also closedness.
        ht, xt = H["prim_pts"], X["prim_pts"]
        bad = wind = cyc = 0
        for u, v in zip(ht, xt):
            if u == v:
                continue
            # MATCH(cyclic): same vertex cycle, same winding, rotated start vertex (Houdini start vertex of split polys is
            # a function of its internal triangulation order, docs/parity/boolean-topo.md) -> not a topology difference
            if len(u) == len(v) and u and v[:1] and u.count(v[0]) == 1 and (lambda k: u[k:] + u[:k] == v)(u.index(v[0])):
                cyc += 1
                continue
            if len(u) == len(v) and u and v == [u[0]] + u[:0:-1]:
                wind += 1
            else:
                bad += 1
        if bad or H.get("prim_closed") != X.get("prim_closed") or len(ht) != len(xt) or H.get("prim_types") != X.get("prim_types"):
            flags.append("TOPO-DIFF")
            res["topo_bad_prims"] = bad
        elif wind:
            flags.append("WINDING-DIFF")
            res["winding_prims"] = wind
        if cyc:
            res["cyclic_prims"] = cyc  # verdict stays MATCH (rule: equal up to rotation, same winding)
    if pstate in ("ORDER-DIFF", "P-DIFF"):
        flags.append(pstate)
    res["flags"] = flags
    res["verdict"] = flags[0] if flags else "MATCH"
    res["bbox"] = {"houdini": [H["bbox_min"], H["bbox_max"]], "hexe": [X["bbox_min"], X["bbox_max"]]}
    res["prim_types"] = {"houdini": H["prim_types"], "hexe": X["prim_types"]}
    res["attribs"] = {"houdini": H["attribs"], "hexe": X["attribs"]}
    return res


results = [cmp_one(s) for s in sops]
json.dump(results, open(os.path.join(HERE, "results.json"), "w"), indent=1)
hist = {}
for r in results:
    hist[r["verdict"]] = hist.get(r["verdict"], 0) + 1
for r in results:
    print(r["sop"], r["verdict"], r.get("flags"), r.get("houdini"), r.get("hexe"), r.get("p"), r.get("detail", "")[:160])
print(hist)
