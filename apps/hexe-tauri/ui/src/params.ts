// Param pane generated from the selected node's schema (list_node_types). Edits -> node_set_param -> onChange (re-cook).
import type { NodeType, NodeView, ParamInfo, PVal } from "./api";
import { type CodeEditor, createCodeEditor, parseCodeError } from "./codeeditor";
import { attachScrub, scrubSpeed } from "./scrub";
import { nodeIcon } from "./shelf";
import { labelOf } from "./tabcats";
import { formatPoints, parsePoints, resizePoints } from "./pointlist";

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
document.addEventListener("mousedown", (e) => { if (openCog && !openCog.contains(e.target as Node)) closeCog(); });
document.addEventListener("keydown", (e) => { if (e.key === "Escape") closeCog(); });
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

export function renderParams(host: HTMLElement, node: NodeView | null, type: NodeType | undefined, set: SetParam): void {
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
  const revert = async () => {
    for (const p of type.params) {
      const cur = node.params[p.name] ?? p.default;
      if (JSON.stringify(cur) === JSON.stringify(p.default)) continue;
      node.params[p.name] = p.default;
      await set(node.id, p.name, p.default).catch(() => {});
    }
    renderParams(host, node, type, set);
  };
  const cog = cogMenu({ "Revert to Factory Defaults": () => void revert() });
  host.append(el("div", { className: "phead", title: `${node.type} · #${node.id}` }, ico, el("span", { className: "phead-ty", textContent: labelOf(node.type) }), el("b", { textContent: node.name }), cog));
  if (!type.params.length) host.append(el("div", { className: "hint", textContent: "No parameters." }));
  shown.node = node.id;
  // Houdini folders: untabbed params first, then a tab bar; only the active tab's rows show
  const tabs = [...new Set(type.params.map((p) => p.folder).filter((f): f is string => !!f))];
  const pages = new Map<string, HTMLElement>();
  if (tabs.length) {
    const want = activeTab.get(type.type);
    const on = want && tabs.includes(want) ? want : tabs[0];
    const bar = el("div", { className: "pfold" });
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
        for (const x of bar.children) x.classList.toggle("on", (x as HTMLElement).dataset.tab === t);
      });
      bar.append(b);
    }
    pages.set("", bar); // appended after the untabbed rows
  }
  for (const p of type.params) {
    const target = (p.folder && pages.get(p.folder)) || host;
    const value = node.params[p.name] ?? p.default;
    const code = !!p.language && p.kind === "string";
    const row = el("div", { className: code ? "prow code" : "prow" }, el("label", { textContent: code ? `${p.label} (${p.language})` : p.label, title: p.name }));
    const commit = (v: PVal) => {
      node.params[p.name] = v;
      const clear = () => host.querySelector(".err-msg")?.remove();
      set(node.id, p.name, v).then(clear, (e) => {
        clear();
        host.prepend(el("div", { className: "err-msg", textContent: `${p.label}: ${typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e)}` }));
      });
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
    target.append(row);
  }
  const bar = pages.get("");
  if (bar) { pages.delete(""); host.append(bar, ...pages.values()); }
  paintCookError();
}

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
