import { expect, test } from "bun:test";
import { categoryOf, grouped, groupedObj, labelOf, OBJ_CATEGORIES, OBJ_TYPES, objLabelOf, pushHistory, searchTypes } from "./tabcats";

test("known types land in their submenu, unknown ones in Other", () => {
  expect(categoryOf("box")).toBe("Primitive");
  expect(categoryOf("polyextrude")).toBe("Polygon");
  expect(categoryOf("not_a_sop")).toBe("Other");
});

test("labels: curated, else title-cased words", () => {
  expect(labelOf("polyextrude")).toBe("PolyExtrude");
  expect(labelOf("box")).toBe("Box");
  expect(labelOf("fall_from_height")).toBe("Fall From Height");
  expect(labelOf("__")).toBe("");
});

test("grouped keeps menu order, drops empty categories, sorts by label", () => {
  const g = grouped(["sphere", "zzz", "box", "merge"]);
  expect(g.map(([c]) => c)).toEqual(["Primitive", "Utility", "Other"]);
  expect(g[0][1]).toEqual(["box", "sphere"]);
  expect(grouped([])).toEqual([]);
});

test("history: most recent first, dedup, capped", () => {
  expect(pushHistory(["a", "b", "c"], "b")).toEqual(["b", "a", "c"]);
  expect(pushHistory(["a", "b"], "c", 2)).toEqual(["c", "a"]);
  expect(pushHistory([], "a", 0)).toEqual([]);
});

test("/obj menu: every Houdini OBJ category in order, Geometry and Null filed, unknown types dropped", () => {
  const g = groupedObj([...OBJ_TYPES, "not_an_obj"]);
  expect(g.map(([c]) => c)).toEqual([...OBJ_CATEGORIES]);
  expect(g.find(([c]) => c === "Geometry")?.[1]).toEqual(["geo"]);
  expect(g.find(([c]) => c === "Utility")?.[1]).toEqual(["null"]);
  expect(g.find(([c]) => c === "Cameras")?.[1]).toEqual([]);
  expect(objLabelOf("geo")).toBe("Geometry");
});

test("search: type names first, then labels (transform -> xform, geom -> geo), each type once", () => {
  expect(searchTypes(["xform", "box", "transform_x"], "transform")).toEqual(["transform_x", "xform"]);
  expect(searchTypes(["xform", "box"], "box")).toEqual(["box"]);
  expect(searchTypes(OBJ_TYPES, "geom", objLabelOf)).toEqual(["geo"]);
  expect(searchTypes(["polyextrude"], "polyext")).toEqual(["polyextrude"]);
  expect(searchTypes(["box"], "zzz")).toEqual([]);
});
