import { expect, test } from "bun:test";
import { scrubSpeed, scrubbed } from "./scrub";

const f = (min: number | null, max: number | null) => ({ kind: "float" as const, min, max });
const i = (min: number | null, max: number | null) => ({ kind: "int" as const, min, max });

test("speed = registry range / 200px; defaults without a range", () => {
  expect(scrubSpeed(f(-1, 1))).toBeCloseTo(0.01);
  expect(scrubSpeed(f(0, 100))).toBeCloseTo(0.5);
  expect(scrubSpeed(f(null, null))).toBe(0.01);
  expect(scrubSpeed(f(null, 5))).toBe(0.01);
  expect(scrubSpeed(i(null, null))).toBeCloseTo(1 / 12);
  expect(scrubSpeed(i(0, 1000))).toBeCloseTo(5);
  expect(scrubSpeed(i(0, 4))).toBeCloseTo(1 / 12); // tiny int range: never slower than 1 unit / 12px
});

test("drag maps px -> value; modifiers scale it", () => {
  expect(scrubbed(f(-10, 10), 0, 50)).toBeCloseTo(5);
  expect(scrubbed(f(-10, 10), 0, 50, { fine: true })).toBeCloseTo(0.5);
  expect(scrubbed(f(-10, 10), 0, 5, { coarse: true })).toBeCloseTo(5);
  expect(scrubbed(f(null, null), 1, -30)).toBeCloseTo(0.7);
});

test("ints round, ranges clamp, floats stay clean", () => {
  expect(scrubbed(i(0, 20), 5, 60)).toBe(11);
  expect(scrubbed(i(1, 20), 3, -500)).toBe(1);
  expect(scrubbed(i(1, 20), 3, 5000)).toBe(20);
  expect(scrubbed(f(0, 1), 0.2, 10)).toBe(0.25);
  expect(String(scrubbed(f(null, null), 0.1, 20))).toBe("0.3");
});
