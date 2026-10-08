import { expect, test } from "bun:test";
import { LINE_H, PAD_Y, lineCount, lineTop, parseCodeError, spliceText } from "./codeeditor";

test("parseCodeError pulls line:col and the message out of a cook error", () => {
  expect(parseCodeError("wrangle1: wrangle: 2:9: expected an expression, found `;`")).toEqual({ line: 2, col: 9, msg: "expected an expression, found `;`" });
  expect(parseCodeError("3:1: type mismatch: float vs vector")).toEqual({ line: 3, col: 1, msg: "type mismatch: float vs vector" });
  expect(parseCodeError("wrangle1: wrangle: step budget exhausted")).toEqual({ line: null, col: null, msg: "wrangle1: wrangle: step budget exhausted" });
  expect(parseCodeError(null)).toBeNull();
  expect(parseCodeError("")).toBeNull();
});

test("line geometry: the strip sits on the textarea line, minus scroll", () => {
  expect(lineTop(1, 0)).toBe(PAD_Y);
  expect(lineTop(3, 0)).toBe(PAD_Y + 2 * LINE_H);
  expect(lineTop(10, 100)).toBe(PAD_Y + 9 * LINE_H - 100);
  expect(lineCount("")).toBe(1);
  expect(lineCount("a\nb\n")).toBe(3);
});

test("spliceText replaces the selection and returns the caret", () => {
  expect(spliceText("abcdef", 2, 2, "  ")).toEqual({ text: "ab  cdef", caret: 4 });
  expect(spliceText("abcdef", 1, 4, "X")).toEqual({ text: "aXef", caret: 2 });
});
