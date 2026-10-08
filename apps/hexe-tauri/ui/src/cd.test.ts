import { expect, test } from "bun:test";
import { cornerColors, cornerCount, cornerGeometry } from "./cd";

const quad = { positions: [0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0, 5, 5, 5], prims: [[0, 1, 2, 3], [1, 2, 4]], closed: [true, true] };

test("corner count and cd shape check", () => {
  expect(cornerCount(quad.prims)).toBe(7);
  expect(cornerColors({ prims: quad.prims, cd: null })).toBeNull();
  expect(cornerColors({ prims: quad.prims, cd: [1, 0, 0] })).toBeNull();
  const cd = Array.from({ length: 21 }, (_, i) => i / 21);
  expect(Array.from(cornerColors({ prims: quad.prims, cd })!)).toHaveLength(21);
});

test("corner geometry duplicates shared points per prim and renumbers consecutively", () => {
  const g = cornerGeometry(quad);
  expect(g.prims).toEqual([[0, 1, 2, 3], [4, 5, 6]]);
  expect(Array.from(g.positions.slice(12, 18))).toEqual([1, 0, 0, 1, 1, 0]); // prim 1 starts at point 1, 2
  expect(g.closed).toEqual([true, true]);
});

test("an out-of-range point opens its prim so triangulate skips it", () => {
  const g = cornerGeometry({ positions: [0, 0, 0, 1, 0, 0, 1, 1, 0], prims: [[0, 1, 9]], closed: [true] });
  expect(g.closed).toEqual([false]);
});
