import { expect, test } from "bun:test";
import { clampSel, combine, describe, inBox, nearest, projectAll } from "./pick";

// orthographic-ish identity matrix: ndc = xyz, w = 1 (column-major identity)
const I = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];

test("projectAll maps ndc to viewport px (y flipped) and hides points outside the depth range", () => {
  const s = projectAll([0, 0, 0, 1, 1, 0, -1, -1, 0, 0, 0, 2], I, 200, 100);
  expect([s[0], s[1]]).toEqual([100, 50]);
  expect([s[2], s[3]]).toEqual([200, 0]);
  expect([s[4], s[5]]).toEqual([0, 100]);
  expect(Number.isNaN(s[6])).toBe(true); // |z| > w: clipped
});

test("projectAll rejects points behind the eye (w <= 0)", () => {
  const behind = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0]; // w = z
  const s = projectAll([0, 0, -1], behind, 100, 100);
  expect(Number.isNaN(s[0])).toBe(true);
});

test("nearest: within 8 px, closest wins, NaN ignored, none = -1", () => {
  const s = [10, 10, 20, 10, NaN, NaN, 100, 100];
  expect(nearest(s, 12, 11)).toBe(0);
  expect(nearest(s, 18, 10)).toBe(1);
  expect(nearest(s, 15, 10)).toBe(0); // tie 5/5: lowest index
  expect(nearest(s, 60, 60)).toBe(-1);
  expect(nearest(s, 100, 108)).toBe(3); // exactly 8 px: inside (inclusive)
  expect(nearest(s, 100, 109)).toBe(-1);
});

test("inBox: unordered corners, inclusive, skips invisible", () => {
  const s = [10, 10, 20, 20, NaN, NaN, 30, 5];
  expect(inBox(s, 25, 25, 5, 5)).toEqual([0, 1]);
  expect(inBox(s, 0, 0, 100, 100)).toEqual([0, 1, 3]);
  expect(inBox(s, 50, 50, 60, 60)).toEqual([]);
});

test("combine: replace / union / toggle", () => {
  expect(combine([1, 2], [5], false)).toEqual([5]);
  expect(combine([1, 2], [], false)).toEqual([]);
  expect(combine([1, 2], [5], true)).toEqual([1, 2, 5]);
  expect(combine([1, 2], [2], true)).toEqual([1]);
  expect(combine([1, 2], [2, 3], true)).toEqual([1, 2, 3]);
});

test("describe + clampSel", () => {
  const p = [0, 0, 0, 1, 2, 3];
  expect(describe([], p)).toBe("selection cleared");
  expect(describe([1], p)).toBe("pt 1  P (1.000, 2.000, 3.000)");
  expect(describe([0, 1], p)).toBe("2 points selected: 0 1");
  expect(describe(Array.from({ length: 20 }, (_, i) => i), p)).toContain("20 points selected: 0 1 2");
  expect(describe(Array.from({ length: 20 }, (_, i) => i), p)).toContain("...");
  expect(clampSel([0, 3, 9], 4)).toEqual([0, 3]);
});
