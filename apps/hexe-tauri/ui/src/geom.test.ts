import { expect, test } from "bun:test";
import { bounds, fitPositions, globMatch, edgeSegments, fanTriangulate, filterTypes, wirePath } from "./geom";

test("fan triangulates quads and skips open/short/out-of-range prims", () => {
  const t = fanTriangulate([[0, 1, 2, 3], [0, 1], [0, 1, 2], [0, 1, 9]], [true, true, false, true], 4);
  expect([...t]).toEqual([0, 1, 2, 0, 2, 3]);
});

test("edge segments close loops, keep open chains open", () => {
  expect([...edgeSegments([[0, 1, 2]], [true], 3)]).toEqual([0, 1, 1, 2, 2, 0]);
  expect([...edgeSegments([[0, 1, 2]], [false], 3)]).toEqual([0, 1, 1, 2]);
  expect([...edgeSegments([[0, 1, 2], [0, 1]], [true, false], 3, true)]).toEqual([0, 1]);
});

test("bounds", () => {
  const b = bounds([0, 0, 0, 2, 4, 4]);
  expect(b?.center).toEqual([1, 2, 2]);
  expect(b?.radius).toBe(3);
  expect(bounds([])).toBeNull();
  expect(bounds([0, 0, NaN])).toBeNull();
});

test("tab menu filter ranks prefix before substring before subsequence", () => {
  expect(filterTypes(["merge", "polyextrude", "extrude", "xform"], "ext")).toEqual(["extrude", "polyextrude"]);
  expect(filterTypes(["box", "merge"], "")).toEqual(["box", "merge"]);
  expect(filterTypes(["polyextrude"], "pex")).toEqual(["polyextrude"]);
});

test("wire path starts at output and ends at input", () => {
  expect(wirePath([10, 20], [30, 120])).toMatch(/^M10,20 C.* 30,120$/);
});

import { newell, shapeWire, triangulate } from "./geom";

test("earcut fills a concave L-shape the fan gets wrong", () => {
  const pos = [0, 0, 0, 2, 0, 0, 2, 1, 0, 1, 1, 0, 1, 2, 0, 0, 2, 0];
  const t = triangulate([[0, 1, 2, 3, 4, 5]], [true], pos);
  expect(t.length).toBe(12);
  let area = 0;
  for (let i = 0; i < t.length; i += 3) area += Math.abs(newell([t[i], t[i + 1], t[i + 2]], pos)[2]) / 2;
  expect(area).toBeCloseTo(3, 6);
});

test("earcut projects on the Newell normal (wall in the XZ-plane, wall in YZ) and falls back on zero area", () => {
  const xz = [0, 0, 0, 1, 0, 0, 1, 0, 1, 0, 0, 1];
  expect(triangulate([[0, 1, 2, 3]], [true], xz).length).toBe(6);
  const yz = [0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1];
  expect(triangulate([[0, 1, 2, 3]], [true], yz).length).toBe(6);
  const flat = [0, 0, 0, 1, 0, 0, 2, 0, 0, 3, 0, 0];
  expect(triangulate([[0, 1, 2, 3]], [true], flat).length).toBe(6);
  expect(triangulate([[0, 1, 9]], [true], yz).length).toBe(0);
});

test("sphere / tube wire proxies: segment counts + transform + centre", () => {
  const id = [1, 0, 0, 0, 1, 0, 0, 0, 1];
  const sp = shapeWire({ kind: "sphere", center: [1, 2, 3], xform: id, taper: 0 }, 8);
  expect(sp.length).toBe(3 * 8 * 2 * 3);
  expect([sp[0], sp[1], sp[2]]).toEqual([2, 2, 3]);
  const tb = shapeWire({ kind: "tube", center: [0, 0, 0], xform: [2, 0, 0, 0, 4, 0, 0, 0, 2], taper: 0.5 }, 8);
  expect(tb.length).toBe((2 * 8 * 2 + 4 * 2) * 3);
  expect(Math.max(...Array.from({ length: tb.length / 3 }, (_, i) => tb[i * 3 + 1]))).toBe(2);
});

import { capCheck, primCentroids, triEstimate, viewState } from "./geom";

test("draw caps skip the over-cap layer with a warning", () => {
  expect(capCheck(10, 10, 10).warnings).toEqual([]);
  const c = capCheck(1_000_001, 2_000_001, 500_001);
  expect([c.mesh, c.edges, c.points]).toEqual([false, false, false]);
  expect(c.warnings.length).toBe(3);
  expect(triEstimate([[0, 1, 2, 3], [0, 1]], [true, false])).toBe(2);
});

test("prim centroids + view-state label", () => {
  const c = primCentroids([[0, 1], []], [0, 0, 0, 2, 4, 6]);
  expect([...c.slice(0, 3)]).toEqual([1, 2, 3]);
  expect(Number.isNaN(c[3])).toBe(true);
  expect(viewState(false, true, true)).toBe("shaded +P +N");
  expect(viewState(true, false, false)).toBe("wire");
});

test("fit excludes prims in groups matching the glob param (the background slab), not hardcoded", () => {
  // prim0 = 2 characters points near origin, prim1 = 100 m slab, prim2 shares point 1 with the slab
  const pos = [0, 0, 0, 1, 2, 0, -50, 0, -50, 50, 0, 50];
  const prims = [[0, 1], [2, 3], [1, 0]];
  const groups = { char_character01: [0, 2], char_background: [1], ground: [] };
  const full = bounds(fitPositions(pos, prims, groups, ""))!;
  expect(full.max[0]).toBe(50);
  const fit = bounds(fitPositions(pos, prims, groups, "*background*"))!;
  expect(fit.min).toEqual([0, 0, 0]);
  expect(fit.max).toEqual([1, 2, 0]);
  expect(fit.radius).toBeLessThan(3);
  // param: another pattern excludes another group; a non-matching pattern changes nothing
  expect(bounds(fitPositions(pos, prims, groups, "char_char*"))!.max[0]).toBe(50);
  expect(bounds(fitPositions(pos, prims, groups, "nomatch"))!.max[0]).toBe(50);
  // comma / space separated list; everything excluded or no groups -> full extent (never an empty fit)
  expect(fitPositions(pos, prims, groups, "*background*, char_character01")).toBe(pos);
  expect(fitPositions(pos, prims, undefined, "*background*")).toBe(pos);
  expect(globMatch("*BACKGROUND*", "char_background")).toBe(true);
  expect(globMatch("char_?", "char_ab")).toBe(false);
  expect(globMatch("a.b", "axb")).toBe(false);
});

import { roundedPath } from "./geom";
test("roundedPath: every corner becomes a curve through its vertex, radius clamped to half of each side", () => {
  const sq = roundedPath([[0, 0], [10, 0], [10, 10], [0, 10]], [2, 2, 2, 2]);
  expect(sq.startsWith("M")).toBe(true);
  expect(sq.endsWith("Z")).toBe(true);
  expect((sq.match(/Q/g) ?? []).length).toBe(4);
  expect(sq).toContain("Q10,0 10,2"); // corner (10, 0): in from (8, 0), out to (10, 2)
  // a radius bigger than the sides allow is clamped: corner points never pass a side's midpoint
  expect(roundedPath([[0, 0], [4, 0], [4, 4], [0, 4]], [50, 50, 50, 50])).toContain("Q4,0 4,2");
  expect(roundedPath([[0, 0], [1, 0]], [1, 1])).toBe(""); // not a polygon
  expect(roundedPath([[0, 0], [Number.NaN, 0], [1, 1]], [1, 1, 1])).toBe("");
});
