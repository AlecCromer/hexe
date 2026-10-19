import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { objectMatrix } from "./objxform";

type Case = { parms: Record<string, number | number[]>; m: number[] };
const cases: Case[] = JSON.parse(readFileSync(join(import.meta.dir, "../../../../fixtures/obj-null-xform/cases.json"), "utf8"));

test("Null object transform matches Houdini worldTransform for every xOrd / rOrd and pivots", () => {
  expect(cases.length).toBeGreaterThanOrEqual(55);
  for (const c of cases) {
    const m = objectMatrix(c.parms as never);
    for (let i = 0; i < 16; i++) {
      if (Math.abs(m[i] - c.m[i]) > 1e-5) throw new Error(`${JSON.stringify(c.parms)} [${i}] ${m[i]} vs ${c.m[i]}`);
    }
  }
});

test("hostile object transform input falls back to defaults, never NaN", () => {
  const m = objectMatrix({ t: [Number.NaN, 1, 2], r: [Infinity, 0, 0], scale: Number.NaN, xOrd: 99, rOrd: -3 });
  expect(m.every(Number.isFinite)).toBe(true);
  expect([m[12], m[13], m[14]]).toEqual([0, 1, 2]);
});
