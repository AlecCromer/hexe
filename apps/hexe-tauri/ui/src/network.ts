// Network editor: plain SVG, no library. Nodes = boxes (type + name), inputs on top (port index beside each dot), output at the bottom.
// click = select · shift-click = toggle in selection · drag node(s) = move · drag output -> input / node body / near a port = wire
// right-click or flag dot or D = display TOGGLE (2nd press clears) · double-click name (or F2) = rename · double-click node = display
// Tab = add-node menu (node lands under the cursor) · Delete/Backspace = remove ALL selected nodes (or the selected wire)
// Cmd+Z / Cmd+Shift+Z = undo / redo · Cmd+C / Cmd+V = copy / paste selected nodes (+20 px, inner wires kept) · wheel = zoom · drag background = pan.
import "./t3.css";
import { api, msg } from "./api";
import type { GraphView, NodeType, NodeView } from "./api";
import { NODE_H, NODE_W, filterTypes, inPort, outPort, wirePath } from "./geom";
import { DROP_PX, centred, clientToGraph, displayToggle, graphToHost, nodeAt, resolveDrop, toggled } from "./netlogic";
const NS = "http://www.w3.org/2000/svg";
function s<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number> = {}, ...kids: (Node | string)[]): SVGElementTagNameMap[K] {
  const e = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  e.append(...kids);
  return e;
}
export interface NetHooks {
  select(id: number | null): void;
  add(type: string, x: number, y: number): Promise<void>;
  remove(id: number): Promise<void>;
  move(id: number, x: number, y: number): Promise<void>;
  connect(from: number, to: number, input: number): Promise<void>;
  disconnect(to: number, input: number): Promise<void>;
  display(id: number): Promise<void>;
  refresh(): Promise<void>;
  error(m: string): void;
}
type Drag =
  | { kind: "move"; ids: number[]; start: Map<number, [number, number]>; p0: { x: number; y: number }; click: number; moved: boolean }
  | { kind: "wire"; from: number }
  | { kind: "pan"; sx: number; sy: number; vx: number; vy: number };
/** The live editor (the `--script` shell and the native menu drive it). */
export let currentNetwork: Network | null = null;
export class Network {
  private svg = s("svg", { tabindex: -1 });
  private root = s("g");
  private wires = s("g");
  private nodes = s("g");
  private pending = s("path", { class: "edge pending", d: "" });
  private view = { x: 20, y: 20, k: 1 };
  private graph: GraphView = { nodes: [], edges: [], display: null };
  /** selected nodes; `primary` (last clicked) feeds the param pane */
  private sel = new Set<number>();
  private primary: number | null = null;
  private edgeSel: { to: number; input: number } | null = null;
  private drag: Drag | null = null;
  private menu: HTMLElement | null = null;
  private renaming: HTMLInputElement | null = null;
  /** pointer in graph coords / client px / over the pane? */
  private mouse = { x: 0, y: 0 };
  private over = false;
  private hover: number | null = null;
  /** names to re-select once the next graph arrives (ids change across undo/redo) */
  private pendingNames: string[] | null = null;
  constructor(private host: HTMLElement, private types: NodeType[], private hooks: NetHooks) {
    this.root.append(this.wires, this.nodes, this.pending);
    this.svg.append(this.root);
    host.append(this.svg);
    currentNetwork = this;
    this.svg.addEventListener("mousedown", (e) => this.down(e));
    window.addEventListener("mousemove", (e) => this.moveEv(e));
    window.addEventListener("mouseup", (e) => this.up(e));
    host.addEventListener("mouseenter", () => (this.over = true));
    host.addEventListener("mouseleave", () => { this.over = false; this.hover = null; });
    this.svg.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      const n = (e.target as Element).closest<SVGElement>("[data-node]");
      if (n) this.toggleDisplay(Number(n.dataset.node));
    });
    this.svg.addEventListener("dblclick", (e) => {
      const t = e.target as Element;
      const n = t.closest<SVGElement>("[data-node]");
      if (!n) return;
      const id = Number(n.dataset.node);
      if (t.closest("[data-name]")) this.startRename(id);
      else this.hooks.display(id).catch((x) => this.hooks.error(msg(x)));
    });
    this.svg.addEventListener("wheel", (e) => this.wheel(e), { passive: false });
    window.addEventListener("keydown", (e) => this.key(e));
    this.applyView();
  }
  /** primary selected node (the param pane shows it) */
  get selectedNode(): number | null { return this.primary !== null && this.sel.has(this.primary) ? this.primary : null; }
  get selectedNodes(): number[] { return [...this.sel]; }
  setGraph(g: GraphView): void {
    this.graph = g;
    const live = new Set(g.nodes.map((n) => n.id));
    if (this.pendingNames) {
      const ids = this.pendingNames.map((nm) => g.nodes.find((n) => n.name === nm)?.id).filter((i): i is number => i !== undefined);
      this.sel = new Set(ids);
      this.primary = ids.length ? ids[ids.length - 1] : null;
      this.pendingNames = null;
    }
    this.sel = new Set([...this.sel].filter((i) => live.has(i)));
    if (this.primary !== null && !this.sel.has(this.primary)) this.primary = this.sel.size ? [...this.sel][this.sel.size - 1] : null;
    const e = this.edgeSel;
    if (e && !g.edges.some((x) => x.to === e.to && x.input === e.input)) this.edgeSel = null;
    this.render();
  }
  selectNode(id: number | null): void {
    this.sel = id === null ? new Set() : new Set([id]);
    this.primary = id;
    this.edgeSel = null;
    this.render();
    this.hooks.select(id);
  }
  /** Re-read the graph from Rust (script / menu driven edits); `select` = node names to select afterwards. */
  async resync(select?: string[] | null): Promise<void> {
    if (select) this.pendingNames = select;
    await this.hooks.refresh();
  }
  private applyView(): void {
    this.root.setAttribute("transform", `translate(${this.view.x},${this.view.y}) scale(${this.view.k})`);
  }
  /** client -> graph coordinates */
  private toGraph(cx: number, cy: number): { x: number; y: number } {
    return clientToGraph(cx, cy, this.svg.getBoundingClientRect(), this.view);
  }
  private node(id: number): NodeView | undefined { return this.graph.nodes.find((n) => n.id === id); }
  private fail = (x: unknown): void => this.hooks.error(msg(x));
  render(): void {
    this.wires.replaceChildren();
    this.nodes.replaceChildren();
    for (const e of this.graph.edges) {
      const a = this.node(e.from), b = this.node(e.to);
      if (!a || !b) continue;
      const d = wirePath(outPort(a.x, a.y), inPort(b.x, b.y, e.input, b.inputs.length));
      const on = this.edgeSel?.to === e.to && this.edgeSel.input === e.input;
      const g = s("g", { "data-edge-to": e.to, "data-edge-input": e.input }, s("path", { class: on ? "edge sel" : "edge", d }), s("path", { class: "edge-hit", d }));
      this.wires.append(g);
    }
    for (const n of this.graph.nodes) {
      const sel = this.sel.has(n.id);
      const disp = this.graph.display === n.id;
      const g = s("g", { class: `node${sel ? " sel" : ""}${disp ? " disp" : ""}`, "data-node": n.id, transform: `translate(${n.x},${n.y})` });
      g.append(
        s("rect", { class: "ring", x: -4, y: -4, width: NODE_W + 8, height: NODE_H + 8, rx: 9 }),
        s("rect", { class: "body", width: NODE_W, height: NODE_H, rx: 6 }),
        s("text", { class: "ty", x: 10, y: 15 }, n.type),
        s("text", { class: "nm", x: 10, y: 31, "font-size": 13, "data-name": n.id }, n.name),
        s("circle", { class: "flag", cx: NODE_W - 12, cy: NODE_H / 2, r: 6, "data-flag": n.id }),
        s("circle", { class: "port out", cx: NODE_W / 2, cy: NODE_H, r: 5, "data-out": n.id }),
      );
      n.inputs.forEach((name, i) => {
        const [px] = inPort(0, 0, i, n.inputs.length);
        g.append(
          s("circle", { class: "port in", cx: px, cy: 0, r: 5, "data-in": n.id, "data-input": i }, s("title", {}, name)),
          s("text", { class: "pidx", x: px + 8, y: -5 }, String(i)),
        );
      });
      this.nodes.append(g);
    }
  }
  /** display flag TOGGLE: the display node's flag clears it, any other node takes it */
  private async toggleDisplay(id: number): Promise<void> {
    try { await api.setDisplay(displayToggle(this.graph.display, id)); } catch (x) { this.fail(x); }
    await this.hooks.refresh().catch(this.fail);
  }
  private down(e: MouseEvent): void {
    this.closeMenu();
    (this.host as HTMLElement).focus();
    const t = e.target as Element;
    const flag = t.closest<SVGElement>("[data-flag]");
    if (flag && e.button === 0) { this.toggleDisplay(Number(flag.dataset.flag)); e.preventDefault(); return; }
    const out = t.closest<SVGElement>("[data-out]");
    if (out && e.button === 0) { this.drag = { kind: "wire", from: Number(out.dataset.out) }; e.preventDefault(); return; }
    const inp = t.closest<SVGElement>("[data-in]");
    if (inp && e.button === 0) {
      // grab an existing wire off its input: re-drop it elsewhere (or nowhere = cut)
      const to = Number(inp.dataset.in), input = Number(inp.dataset.input);
      const edge = this.graph.edges.find((x) => x.to === to && x.input === input);
      if (edge) {
        this.drag = { kind: "wire", from: edge.from };
        this.hooks.disconnect(to, input).catch(this.fail);
        e.preventDefault();
      }
      return;
    }
    const nodeEl = t.closest<SVGElement>("[data-node]");
    if (nodeEl && e.button === 0) {
      const id = Number(nodeEl.dataset.node);
      const n = this.node(id);
      if (!n) return;
      this.edgeSel = null;
      e.preventDefault();
      if (e.shiftKey) {
        // shift-click toggles the node in the selection (no drag)
        this.sel = toggled(this.sel, id);
        this.primary = this.sel.has(id) ? id : this.sel.size ? [...this.sel][this.sel.size - 1] : null;
        this.render();
        this.hooks.select(this.primary);
        return;
      }
      if (!this.sel.has(id)) this.sel = new Set([id]);
      this.primary = id;
      const ids = [...this.sel];
      const start = new Map<number, [number, number]>();
      for (const i of ids) { const q = this.node(i); if (q) start.set(i, [q.x, q.y]); }
      this.drag = { kind: "move", ids, start, p0: this.toGraph(e.clientX, e.clientY), click: id, moved: false };
      this.render();
      this.hooks.select(id);
      return;
    }
    if (e.button !== 0) return;
    const edgeEl = t.closest<SVGElement>("[data-edge-to]");
    if (edgeEl) {
      this.edgeSel = { to: Number(edgeEl.dataset.edgeTo), input: Number(edgeEl.dataset.edgeInput) };
      this.sel = new Set();
      this.primary = null;
      this.render();
      this.hooks.select(null);
      return;
    }
    if (!e.shiftKey) {
      this.sel = new Set();
      this.primary = null;
      this.edgeSel = null;
      this.render();
      this.hooks.select(null);
    }
    this.drag = { kind: "pan", sx: e.clientX, sy: e.clientY, vx: this.view.x, vy: this.view.y };
  }
  private moveEv(e: MouseEvent): void {
    this.mouse = this.toGraph(e.clientX, e.clientY);
    if (this.over && !this.drag) this.hover = nodeAt(this.graph.nodes, this.mouse);
    const d = this.drag;
    if (!d) return;
    if (d.kind === "move") {
      const dx = this.mouse.x - d.p0.x, dy = this.mouse.y - d.p0.y;
      if (!d.moved && Math.hypot(dx, dy) * this.view.k < 2) return;
      for (const [id, [x, y]] of d.start) {
        const n = this.node(id);
        if (n) { n.x = x + dx; n.y = y + dy; }
      }
      d.moved = true;
      this.render();
    } else if (d.kind === "pan") {
      this.view.x = d.vx + (e.clientX - d.sx);
      this.view.y = d.vy + (e.clientY - d.sy);
      this.applyView();
    } else {
      const a = this.node(d.from);
      if (a) this.pending.setAttribute("d", wirePath(outPort(a.x, a.y), [this.mouse.x, this.mouse.y]));
      this.svg.querySelectorAll(".port.hot").forEach((p) => p.classList.remove("hot"));
      const hit = resolveDrop(this.graph.nodes, this.graph.edges, d.from, this.mouse, DROP_PX / this.view.k);
      if (hit) this.svg.querySelector(`[data-in="${hit.to}"][data-input="${hit.input}"]`)?.classList.add("hot");
    }
  }
  private up(e: MouseEvent): void {
    const d = this.drag;
    this.drag = null;
    if (!d) return;
    if (d.kind === "move") {
      if (d.moved) {
        const moves = d.ids.flatMap((id): [number, number, number][] => { const n = this.node(id); return n ? [[id, n.x, n.y]] : []; });
        api.nodesMove(moves).catch(this.fail);
      } else if (this.sel.size > 1) {
        // plain click on a member of a multi-selection collapses to that node
        this.sel = new Set([d.click]);
        this.primary = d.click;
        this.render();
        this.hooks.select(d.click);
      }
    } else if (d.kind === "wire") {
      this.pending.setAttribute("d", "");
      this.svg.querySelectorAll(".port.hot").forEach((p) => p.classList.remove("hot"));
      const p = this.toGraph(e.clientX, e.clientY);
      const hit = resolveDrop(this.graph.nodes, this.graph.edges, d.from, p, DROP_PX / this.view.k);
      if (hit) this.hooks.connect(d.from, hit.to, hit.input).catch(this.fail);
      else this.hooks.refresh().catch(this.fail); // dropped nowhere: re-sync (a grabbed wire stays cut)
    }
  }
  private wheel(e: WheelEvent): void {
    e.preventDefault();
    const p = this.toGraph(e.clientX, e.clientY);
    const k = Math.min(2.5, Math.max(0.3, this.view.k * Math.exp(-e.deltaY * 0.0015)));
    const r = this.svg.getBoundingClientRect();
    this.view = { k, x: e.clientX - r.left - p.x * k, y: e.clientY - r.top - p.y * k };
    this.applyView();
  }
  private key(e: KeyboardEvent): void {
    const t = e.target as HTMLElement;
    const typing = (t.tagName === "INPUT" && !t.matches('[type="checkbox"],[type="range"],[type="button"]')) || t.tagName === "SELECT" || t.tagName === "TEXTAREA";
    if (this.menu || this.renaming || typing) return;
    const mod = e.metaKey || e.ctrlKey;
    const k = e.key.toLowerCase();
    if (e.key === "Tab") {
      e.preventDefault();
      this.openMenu();
    } else if (mod && k === "z") {
      e.preventDefault();
      this.undo(e.shiftKey).catch(this.fail);
    } else if (mod && k === "y") {
      e.preventDefault();
      this.undo(true).catch(this.fail);
    } else if (mod && k === "c") {
      e.preventDefault();
      this.copy().catch(this.fail);
    } else if (mod && k === "v") {
      e.preventDefault();
      this.paste().catch(this.fail);
    } else if (mod && k === "a") {
      e.preventDefault();
      this.sel = new Set(this.graph.nodes.map((n) => n.id));
      this.primary = this.graph.nodes.length ? this.graph.nodes[this.graph.nodes.length - 1].id : null;
      this.render();
      this.hooks.select(this.primary);
    } else if (k === "d" && !mod && !e.altKey && this.over) {
      const id = this.hover ?? this.selectedNode;
      if (id !== null) this.toggleDisplay(id);
    } else if (e.key === "F2" && this.selectedNode !== null) {
      e.preventDefault();
      this.startRename(this.selectedNode);
    } else if ((e.key === "Delete" || e.key === "Backspace") && !mod) {
      if (this.sel.size) {
        e.preventDefault();
        const ids = [...this.sel];
        api.nodesRemove(ids).catch(this.fail).finally(() => this.hooks.refresh().catch(this.fail));
      } else if (this.edgeSel) {
        e.preventDefault();
        this.hooks.disconnect(this.edgeSel.to, this.edgeSel.input).catch(this.fail);
      }
    }
  }
  private selNames(): string[] {
    return [...this.sel].flatMap((i) => this.node(i)?.name ?? []);
  }
  /** Cmd+Z / Cmd+Shift+Z (also fired by the native Edit menu). Selection survives by node name. */
  async undo(redo = false): Promise<void> {
    const names = this.selNames();
    let did = false;
    try { did = redo ? await api.redo() : await api.undo(); } catch (x) { this.fail(x); }
    if (!did) return;
    this.pendingNames = names;
    await this.hooks.refresh();
  }
  async copy(): Promise<void> {
    if (!this.sel.size) return;
    await api.copyNodes([...this.sel]);
  }
  async paste(): Promise<void> {
    const ids = await api.pasteNodes();
    this.sel = new Set(ids);
    this.primary = ids.length ? ids[ids.length - 1] : null;
    this.edgeSel = null;
    await this.hooks.refresh();
  }
  /** inline rename box over the node's name; Enter / blur commits (unique names enforced in Rust), Esc cancels */
  startRename(id: number): void {
    const n = this.node(id);
    if (!n) return;
    this.renaming?.remove();
    const at = graphToHost(n.x + 6, n.y + 17, this.view);
    const input = document.createElement("input");
    input.id = "renamebox";
    input.value = n.name;
    input.spellcheck = false;
    input.style.left = `${at.x}px`;
    input.style.top = `${at.y}px`;
    input.style.width = `${(NODE_W - 12) * this.view.k}px`;
    this.host.append(input);
    this.renaming = input;
    let done = false;
    const finish = (commit: boolean) => {
      if (done) return;
      done = true;
      const name = input.value.trim();
      input.remove();
      this.renaming = null;
      this.host.focus();
      if (commit && name && name !== n.name) {
        api.nodeRename(id, name).catch(this.fail).finally(() => this.hooks.refresh().catch(this.fail));
      }
    };
    input.addEventListener("keydown", (ev) => {
      ev.stopPropagation();
      if (ev.key === "Enter") { ev.preventDefault(); finish(true); }
      else if (ev.key === "Escape") { ev.preventDefault(); finish(false); }
    });
    input.addEventListener("blur", () => finish(true));
    input.addEventListener("mousedown", (ev) => ev.stopPropagation());
    input.focus();
    input.select();
  }
  /** Tab menu: filter box + list of node types; Enter / click adds under the cursor (pane centre if the pointer is elsewhere). */
  private openMenu(): void {
    const r = this.host.getBoundingClientRect();
    const at = this.over ? { ...this.mouse } : this.toGraph(r.left + r.width / 2, r.top + r.height / 2);
    const m = graphToHost(at.x, at.y, this.view);
    const sx = Math.min(Math.max(m.x, 0), Math.max(0, r.width - 200));
    const sy = Math.min(Math.max(m.y, 0), Math.max(0, r.height - 250));
    const box = document.createElement("div");
    box.id = "tabmenu";
    box.style.left = `${sx}px`;
    box.style.top = `${sy}px`;
    const input = document.createElement("input");
    input.placeholder = "add node…";
    input.spellcheck = false;
    const list = document.createElement("ul");
    box.append(input, list);
    this.host.append(box);
    this.menu = box;
    const all = this.types.map((t) => t.type);
    let shown: string[] = [], on = 0;
    const pick = (t: string | undefined) => {
      this.closeMenu();
      if (t) { const p = centred(at); this.hooks.add(t, p.x, p.y).catch(this.fail); }
    };
    const paint = () => {
      shown = filterTypes(all, input.value);
      on = Math.min(on, Math.max(0, shown.length - 1));
      list.replaceChildren(...shown.map((t, i) => {
        const li = document.createElement("li");
        li.textContent = t;
        if (i === on) li.className = "on";
        li.addEventListener("mousedown", (ev) => { ev.preventDefault(); ev.stopPropagation(); pick(t); });
        return li;
      }));
      list.querySelector(".on")?.scrollIntoView({ block: "nearest" });
    };
    input.addEventListener("input", () => { on = 0; paint(); });
    input.addEventListener("keydown", (ev) => {
      ev.stopPropagation();
      if (ev.key === "Escape" || ev.key === "Tab") { ev.preventDefault(); this.closeMenu(); }
      else if (ev.key === "Enter") { ev.preventDefault(); pick(shown[on]); }
      else if (ev.key === "ArrowDown") { ev.preventDefault(); on = Math.min(on + 1, shown.length - 1); paint(); }
      else if (ev.key === "ArrowUp") { ev.preventDefault(); on = Math.max(on - 1, 0); paint(); }
    });
    paint();
    input.focus();
  }
  private closeMenu(): void {
    this.menu?.remove();
    this.menu = null;
    this.host.focus();
  }
}
