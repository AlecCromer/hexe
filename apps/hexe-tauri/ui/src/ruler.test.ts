import { expect, test } from "bun:test";
import { rulerTicks } from "./ruler";

test("240 frames over ~1500 px: majors every 24, minors every 6", () => {
  const t = rulerTicks(1, 240, 1500);
  const majors = t.filter(([, m]) => m).map(([f]) => f);
  expect(majors.slice(0, 3)).toEqual([24, 48, 72]);
  expect(t[0]).toEqual([6, false]);
  expect(t.every(([f]) => f >= 1 && f <= 240)).toBe(true);
});

test("hostile ranges yield nothing or a bounded list", () => {
  expect(rulerTicks(5, 5, 100)).toEqual([]);
  expect(rulerTicks(10, 1, 100)).toEqual([]);
  expect(rulerTicks(1, 100, 0)).toEqual([]);
  expect(rulerTicks(1, Number.NaN, 100)).toEqual([]);
  expect(rulerTicks(1, 1e12, 1e6).length).toBeLessThanOrEqual(4096);
});
