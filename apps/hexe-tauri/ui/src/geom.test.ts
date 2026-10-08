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
