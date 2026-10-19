import { expect, test } from "bun:test";
import { parseTool, shelfTypes } from "./shelf";
import { categoryOf } from "./tabcats";

test("tool spec parsing", () => {
  expect(parseTool("Box=box")).toEqual(["Box", "box"]);
  expect(parseTool("Terrain: Hills")).toEqual(["Terrain: Hills", null]);
  expect(parseTool("x=")).toEqual(["x", null]);
});

test("every shelf -> node mapping names a known SOP type (no typos)", () => {
  const unknown = shelfTypes().filter((t) => categoryOf(t) === "Other");
  expect(unknown).toEqual([]);
});
