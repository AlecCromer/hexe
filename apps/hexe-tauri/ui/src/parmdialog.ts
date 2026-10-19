// Edit Parameter Interface (Houdini): Create Parameters (types) | Existing Parameters (tree) | Parameter Description.
// Edits a working copy of the node's spare parameters; Apply / Accept hand it to Rust (`node_spare_set`, undoable).
import type { ParamInfo, SpareKind, SpareParm } from "./api";

/** One entry of Houdini's "By Type" list: the spare kind / size it makes (`null` = not in hexe yet). */
export interface ParmType { label: string; kind: SpareKind | null; size?: number; def?: number | number[] | boolean | string; min?: number; max?: number }
export const PARM_TYPES: ParmType[] = [
  { label: "Angle", kind: "float", def: 0, min: 0, max: 360 },
  { label: "Button", kind: null },
  { label: "Button Strip", kind: null },
  { label: "Color", kind: "float", size: 3, def: [1, 1, 1], min: 0, max: 1 },
  { label: "Color and Alpha", kind: "float", size: 4, def: [1, 1, 1, 1], min: 0, max: 1 },
  { label: "Data", kind: null },
  { label: "Direction Vector", kind: "float", size: 3, def: [0, 0, 0], min: -1, max: 1 },
  { label: "File", kind: "string", def: "" },
  { label: "File - Directory", kind: "string", def: "" },
  { label: "File - Geometry", kind: "string", def: "" },
  { label: "File - Image", kind: "string", def: "" },
  { label: "Float", kind: "float", def: 0, min: 0, max: 10 },
  { label: "Float - Mono", kind: "float", def: 0, min: 0, max: 10 },
  { label: "Float Vector 2", kind: "float", size: 2, def: [0, 0], min: 0, max: 10 },
  { label: "Float Vector 3", kind: "float", size: 3, def: [0, 0, 0], min: 0, max: 10 },
  { label: "Float Vector 4", kind: "float", size: 4, def: [0, 0, 0, 0], min: 0, max: 10 },
  { label: "Folder", kind: "folder" },
  { label: "Geometry Data", kind: null },
  { label: "Hue Circle", kind: null },
  { label: "Icon Strip", kind: null },
  { label: "Integer", kind: "int", def: 0, min: 0, max: 10 },
  { label: "Integer Vector 2", kind: "int", size: 2, def: [0, 0], min: 0, max: 10 },
  { label: "Integer Vector 3", kind: "int", size: 3, def: [0, 0, 0], min: 0, max: 10 },
  { label: "Integer Vector 4", kind: "int", size: 4, def: [0, 0, 0, 0], min: 0, max: 10 },
  { label: "Key-Value Dictionary", kind: null },
  { label: "Label", kind: "label" },
  { label: "Label - Heading", kind: "label" },
  { label: "Label - Message", kind: "label" },
  { label: "Logarithmic Float", kind: "float", def: 1, min: 0.001, max: 1000 },
  { label: "Logarithmic Integer", kind: "int", def: 1, min: 1, max: 1000 },
  { label: "Min/Max Float", kind: "float", size: 2, def: [0, 1], min: 0, max: 1 },
  { label: "Min/Max Integer", kind: "int", size: 2, def: [0, 10], min: 0, max: 10 },
  { label: "Operator List", kind: "string", def: "" },
  { label: "Operator Path", kind: "string", def: "" },
  { label: "Ordered Menu", kind: "menu", def: 0 },
  { label: "RGBA Mask", kind: null },
  { label: "Ramp (Color)", kind: null },
  { label: "Ramp (Float)", kind: null },
  { label: "Separator", kind: "separator" },
  { label: "Spacer", kind: "separator" },
  { label: "String", kind: "string", def: "" },
  { label: "Toggle", kind: "toggle", def: false },
  { label: "UV", kind: "float", size: 2, def: [0, 0], min: 0, max: 1 },
  { label: "UVW", kind: "float", size: 3, def: [0, 0, 0], min: 0, max: 1 },
];

/** Houdini's next free name on `base` (`newparameter`, `newparameter2`, ...) among `taken`. */
export function freeName(base: string, taken: ReadonlySet<string>): string {
  if (!taken.has(base)) return base;
  for (let i = 2; i < 100_000; i++) if (!taken.has(`${base}${i}`)) return `${base}${i}`;
  return `${base}_${Date.now()}`;
}

/** A new spare parameter of type `t`, named clear of `taken`, on folder `folder`. */
export function newSpare(t: ParmType, taken: ReadonlySet<string>, folder: string | null): SpareParm | null {
  if (!t.kind) return null;
  const isFolder = t.kind === "folder";
  const base = isFolder ? "folder" : t.kind === "separator" ? "sepparm" : t.kind === "label" ? "labelparm" : "newparameter";
  const size = t.size ?? 1;
  return {
    name: freeName(base, taken), label: isFolder ? "Folder Name" : t.kind === "separator" ? "Separator" : "Label", kind: t.kind, size,
    default: (t.def ?? null) as SpareParm["default"], value: null, min: t.min ?? null, max: t.max ?? null,
    menu: t.kind === "menu" ? [["item1", "Item 1"], ["item2", "Item 2"]] : [], folder: isFolder ? null : folder, invisible: false, join: false,
    help: "", disable_when: "", hide_when: "", ui: t.label,
  };
}

/** Re-type `p` to `t`, keeping name / label / folder / flags; the default becomes the new type's. */
export function retype(p: SpareParm, t: ParmType): SpareParm {
  const fresh = newSpare(t, new Set(), p.folder);
  if (!fresh) return p;
  return { ...fresh, name: p.name, label: p.label, folder: t.kind === "folder" ? null : p.folder, invisible: p.invisible, join: p.join, help: p.help, disable_when: p.disable_when, hide_when: p.hide_when };
}

/** Insert `n` after the selected entry (`at`, -1 = end); a selected folder takes it inside. Returns the new list. */
export function insertSpare(list: readonly SpareParm[], at: number, n: SpareParm): SpareParm[] {
  const out = [...list];
  const sel = at >= 0 ? out[at] : undefined;
  if (sel && n.kind !== "folder") n.folder = sel.kind === "folder" ? sel.name : sel.folder;
  out.splice(at >= 0 ? at + 1 : out.length, 0, n);
  return out;
}

/** Remove entry `at`; a removed folder's members move to the root. */
export function removeSpare(list: readonly SpareParm[], at: number): SpareParm[] {
  const gone = list[at];
  if (!gone) return [...list];
  return list.filter((_, i) => i !== at).map((p) => (gone.kind === "folder" && p.folder === gone.name ? { ...p, folder: null } : p));
}

const el = <K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> & Record<string, unknown> = {}, ...kids: (Node | string)[]): HTMLElementTagNameMap[K] => {
  const e = document.createElement(tag);
  Object.assign(e, props);
  e.append(...kids);
  return e;
};
const typeOf = (p: SpareParm): ParmType => PARM_TYPES.find((t) => t.label === p.ui) ?? PARM_TYPES.find((t) => t.kind === p.kind && (t.size ?? 1) === p.size) ?? PARM_TYPES[11];

/** Open the dialog for node `path` with its own params `own` (shown greyed) and spare params `spare`. `apply` = Rust. */
export function openParmDialog(path: string, own: ParamInfo[], spare: SpareParm[], apply: (parms: SpareParm[]) => Promise<void>): void {
  document.querySelector(".pd-back")?.remove();
  let list: SpareParm[] = spare.map((p) => ({ ...p, menu: p.menu.map((m) => [m[0], m[1]] as [string, string]) }));
  let sel = list.length ? 0 : -1;
  let typeSel = PARM_TYPES.findIndex((t) => t.label === "Float");
  let showInvisible = false;
  let descTab = "Parameter";
  const back = el("div", { className: "pd-back" });
  const win = el("div", { className: "pd", role: "dialog" });
  back.append(win);
  const close = () => { back.remove(); document.removeEventListener("keydown", key, true); };
  const key = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); close(); } };
  document.addEventListener("keydown", key, true);
  const err = el("div", { className: "pd-err" });
  const taken = () => new Set([...own.map((p) => p.name), ...list.map((p) => p.name)]);

  // --- left: Create Parameters
  const typeFilter = el("input", { type: "text", placeholder: "", spellcheck: false });
  const types = el("div", { className: "pd-list" });
  const paintTypes = () => {
    const q = typeFilter.value.trim().toLowerCase();
    types.replaceChildren(...PARM_TYPES.map((t, i) => {
      if (q && !t.label.toLowerCase().includes(q)) return null;
      const r = el("div", { className: `pd-item${i === typeSel ? " on" : ""}${t.kind ? "" : " off"}`, textContent: t.label, title: t.kind ? "" : "Not in hexe yet" });
      r.dataset.type = t.label;
      r.addEventListener("click", () => { typeSel = i; paintTypes(); });
      r.addEventListener("dblclick", () => { typeSel = i; add(); });
      return r;
    }).filter((r): r is HTMLDivElement => r !== null));
  };
  typeFilter.addEventListener("input", paintTypes);
  const createTabs = el("div", { className: "pd-tabs" }, ...["By Type", "Render Properties", "Node Properties", "From Nodes"].map((t, i) => el("button", { className: i === 0 ? "on" : "", textContent: t, disabled: i !== 0, title: i ? "Not in hexe yet" : "" })));
  const left = el("section", { className: "pd-col" }, el("h3", { textContent: "Create Parameters" }), createTabs, types, el("label", { className: "pd-filter" }, "Filter ", typeFilter));

  // --- middle: Existing Parameters
  const tree = el("div", { className: "pd-list pd-tree" });
  const invis = el("input", { type: "checkbox" });
  invis.addEventListener("change", () => { showInvisible = invis.checked; paintTree(); });
  const treeFilter = el("input", { type: "text", spellcheck: false });
  treeFilter.addEventListener("input", () => paintTree());
  const paintTree = () => {
    const q = treeFilter.value.trim().toLowerCase();
    const hit = (s: string) => !q || s.toLowerCase().includes(q);
    const rows: HTMLElement[] = [el("div", { className: "pd-item pd-root", textContent: "root" })];
    for (const p of own) if (hit(p.label + p.name)) rows.push(el("div", { className: "pd-item pd-own", textContent: `${p.label} (${p.name})`, title: "The node's own parameter (not editable here)" }));
    const row = (p: SpareParm, i: number, depth: number) => {
      if ((p.invisible && !showInvisible) || !hit(p.label + p.name)) return;
      const r = el("div", { className: `pd-item${i === sel ? " on" : ""}${p.invisible ? " invis" : ""}`, textContent: p.kind === "separator" ? `—— (${p.name})` : `${p.label} (${p.name})` });
      r.style.paddingLeft = `${18 + depth * 16}px`;
      r.dataset.spare = p.name;
      r.addEventListener("click", () => { sel = i; paintTree(); paintDesc(); });
      rows.push(r);
    };
    list.forEach((p, i) => {
      if (p.folder) return;
      row(p, i, 0);
      if (p.kind === "folder") list.forEach((c, j) => { if (c.folder === p.name) row(c, j, 1); });
    });
    tree.replaceChildren(...rows);
  };
  const add = () => {
    const t = PARM_TYPES[typeSel];
    const n = t && newSpare(t, taken(), null);
    if (!n) { err.textContent = `${t?.label ?? "?"}: not in hexe yet`; return; }
    err.textContent = "";
    list = insertSpare(list, sel, n);
    sel = list.indexOf(n);
    paintTree();
    paintDesc();
  };
  const remove = () => {
    if (sel < 0) return;
    list = removeSpare(list, sel);
    sel = Math.min(sel, list.length - 1);
    paintTree();
    paintDesc();
  };
  const arrows = el("div", { className: "pd-arrows" },
    Object.assign(el("button", { className: "pd-arrow", title: "Add the selected type to the existing parameters", textContent: "➜" }), { onclick: add }),
    Object.assign(el("button", { className: "pd-arrow back", title: "Remove the selected parameter", textContent: "➜" }), { onclick: remove }));
  const mid = el("section", { className: "pd-col" }, el("h3", { textContent: "Existing Parameters" }), el("label", { className: "pd-check" }, invis, " Show Invisible Parameters"), tree, el("label", { className: "pd-filter" }, "Filter ", treeFilter));

  // --- right: Parameter Description
  const desc = el("div", { className: "pd-desc" });
  const descTabs = el("div", { className: "pd-tabs" });
  const field = (label: string, ctl: HTMLElement) => el("div", { className: "pd-row" }, el("label", { textContent: label }), ctl);
  const text = (v: string, set: (s: string) => void, dis = false) => {
    const i = el("input", { type: "text", value: v, spellcheck: false, disabled: dis });
    i.addEventListener("change", () => { set(i.value); paintTree(); });
    return i;
  };
  const check = (v: boolean, set: (b: boolean) => void) => {
    const c = el("input", { type: "checkbox", checked: v });
    c.addEventListener("change", () => { set(c.checked); paintTree(); });
    return c;
  };
  const nums = (vals: number[], set: (v: number[]) => void, isInt: boolean) => {
    const box = el("div", { className: "pd-nums" });
    const cur = [...vals];
    cur.forEach((x, k) => {
      const i = el("input", { type: "number", value: String(x), step: isInt ? "1" : "any" });
      i.addEventListener("change", () => { const f = Number(i.value); if (Number.isFinite(f)) { cur[k] = isInt ? Math.round(f) : f; set([...cur]); } });
      box.append(i);
    });
    return box;
  };
  const paintDesc = () => {
    const p = sel >= 0 ? list[sel] : undefined;
    const tabs = ["Parameter", "Channels", "Menu", "Import", "Action Button"];
    const usable = (t: string) => t === "Parameter" || (t === "Channels" && !!p && ["float", "int", "toggle", "string", "menu"].includes(p.kind)) || (t === "Menu" && p?.kind === "menu");
    if (!usable(descTab)) descTab = "Parameter";
    descTabs.replaceChildren(...tabs.map((t) => {
      const b = el("button", { className: t === descTab ? "on" : "", textContent: t, disabled: !usable(t), title: usable(t) ? "" : t === "Import" || t === "Action Button" ? "Not in hexe yet" : "" });
      b.addEventListener("click", () => { descTab = t; paintDesc(); });
      return b;
    }));
    if (!p) { desc.replaceChildren(el("div", { className: "pd-hint", textContent: "Select a parameter in Existing Parameters, or add one with the arrow." })); return; }
    const rows: HTMLElement[] = [];
    if (descTab === "Parameter") {
      rows.push(field("Name", text(p.name, (s) => { const old = p.name; p.name = s.trim(); for (const c of list) if (c.folder === old) c.folder = p.name; })));
      rows.push(field("Label", text(p.label, (s) => { p.label = s; })));
      const ty = el("select", {});
      PARM_TYPES.forEach((t) => ty.append(el("option", { value: t.label, textContent: t.label, disabled: !t.kind, selected: t === typeOf(p) })));
      ty.addEventListener("change", () => { const t = PARM_TYPES.find((x) => x.label === ty.value); if (t?.kind) { list[sel] = retype(p, t); paintTree(); paintDesc(); } });
      rows.push(field("Type", ty));
      const size = el("input", { type: "number", min: "1", max: "4", step: "1", value: String(p.size), disabled: !(p.kind === "float" || p.kind === "int") });
      size.addEventListener("change", () => {
        const n = Math.max(1, Math.min(4, Math.round(Number(size.value)) || 1));
        const t = PARM_TYPES.find((x) => x.kind === p.kind && (x.size ?? 1) === n && !x.label.includes("Mono"));
        if (t) { list[sel] = { ...retype(p, t), ui: t.label }; paintTree(); paintDesc(); }
      });
      rows.push(field("Size", size));
      rows.push(field("Callback Script", el("input", { type: "text", disabled: true, title: "Not in hexe yet" })));
      rows.push(el("div", { className: "pd-sub", textContent: "Interface Options" }));
      rows.push(field("", el("label", { className: "pd-check" }, check(p.invisible, (b) => { p.invisible = b; }), " Invisible")));
      rows.push(field("", el("label", { className: "pd-check" }, check(p.join, (b) => { p.join = b; }), " Horizontally Join to Next Parameter")));
      if (p.kind === "float" || p.kind === "int") {
        const lo = el("input", { type: "number", value: String(p.min ?? 0), step: "any" });
        const hi = el("input", { type: "number", value: String(p.max ?? 1), step: "any" });
        const upd = () => { const a = Number(lo.value), b = Number(hi.value); if (Number.isFinite(a) && Number.isFinite(b)) { p.min = a; p.max = b; } };
        lo.addEventListener("change", upd);
        hi.addEventListener("change", upd);
        rows.push(field("Range", el("div", { className: "pd-nums" }, lo, hi)));
      }
      const show = el("select", { disabled: true, title: "Not in hexe yet" }, el("option", { textContent: "Main Dialog Only" }));
      rows.push(field("Show Parm In", show));
      rows.push(field("Disable When", text(p.disable_when, (s) => { p.disable_when = s; })));
      rows.push(field("Hide When", text(p.hide_when, (s) => { p.hide_when = s; })));
      if (p.kind !== "folder") {
        const fsel = el("select", {});
        fsel.append(el("option", { value: "", textContent: "(root)" }));
        for (const f of list.filter((x) => x.kind === "folder")) fsel.append(el("option", { value: f.name, textContent: f.label || f.name, selected: p.folder === f.name }));
        fsel.addEventListener("change", () => { p.folder = fsel.value || null; paintTree(); });
        rows.push(field("Folder", fsel));
      }
      const help = el("textarea", { value: p.help, rows: 3, spellcheck: false });
      help.addEventListener("change", () => { p.help = help.value; });
      rows.push(field("Help", help));
    } else if (descTab === "Channels") {
      const isInt = p.kind === "int" || p.kind === "menu";
      if (p.kind === "toggle") rows.push(field("Default", check(Boolean(p.default), (b) => { p.default = b; })));
      else if (p.kind === "string") rows.push(field("Default", text(String(p.default ?? ""), (s) => { p.default = s; })));
      else {
        const d = Array.isArray(p.default) ? (p.default as number[]) : [Number(p.default ?? 0)];
        rows.push(field("Defaults", nums(d, (v) => { p.default = (v.length === 1 ? v[0] : v) as SpareParm["default"]; }, isInt)));
      }
    } else if (descTab === "Menu") {
      const box = el("div", { className: "pd-menu" });
      const paintMenu = () => {
        box.replaceChildren(el("div", { className: "pd-mrow pd-mhead" }, el("span", { textContent: "Token" }), el("span", { textContent: "Label" })),
          ...p.menu.map((m, i) => el("div", { className: "pd-mrow" }, text(m[0], (s) => { m[0] = s; }), text(m[1], (s) => { m[1] = s; }),
            Object.assign(el("button", { textContent: "−", title: "Remove item" }), { onclick: () => { p.menu.splice(i, 1); paintMenu(); } }))),
          Object.assign(el("button", { textContent: "+", title: "Add item" }), { onclick: () => { const n = p.menu.length + 1; p.menu.push([`item${n}`, `Item ${n}`]); paintMenu(); } }));
      };
      paintMenu();
      rows.push(box);
    }
    desc.replaceChildren(...rows);
  };
  const right = el("section", { className: "pd-col pd-right" }, el("h3", { textContent: "Parameter Description" }), descTabs, desc);

  // --- footer
  const doApply = async (thenClose: boolean) => {
    err.textContent = "";
    try {
      await apply(list.map((p) => ({ ...p })));
      if (thenClose) close();
    } catch (e) { err.textContent = typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e); }
  };
  const foot = el("div", { className: "pd-foot" }, err,
    Object.assign(el("button", { textContent: "Accept" }), { onclick: () => void doApply(true) }),
    Object.assign(el("button", { textContent: "Apply" }), { onclick: () => void doApply(false) }),
    Object.assign(el("button", { textContent: "Discard" }), { onclick: close }));
  win.append(el("div", { className: "pd-head" }, el("b", { textContent: "Editing Node:" }), el("span", { textContent: path })),
    el("div", { className: "pd-body" }, left, arrows, mid, right), foot);
  document.body.append(back);
  paintTypes();
  paintTree();
  paintDesc();
}
