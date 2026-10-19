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

export function renderParams(host: HTMLElement, node: NodeView | null, type: NodeType | undefined, set: SetParam): void {
  host.replaceChildren();
  shown = { node: -1, editors: [] };
  if (!node || !type) {
    host.append(el("div", { className: "hint", textContent: "Select a node in the network to edit its parameters." }));
    return;
  }
  // Houdini header: type icon · Type label · node name (id in the tooltip)
  const ico = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  ico.setAttribute("viewBox", "0 0 24 24");
  ico.setAttribute("class", "phead-ico");
  ico.innerHTML = nodeIcon(node.type);
  host.append(el("div", { className: "phead", title: `${node.type} · #${node.id}` }, ico, el("span", { className: "phead-ty", textContent: labelOf(node.type) }), el("b", { textContent: node.name })));
  if (!type.params.length) host.append(el("div", { className: "hint", textContent: "No parameters." }));
  shown.node = node.id;
  for (const p of type.params) {
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
      host.append(...pointList(String(value), commit));
      continue;
    }
    if (code) {
      const ed = createCodeEditor(String(value), (t) => commit(t));
      shown.editors.push(ed);
      row.append(ed.el);
    } else row.append(control(p, value, commit));
    host.append(row);
  }
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
