// TAB menu model (no DOM: unit-tested with `bun test`): SOP type -> submenu category, display labels, recent-pick history.
import { filterTypes } from "./geom";

/** Submenu order of the TAB menu (SOP context). Types not listed in CAT land in "Other" (only shown when non-empty). */
export const CATEGORIES = ["Attribute", "Deform", "Export", "Group", "Import", "Manipulate", "Polygon", "Primitive", "Utility", "UV", "VEX", "Other"] as const;
export type Category = (typeof CATEGORIES)[number];

const CAT: Record<string, Category> = {
  attribcopy: "Attribute", attribcreate: "Attribute", attribdelete: "Attribute", attribnoise: "Attribute", attribpromote: "Attribute",
  attribrandomize: "Attribute", attribswap: "Attribute", attribtransfer: "Attribute", color: "Attribute", measure: "Attribute",
  name: "Attribute", normal: "Attribute", connectivity: "Attribute",
  bend: "Deform", lattice: "Deform", mountain: "Deform", peak: "Deform", pointjitter: "Deform", smooth: "Deform", twist: "Deform",
  rop_geometry: "Export", output: "Export",
  group: "Group", groupcombine: "Group", groupcreate: "Group", groupdelete: "Group", groupexpand: "Group", groupinvert: "Group", grouppromote: "Group",
  file: "Import", motion: "Import",
  xform: "Manipulate", transform: "Manipulate", mirror: "Manipulate", matchsize: "Manipulate", copytopoints: "Manipulate", ray: "Manipulate", sort: "Manipulate",
  polybevel: "Polygon", polyextrude: "Polygon", polyfill: "Polygon", polyframe: "Polygon", polyreduce: "Polygon", polysplit: "Polygon",
  subdivide: "Polygon", divide: "Polygon", edgedivide: "Polygon", facet: "Polygon", fuse: "Polygon", clean: "Polygon", dissolve: "Polygon",
  boolean: "Polygon", clip: "Polygon", knife: "Polygon", reverse: "Polygon", convert: "Polygon", resample: "Polygon", carve: "Polygon",
  revolve: "Polygon", skin: "Polygon", sweep: "Polygon", scatter: "Polygon",
  box: "Primitive", sphere: "Primitive", tube: "Primitive", torus: "Primitive", grid: "Primitive", circle: "Primitive", line: "Primitive",
  curve: "Primitive", platonic: "Primitive", add: "Primitive", bound: "Primitive",
  merge: "Utility", switch: "Utility", null: "Utility", blast: "Utility", delete: "Utility", split: "Utility", block_begin: "Utility", block_end: "Utility",
  uvlayout: "UV", uvproject: "UV", uvunwrap: "UV",
  wrangle: "VEX",
};

const LABEL: Record<string, string> = {
  attribcopy: "Attribute Copy", attribcreate: "Attribute Create", attribdelete: "Attribute Delete", attribnoise: "Attribute Noise",
  attribpromote: "Attribute Promote", attribrandomize: "Attribute Randomize", attribswap: "Attribute Swap", attribtransfer: "Attribute Transfer",
  groupcombine: "Group Combine", groupcreate: "Group Create", groupdelete: "Group Delete", groupexpand: "Group Expand", groupinvert: "Group Invert",
  grouppromote: "Group Promote", polybevel: "PolyBevel", polyextrude: "PolyExtrude", polyfill: "PolyFill", polyframe: "PolyFrame",
  polyreduce: "PolyReduce", polysplit: "PolySplit", edgedivide: "Edge Divide", pointjitter: "Point Jitter", copytopoints: "Copy to Points",
  matchsize: "Match Size", rop_geometry: "ROP Geometry Output", uvlayout: "UV Layout", uvproject: "UV Project", uvunwrap: "UV Unwrap",
  xform: "Transform", wrangle: "Attribute Wrangle", block_begin: "Block Begin", block_end: "Block End", null: "Null",
};

export const categoryOf = (type: string): Category => CAT[type] ?? "Other";

/** Menu label for a type: curated name, else words split on `_` and capitalised (`foo_bar` -> `Foo Bar`). */
export function labelOf(type: string): string {
  return LABEL[type] ?? type.split("_").filter(Boolean).map((w) => w[0].toUpperCase() + w.slice(1)).join(" ");
}

/** Non-empty categories in menu order, each with its types sorted by label. */
export function grouped(types: string[]): [Category, string[]][] {
  const by = new Map<Category, string[]>();
  for (const t of types) {
    const c = categoryOf(t);
    by.set(c, [...(by.get(c) ?? []), t]);
  }
  return CATEGORIES.flatMap((c): [Category, string[]][] => {
    const ts = by.get(c);
    return ts ? [[c, [...ts].sort((a, b) => labelOf(a).localeCompare(labelOf(b)))]] : [];
  });
}

/** History list after picking `type`: most recent first, deduplicated, at most `max` entries. */
export function pushHistory(hist: string[], type: string, max = 8): string[] {
  return [type, ...hist.filter((t) => t !== type)].slice(0, Math.max(0, max));
}

/** Submenu order of the TAB menu at /obj (Houdini's OBJ context, all listed even when hexe has nothing in them yet). */
export const OBJ_CATEGORIES = ["Animation", "Cameras", "Character FX", "Characters", "Dynamics", "Environments", "File IO", "Geometry",
  "Lights", "Managers", "Parenting", "Render", "Scene Graph", "Sound", "Utility"] as const;
export type ObjCategory = (typeof OBJ_CATEGORIES)[number];

/** Object types hexe can make at /obj: kind -> [menu label, submenu]. */
const OBJ: Record<string, [string, ObjCategory]> = { geo: ["Geometry", "Geometry"], null: ["Null", "Utility"] };
export const OBJ_TYPES = Object.keys(OBJ);

export const objLabelOf = (type: string): string => OBJ[type]?.[0] ?? labelOf(type);

/** Every /obj category in menu order (empty ones included), each with its known object types sorted by label. */
export function groupedObj(types: string[]): [ObjCategory, string[]][] {
  return OBJ_CATEGORIES.map((c): [ObjCategory, string[]] => [c, types.filter((t) => OBJ[t]?.[1] === c).sort((a, b) => objLabelOf(a).localeCompare(objLabelOf(b)))]);
}

/** TAB menu search: type-name matches first (ranked by `filterTypes`), then menu-label matches, so `transform` finds
 *  Transform (type `xform`) and `geom` finds Geometry (type `geo`). Each type once. */
export function searchTypes(all: string[], q: string, label: (t: string) => string = labelOf): string[] {
  const byLabel = new Map<string, string>();
  for (const t of all) { const l = label(t).toLowerCase(); if (!byLabel.has(l)) byLabel.set(l, t); }
  const viaLabel = filterTypes([...byLabel.keys()], q).map((l) => byLabel.get(l) as string);
  return [...new Set([...filterTypes(all, q), ...viaLabel])];
}
