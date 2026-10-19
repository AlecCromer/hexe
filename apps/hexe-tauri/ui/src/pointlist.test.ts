import { expect, test } from "bun:test";
import { formatPoints, parsePoints, resizePoints } from "./pointlist";

test("Add SOP points text round-trips through the point list", () => {
  expect(parsePoints("0,0,0")).toEqual([[0, 0, 0]]);
  expect(parsePoints("  1,2,3\n-1.5,0,4e2 ")).toEqual([[1, 2, 3], [-1.5, 0, 400]]);
  expect(formatPoints([[1, 2, 3], [-1.5, 0, 400]])).toBe("1,2,3 -1.5,0,400");
  expect(parsePoints(formatPoints([[0.25, -7, 1e-3]]))).toEqual([[0.25, -7, 1e-3]]);
  expect(parsePoints("")).toEqual([]);
});

test("hostile point text is skipped, never invented", () => {
  expect(parsePoints("1,2 a,b,c 1,2,3,4 NaN,0,0 Infinity,0,0 5,6,7")).toEqual([[5, 6, 7]]);
});

test("Number of Points grows with origin rows and shrinks from the end, capped", () => {
  expect(resizePoints([[1, 1, 1]], 3)).toEqual([[1, 1, 1], [0, 0, 0], [0, 0, 0]]);
  expect(resizePoints([[1, 1, 1], [2, 2, 2]], 1)).toEqual([[1, 1, 1]]);
  expect(resizePoints([[1, 1, 1]], -4)).toEqual([]);
  expect(resizePoints([], Number.NaN)).toEqual([]);
  expect(resizePoints([], 1e9, 5).length).toBe(5);
});
