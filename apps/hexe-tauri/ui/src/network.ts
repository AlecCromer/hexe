// Network editor: plain SVG, no library. Nodes = boxes (type + name), inputs on top, output at the bottom.
// drag node = move · click = select · drag output->input = wire · click wire = select it · flag / double-click = display
// Tab = add-node menu · Delete/Backspace = remove selected node or wire · wheel = zoom · drag background = pan.
import type { GraphView, NodeType, NodeView } from "./api";
import { NODE_H, NODE_W, filterTypes, inPort, outPort, wirePath } from "./geom";

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

type Sel = { kind: "node"; id: number } | { kind: "edge"; to: number; input: number } | null;

export class Network {
  private svg = s("svg", { tabindex: -1 });
  private root = s("g");
  private wires = s("g");
  private nodes = s("g");
  private pending = s("path", { class: "edge pending", d: "" });
  private view = { x: 20, y: 20, k: 1 };
  private graph: GraphView = { nodes: [], edges: [], display: null };
  private sel: Sel = null;
  private drag: null | { kind: "move"; id: number; dx: number; dy: number; moved: boolean } | { kind: "wire"; from: number } | { kind: "pan"; sx: number; sy: number; vx: number; vy: number } = null;
  private menu: HTMLElement | null = null;
  private mouse = { x: 0, y: 0 };

  constructor(private host: HTMLElement, private types: NodeType[], private hooks: NetHooks) {
    this.root.append(this.wires, this.nodes, this.pending);
    this.svg.append(this.root);
    host.append(this.svg);
    this.svg.addEventListener("mousedown", (e) => this.down(e));
    window.addEventListener("mousemove", (e) => this.moveEv(e));
    window.addEventListener("mouseup", (e) => this.up(e));
    this.svg.addEventListener("dblclick", (e) => {
      const n = (e.target as Element).closest<SVGElement>("[data-node]");
      if (n) this.hooks.display(Number(n.dataset.node)).catch((x) => this.hooks.error(String(x)));
    });
    this.svg.addEventListener("wheel", (e) => this.wheel(e), { passive: false });
    window.addEventListener("keydown", (e) => this.key(e));
    this.applyView();
  }

  get selectedNode(): number | null { return this.sel?.kind === "node" ? this.sel.id : null; }

  setGraph(g: GraphView): void {
    this.graph = g;
    const sel = this.sel;
    if (sel?.kind === "node" && !g.nodes.some((n) => n.id === sel.id)) this.sel = null;
    if (sel?.kind === "edge" && !g.edges.some((e) => e.to === sel.to && e.input === sel.input)) this.sel = null;
    this.render();
  }

  selectNode(id: number | null): void {
    this.sel = id === null ? null : { kind: "node", id };
    this.render();
    this.hooks.select(id);
  }

  private applyView(): void {
    this.root.setAttribute("transform", `translate(${this.view.x},${this.view.y}) scale(${this.view.k})`);
  }

  /** client -> graph coordinates */
  private toGraph(cx: number, cy: number): { x: number; y: number } {
    const r = this.svg.getBoundingClientRect();
    return { x: (cx - r.left - this.view.x) / this.view.k, y: (cy - r.top - this.view.y) / this.view.k };
  }

  private node(id: number): NodeView | undefined { return this.graph.nodes.find((n) => n.id === id); }

  render(): void {
    this.wires.replaceChildren();
    this.nodes.replaceChildren();
    for (const e of this.graph.edges) {
      const a = this.node(e.from), b = this.node(e.to);
      if (!a || !b) continue;
      const d = wirePath(outPort(a.x, a.y), inPort(b.x, b.y, e.input, b.inputs.length));
      const on = this.sel?.kind === "edge" && this.sel.to === e.to && this.sel.input === e.input;
      const g = s("g", { "data-edge-to": e.to, "data-edge-input": e.input }, s("path", { class: on ? "edge sel" : "edge", d }), s("path", { class: "edge-hit", d }));
      this.wires.append(g);
    }
    for (const n of this.graph.nodes) {
      const sel = this.sel?.kind === "node" && this.sel.id === n.id;
      const disp = this.graph.display === n.id;
      const g = s("g", { class: `node${sel ? " sel" : ""}${disp ? " disp" : ""}`, "data-node": n.id, transform: `translate(${n.x},${n.y})` });
      g.append(
        s("rect", { class: "ring", x: -4, y: -4, width: NODE_W + 8, height: NODE_H + 8, rx: 9 }),
        s("rect", { class: "body", width: NODE_W, height: NODE_H, rx: 6 }),
        s("text", { class: "ty", x: 10, y: 15 }, n.type),
        s("text", { x: 10, y: 31, "font-size": 13 }, n.name),
        s("circle", { class: "flag", cx: NODE_W - 12, cy: NODE_H / 2, r: 6, "data-flag": n.id }),
        s("circle", { class: "port out", cx: NODE_W / 2, cy: NODE_H, r: 5, "data-out": n.id }),
      );
      n.inputs.forEach((name, i) => {
        const [px] = inPort(0, 0, i, n.inputs.length);
        g.append(s("circle", { class: "port in", cx: px, cy: 0, r: 5, "data-in": n.id, "data-input": i }, s("title", {}, name)));
      });
      this.nodes.append(g);
    }
  }

  private down(e: MouseEvent): void {
    this.closeMenu();
    (this.host as HTMLElement).focus();
    const t = e.target as Element;
    const flag = t.closest<SVGElement>("[data-flag]");
    if (flag) { this.hooks.display(Number(flag.dataset.flag)).catch((x) => this.hooks.error(String(x))); e.preventDefault(); return; }
    const out = t.closest<SVGElement>("[data-out]");
    if (out && e.button === 0) { this.drag = { kind: "wire", from: Number(out.dataset.out) }; e.preventDefault(); return; }
    const inp = t.closest<SVGElement>("[data-in]");
    if (inp && e.button === 0) {
      // grab an existing wire off its input: re-drop it elsewhere (or nowhere = cut)
      const to = Number(inp.dataset.in), input = Number(inp.dataset.input);
      const edge = this.graph.edges.find((x) => x.to === to && x.input === input);
      if (edge) {
        this.drag = { kind: "wire", from: edge.from };
        this.hooks.disconnect(to, input).catch((x) => this.hooks.error(String(x)));
        e.preventDefault();
      }
      return;
    }
    const nodeEl = t.closest<SVGElement>("[data-node]");
    if (nodeEl && e.button === 0) {
      const id = Number(nodeEl.dataset.node);
      const n = this.node(id);
      if (!n) return;
      const p = this.toGraph(e.clientX, e.clientY);
      this.drag = { kind: "move", id, dx: p.x - n.x, dy: p.y - n.y, moved: false };
      this.sel = { kind: "node", id };
      this.render();
      this.hooks.select(id);
      e.preventDefault();
      return;
    }
    const edgeEl = t.closest<SVGElement>("[data-edge-to]");
    if (edgeEl) {
      this.sel = { kind: "edge", to: Number(edgeEl.dataset.edgeTo), input: Number(edgeEl.dataset.edgeInput) };
      this.render();
      this.hooks.select(null);
      return;
    }
    this.sel = null;
    this.render();
    this.hooks.select(null);
    this.drag = { kind: "pan", sx: e.clientX, sy: e.clientY, vx: this.view.x, vy: this.view.y };
  }

  private moveEv(e: MouseEvent): void {
    this.mouse = this.toGraph(e.clientX, e.clientY);
    const d = this.drag;
    if (!d) return;
    if (d.kind === "move") {
      const n = this.node(d.id);
      if (!n) return;
      n.x = this.mouse.x - d.dx;
      n.y = this.mouse.y - d.dy;
      d.moved = true;
      this.render();
    } else if (d.kind === "pan") {
      this.view.x = d.vx + (e.clientX - d.sx);
      this.view.y = d.vy + (e.clientY - d.sy);
      this.applyView();
    } else {
      const a = this.node(d.from);
      if (a) this.pending.setAttribute("d", wirePath(outPort(a.x, a.y), [this.mouse.x, this.mouse.y]));
      const hit = document.elementFromPoint(e.clientX, e.clientY)?.closest<SVGElement>("[data-in]");
      this.svg.querySelectorAll(".port.hot").forEach((p) => p.classList.remove("hot"));
      hit?.classList.add("hot");
    }
  }

  private up(e: MouseEvent): void {
    const d = this.drag;
    this.drag = null;
    if (!d) return;
    if (d.kind === "move" && d.moved) {
      const n = this.node(d.id);
      if (n) this.hooks.move(n.id, n.x, n.y).catch((x) => this.hooks.error(String(x)));
    } else if (d.kind === "wire") {
      this.pending.setAttribute("d", "");
      const hit = document.elementFromPoint(e.clientX, e.clientY)?.closest<SVGElement>("[data-in]");
      if (hit) this.hooks.connect(d.from, Number(hit.dataset.in), Number(hit.dataset.input)).catch((x) => this.hooks.error(String(x)));
      else this.hooks.refresh().catch((x) => this.hooks.error(String(x))); // dropped nowhere: re-sync (a grabbed wire stays cut)
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
    const typing = (t.tagName === "INPUT" && !["checkbox", "range", "button"].includes((t as HTMLInputElement).type)) || t.tagName === "SELECT" || t.tagName === "TEXTAREA";
    if (this.menu) return;
    if (e.key === "Tab" && !typing) {
      e.preventDefault();
      this.openMenu();
    } else if ((e.key === "Delete" || e.key === "Backspace") && !typing && this.sel) {
      e.preventDefault();
      const sel = this.sel;
      (sel.kind === "node" ? this.hooks.remove(sel.id) : this.hooks.disconnect(sel.to, sel.input)).catch((x) => this.hooks.error(String(x)));
    }
  }

  /** Tab menu: filter box + list of node types; Enter / click adds at the mouse position. */
  private openMenu(): void {
    const at = { ...this.mouse };
    const r = this.host.getBoundingClientRect();
    const sx = Math.min(Math.max(at.x * this.view.k + this.view.x, 0), r.width - 200);
    const sy = Math.min(Math.max(at.y * this.view.k + this.view.y, 0), r.height - 250);
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
      if (t) this.hooks.add(t, at.x - NODE_W / 2, at.y - NODE_H / 2).catch((x) => this.hooks.error(String(x)));
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
