import { expect, test } from "bun:test";
import { channelNames, parmMenuEntries, referenceExprs, relPath } from "./parmmenu";

test("channel names: scalars keep the name, tuples get Houdini's x y z w components", () => {
  expect(channelNames("scale", 1)).toEqual(["scale"]);
  expect(channelNames("t", 3)).toEqual(["tx", "ty", "tz"]);
  expect(channelNames("newparameter", 2)).toEqual(["newparameterx", "newparametery"]);
  expect(channelNames("c", 9)).toEqual(["cx", "cy", "cz", "cw"]);
});

test("relative paths: same node, sibling, other network", () => {
  expect(relPath("/obj/null1", "control", "/obj/null1", "control")).toBe("");
  expect(relPath("/obj/null1", "control", "/obj/null1", "null1")).toBe("../null1");
  expect(relPath("/obj/geo1", "box1", "/obj/null1", "null1")).toBe("../../null1/null1");
});

test("Paste Relative References writes ch(\"../null1/newparameter2\") into Uniform Scale; tuples map component by component", () => {
  const src = { net: "/obj/null1", node: "null1", parm: "newparameter2", chans: ["newparameter2"], value: 3 };
  expect(referenceExprs(src, "/obj/null1", "control", ["scale"])).toEqual([["scale", 'ch("../null1/newparameter2")']]);
  expect(referenceExprs(src, "/obj/null1", "control", ["scale"], true)).toEqual([["scale", 'ch("/obj/null1/null1/newparameter2")']]);
  const v = { net: "/obj/geo1", node: "box1", parm: "size", chans: ["sizex", "sizey", "sizez"], value: [1, 2, 3] };
  expect(referenceExprs(v, "/obj/geo1", "xform1", ["tx", "ty", "tz"])).toEqual([["tx", 'ch("../box1/sizex")'], ["ty", 'ch("../box1/sizey")'], ["tz", 'ch("../box1/sizez")']]);
  // a scalar onto a vector feeds the first component only; same node = a bare channel name
  expect(referenceExprs(src, "/obj/null1", "null1", ["tx", "ty", "tz"])).toEqual([["tx", 'ch("newparameter2")']]);
});

test("right-click menu: Houdini's entries in order, shortcuts, missing handlers grey out", () => {
  const h = { copy: () => {}, pasteRelative: () => {}, lock: () => {} };
  const e = parmMenuEntries(h);
  expect(e.map((x) => (x ? x.label : "---"))).toEqual([
    "Revert to Previous Value", "Channels", "Keyframes", "Expression", "Motion FX", "Reference", "---",
    "Copy Parameter", "Paste Values", "Paste Expressions / Channels", "Paste Relative References", "Paste Other", "Delete Channels",
    "Include in Take", "Lock Parameter", "Unlock Parameter", "Revert to Defaults", "Revert to and Restore Factory Defaults",
    "Make Current Value Default", "Export Parameter to Type Properties", "More",
  ]);
  const by = (l: string) => e.find((x) => x?.label === l)!;
  expect([by("Revert to Previous Value").key, by("Delete Channels").key, by("Revert to Defaults").key, by("Export Parameter to Type Properties").key])
    .toEqual(["Shift+RMB", "Ctrl+Shift+LMB", "Ctrl+MMB", "Alt+MMB"]);
  expect(by("Expression").sub!.map((x) => x.label)).toEqual(["Edit Expression...", "Delete Expression"]);
  expect(by("Paste Other").sub!.map((x) => x.label)).toEqual(["Paste Absolute References"]);
  expect([!!by("Copy Parameter").run, !!by("Paste Values").run, !!by("Lock Parameter").run, !!by("Unlock Parameter").run]).toEqual([true, false, true, false]);
});