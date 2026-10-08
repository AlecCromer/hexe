import { expect, test } from "bun:test";
import { bounds, edgeSegments, fanTriangulate, filterTypes, wirePath } from "./geom";

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
