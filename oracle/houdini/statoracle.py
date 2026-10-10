"""Statistical oracle metrics (oracle tooling, not product code). Mirror of xtask/src/oracle.rs `stat` / `stat-disp`; keep tolerances in sync.
kind points: count exact | bbox min/max per axis within BBOX of the Houdini extent (+ABS) | mean nearest-neighbour distance within NN | P std per axis within STD | attribute names exact.
kind disp:   count exact | mean and std of |P - base| within STD | attribute names exact."""
BBOX, NN, STD, ABS = 0.02, 0.05, 0.05, 1e-4


def mean_std(v):
    n = len(v)
    if not n:
        return 0.0, 0.0
    m = sum(v) / n
    return m, (sum((x - m) ** 2 for x in v) / n) ** 0.5


def mean_nn(P):
    """Mean nearest-neighbour distance (exact): x-sorted sweep with pruning."""
    n = len(P)
    if n < 2:
        return 0.0
    S = sorted(P)
    tot = 0.0
    for i, p in enumerate(S):
        best = float("inf")
        for rng in (range(i + 1, n), range(i - 1, -1, -1)):
            for j in rng:
                q = S[j]
                dx = q[0] - p[0]
                if dx * dx >= best:
                    break
                d = dx * dx + (q[1] - p[1]) ** 2 + (q[2] - p[2]) ** 2
                if d < best:
                    best = d
        tot += best ** 0.5
    return tot / n


def rel_ok(h, x, tol):
    return abs(x - h) <= tol * abs(h) + ABS


def names(att):
    return {c: sorted(v) for c, v in att.items()}


def compare(H, X):
    """H, X = houdini.json / hexe summary dicts. Returns (ok, metrics-dict, failures-list)."""
    m, bad = {}, []
    if H["points"] != X["points"]:
        bad.append("count %d != %d" % (X["points"], H["points"]))
    if names(H["attribs"]) != names(X["attribs"]):
        bad.append("attrib names hexe %s != houdini %s" % (names(X["attribs"]), names(H["attribs"])))
    hp, xp = H["P_all"], X["P_all"]
    if len(hp) != len(xp) or not hp:
        bad.append("P count")
        return False, m, bad
    if H.get("stat") == "disp":
        base = H["base_P_all"]
        dh = [sum((a - b) ** 2 for a, b in zip(p, q)) ** 0.5 for p, q in zip(hp, base)]
        dx = [sum((a - b) ** 2 for a, b in zip(p, q)) ** 0.5 for p, q in zip(xp, base)]
        (mh, sh), (mx, sx) = mean_std(dh), mean_std(dx)
        m.update(disp_mean=(mh, mx), disp_std=(sh, sx))
        if not rel_ok(mh, mx, STD):
            bad.append("disp mean %.5f vs %.5f" % (mx, mh))
        if not rel_ok(sh, sx, STD):
            bad.append("disp std %.5f vs %.5f" % (sx, sh))
        return not bad, m, bad
    for ax in range(3):
        ch, cx = [p[ax] for p in hp], [p[ax] for p in xp]
        ext = max(ch) - min(ch)
        for lab, f in (("min", min), ("max", max)):
            if abs(f(cx) - f(ch)) > BBOX * ext + ABS:
                bad.append("bbox %s[%d] %.4f vs %.4f" % (lab, ax, f(cx), f(ch)))
        (_, sh), (_, sx) = mean_std(ch), mean_std(cx)
        m["std" + "xyz"[ax]] = (sh, sx)
        if not rel_ok(sh, sx, STD):
            bad.append("std[%d] %.4f vs %.4f" % (ax, sx, sh))
    nh, nx = mean_nn(hp), mean_nn(xp)
    m["nn"] = (nh, nx)
    if not rel_ok(nh, nx, NN):
        bad.append("nn %.5f vs %.5f" % (nx, nh))
    return not bad, m, bad


def fmt(m):
    """One-line metrics: name houdini/hexe (rel dev %)."""
    return " ".join("%s %.4g/%.4g(%+.1f%%)" % (k, h, x, 100.0 * (x - h) / h if h else 0.0) for k, (h, x) in m.items())
