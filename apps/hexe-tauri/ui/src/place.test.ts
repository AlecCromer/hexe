import { expect, test } from "bun:test";
import { fmtPos, groundHit, previewSegments } from "./place";

test("box preview: 12 edges of the default unit cube", () => {
  const s = previewSegments("box", { size: [1, 1, 1], scale: 1 });
  expect(s.length).toBe(12 * 6);
  expect(Math.max(...s)).toBeCloseTo(0.5);
  expect(Math.min(...s)).toBeCloseTo(-0.5);
});

test("preview follows params, clamps hostile values", () => {
  expect(Math.max(...previewSegments("box", { size: [4, 2, 2], scale: 1 }))).toBeCloseTo(2);
  const huge = previewSegments("box", { size: [1e30, Number.NaN, -3], scale: 1 });
  expect(huge.every(Number.isFinite)).toBe(true);
  expect(Math.max(...huge)).toBeLessThanOrEqual(5e3);
  expect(previewSegments("grid", { size: [10, 10, 0], rows: 1e9, cols: -5 }).length).toBeLessThanOrEqual(202 * 6);
  expect(previewSegments("nope", {}).length).toBeGreaterThan(0);
});

test("ground hit: y = 0 plane, rejects parallel / behind / non-finite rays", () => {
  expect(groundHit([0, 10, 0], [0, -1, 0])).toEqual([0, 0, 0]);
  expect(groundHit([1, 2, 3], [1, -1, 0])).toEqual([3, 0, 3]);
  expect(groundHit([0, 10, 0], [1, 0, 0])).toBeNull();
  expect(groundHit([0, 10, 0], [0, 1, 0])).toBeNull();
  expect(groundHit([0, Number.NaN, 0], [0, -1, 0])).toBeNull();
});

test("coordinate readout", () => {
  expect(fmtPos([-0.00942159123, 0, 1.5])).toBe("(-0.00942159, 0, 1.5)");
});
