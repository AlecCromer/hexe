import { expect, test } from "bun:test";
import { headerLabel, pagesFor, showing, visibleRange } from "./sheetwin";
test("visible range follows scroll, overscans, clamps", () => {
  expect(visibleRange(0, 180, 18, 1000, 0)).toEqual([0, 10]);
  expect(visibleRange(180, 180, 18, 1000, 2)).toEqual([8, 22]);
  expect(visibleRange(1e9, 180, 18, 50)).toEqual([50, 50]);
  expect(visibleRange(0, 180, 18, 3)).toEqual([0, 3]);
  expect(visibleRange(0, 180, 18, 0)).toEqual([0, 0]);
  expect(visibleRange(-50, 180, 18, 100, 0)).toEqual([0, 10]);
});
test("pages cover the range", () => {
  expect(pagesFor(0, 10, 200)).toEqual([0]);
  expect(pagesFor(190, 410, 200)).toEqual([0, 1, 2]);
  expect(pagesFor(200, 400, 200)).toEqual([1]);
  expect(pagesFor(5, 5, 200)).toEqual([]);
});
test("showing N of M caps at 10k", () => {
  expect(showing(8)).toBe("showing 8 of 8");
  expect(showing(12345)).toBe("showing 10000 of 12345");
});

test("header labels: P.x -> P[x], index column blank, plain names kept", () => {
  expect(headerLabel("ptnum", 0)).toBe("");
  expect(headerLabel("P.x", 1)).toBe("P[x]");
  expect(headerLabel("uv.3", 2)).toBe("uv[3]");
  expect(headerLabel("name", 3)).toBe("name");
  expect(headerLabel("a.b.c", 4)).toBe("a.b[c]");
});
