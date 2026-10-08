// Param pane generated from the selected node's schema (list_node_types). Edits -> node_set_param -> onChange (re-cook).
import type { NodeType, NodeView, ParamInfo, PVal } from "./api";
import { type CodeEditor, createCodeEditor, parseCodeError } from "./codeeditor";
import { attachScrub, scrubSpeed } from "./scrub";

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
  host.append(el("div", { className: "phead" }, el("b", { textContent: node.name }), el("span", { textContent: `${node.type} · #${node.id}` })));
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
    if (code) {
      const ed = createCodeEditor(String(value), (t) => commit(t));
      shown.editors.push(ed);
      row.append(ed.el);
    } else row.append(control(p, value, commit));
    host.append(row);
  }
  paintCookError();
}
