import { expect, test } from "bun:test";
import { PARM_TYPES, freeName, insertSpare, newSpare, removeSpare, retype } from "./parmdialog";
import { spareInfo } from "./params";

const T = (label: string) => PARM_TYPES.find((t) => t.label === label)!;

test("parm dialog: Houdini names (newparameter, newparameter2), types map to spare kinds, unsupported types make nothing", () => {
  expect(freeName("newparameter", new Set())).toBe("newparameter");
  expect(freeName("newparameter", new Set(["newparameter"]))).toBe("newparameter2");
  expect(freeName("newparameter", new Set(["newparameter", "newparameter2"]))).toBe("newparameter3");
  const i = newSpare(T("Integer"), new Set(["newparameter"]), null)!;
  expect([i.name, i.kind, i.size, i.default, i.ui]).toEqual(["newparameter2", "int", 1, 0, "Integer"]);
  const c = newSpare(T("Color"), new Set(), null)!;
  expect([c.kind, c.size, c.default]).toEqual(["float", 3, [1, 1, 1]]);
  expect(newSpare(T("Folder"), new Set(), "x")!.folder).toBeNull();
  expect(newSpare(T("Ramp (Float)"), new Set(), null)).toBeNull();
  expect(newSpare(T("Ordered Menu"), new Set(), null)!.menu.length).toBe(2);
  // every listed type: a kind or explicitly not-yet
  for (const t of PARM_TYPES) expect(t.kind === null || ["float", "int", "toggle", "string", "menu", "folder", "separator", "label"].includes(t.kind)).toBe(true);
});

test("parm dialog: insert after the selection / into a selected folder, remove moves folder members to root, retype keeps identity", () => {
  const f = newSpare(T("Folder"), new Set(), null)!;
  const a = newSpare(T("Float"), new Set(["folder"]), null)!;
  let list = insertSpare([], -1, f);
  list = insertSpare(list, 0, a); // selected = the folder -> inside it
  expect(list.map((p) => p.name)).toEqual(["folder", "newparameter"]);
  expect(list[1].folder).toBe("folder");
  const b = newSpare(T("Toggle"), new Set(["folder", "newparameter"]), null)!;
  list = insertSpare(list, 1, b); // after a member -> same folder
  expect([list[2].name, list[2].folder]).toEqual(["newparameter2", "folder"]);
  list = removeSpare(list, 0);
  expect(list.map((p) => [p.name, p.folder])).toEqual([["newparameter", null], ["newparameter2", null]]);
  expect(removeSpare(list, 9)).toEqual(list);
  const r = retype({ ...list[0], label: "Width", help: "h" }, T("Integer Vector 3"));
  expect([r.name, r.label, r.help, r.kind, r.size, r.default]).toEqual(["newparameter", "Width", "h", "int", 3, [0, 0, 0]]);
});

test("spare params render with the pane widgets: vectors any size, menus with entries", () => {
  const v = spareInfo(newSpare(T("Float Vector 4"), new Set(), null)!);
  expect([v.kind, v.default] as unknown[]).toEqual(["vector3", [0, 0, 0, 0]]);
  const m = spareInfo(newSpare(T("Ordered Menu"), new Set(), null)!);
  expect([m.kind, m.menu.map((e) => e.token)]).toEqual(["menu", ["item1", "item2"]]);
  expect(spareInfo(newSpare(T("Integer"), new Set(), null)!).kind).toBe("int");
});
