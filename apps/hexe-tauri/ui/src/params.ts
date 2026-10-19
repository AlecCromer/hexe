// Param pane generated from the selected node's schema (list_node_types). Edits -> node_set_param -> onChange (re-cook).
import type { Kind, NodeType, NodeView, ParamInfo, PVal, SpareParm } from "./api";
import { type CodeEditor, createCodeEditor, parseCodeError } from "./codeeditor";
import { attachScrub, scrubSpeed } from "./scrub";
import { nodeIcon } from "./shelf";
import { labelOf } from "./tabcats";
import { formatPoints, parsePoints, resizePoints } from "./pointlist";
import { channelNames, clipboard, copyParm, type MenuHandlers, referenceExprs, showParmMenu } from "./parmmenu";

/** value before the last edit of each `node:parm` (Revert to Previous Value) */
const prevValues = new Map<string, PVal | number[]>();

export type SetParam = (id: number, name: string, value: PVal) => Promise<void>;

const el = <K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> & Record<string, unknown> = {}, ...kids: (Node | string)[]): HTMLElementTagNameMap[K] => {
  const e = document.createElement(tag);
  Object.assign(e, props);
  e.append(...kids);
  return e;
};

function stepFor(p: ParamInfo): number {
  if (p.kind === "int" || p.kind === "menu") return 1;
  return scrubSpeed(p) || 0.01; // = registry range / 200 (the same speed the click-drag scrub uses)
}

function numberInput(p: ParamInfo, value: number, commit: (v: number) => void, live: boolean): HTMLElement {
  const wrap = el("div", { className: "pctl" });
  const isInt = p.kind === "int" || p.kind === "menu";
  const num = el("input", { type: "number", value: String(value), step: String(stepFor(p)), name: p.name });
  if (p.kind === "menu") num.min = "0";
  num.dataset.param = p.name;
  const parse = (s: string) => (isInt ? Math.round(Number(s)) : Number(s));
  num.addEventListener("change", () => { const v = parse(num.value); if (Number.isFinite(v)) commit(v); });
  attachScrub(num, p, commit);
  wrap.append(num);
  if (live && p.min !== null && p.max !== null && p.kind !== "menu") {
    const r = el("input", { type: "range", min: String(p.min), max: String(p.max), step: String(stepFor(p)), value: String(value) });
    r.addEventListener("input", () => { num.value = r.value; const v = parse(r.value); if (Number.isFinite(v)) commit(v); });
    num.addEventListener("input", () => { r.value = num.value; });
    wrap.append(r);
  }
  return wrap;
}

function control(p: ParamInfo, value: PVal, commit: (v: PVal) => void): HTMLElement {
  switch (p.kind) {
    case "toggle": {
      const c = el("input", { type: "checkbox", checked: Boolean(value) });
      c.dataset.param = p.name;
      c.addEventListener("change", () => commit(c.checked));
      return el("div", { className: "pctl" }, c);
    }
        case "string": {
          if (p.choices.length) { // e.g. wrangle Run Over: points / prims / vertices / detail
            const opts = p.choices.includes(String(value)) ? p.choices : [String(value), ...p.choices];
            const s = el("select", {});
            opts.forEach((c) => s.append(el("option", { value: c, textContent: c, selected: c === String(value) })));
            s.dataset.param = p.name;
            s.addEventListener("change", () => commit(s.value));
            return el("div", { className: "pctl" }, s);
          }
          const t = el("input", { type: "text", value: String(value), spellcheck: false });
      t.dataset.param = p.name;
      t.addEventListener("change", () => commit(t.value));
      return el("div", { className: "pctl" }, t);
    }
    case "vector3": {
      const v = [...(value as number[])];
      const box = el("div", { className: "pctl v3" });
      v.forEach((x, i) => {
        const n = el("input", { type: "number", value: String(x), step: String(stepFor(p)) });
        n.dataset.param = `${p.name}.${i}`;
        // scrub per component: the other two keep their current values
        attachScrub(n, { kind: "float", min: p.min, max: p.max }, (f) => { v[i] = f; commit([v[0], v[1], v[2]]); });
        n.addEventListener("change", () => { const f = Number(n.value); if (Number.isFinite(f)) { v[i] = f; commit([v[0], v[1], v[2]]); } });
        box.append(n);
      });
      return box;
    }
    case "menu": {
      if (p.menu.length) {
        const s = el("select", {});
        p.menu.forEach((m) => s.append(el("option", { value: String(m.value), textContent: m.label, title: m.token, selected: m.value === Number(value) })));
        s.dataset.param = p.name;
        s.addEventListener("change", () => commit(Number(s.value)));
        return el("div", { className: "pctl" }, s);
      }
      const w = numberInput(p, Number(value), commit, false);
      w.title = "menu index (the engine publishes no entry labels)";
      return w;
    }
    default:
      return numberInput(p, Number(value), commit, true);
  }
}

/** Add SOP point list (Houdini multiparm): Number of Points, then `Point N` = x y z + Weight. The engine has no point
 *  weight yet, so Weight shows 1 and is read-only (said in its tooltip, not faked as editable). */
function pointList(text: string, commit: (v: PVal) => void): HTMLElement[] {
  let pts = parsePoints(text);
  const save = () => commit(formatPoints(pts));
  const count = el("input", { type: "number", min: "0", step: "1", value: String(pts.length) });
  count.dataset.param = "points.count";
  const rows = el("div", { className: "plist" });
  const paint = () => {
    rows.replaceChildren(...pts.map((pt, i) => {
      const box = el("div", { className: "pctl v3" });
      pt.forEach((x, k) => {
        const n = el("input", { type: "number", value: String(x), step: "0.01" });
        n.dataset.param = `points.${i}.${k}`;
        const set = (f: number) => { pts[i][k] = f; save(); };
        attachScrub(n, { kind: "float", min: null, max: null }, set);
        n.addEventListener("change", () => { const f = Number(n.value); if (Number.isFinite(f)) set(f); });
        box.append(n);
      });
      const w = el("input", { type: "number", value: "1", disabled: true, title: "Point weight: hexe has no point weights yet (always 1)" });
      return el("div", { className: "prow" }, el("label", { textContent: `Point ${i}` }), box, el("span", { className: "pw-lab", textContent: "Weight" }), w);
    }));
  };
  count.addEventListener("change", () => { pts = resizePoints(pts, Number(count.value)); count.value = String(pts.length); paint(); save(); });
  paint();
  return [el("div", { className: "prow" }, el("label", { textContent: "Number of Points", title: "points" }), el("div", { className: "pctl" }, count)), rows];
}

/** Editors of the pane currently shown + the last cook failure: a failing node's code editor shows the engine message inline. */
let shown: { node: number; editors: CodeEditor[] } = { node: -1, editors: [] };
let cookErr: { node: number | null; text: string | null } = { node: null, text: null };
function paintCookError(): void {
  const mine = cookErr.node !== null && cookErr.node === shown.node;
  for (const e of shown.editors) e.setError(mine ? parseCodeError(cookErr.text) : null);
}
/** Call after every cook: `node`/`text` = the failing node + message, or null/null when the cook succeeded. */
export function showCookError(node: number | null, text: string | null): void {
  cookErr = { node, text };
  paintCookError();
}

/** Houdini's parameter-pane gear menu (above the parameter list). `null` = a separator; items without `run` are shown
 *  disabled with a "not in hexe yet" tooltip (said, not faked). */
type CogItem = { label: string; run?: () => void; sub?: boolean } | null;
export const COG_ITEMS = [
  "Edit Parameter Interface...", "Edit Rendering Properties...", "Edit Standard Node Properties...", "Edit Deletion Script...", "Edit Comments...", null,
  "Revert Parameter Interface", "Delete All Spare Parameters", "Add Spare Input", null,
  "Add Visualizer", "Attached Visualizers...", null,
  "Legacy Presets", "Revert to Factory Defaults", "Revert to and Restore Factory Defaults", "Override Factory Defaults for This Session", "Type Properties...",
] as const;
const COG_SUB = new Set(["Add Visualizer", "Legacy Presets"]);
let openCog: HTMLElement | null = null;
function closeCog(): void { openCog?.classList.remove("open"); openCog = null; }
if (typeof document !== "undefined") {
  document.addEventListener("mousedown", (e) => { if (openCog && !openCog.contains(e.target as Node)) closeCog(); });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") closeCog(); });
}
function cogMenu(actions: Record<string, () => void>): HTMLElement {
  const items: CogItem[] = COG_ITEMS.map((l) => (l === null ? null : { label: l, run: actions[l], sub: COG_SUB.has(l) }));
  const drop = el("div", { className: "mdrop cogdrop" });
  for (const it of items) {
    if (!it) { drop.append(el("hr", { className: "msep" })); continue; }
    const b = el("button", { className: "mi", textContent: it.label, disabled: !it.run });
    if (it.sub) b.append(el("span", { className: "msub", textContent: "▶" }));
    if (!it.run) b.title = "Not in hexe yet";
    b.dataset.cog = it.label;
    b.addEventListener("click", () => { closeCog(); it.run?.(); });
    drop.append(b);
  }
  const btn = el("button", { className: "pcog", title: "Parameter options" });
  btn.innerHTML = `<svg viewBox="0 0 20 20"><path d="M8.6 1.5h2.8l.4 2.3 1.6.7 1.9-1.4 2 2-1.4 1.9.7 1.6 2.3.4v2.8l-2.3.4-.7 1.6 1.4 1.9-2 2-1.9-1.4-1.6.7-.4 2.3H8.6l-.4-2.3-1.6-.7-1.9 1.4-2-2 1.4-1.9-.7-1.6-2.3-.4V8.6l2.3-.4.7-1.6-1.4-1.9 2-2 1.9 1.4 1.6-.7z"/><circle cx="10" cy="10" r="2.6"/></svg>`;
  const wrap = el("div", { className: "menu pcogmenu" }, btn, drop);
  btn.addEventListener("click", () => {
    const was = openCog === wrap;
    closeCog();
    if (!was) { wrap.classList.add("open"); openCog = wrap; }
  });
  return wrap;
}

/** last tab shown per node type (Houdini keeps the folder you were on) */
const activeTab = new Map<string, string>();

/** Pane extras: spare-parameter value edits and the Edit Parameter Interface dialog (SOP nodes; absent = not offered). */
export interface PaneOpts {
  spareSet?: (name: string, value: PVal | number[]) => Promise<void>;
  editInterface?: () => void;
  /** set (string) / delete (null) a channel expression; absent = no Paste References / Edit Expression */
  setChannel?: (chan: string, expr: string | null) => Promise<void>;
  /** network the node lives in (`/obj/geo1`): Copy Parameter remembers it for relative paths */
  net?: string;
  /** after channel edits: re-read the graph and repaint */
  changed?: () => void;
  /** Lock / Unlock Parameter */
  setLock?: (parm: string, on: boolean) => Promise<void>;
}

/** A spare parameter as a pane `ParamInfo` (value kinds only). Vectors of any size use the vector widget. */
export function spareInfo(s: SpareParm): ParamInfo {
  const kind: Kind = s.kind === "float" || s.kind === "int"
    ? (s.size > 1 ? "vector3" : s.kind)
    : s.kind === "toggle" || s.kind === "string" || s.kind === "menu" ? s.kind : "string";
  return {
    name: s.name, label: s.label || s.name, kind, default: (s.default ?? 0) as PVal, min: s.min, max: s.max,
    menu: s.menu.map(([token, label], value) => ({ value, token, label })), language: null, choices: [], folder: null,
  };
}

export function renderParams(host: HTMLElement, node: NodeView | null, type: NodeType | undefined, set: SetParam, opts: PaneOpts = {}): void {
  host.replaceChildren();
  shown = { node: -1, editors: [] };
  if (!node || !type) {
    host.append(el("div", { className: "hint", textContent: "Select a node in the network to edit its parameters." }));
    return;
  }
  // Houdini header: type icon · Type label · node name (id in the tooltip) · gear menu on the right
  const ico = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  ico.setAttribute("viewBox", "0 0 24 24");
  ico.setAttribute("class", "phead-ico");
  ico.innerHTML = nodeIcon(node.type);
  const spare = (node.spare ?? []).filter((s) => !s.invisible);
  const revert = async () => {
    for (const p of type.params) {
      const cur = node.params[p.name] ?? p.default;
      if (JSON.stringify(cur) === JSON.stringify(p.default)) continue;
      node.params[p.name] = p.default;
      await set(node.id, p.name, p.default).catch(() => {});
    }
    for (const s of node.spare ?? []) {
      if (s.value === null || !opts.spareSet || s.default === null) continue;
      s.value = null;
      await opts.spareSet(s.name, s.default).catch(() => {});
    }
    renderParams(host, node, type, set, opts);
  };
  const actions: Record<string, () => void> = { "Revert to Factory Defaults": () => void revert() };
  if (opts.editInterface) actions["Edit Parameter Interface..."] = opts.editInterface;
  if (opts.editInterface && (node.spare ?? []).length) actions["Delete All Spare Parameters"] = () => deleteAllSpare(node, host, type, set, opts);
  const cog = cogMenu(actions);
  host.append(el("div", { className: "phead", title: `${node.type} · #${node.id}` }, ico, el("span", { className: "phead-ty", textContent: labelOf(node.type) }), el("b", { textContent: node.name }), cog));
  if (!type.params.length && !spare.length) host.append(el("div", { className: "hint", textContent: "No parameters." }));
  shown.node = node.id;
  // Houdini folders: untabbed params first, then a tab bar; only the active tab's rows show. Tabs = the node's own
  // folders, then its spare folders (by label; the same label shares one tab)
  const spareTab = new Map(spare.filter((s) => s.kind === "folder").map((s) => [s.name, s.label || s.name]));
  const tabs = [...new Set([...type.params.map((p) => p.folder), ...spareTab.values()].filter((f): f is string => !!f))];
  const pages = new Map<string, HTMLElement>();
  let bar: HTMLElement | null = null;
  if (tabs.length) {
    const want = activeTab.get(type.type);
    const on = want && tabs.includes(want) ? want : tabs[0];
    const tb = el("div", { className: "pfold" });
    for (const t of tabs) {
      const page = el("div", { className: "ppage" });
      page.dataset.tab = t;
      page.hidden = t !== on;
      pages.set(t, page);
      const b = el("button", { className: t === on ? "pftab on" : "pftab", textContent: t });
      b.dataset.tab = t;
      b.addEventListener("click", () => {
        activeTab.set(type.type, t);
        for (const [k, pg] of pages) pg.hidden = k !== t;
        for (const x of tb.children) x.classList.toggle("on", (x as HTMLElement).dataset.tab === t);
      });
      tb.append(b);
    }
    bar = tb;
  }
  const fail = (label: string) => (e: unknown) => {
    host.querySelector(".err-msg")?.remove();
    host.prepend(el("div", { className: "err-msg", textContent: `${label}: ${typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e)}` }));
  };
  // right-click menu + expression / lock look of a row: `chans` = its channel names, `value` its current value
  const applyExprs = async (pairs: [string, string | null][], label: string) => {
    for (const [c, e] of pairs) await opts.setChannel?.(c, e).catch(fail(label));
    opts.changed?.();
  };
  const wire = (row: HTMLElement, label: string, parm: string, chans: string[], cur: () => PVal | number[], def: PVal | number[], setValue: (v: PVal | number[]) => void, makeDefault?: () => void) => {
    const ex = chans.filter((c) => node.chans?.[c]);
    const er = chans.filter((c) => node.chan_errors?.[c]);
    const locked = (node.locked ?? []).includes(parm);
    if (ex.length) row.classList.add("expr");
    if (er.length) row.classList.add("experr");
    if (locked) {
      row.classList.add("locked");
      for (const i of row.querySelectorAll<HTMLInputElement>("input,select,textarea")) i.disabled = true;
    }
    if (ex.length || er.length || locked) {
      row.title = [locked ? "Locked" : "", ...ex.map((c) => `${c} = ${node.chans?.[c]}`), ...er.map((c) => `${c}: ${node.chan_errors?.[c]}`)].filter(Boolean).join("\n");
    }
    const key = `${node.name}:${parm}`;
    const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
    const put = (v: PVal | number[]) => { prevValues.set(key, cur()); setValue(v); };
    const h = (): MenuHandlers => {
      const value = cur();
      const c = clipboard();
      const shape = c && Array.isArray(c.value) === Array.isArray(value) && (!Array.isArray(value) || (c.value as number[]).length === value.length);
      const can = !locked;
      const prev = prevValues.get(key);
      const srcExprs = c ? chans.flatMap((t, i): [string, string][] => { const e = c.exprs?.[c.chans[i] ?? ""]; return e ? [[t, e]] : []; }) : [];
      return {
        copy: () => copyParm({ net: opts.net ?? "", node: node.name, parm, chans, value, exprs: Object.fromEntries(ex.map((x) => [x, node.chans?.[x] ?? ""])) }),
        revertPrevious: can && prev !== undefined && !same(prev, value) ? () => put(prev) : undefined,
        editExpression: can && opts.setChannel ? () => editExpr(row, label, chans) : undefined,
        deleteExpression: can && ex.length && opts.setChannel ? () => void applyExprs(ex.map((x) => [x, null]), label) : undefined,
        pasteValues: can && c && shape ? () => put(c.value) : undefined,
        pasteExpressions: can && srcExprs.length && opts.setChannel ? () => void applyExprs(srcExprs, label) : undefined,
        pasteRelative: can && c && opts.setChannel ? () => void applyExprs(referenceExprs(c, opts.net ?? "", node.name, chans), label) : undefined,
        pasteAbsolute: can && c && opts.setChannel ? () => void applyExprs(referenceExprs(c, opts.net ?? "", node.name, chans, true), label) : undefined,
        deleteChannels: can && ex.length && opts.setChannel ? () => void applyExprs(ex.map((x) => [x, null]), label) : undefined,
        lock: !locked && opts.setLock ? () => void opts.setLock?.(parm, true).then(() => opts.changed?.(), fail(label)) : undefined,
        unlock: locked && opts.setLock ? () => void opts.setLock?.(parm, false).then(() => opts.changed?.(), fail(label)) : undefined,
        revertDefaults: can && !same(value, def) ? () => put(def) : undefined,
        makeDefault: can && makeDefault && !same(value, def) ? makeDefault : undefined,
      };
    };
    row.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      if (e.shiftKey) { h().revertPrevious?.(); return; } // Shift+RMB
      showParmMenu(e.clientX, e.clientY, h());
    });
    // Houdini mouse shortcuts: Ctrl+MMB = Revert to Defaults, Ctrl+Shift+LMB = Delete Channels
    row.addEventListener("mousedown", (e) => {
      if (e.button === 1 && e.ctrlKey && !e.shiftKey) { e.preventDefault(); h().revertDefaults?.(); }
      else if (e.button === 0 && e.ctrlKey && e.shiftKey) { e.preventDefault(); h().deleteChannels?.(); }
    });
  };
  /** Edit Expression: one text field per channel in place of the row's widget; Enter applies (empty = delete). */
  const editExpr = (row: HTMLElement, label: string, chans: string[]) => {
    const box = el("div", { className: "pctl pexpr" });
    const ins = chans.map((c) => {
      const i = el("input", { type: "text", value: node.chans?.[c] ?? "", placeholder: c, spellcheck: false });
      i.dataset.expr = c;
      box.append(i);
      return i;
    });
    row.querySelector(".pctl")?.replaceWith(box);
    ins[0]?.focus();
    const go = () => void applyExprs(ins.map((i, k) => [chans[k], i.value.trim() || null]), label);
    for (const i of ins) i.addEventListener("keydown", (e) => { if (e.key === "Enter") go(); if (e.key === "Escape") opts.changed?.(); });
  };  for (const p of type.params) {
    const target = (p.folder && pages.get(p.folder)) || host;
    const value = node.params[p.name] ?? p.default;
    const code = !!p.language && p.kind === "string";
    const row = el("div", { className: code ? "prow code" : "prow" }, el("label", { textContent: code ? `${p.label} (${p.language})` : p.label, title: p.name }));
    const commit = (v: PVal) => {
      prevValues.set(`${node.name}:${p.name}`, node.params[p.name] ?? p.default);
      node.params[p.name] = v;
      set(node.id, p.name, v).then(() => host.querySelector(".err-msg")?.remove(), fail(p.label));
    };
    if (node.type === "add" && p.name === "points") {
      target.append(...pointList(String(value), commit));
      continue;
    }
    if (code) {
      const ed = createCodeEditor(String(value), (t) => commit(t));
      shown.editors.push(ed);
      row.append(ed.el);
    } else row.append(control(p, value, commit));
    wire(row, p.label, p.name, channelNames(p.name, p.kind === "vector3" ? 3 : 1), () => node.params[p.name] ?? p.default, p.default, (v) => { commit(v as PVal); opts.changed?.(); });
    target.append(row);
  }
  // spare parameters: on their folder's tab, else after the node's own (untabbed ones before the tab bar, like Houdini's
  // root level)
  const tail: HTMLElement[] = [];
  for (const s of spare) {
    if (s.kind === "folder") continue;
    const tab = s.folder ? spareTab.get(s.folder) : undefined;
    const put = (e: HTMLElement) => { const pg = tab ? pages.get(tab) : undefined; if (pg) pg.append(e); else tail.push(e); };
    if (s.kind === "separator") { put(el("hr", { className: "psep" })); continue; }
    if (s.kind === "label") { put(el("div", { className: "prow plabel" }, el("label", { textContent: "" }), el("div", { className: "pctl", textContent: s.label }))); continue; }
    const info = spareInfo(s);
    const row = el("div", { className: "prow spare", title: s.help || s.name }, el("label", { textContent: info.label, title: s.name }));
    const commit = (v: PVal) => {
      prevValues.set(`${node.name}:${s.name}`, (s.value ?? s.default ?? 0) as PVal);
      s.value = v as SpareParm["value"];
      opts.spareSet?.(s.name, v).then(() => host.querySelector(".err-msg")?.remove(), fail(info.label));
    };
    const ctl = control(info, (s.value ?? s.default ?? 0) as PVal, commit);
    for (const i of ctl.querySelectorAll<HTMLElement>("[data-param]")) i.dataset.param = `spare.${i.dataset.param}`;
    if (!opts.spareSet) for (const i of ctl.querySelectorAll<HTMLInputElement>("input,select")) i.disabled = true;
    row.append(ctl);
    // Make Current Value Default: the spare interface again, with this value as its default
    const makeDefault = () => void spareInterfaceHook?.(node.id, (node.spare ?? []).map((q) => (q.name === s.name ? { ...q, default: q.value ?? q.default, value: null } : q))).then(() => opts.changed?.());
    wire(row, info.label, s.name, channelNames(s.name, s.kind === "float" || s.kind === "int" ? s.size : 1), () => (s.value ?? s.default ?? 0) as PVal, (s.default ?? 0) as PVal, (v) => { commit(v as PVal); opts.changed?.(); }, makeDefault);
    put(row);
  }
  host.append(...tail);
  if (bar) host.append(bar, ...pages.values());
  paintCookError();
}

/** Cog > Delete All Spare Parameters: the interface emptied through the dialog's own hook (undoable in Rust). */
function deleteAllSpare(node: NodeView, host: HTMLElement, type: NodeType, set: SetParam, opts: PaneOpts): void {
  void spareInterfaceHook?.(node.id, []).then(() => { node.spare = []; renderParams(host, node, type, set, opts); });
}
/** Set by main.ts: replace a node's spare interface (`api.nodeSpareSet`). */
export let spareInterfaceHook: ((id: number, parms: SpareParm[]) => Promise<void>) | null = null;
export function setSpareInterfaceHook(f: (id: number, parms: SpareParm[]) => Promise<void>): void { spareInterfaceHook = f; }

/** Placement-style params the viewport handle owns in Houdini; the operator bar leaves them to the pane. */
const OPBAR_SKIP = new Set(["t", "r", "center", "origin"]);
/** Viewport operator bar (Houdini: `Box  Primitive Type [..]  Size [..]` above the viewport): the selected node's first few
 *  menu / toggle / number / vector params, same widgets as the pane but without `data-param` (the pane stays the
 *  scriptable one). `changed` runs after each commit so the pane can repaint. */
export function renderOpBar(host: HTMLElement, node: NodeView | null, type: NodeType | undefined, set: SetParam, changed: () => void, max = 5): void {
  host.replaceChildren();
  if (!node || !type) return;
  host.append(el("span", { className: "op-name", textContent: labelOf(type.type) }));
  const ps = type.params.filter((p) => !OPBAR_SKIP.has(p.name) && !p.language && (p.kind !== "string" || p.choices.length > 0)).slice(0, Math.max(0, max));
  for (const p of ps) {
    const commit = (v: PVal) => { node.params[p.name] = v; set(node.id, p.name, v).then(changed, () => changed()); };
    const c = control(p, node.params[p.name] ?? p.default, commit);
    c.querySelectorAll("[data-param]").forEach((e) => e.removeAttribute("data-param"));
    c.querySelectorAll("input[type=range]").forEach((e) => e.remove());
    host.append(el("label", { className: `op-p op-${p.kind}`, title: p.name }, el("span", { textContent: p.label }), c));
  }
}
