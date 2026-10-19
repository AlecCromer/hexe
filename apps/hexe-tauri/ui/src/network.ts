// Network editor: plain SVG, no library. Nodes = boxes (type + name), inputs on top (port index beside each dot), output at the bottom.
// click = select · shift-click = toggle in selection · drag node(s) = move · drag output -> input / node body / near a port = wire
// (or drag a free input up -> output / node body) · at /obj the same drags parent objects (output = parent, input = child)
// network dots: Alt (held key or Alt-click) while drawing a wire drops a dot there and the wire carries on from it · click a
// dot = wire out of it follows the cursor (click nothing = cancel) · drag a dot = move · Alt-click a wire = dot on it ·
// Alt-click a dot = delete it (its source rewires straight through) · wires up the network show a flow arrow
// right-click node or flag dot or D = display TOGGLE (2nd press clears) · double-click name (or F2) = rename · double-click node = display
// Tab or right-click empty canvas = TAB menu (categories / All / History / search; node lands under the cursor) · Delete/Backspace = remove ALL selected nodes (or the selected wire)
// Cmd+Z / Cmd+Shift+Z = undo / redo · Cmd+C / Cmd+V = copy / paste selected nodes (+20 px, inner wires kept) · wheel = zoom · drag background = pan.
import "./t3.css";
import { api, msg } from "./api";
import type { BoxView, GraphView, NodeType, NodeView, NoteView, ObjInfo, ObjKind } from "./api";
import { DOT_R, NODE_H, NODE_W, inPort, isDot, linePath, nodeIn, nodeOut, nodeRect, roundedPath, wireArrow, wirePath } from "./geom";
import { DROP_PX, centred, edgeKey, clientToGraph, displayToggle, graphToHost, nodeAt, nodesInBox, brokenNodes, loopNodes, objChildDrop, objCycleCut, objInPort, objOutPort, fitView, objParentDrop, polylineInBox, resolveDrop, resolveDropUp, toggled, uniqueName, missingInputs, optionalInput, blockPairs } from "./netlogic";
import { grouped, groupedObj, labelOf, OBJ_TYPES, objLabelOf, pushHistory, searchTypes } from "./tabcats";
import { nodeIcon } from "./shelf";

/** Annular sector (node halo arc) around (cx, cy), radii r0..r1, from a0 to a1 degrees (0 = +x, clockwise on screen). */
function arcPath(cx: number, cy: number, r0: number, r1: number, a0: number, a1: number): string {
  const p = (r: number, a: number) => `${(cx + r * Math.cos((a * Math.PI) / 180)).toFixed(2)},${(cy + r * Math.sin((a * Math.PI) / 180)).toFixed(2)}`;
  return `M${p(r1, a0)} A${r1},${r1} 0 0 1 ${p(r1, a1)} L${p(r0, a1)} A${r0},${r0} 0 0 0 ${p(r0, a0)} Z`;
}
/** Geometry object icon (cylinder, cone, sphere) */
const GEO_ICON = '<ellipse cx="8" cy="5" rx="4" ry="1.6" fill="#9cc3e8"/><path d="M4 5v10a4 1.6 0 008 0V5" fill="#4f86c0"/>' +
  '<path d="M12 22l-5-0.5 4-9 4 9z" fill="#e8c840"/><circle cx="17" cy="14" r="4.5" fill="#c8302a"/><circle cx="15.6" cy="12.6" r="1.4" fill="#f09a90"/>';
/** Null object icon (axes) and SOP null icon (dotted ring) */
const NULL_OBJ_ICON = '<path d="M12 12V3M12 12l8 5M12 12l-8 5" stroke-width="2.2" fill="none" stroke="#2f6fc0"/><path d="M12 3v0" stroke="#5cb85c"/><circle cx="12" cy="3" r="1.8" fill="#5cb85c"/><circle cx="20" cy="17" r="1.8" fill="#c0392b"/><circle cx="4" cy="17" r="1.8" fill="#2f6fc0"/><circle cx="12" cy="12" r="1.6" fill="#666"/>';
/** Subnetwork / digital asset object icon (a box holding nodes) */
const SUBNET_ICON = '<rect x="3" y="4" width="18" height="16" rx="2" fill="#d8dee6" stroke="#5a6670" stroke-width="1.4"/><rect x="6" y="7" width="5" height="3.5" rx="0.8" fill="#4f86c0"/><rect x="13" y="13.5" width="5" height="3.5" rx="0.8" fill="#4f86c0"/><path d="M8.5 10.5v2.5h7v0.5" stroke="#5a6670" stroke-width="1.2" fill="none"/>';
const NULL_SOP_ICON ='<circle cx="12" cy="12" r="8" fill="#dfe8f0" stroke="#5a6670" stroke-width="1.4" stroke-dasharray="1.6 1.6"/><circle cx="12" cy="12" r="5" fill="#c4d2de"/>';
/** Bow-tie outline (Null nodes) over the NODE_W x NODE_H box: an X of two rounded bars, arms poking 10 px above / below. */
const BOWTIE = (() => {
  const W = NODE_W, H = NODE_H, c = W / 2, m = H / 2;
  // Houdini's Null: the four lobe tips round off fully (big radii), the waist and the side notches softly
  const pts: [number, number][] = [[W * 0.14, -10], [c, m - 6], [W * 0.86, -10], [W - 2, m - 9], [c + 16, m], [W - 2, m + 9], [W * 0.86, H + 10], [c, m + 6], [W * 0.14, H + 10], [2, m + 9], [c - 16, m], [2, m - 9]];
  const tip = 11, soft = 5;
  return roundedPath(pts, [tip, soft, tip, tip, soft, tip, tip, soft, tip, tip, soft, tip]);
})();
/** Null node parts: shadow, selection ring, body, two flag lobes (left / right, clipped to the bow-tie: [class, data attr,
 *  value, tip]), icon. Shared by the Null object (/obj) and the null SOP. */
function bowtie(clipId: string, lobes: { left?: [string, string, number, string]; right?: [string, string, number, string] }, iconSvg: string): SVGElement[] {
  const W = NODE_W, H = NODE_H;
  const clip = s("clipPath", { id: clipId }, s("path", { d: BOWTIE }));
  const lobe = (spec: [string, string, number, string] | undefined, x: number, w: number) =>
    spec ? [s("rect", { class: `lobe ${spec[0]}`, x, y: -12, width: w, height: H + 24, "clip-path": `url(#${clipId})`, [spec[1]]: spec[2], "data-tip": spec[3] })] : [];
  const icon = s("svg", { class: "nicon", x: W / 2 - 12, y: H / 2 - 12, width: 24, height: 24, viewBox: "0 0 24 24" });
  icon.innerHTML = iconSvg;
  return [
    s("defs", {}, clip),
    s("path", { class: "shadow", d: BOWTIE, transform: "translate(4,4)" }),
    s("path", { class: "ring bt", d: BOWTIE }),
    s("path", { class: "body bt", d: BOWTIE }),
    ...lobe(lobes.left, 0, W * 0.3),
    ...lobe(lobes.right, W * 0.7, W * 0.3),
    s("path", { class: "btline", d: BOWTIE }),
    icon,
  ];
}
/** small eye glyph (display arc) centred at (x, y) */
const eyePath = (x: number, y: number): string =>
  `M${x - 8},${y} Q${x},${y - 7} ${x + 8},${y} Q${x},${y + 7} ${x - 8},${y} Z M${x - 2.6},${y} a2.6,2.6 0 1 0 5.2,0 a2.6,2.6 0 1 0 -5.2,0 Z`;
/** /obj node type labels */
const OBJ_KIND_LABEL: Record<ObjKind, string> = { geo: "Geometry", null: "Null", subnet: "Subnetwork" };
/** bypass glyph (halo arc): a wire stepping round a gap, centred at (x, y) */
const bypassGlyph = (x: number, y: number): string =>
  `M${x - 7},${y + 3} H${x - 3} V${y - 3} H${x + 3} V${y + 3} H${x + 7} V${y + 5} H${x + 1} V${y - 1} H${x - 1} V${y + 5} H${x - 7} Z`;
const NS = "http://www.w3.org/2000/svg";
function s<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number> = {}, ...kids: (Node | string)[]): SVGElementTagNameMap[K] {
  const e = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  e.append(...kids);
  return e;
}
/** An object at /obj (`name` = its Rust object): a Geometry object (one SOP network), a Null (its network: `control`,
 *  the viewport indicator, + `point1`) or a container (subnet / digital asset: holds other objects, dive in to see them). */
export interface ObjNode {
  id: number;
  kind: ObjKind;
  name: string;
  /** container object it lives in (null = directly under /obj) */
  net: string | null;
  /** network position (null = not placed yet: centred on first draw) */
  pos: { x: number; y: number } | null;
  /** display flag (blue): drawn in the viewport */
  visible: boolean;
  /** selectable flag (green): its points / marker can be picked in the viewport */
  selectable: boolean;
  /** viewport position (Null marker) */
  at: [number, number, number];
  /** parent object (an /obj wire: parent's output ball -> this object's input ball); editor state, no transform effect yet */
  parent: number | null;
  /** Geometry object placed from the TAB menu / loaded / created: shown at /obj even while its network is empty */
  placed?: boolean;
}
export interface NetHooks {
  select(id: number | null): void;
  /** add a `type` node at (x, y); `into` = splice it into that wire (TAB menu with a wire selected) */
  add(type: string, x: number, y: number, into?: { from: number; to: number; input: number }): Promise<void>;
  remove(id: number): Promise<void>;
  move(id: number, x: number, y: number): Promise<void>;
  connect(from: number, to: number, input: number): Promise<void>;
  disconnect(to: number, input: number): Promise<void>;
  display(id: number): Promise<void>;
  refresh(): Promise<void>;
  error(m: string): void;
  /** the template-flag set changed (ids) */
  templates(ids: number[]): void;
  /** the network level changed (path bars, params, viewport highlight follow) */
  level(l: "obj" | "sop"): void;
  /** object display flag (/obj blue strip): show / hide the object's geometry in the viewport */
  /** an object's display / selectable flag or the /obj selection changed (viewport + pane follow) */
  objects(): void;
}
type Drag =
  | { kind: "move"; ids: number[]; start: Map<number, [number, number]>; p0: { x: number; y: number }; click: number; moved: boolean }
  | { kind: "wire"; from: number; sx: number; sy: number }
  /** dragged up off a free input: the node it is dropped on feeds input `input` of `to` */
  | { kind: "wireup"; to: number; input: number }
  | { kind: "pan"; sx: number; sy: number; vx: number; vy: number }
  | { kind: "obj"; id: number; dx: number; dy: number }
  | { kind: "objwire"; from: number; sx: number; sy: number }
  /** dragged up off a free input ball: `child` takes whichever object the wire is dropped on as its parent */
  | { kind: "objwireup"; child: number; sx: number; sy: number }
  | { kind: "box"; p0: { x: number; y: number }; add: boolean; base: Set<number> };
/** The live editor (the `--script` shell and the native menu drive it). */
export let currentNetwork: Network | null = null;
export class Network {
  private svg = s("svg", { tabindex: -1 });
  private root = s("g");
  /** display discs: under the wires, so a wire leaving the display node stays visible over its disc */
  private discs = s("g");
  private wires = s("g");
  private nodes = s("g");
  private pending = s("path", { class: "edge pending", d: "" });
  private view = { x: 20, y: 20, k: 1 };
  private graph: GraphView = { nodes: [], edges: [], display: null };
  /** selected nodes; `primary` (last clicked) feeds the param pane */
  private sel = new Set<number>();
  private primary: number | null = null;
  /** selected wires (yellow), keyed `to:input` (`edgeKey`): click / shift-click / box select; Delete disconnects them */
  private edgeSel = new Set<string>();
  private drag: Drag | null = null;
  private menu: HTMLElement | null = null;
  /** open TAB menu flyout (category submenu) */
  private sub: HTMLElement | null = null;
  /** "Empty Network / Press Tab to Add Nodes" overlay, shown while the graph has no nodes */
  private empty = document.createElement("div");
  /** box-select rectangle (host px) */
  private boxEl = Object.assign(document.createElement("div"), { className: "net-box" });
  /** context watermark top-right: "Objects" at /obj, "Geometry" inside */
  private mark = document.createElement("div");
  /** hovered-node info line at the bottom of the pane */
  private info = document.createElement("div");
  private infoPinned = false;
  /** output node of a click-started wire that follows the pointer until a connector is clicked (Esc cancels) */
  private sticky: number | null = null;
  /** a dot drop (Alt while wiring) is on its way to Rust: a second Alt waits for it */
  private dropping = false;
  /** last left press (real double-click detection) and when the last double-click ran (dedupes the native dblclick) */
  private lastDown: { t: number; x: number; y: number } | null = null;
  private lastDouble = -1e9;
  /** Network level. "obj" = Houdini /obj: the objects - Geometry objects (each its own SOP network, in Rust: `current`
   *  is the one the SOP commands edit) and Null objects (transform markers, editor state). "sop" = inside the current
   *  Geometry object: its SOP nodes. Object positions / flags / Nulls are editor state (not saved). */
  level: "obj" | "sop" = "sop";
  objects: ObjNode[] = [Network.firstGeo()];
  /** the current Geometry object (Rust's; the network the SOP commands edit and `graph` shows) */
  current = "geo1";
  private static firstGeo(): ObjNode { return { id: 0, kind: "geo", name: "geo1", net: null, pos: null, visible: true, selectable: true, at: [0, 0, 0], parent: null }; }
  /** the container shown at the object level (null = /obj itself); double-click a container = in, `u` = up */
  inside: string | null = null;
  /** sticky notes of each container's own network */
  private boxNotes = new Map<string, NoteView[]>();
  private objSelIds = new Set<number>();
  /** selected /obj parenting wires (yellow), by child object id: click / shift-click / box select; Delete unparents */
  private objWireSel = new Set<number>();
  /** /obj wire started by a CLICK on a ball: follows the pointer until the next click (`up` = off an input ball) */
  private objSticky: { up: boolean; id: number } | null = null;
  private nextObjId = 1;
  /** the current object's node (the one whose SOP network the graph shows: a Geometry object or a Null) */
  get geo(): ObjNode { return this.objects.find((o) => o.name === this.current) ?? (this.objects.find((o) => o.kind === "geo") as ObjNode); }
  /** the current object's name */
  get objName(): string { return this.current; }
  /** containers from /obj down to `name`'s container (outermost first) */
  private containersOf(name: string | null): string[] {
    const out: string[] = [];
    let at = name === null ? null : this.objects.find((o) => o.name === name)?.net ?? null;
    while (at !== null && !out.includes(at) && out.length < 1000) { out.unshift(at); at = this.objects.find((o) => o.name === at)?.net ?? null; }
    return out;
  }
  /** path bars: the containers down to where the editor is, plus the current object inside it (SOP level).
   *  `/obj/asset1/inner_geo1` = ["asset1", "inner_geo1"] */
  get objPath(): string[] {
    if (this.level === "sop") return [...this.containersOf(this.current), this.current];
    return this.inside === null ? [] : [...this.containersOf(this.inside), this.inside];
  }
  /** path-bar click: show the object level at depth `i` of `objPath` (-1 = /obj itself) */
  goToPath(i: number): void {
    const p = this.objPath;
    const target = i < 0 ? null : p[i] ?? null;
    if (this.level === "sop" && target === this.current) return;
    this.level = "obj"; // (setLevel would put us in the current object's container)
    this.inside = target;
    this.setLevel("obj", false);
  }
  /** the current Geometry object is selected at /obj (the pane shows it; its geometry is highlighted) */
  get objectSelected(): boolean { return this.level === "obj" && this.objSelIds.has(this.geo.id) && this.isShown(this.geo); }
  /** a Geometry object shows at /obj once placed / loaded / created, or (the first, implicit one) once it has nodes */
  private isShown(o: ObjNode): boolean { return o.kind !== "geo" || !!o.placed || (o.name === this.current && this.graph.nodes.length > 0); }
  /** the objects of the level on screen (/obj or the container dived into) */
  private get shownObjects(): ObjNode[] { return this.objects.filter((o) => this.isShown(o) && o.net === this.inside); }
  /** an object draws in the viewport when its display flag and every enclosing container's are on */
  private drawn(o: ObjNode): boolean {
    return o.visible && this.containersOf(o.name).every((c) => this.objects.find((q) => q.name === c)?.visible !== false);
  }
  /** visible Geometry objects other than the current one, inside containers too (the viewport draws them) */
  get otherVisibleGeos(): string[] { return this.objects.filter((o) => this.isShown(o) && o.kind === "geo" && o.name !== this.current && this.drawn(o)).map((o) => o.name); }
  /** Match the objects to Rust's list (after load / add / remove): new names get a node (shown; position and flags
   *  from the file), gone ones drop; kind, container and a container's notes follow Rust. */
  syncObjects(infos: ObjInfo[], current: string): void {
    const live = new Map(infos.map((i) => [i.name, i]));
    const gone = new Set(this.objects.filter((o) => !live.has(o.name)).map((o) => o.id));
    this.objects = this.objects.filter((o) => !gone.has(o.id));
    for (const o of this.objects) if (o.parent !== null && gone.has(o.parent)) o.parent = null;
    this.boxNotes.clear();
    for (const i of infos) {
      if (i.kind === "subnet") this.boxNotes.set(i.name, i.notes);
      const o = this.objects.find((q) => q.name === i.name);
      if (o) { o.kind = i.kind; o.net = i.net; continue; }
      this.objects.push({ id: this.nextObjId++, kind: i.kind, name: i.name, net: i.net, pos: i.pos ? { x: i.pos[0], y: i.pos[1] } : null, visible: i.display, selectable: i.selectable, at: [0, 0, 0], parent: null, placed: true });
    }
    this.current = current;
    if (this.inside !== null && !live.has(this.inside)) this.inside = null;
    this.objSelIds = new Set([...this.objSelIds].filter((id) => !gone.has(id)));
    this.render();
  }
  /** a Geometry object was renamed in Rust (shelf `box_object1` naming): keep its node, position and flags */
  renameGeo(old: string, name: string): void {
    const o = this.objects.find((q) => q.kind === "geo" && q.name === old);
    if (o) o.name = name;
    if (this.current === old) this.current = name;
    this.render();
  }
  /** a loaded multi-object scene: every Geometry object shows at /obj, empty ones too */
  placeAllGeos(): void {
    for (const o of this.objects) if (o.kind === "geo") o.placed = true;
  }
  /** File > New / Open: back to one implicit geo1 (Nulls dropped); `syncGeos` then adds a loaded file's objects */
  resetObjects(): void {
    this.objects = [Network.firstGeo()];
    this.current = "geo1";
    this.inside = null;
    this.objSelIds = new Set();
    this.objWireSel = new Set();
  }
  /** Make `o` the current Geometry object (select at /obj, double-click = `dive` inside): Rust swaps its network in. */
  private async enterGeo(o: ObjNode, dive: boolean): Promise<void> {
    if (o.name !== this.current) {
      try { await api.objectEnter(o.name); } catch (x) { this.fail(x); return; }
      this.current = o.name;
      await this.hooks.refresh().catch(this.fail);
    }
    if (dive) this.setLevel("sop");
  }
  /** TAB-menu node placement: a ghost box follows the pointer, a left click drops it there (Esc / right-click cancels) */
  private ghost: { el: SVGElement; done: (p: { x: number; y: number } | null) => void } | null = null;
  /** Ghost placement of a node box (Houdini TAB pick): resolves to the clicked graph point (box centre), null = cancelled. */
  private placeGhost(): Promise<{ x: number; y: number } | null> {
    this.cancelGhost();
    return new Promise((done) => {
      const el = s("rect", { class: "ghost", width: NODE_W, height: NODE_H, rx: 6 });
      this.root.append(el);
      this.ghost = { el, done };
      this.moveGhost();
      this.showInfo("Click to place the node · Esc cancels", true);
    });
  }
  private ghostCancelled = false;
  /** TAB menu > Geometry at /obj: ghost-place a new Geometry object (the hidden empty geo1 first, then geo2, geo3, ...),
   *  then select it (it becomes current) */
  private async placeGeo(): Promise<void> {
    const at = await this.placeGhost();
    if (!at || this.level !== "obj") return;
    let o = this.inside === null ? this.objects.find((q) => q.kind === "geo" && !this.isShown(q)) : undefined;
    if (!o) {
      let name: string;
      try { name = await api.objectAdd("geo1", "geo", this.inside); } catch (x) { this.fail(x); return; }
      o = { id: this.nextObjId++, kind: "geo", name, net: this.inside, pos: null, visible: true, selectable: true, at: [0, 0, 0], parent: null };
      this.objects.push(o);
    }
    o.pos = centred(at);
    o.placed = true;
    this.objSelIds = new Set([o.id]);
    this.render();
    await this.enterGeo(o, false);
    this.render();
    this.hooks.select(null);
    this.hooks.objects();
  }
  private moveGhost(): void {
    if (!this.ghost) return;
    const p = centred(this.mouse);
    this.ghost.el.setAttribute("x", String(p.x));
    this.ghost.el.setAttribute("y", String(p.y));
  }
  private cancelGhost(at: { x: number; y: number } | null = null): void {
    const g = this.ghost;
    if (!g) return;
    this.ghost = null;
    g.el.remove();
    this.showInfo("");
    g.done(at);
  }
  /** Null objects (viewport indicators: their `control` SOP drawn at `at`) with their selection state; `drawn` = display
   *  flag on, containers included */
  get nulls(): (ObjNode & { selected: boolean; drawn: boolean })[] {
    return this.objects.filter((o) => o.kind === "null").map((o) => ({ ...o, selected: this.level === "obj" && this.objSelIds.has(o.id), drawn: this.drawn(o) }));
  }
  /** add a Null object at viewport position `at` (placed from the shelf / TAB menu) in the container on screen; Rust
   *  makes its network (`control` + `point1`). It becomes the selection. `net` = its network position (TAB menu: under
   *  the cursor), else right of the shown objects */
  async addNull(at: [number, number, number], net?: { x: number; y: number }): Promise<ObjNode | null> {
    let name: string;
    try { name = await api.objectAdd("null1", "null", this.inside); } catch (x) { this.fail(x); return null; }
    const shown = this.shownObjects.filter((o) => o.pos);
    const pos = net ?? (shown.length ? { x: Math.max(...shown.map((o) => (o.pos as { x: number }).x)) + NODE_W + 200, y: Math.min(...shown.map((o) => (o.pos as { y: number }).y)) } : null);
    const o: ObjNode = { id: this.nextObjId++, kind: "null", name, net: this.inside, pos, visible: true, selectable: true, at, parent: null, placed: true };
    this.objects.push(o);
    this.objSelIds = new Set([o.id]);
    this.render();
    return o;
  }
  /** /obj Delete: cut the selected parenting wires and delete the selected objects with their networks (Rust; a
   *  container takes what is inside it along). The last object left is emptied instead (there is always a current
   *  network) and goes back to hidden until placed again. */
  private async deleteObjSelection(): Promise<void> {
    for (const o of this.objects) if (this.objWireSel.has(o.id)) o.parent = null;
    const sel = this.objects.filter((o) => this.objSelIds.has(o.id) && this.isShown(o));
    this.objSelIds = new Set();
    this.objWireSel = new Set();
    const geos = sel;
    if (geos.length) {
      try {
        for (const g of geos) await api.objectRemove(g.name).catch(() => {}); // already gone with its container
        const [infos, cur] = await api.objectsInfo();
        this.syncObjects(infos, cur);
        // the emptied last object stays as the hidden implicit one
        for (const g of geos) { const left = this.objects.find((o) => o.kind === "geo" && o.name === g.name); if (left) { left.placed = false; left.pos = null; left.parent = null; } }
      } catch (x) { this.fail(x); }
      await this.hooks.refresh().catch(this.fail);
    }
    this.render();
    this.hooks.select(null);
    this.hooks.objects();
  }
  /** switch level (double-click the object = in, `u` / the `obj` crumb = up); coming up out of an object shows the
   *  level it lives in (its container) */
  setLevel(l: "obj" | "sop", selectObject = false): void {
    this.cancelGhost();
    this.closeMenu(false);
    if (l === "obj" && this.level === "sop") this.inside = this.geo.net;
    this.level = l;
    this.objSelIds = l === "obj" && selectObject ? new Set([this.geo.id]) : new Set();
    this.objWireSel = new Set();
    this.objSticky = null;
    this.pending.setAttribute("d", "");
    this.hover = null;
    this.showInfo("");
    this.render();
    this.hooks.level(l);
  }
  /** the template-flagged node (Houdini: one per network; the viewport ghosts its geometry); editor state, not saved
   *  with the scene. Per object: switching objects keeps each one's */
  private template: number | null = null;
  private templateOf = new Map<string, number>();
  get templated(): number[] { return this.template === null ? [] : [this.template]; }
  private renaming: HTMLElement | null = null;
  /** pointer in graph coords / client px / over the pane? */
  private mouse = { x: 0, y: 0 };
  private over = false;
  private hover: number | null = null;
  /** names to re-select once the next graph arrives (ids change across undo/redo) */
  private pendingNames: string[] | null = null;
  constructor(private host: HTMLElement, private types: NodeType[], private hooks: NetHooks) {
    this.root.append(this.discs, this.wires, this.nodes, this.pending);
    this.svg.append(this.root);
    host.append(this.svg);
    currentNetwork = this;
    this.svg.addEventListener("mousedown", (e) => this.down(e));
    window.addEventListener("mousemove", (e) => this.moveEv(e));
    window.addEventListener("mouseup", (e) => this.up(e));
    host.addEventListener("mouseenter", () => (this.over = true));
    host.addEventListener("mouseleave", () => { this.over = false; this.hover = null; });
    // right-click: on a node = display flag toggle, on empty canvas = TAB menu at the cursor
    // (on the whole pane, not just the svg: overlays and any uncovered strip must never show the webview's own menu)
    host.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      // the right press that cancelled a ghost placement must not open the TAB menu as well
      if (this.ghostCancelled) { this.ghostCancelled = false; return; }
      if (this.menu?.contains(e.target as Node) || this.sub?.contains(e.target as Node)) return;
      const n = (e.target as Element).closest<SVGElement>("[data-node]");
      if (n) { if (!this.isDotId(Number(n.dataset.node))) this.toggleDisplay(Number(n.dataset.node)); }
      else if (!(e.target as Element).closest("[data-edge-to]")) this.openMenu(this.toGraph(e.clientX, e.clientY));
    });
    // a click anywhere outside the open menu / flyout closes it
    window.addEventListener("mousedown", (e) => {
      const t = e.target as Node;
      if (this.menu && !this.menu.contains(t) && !this.sub?.contains(t) && !this.svg.contains(t)) this.closeMenu(false);
    });
    this.empty.className = "net-empty";
    this.empty.append(Object.assign(document.createElement("div"), { textContent: "Empty Network" }), Object.assign(document.createElement("div"), { textContent: "Press Tab to Add Nodes" }));
    this.mark.className = "net-mark";
    this.info.className = "net-info";
    host.append(this.empty, this.mark, this.info, this.boxEl);
    // native dblclick only fires for synthetic events (the script driver): a real first click re-renders the node, so the
    // browser never sees two clicks on one element. Real double-clicks are detected in `down` instead (same handler, deduped).
    this.svg.addEventListener("dblclick", (e) => { if (performance.now() - this.lastDouble > 500) this.double(e); });
    host.addEventListener("wheel", (e) => { if (!(e.target as Element).closest("#tabmenu, .tm-sub")) this.wheel(e); }, { passive: false }); // whole pane zooms (not the open TAB menu, which scrolls)
    window.addEventListener("keydown", (e) => this.key(e));
    this.applyView();
  }
  /** Double-click: /obj object = dive inside; SOP name label = rename; SOP node = display. Hit-tested by POSITION as well,
   *  because the target of a real double-click may already have been replaced by a re-render. */
  private double(e: MouseEvent): void {
    const t = e.target as Element;
    const p = this.toGraph(e.clientX, e.clientY);
    if (this.level === "obj") {
      // double-click a Geometry object / Null = go inside its SOP network (it becomes current); a container = show the
      // objects inside it
      const g = this.shownObjects.find((o) => o.pos && nodesInBox([{ id: o.id, ...o.pos }], p, p).length);
      if (!g || t.closest("[data-objflag],[data-objinfo],[data-objsel]")) return;
      if (g.kind === "subnet") this.goInto(g.name);
      else void this.enterGeo(g, true);
      return;
    }
    // the name label sits right of the body: a double-click there (up to 160 px of label) renames
    const onName = this.graph.nodes.find((q) => !isDot(q) && p.x > q.x + NODE_W + 4 && p.x < q.x + NODE_W + 160 && p.y >= q.y && p.y <= q.y + NODE_H);
    const live = t.isConnected ? t.closest<SVGElement>("[data-node]") : null;
    if ((t.isConnected && t.closest("[data-name]")) || (!live && onName)) { this.startRename(live ? Number(live.dataset.node) : (onName as NodeView).id); return; }
    if (t.closest("[data-flag],[data-tmpl],[data-byp],[data-info]")) return; // flag strips / halo arcs toggle on each click, not on double
    const id = live ? Number(live.dataset.node) : nodeAt(this.graph.nodes, p);
    if (id === null || !Number.isFinite(id)) return;
    if (this.isDotId(id)) return; // a dot never carries the display flag
    this.hooks.display(id).catch((x) => this.hooks.error(msg(x)));
  }
  /** show the objects inside container `name` (Houdini: dive into a subnet / digital asset) */
  private goInto(name: string): void {
    this.inside = name;
    this.setLevel("obj", false);
    this.frameAll();
  }
  /** `u`: up one level (inside an object = the level it lives in; in a container = the level holding it) */
  private goUp(): void {
    if (this.level === "sop") { this.setLevel("obj", true); return; }
    if (this.inside === null) return;
    const from = this.objects.find((o) => o.name === this.inside);
    this.inside = from?.net ?? null;
    this.setLevel("obj", false);
    if (from) { this.objSelIds = new Set([from.id]); this.render(); }
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
    // the template flag belongs to the object on screen (ids are per network)
    const t = this.templateOf.get(this.current) ?? null, kept = t !== null && live.has(t) ? t : null;
    if (kept !== this.template) { this.template = kept; this.hooks.templates(this.templated); }
    if (kept === null) this.templateOf.delete(this.current);
    if (this.primary !== null && !this.sel.has(this.primary)) this.primary = this.sel.size ? [...this.sel][this.sel.size - 1] : null;
    const liveEdges = new Set(g.edges.map((x) => edgeKey(x.to, x.input)));
    this.edgeSel = new Set([...this.edgeSel].filter((k) => liveEdges.has(k)));
    this.markBroken();
    this.render();
  }
  selectNode(id: number | null): void {
    this.sel = id === null ? new Set() : new Set([id]);
    this.primary = id;
    this.edgeSel = new Set();
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
  private fail = (x: unknown): void => this.report(msg(x));
  /** status line + a toast in the pane (the status line is rewritten by every re-cook; the toast stays 5 s next to the action) */
  private report(m: string): void {
    this.hooks.error(m);
    this.host.querySelector("#nettoast")?.remove();
    const t = document.createElement("div");
    t.id = "nettoast";
    t.textContent = m;
    this.host.append(t);
    setTimeout(() => t.remove(), 5000);
  }
  /** run a command, re-sync the graph (which re-cooks and rewrites the status line), THEN show a rejection so it stays visible */
  private async cmd(f: () => Promise<unknown>): Promise<void> {
    let err: string | null = null;
    try { await f(); } catch (x) { err = msg(x); }
    await this.hooks.refresh().catch(this.fail);
    if (err) this.report(err);
  }
  render(): void {
    this.empty.hidden = this.graph.nodes.length > 0;
    this.mark.textContent = this.level === "obj" ? "Objects" : "Geometry";
    this.discs.replaceChildren();
    this.wires.replaceChildren();
    this.nodes.replaceChildren();
    if (this.level === "obj") { this.renderObjects(); return; }
    for (const b of this.graph.boxes ?? []) this.discs.append(this.netBoxEl(b));
    for (const n of this.graph.notes ?? []) this.discs.append(this.noteEl(n));
    // For-Each pairs: a dotted link from each Block End back to its Block Begin (its `begin` param)
    for (const [b, e] of blockPairs(this.graph.nodes, this.graph.edges)) {
      const a = this.node(b), z = this.node(e);
      if (!a || !z) continue;
      const x = Math.max(a.x, z.x) + NODE_W + 130;
      this.discs.append(s("path", { class: "blocklink", d: `M${a.x + NODE_W},${a.y + NODE_H / 2} H${x} V${z.y + NODE_H / 2} H${z.x + NODE_W}` }, s("title", {}, `For-Each block: ${a.name} … ${z.name}`)));
    }
    for (const e of this.graph.edges) {
      const a = this.node(e.from), b = this.node(e.to);
      if (!a || !b) continue;
      // a segment into / out of a dot runs straight; one running up the network carries a flow arrow
      const p = nodeOut(a), q = nodeIn(b, e.input), straight = isDot(a) || isDot(b);
      const d = straight ? linePath(p, q) : wirePath(p, q);
      const arrow = wireArrow(p, q, straight);
      const on = this.edgeSel.has(edgeKey(e.to, e.input));
      // a wire into an optional input (past the node's required ones) is dashed
      const opt = !isDot(b) && optionalInput(b, e.input);
      const g = s("g", { "data-edge-to": e.to, "data-edge-input": e.input }, s("path", { class: `edge${on ? " sel" : ""}${opt ? " opt" : ""}`, d }), s("path", { class: "edge-hit", d }));
      if (arrow) g.append(s("path", { class: on ? "warrow sel" : "warrow", d: arrow }));
      this.wires.append(g);
    }
    for (const n of this.graph.nodes) {
      const sel = this.sel.has(n.id);
      if (isDot(n)) {
        // network dot: a ball centred on (x, y) (grab ring around it); no flags, halo or name
        this.nodes.append(s("g", { class: `node dot${sel ? " sel" : ""}`, "data-node": n.id, transform: `translate(${n.x},${n.y})` },
          s("circle", { class: "dotgrab", r: DOT_R + 3 }),
          s("circle", { class: "dotbody", r: DOT_R }),
        ));
        continue;
      }
      const disp = this.graph.display === n.id;
      const tmpl = this.template === n.id;
      // user node colour (Houdini colour palette) tints the body; a bypassed node keeps its yellow
      const uc = this.graph.colors?.[n.name], ucol = uc && !n.bypass ? `rgb(${uc.map((c) => Math.round(Math.min(1, Math.max(0, c)) * 255)).join(",")})` : null;
      const g = s("g", { class: `node${sel ? " sel" : ""}${disp ? " disp" : ""}${tmpl ? " tmpl" : ""}${n.bypass ? " byp" : ""}`, "data-node": n.id, transform: `translate(${n.x},${n.y})` });
      // Houdini SOP node: display disc behind the display node (in the disc layer, under the wires) (blue = display /
      // render: one flag in hexe), hover halo (bypass arc top-left, info left, display eye top-right, template
      // bottom-right), grey body, flag strips (yellow = bypass on the left; purple = template, blue = display on the
      // right; click toggles), type icon centred, name to the right
      const cx = NODE_W / 2, cy = NODE_H / 2, r0 = NODE_W * 0.66, r1 = r0 + 26, mid = (r0 + r1) / 2;
      const at = (a: number): [number, number] => [cx + Math.cos((a * Math.PI) / 180) * mid, cy + Math.sin((a * Math.PI) / 180) * mid];
      const icon = s("svg", { class: "nicon", x: cx - 11, y: cy - 11, width: 22, height: 22, viewBox: "0 0 24 24" });
      icon.innerHTML = nodeIcon(n.type);
      const [ex, ey] = at(-42), [tx, ty] = at(42);
      const W = NODE_W, H = NODE_H;
      if (disp) {
        this.discs.append(s("g", { transform: `translate(${n.x},${n.y})` },
          s("circle", { class: "ddisc outer", cx, cy, r: NODE_W * 0.62 }),
          s("circle", { class: "ddisc inner", cx, cy, r: NODE_W * 0.44 }),
        ));
      }
      g.append(
        s("circle", { class: "halohit", cx, cy, r: r1 + 2 }), // keeps :hover alive across the gap to the arcs
        s("g", { class: "halo" },
          s("path", { class: "arc byparc", d: arcPath(cx, cy, r0, r1, -158, -118), "data-byp": n.id, "data-tip": "Bypass flag\nClick to bypass this node: it passes its first input through unchanged." }),
          s("path", { class: "arcglyph bglyph", d: bypassGlyph(cx + Math.cos((-138 * Math.PI) / 180) * mid, cy + Math.sin((-138 * Math.PI) / 180) * mid) }),
          s("path", { class: "arc info", d: arcPath(cx, cy, r0, r1, 160, 200), "data-info": n.id, "data-tip": "Node info" }),
          s("text", { class: "arcglyph", x: cx - mid, y: cy + 5, "text-anchor": "middle" }, "i"),
          s("path", { class: "arc disp", d: arcPath(cx, cy, r0, r1, -62, -22), "data-flag": n.id, "data-tip": "Display / Render flag\nClick to make this the node shown in the viewport (one per network)." }),
          s("path", { class: "arcglyph eye", d: eyePath(ex, ey) }),
          s("path", { class: "arc tmpl", d: arcPath(cx, cy, r0, r1, 22, 62), "data-tmpl": n.id, "data-tip": "Template / Selectable Template flag\nClick to show this node's geometry ghosted in the viewport (one per network)." }),
          s("text", { class: "arcglyph tglyph", x: tx, y: ty + 5, "text-anchor": "middle" }, "T"),
        ),
        // the null SOP is Houdini's bow-tie (left lobes = bypass, right = display); everything else the flagged box
        ...(n.type === "null"
          ? bowtie(`sbt${n.id}`, { left: ["bstrip", "data-byp", n.id, "Bypass flag"], right: ["dstrip", "data-flag", n.id, "Display / Render flag"] }, NULL_SOP_ICON)
          : [
            s("rect", { class: "shadow", x: 4, y: 4, width: W, height: H, rx: 4 }),
            s("rect", { class: "ring", x: -2, y: -2, width: W + 4, height: H + 4, rx: 6 }),
            s("rect", { class: "body", width: W, height: H, rx: 4, ...(ucol ? { style: `fill:${ucol}` } : {}) }),
            s("path", { class: "bstrip", d: `M4,0 H18 L14,${H} H4 a4,4 0 0 1 -4,-4 V4 a4,4 0 0 1 4,-4 Z`, "data-byp": n.id, "data-tip": "Bypass flag" }),
            s("path", { class: "tstrip", d: `M${W - 30},0 H${W - 16} L${W - 20},${H} H${W - 34} Z`, "data-tmpl": n.id, "data-tip": "Template / Selectable Template flag" }),
            s("path", { class: "dstrip", d: `M${W - 16},0 H${W - 4} a4,4 0 0 1 4,4 V${H - 4} a4,4 0 0 1 -4,4 H${W - 20} Z`, "data-flag": n.id, "data-tip": "Display / Render flag" }),
            icon,
          ]),
        s("text", { class: "nm", x: W + 10, y: cy + 6, "font-size": 16, "data-name": n.id }, n.name),
        s("circle", { class: "port out", cx: NODE_W / 2, cy: NODE_H, r: 4, "data-out": n.id }),
      );
      const missing = new Set(missingInputs(n, this.graph.edges));
      n.inputs.forEach((name, i) => {
        const [px] = inPort(0, 0, i, n.inputs.length);
        const need = !optionalInput(n, i);
        g.append(
          s("circle", { class: "port in", cx: px, cy: 0, r: 4, "data-in": n.id, "data-input": i }, s("title", {}, `${name}${need ? "" : " (optional)"}`)),
          s("text", { class: "pidx", x: px + 7, y: -9 }, String(i)),
        );
        // a required input with nothing wired: Houdini's small red arrow pointing down at the input ball
        if (missing.has(i)) g.append(s("path", { class: "needin", d: `M${px - 1.2},-19 h2.4 v5 h3 l-4.2,6.5 l-4.2,-6.5 h3 Z` }, s("title", {}, `${name}: required input not connected`)));
      });
      // broken (cook error, on a wired loop, or up to 2 nodes below one): Houdini's big red ! left of the node
      if (this.broken.has(n.id)) {
        const h = 54, w = 62, right = cx - r0 - 8, mx = right - w / 2, top = cy - h / 2;
        g.append(s("g", { class: "broken" }, s("title", {}, this.brokenWhy(n.id)),
          s("path", { class: "btri", d: roundedPath([[mx, top], [right, top + h], [right - w, top + h]], [6, 6, 6]) }),
          s("rect", { class: "bmark", x: mx - 4.5, y: top + 16, width: 9, height: 20, rx: 1.5 }),
          s("rect", { class: "bmark", x: mx - 4.5, y: top + 40, width: 9, height: 8, rx: 1.5 }),
        ));
      }
      this.nodes.append(g);
    }
  }
  /** /obj: every shown object. Geometry = white body, green selectable strip, blue display strip, "Geometry" over the name;
   *  Null = the bow-tie (green / blue lobes = the same flags). Halo = selectable arrow, info, display eye. */
  private renderObjects(): void {
    const W = NODE_W, H = NODE_H, cx = W / 2, cy = H / 2, r0 = W * 0.66, r1 = r0 + 26, mid = (r0 + r1) / 2;
    const ea = (-42 * Math.PI) / 180, sa = (-138 * Math.PI) / 180;
    const arrow = (x: number, y: number) => `M${x - 4},${y - 7} L${x + 5},${y + 1} L${x + 1},${y + 2} L${x + 4},${y + 8} L${x + 2},${y + 9} L${x - 1},${y + 3} L${x - 4},${y + 6} Z`;
    const shown = this.shownObjects;
    this.empty.hidden = shown.length > 0 || this.inside === null;
    if (this.inside !== null) for (const n of this.boxNotes.get(this.inside) ?? []) this.discs.append(this.noteEl(n));
    for (const o of shown) {
      if (!o.pos) {
        const r = this.svg.getBoundingClientRect();
        const c = r.width > 0 ? this.toGraph(r.left + r.width / 2, r.top + r.height / 2) : { x: 0, y: 0 };
        o.pos = centred(c);
      }
      const sel = this.objSelIds.has(o.id);
      const g = s("g", { class: `node objnode ${o.kind}${sel ? " sel" : ""}${o.visible ? " disp" : ""}${o.selectable ? " selable" : ""}`, "data-obj": o.id, transform: `translate(${o.pos.x},${o.pos.y})` });
      g.append(
        s("circle", { class: "halohit", cx, cy, r: r1 + 2 }), // keeps :hover alive across the gap to the arcs
        s("g", { class: "halo" },
          s("path", { class: "arc selarc", d: arcPath(cx, cy, r0, r1, -158, -118), "data-objsel": o.id, "data-tip": "Selectable flag\nClick to toggle whether this object can be picked in the viewport." }),
          s("path", { class: "arcglyph selglyph", d: arrow(cx + Math.cos(sa) * mid, cy + Math.sin(sa) * mid) }),
          s("path", { class: "arc info", d: arcPath(cx, cy, r0, r1, 160, 200), "data-objinfo": o.id, "data-tip": "Node info" }),
          s("text", { class: "arcglyph", x: cx - mid, y: cy + 5, "text-anchor": "middle" }, "i"),
          s("path", { class: "arc disp", d: arcPath(cx, cy, r0, r1, -62, -22), "data-objflag": o.id, "data-tip": "Display flag\nClick to show / hide this object in the viewport." }),
          s("path", { class: "arcglyph eye", d: eyePath(cx + Math.cos(ea) * mid, cy + Math.sin(ea) * mid) }),
        ),
      );
      if (o.kind === "geo" || o.kind === "subnet") {
        const icon = s("svg", { class: "nicon", x: cx - 12, y: cy - 12, width: 24, height: 24, viewBox: "0 0 24 24" });
        icon.innerHTML = o.kind === "geo" ? GEO_ICON : SUBNET_ICON;
        g.append(
          s("rect", { class: "shadow", x: 4, y: 4, width: W, height: H, rx: 4 }),
          s("rect", { class: "ring", x: -2, y: -2, width: W + 4, height: H + 4, rx: 6 }),
          s("rect", { class: "body", width: W, height: H, rx: 4 }),
          s("path", { class: "sstrip", d: `M4,0 H18 L14,${H} H4 a4,4 0 0 1 -4,-4 V4 a4,4 0 0 1 4,-4 Z`, "data-objsel": o.id, "data-tip": "Selectable flag" }),
          s("path", { class: "dstrip", d: `M${W - 16},0 H${W - 4} a4,4 0 0 1 4,4 V${H - 4} a4,4 0 0 1 -4,4 H${W - 20} Z`, "data-objflag": o.id, "data-tip": "Display flag" }),
          icon,
          s("text", { class: "ty", x: W + 10, y: cy - 4 }, o.kind === "geo" ? "Geometry" : "Subnetwork"),
        );
      } else {
        g.append(...bowtie(`obt${o.id}`, { left: ["sstrip", "data-objsel", o.id, "Selectable flag"], right: ["dstrip", "data-objflag", o.id, "Display flag"] }, NULL_OBJ_ICON));
      }
      g.append(
        s("text", { class: "onm", x: W + 10, y: o.kind !== "null" ? cy + 14 : cy + 7, "font-size": 18, "data-objname": o.id }, o.name),
        s("circle", { class: "port in", cx: W / 2, cy: o.kind !== "null" ? 0 : -10, r: 4, "data-objin": o.id }, s("title", {}, "Parent input")),
        s("circle", { class: "port out", cx: W / 2, cy: o.kind !== "null" ? H : H + 10, r: 4, "data-objout": o.id }, s("title", {}, "Output: drag onto an object to make it a child")),
      );
      this.nodes.append(g);
    }
    // parenting wires (after the loop: unplaced objects got their position above)
    for (const o of shown) {
      const p = o.parent === null ? undefined : shown.find((q) => q.id === o.parent);
      if (!p) continue;
      const d = wirePath(objOutPort(p), objInPort(o));
      this.wires.append(s("g", { "data-objwire": o.id }, s("path", { class: this.objWireSel.has(o.id) ? "edge sel" : "edge", d }), s("path", { class: "edge-hit", d })));
    }
  }
  /** bottom-of-pane info line (Houdini: `box1 (Box) node` under the hovered node); `pin` keeps it until the next click */
  private showInfo(text: string, pin = false): void {
    this.info.textContent = text;
    this.infoPinned = pin && text !== "";
  }
  /** display flag TOGGLE: the display node's flag clears it, any other node takes it */
  private async toggleDisplay(id: number): Promise<void> {
    await this.cmd(() => api.setDisplay(displayToggle(this.graph.display, id)));
  }
  private down(e: MouseEvent): void {
    // ghost placement: left click drops the node under the pointer, any other button cancels
    if (this.ghost) {
      e.preventDefault();
      this.ghostCancelled = e.button === 2;
      this.cancelGhost(e.button === 0 ? this.toGraph(e.clientX, e.clientY) : null);
      return;
    }
    this.closeMenu();
    this.infoPinned = false;
    (this.host as HTMLElement).focus();
    const t = e.target as Element;
    // real double-click: two left presses within 400 ms / 5 px (see the dblclick listener for why not the native event)
    if (e.button === 0) {
      const now = performance.now(), l = this.lastDown;
      this.lastDown = { t: now, x: e.clientX, y: e.clientY };
      if (l && now - l.t < 400 && Math.hypot(e.clientX - l.x, e.clientY - l.y) < 5 && this.sticky === null && this.objSticky === null) {
        this.lastDown = null;
        this.lastDouble = now;
        e.preventDefault();
        this.double(e);
        return;
      }
    }
    // middle mouse = pan the network from anywhere (over nodes too), at either level
    if (e.button === 1) {
      e.preventDefault();
      this.drag = { kind: "pan", sx: e.clientX, sy: e.clientY, vx: this.view.x, vy: this.view.y };
      return;
    }
    if (this.level === "obj") { this.downObj(e, t); return; }
    // sticky wire: the next left click finishes it on the connector / node under the pointer (nowhere = cancel)
    if (this.sticky !== null) {
      e.preventDefault();
      if (e.button !== 0) { this.cancelSticky(); return; }
      if (e.altKey) { this.dropDot(this.sticky, this.toGraph(e.clientX, e.clientY)); return; }
      const from = this.sticky;
      this.sticky = null;
      this.showInfo("");
      this.finishWire(from, e);
      return;
    }
    if (e.altKey && e.button === 0) {
      const dn = t.closest<SVGElement>("[data-node]"), ed = t.closest<SVGElement>("[data-edge-to]");
      if (dn && this.isDotId(Number(dn.dataset.node))) {
        e.preventDefault();
        const id = Number(dn.dataset.node);
        this.sel.delete(id);
        this.cmd(() => api.nodesRemove([id]));
        return;
      }
      if (ed) {
        e.preventDefault();
        const to = Number(ed.dataset.edgeTo), input = Number(ed.dataset.edgeInput), p = this.toGraph(e.clientX, e.clientY);
        const edge = this.graph.edges.find((x) => x.to === to && x.input === input);
        if (edge) this.cmd(() => api.dotAdd(p.x, p.y, edge.from, to, input));
        return;
      }
    }
    const flag = t.closest<SVGElement>("[data-flag]");
    if (flag && e.button === 0) { this.toggleDisplay(Number(flag.dataset.flag)); e.preventDefault(); return; }
    const byp = t.closest<SVGElement>("[data-byp]");
    if (byp && e.button === 0) {
      e.preventDefault();
      const n = this.node(Number(byp.dataset.byp));
      if (n) this.cmd(() => api.setBypass(n.id, !n.bypass));
      return;
    }
    const tf = t.closest<SVGElement>("[data-tmpl]");
    if (tf && e.button === 0) {
      e.preventDefault();
      const id = Number(tf.dataset.tmpl);
      // one template per network: another node takes it, the flagged one clears it
      this.template = this.template === id ? null : id;
      if (this.template === null) this.templateOf.delete(this.current); else this.templateOf.set(this.current, this.template);
      this.render();
      this.hooks.templates(this.templated);
      return;
    }
    // halo info arc: select the node and pin its info line
    const info = t.closest<SVGElement>("[data-info]");
    if (info && e.button === 0) {
      e.preventDefault();
      const id = Number(info.dataset.info), n = this.node(id);
      this.selectNode(id);
      if (n) this.showInfo(`${n.name} (${labelOf(n.type)}) node · #${n.id} · ${n.inputs.length} input${n.inputs.length === 1 ? "" : "s"}${this.graph.display === n.id ? " · display" : ""}`, true);
      return;
    }
    const out = t.closest<SVGElement>("[data-out]");
    if (out && e.button === 0) { this.drag = { kind: "wire", from: Number(out.dataset.out), sx: e.clientX, sy: e.clientY }; e.preventDefault(); return; }
    const inp = t.closest<SVGElement>("[data-in]");
    if (inp && e.button === 0) {
      // grab an existing wire off its input: re-drop it elsewhere (or nowhere = cut)
      const to = Number(inp.dataset.in), input = Number(inp.dataset.input);
      const edge = this.graph.edges.find((x) => x.to === to && x.input === input);
      if (edge) {
        this.drag = { kind: "wire", from: edge.from, sx: e.clientX, sy: e.clientY };
        this.hooks.disconnect(to, input).catch(this.fail);
      } else {
        // a free input: drag a wire up to the node that should feed it
        this.drag = { kind: "wireup", to, input };
        this.trackWireUp(to, input);
      }
      e.preventDefault();
      return;
    }
    // click on a node's name = select it + rename in place (Houdini)
    const nameEl = t.closest<SVGElement>("[data-name]");
    if (nameEl && e.button === 0 && !e.shiftKey) {
      e.preventDefault();
      const id = Number(nameEl.dataset.name);
      this.selectNode(id);
      this.startRename(id);
      return;
    }
    const nodeEl = t.closest<SVGElement>("[data-node]");
    if (nodeEl && e.button === 0) {
      const id = Number(nodeEl.dataset.node);
      const n = this.node(id);
      if (!n) return;
      this.edgeSel = new Set();
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
      // click = just this wire (nodes deselect); shift-click toggles it in the wire selection, nodes kept
      const k = edgeKey(Number(edgeEl.dataset.edgeTo), Number(edgeEl.dataset.edgeInput));
      if (e.shiftKey) { this.edgeSel = toggled(this.edgeSel, k); this.render(); return; }
      this.edgeSel = new Set([k]);
      this.sel = new Set();
      this.primary = null;
      this.render();
      this.hooks.select(null);
      return;
    }
    if (!e.shiftKey) {
      this.sel = new Set();
      this.primary = null;
      this.edgeSel = new Set();
      this.render();
      this.hooks.select(null);
    }
    this.drag = { kind: "box", p0: this.toGraph(e.clientX, e.clientY), add: e.shiftKey, base: new Set(this.sel) }; // background drag = box select (MMB pans)
  }
  /** /obj clicks: display strip / eye = object visibility, info arc = info line, body = select (+ drag), background = deselect + pan */
  private downObj(e: MouseEvent, t: Element): void {
    // a click-started wire ends on this click: on a ball / object = connect, anywhere else (or not the left button) = cancel
    if (this.objSticky) {
      e.preventDefault();
      const st = this.objSticky;
      this.objSticky = null;
      this.showInfo("");
      if (e.button === 0) this.finishObjWire(st, this.toGraph(e.clientX, e.clientY));
      else this.cancelObjSticky();
      return;
    }
    if (e.button !== 0) return;
    e.preventDefault();
    const obj = (attr: string) => { const el = t.closest(`[${attr}]`); return el ? this.objects.find((o) => o.id === Number(el.getAttribute(attr))) : undefined; };
    // output ball = drag a parenting wire; a parented input ball = grab its wire off (re-drop elsewhere, or nowhere = unparent)
    const out = obj("data-objout"), inp = obj("data-objin");
    if (out) { this.drag = { kind: "objwire", from: out.id, sx: e.clientX, sy: e.clientY }; this.trackObjWire(out.id); return; }
    if (inp && inp.parent !== null) {
      this.drag = { kind: "objwire", from: inp.parent, sx: e.clientX, sy: e.clientY };
      inp.parent = null;
      this.render();
      this.trackObjWire(this.drag.from);
      return;
    }
    if (inp) { this.drag = { kind: "objwireup", child: inp.id, sx: e.clientX, sy: e.clientY }; this.trackObjWireUp(inp.id); return; }
    // click a wire = select it (objects deselect); anything else drops the wire selection
    const wire = t.closest("[data-objwire]");
    if (wire) {
      const id = Number(wire.getAttribute("data-objwire"));
      if (e.shiftKey) { this.objWireSel = toggled(this.objWireSel, id); this.render(); return; }
      this.objWireSel = new Set([id]);
      this.objSelIds = new Set();
      this.render();
      this.hooks.select(null);
      this.hooks.objects();
      return;
    }
    if (this.objWireSel.size && !e.shiftKey) { this.objWireSel = new Set(); this.render(); }
    const named = obj("data-objname");
    if (named && !e.shiftKey) {
      this.objSelIds = new Set([named.id]);
      this.render();
      this.hooks.select(null);
      this.hooks.objects();
      this.startObjRename(named);
      return;
    }
    const flag = obj("data-objflag"), selb = obj("data-objsel"), info = obj("data-objinfo"), body = obj("data-obj");
    if (flag) { flag.visible = !flag.visible; this.render(); this.hooks.objects(); return; }
    if (selb) { selb.selectable = !selb.selectable; this.render(); this.hooks.objects(); return; }
    if (info) {
      this.objSelIds = new Set([info.id]);
      this.render();
      this.hooks.select(null);
      this.hooks.objects();
      const inner = this.objects.filter((o) => o.net === info.name).length;
      const what = info.kind === "geo" ? `(Geometry) node${info.name === this.current ? ` · ${this.graph.nodes.length} SOP node${this.graph.nodes.length === 1 ? "" : "s"} inside` : ""}`
        : info.kind === "subnet" ? `(Subnetwork) node · ${inner} object${inner === 1 ? "" : "s"} inside`
        : `(Null) node · at (${info.at.map((v) => +v.toFixed(3)).join(", ")})`;
      this.showInfo(`${info.name} ${what}`, true);
      return;
    }
    if (body?.pos) {
      if (e.shiftKey) this.objSelIds = toggled(this.objSelIds, body.id);
      else if (!this.objSelIds.has(body.id)) this.objSelIds = new Set([body.id]);
      this.render();
      this.hooks.select(null);
      this.hooks.objects();
      const p = this.toGraph(e.clientX, e.clientY);
      this.drag = { kind: "obj", id: body.id, dx: p.x - body.pos.x, dy: p.y - body.pos.y };
      // a single Geometry object picked becomes current: params, highlight and the viewport follow it
      if (body.kind === "geo" && !e.shiftKey) void this.enterGeo(body, false).then(() => this.hooks.objects());
      return;
    }
    if (this.objSelIds.size && !e.shiftKey) { this.objSelIds = new Set(); this.render(); this.hooks.select(null); this.hooks.objects(); }
    this.drag = { kind: "box", p0: this.toGraph(e.clientX, e.clientY), add: e.shiftKey, base: new Set(this.objSelIds) };
  }
  private moveEv(e: MouseEvent): void {
    this.mouse = this.toGraph(e.clientX, e.clientY);
    if (this.ghost) { this.moveGhost(); return; }
    if (this.drag?.kind === "box") { this.trackBox(this.drag); return; }
    if (this.level === "obj") {
      const d = this.drag;
      if (d?.kind === "obj") {
        const o = this.objects.find((q) => q.id === d.id);
        if (o) { o.pos = { x: this.mouse.x - d.dx, y: this.mouse.y - d.dy }; this.render(); }
      } else if (d?.kind === "objwire") this.trackObjWire(d.from);
      else if (d?.kind === "objwireup") this.trackObjWireUp(d.child);
      else if (!d && this.objSticky) (this.objSticky.up ? this.trackObjWireUp(this.objSticky.id) : this.trackObjWire(this.objSticky.id));
      else if (d?.kind === "pan") { this.view.x = d.vx + (e.clientX - d.sx); this.view.y = d.vy + (e.clientY - d.sy); this.applyView(); }
      if (this.over && !d) {
        const el = document.elementFromPoint(e.clientX, e.clientY)?.closest("[data-obj]");
        const o = el ? this.objects.find((q) => q.id === Number(el.getAttribute("data-obj"))) : undefined;
        if (!this.infoPinned) this.showInfo(o ? `${o.name} (${OBJ_KIND_LABEL[o.kind]}) node` : "");
      }
      return;
    }
    if (this.over && !this.drag) {
      const was = this.hover;
      this.hover = nodeAt(this.graph.nodes, this.mouse);
      if (this.hover !== was && !this.infoPinned) {
        const n = this.hover === null ? undefined : this.node(this.hover);
        this.showInfo(n ? `${n.name} (${labelOf(n.type)}) node` : "");
      }
    }
    const d = this.drag;
    if (!d && this.sticky !== null) { this.trackWire(this.sticky); return; }
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
    } else if (d.kind === "wire") {
      this.trackWire(d.from);
    } else if (d.kind === "wireup") {
      this.trackWireUp(d.to, d.input);
    }
  }
  /** pending wire from `from`'s output to the pointer; the input it would land on lights up */
  private trackWire(from: number): void {
    const a = this.node(from);
    const m: [number, number] = [this.mouse.x, this.mouse.y];
    if (a) this.pending.setAttribute("d", isDot(a) ? linePath(nodeOut(a), m) : wirePath(nodeOut(a), m));
    this.svg.querySelectorAll(".hot").forEach((p) => p.classList.remove("hot"));
    const hit = resolveDrop(this.graph.nodes, this.graph.edges, from, this.mouse, DROP_PX / this.view.k);
    if (hit) this.svg.querySelector(this.isDotId(hit.to) ? `[data-node="${hit.to}"]` : `[data-in="${hit.to}"][data-input="${hit.input}"]`)?.classList.add("hot");
  }
  /** `child.parent = parent`; a link that closes a loop first unhooks `parent` from its own parent */
  private parentObj(child: ObjNode, parent: number): void {
    const cut = this.objects.find((o) => o.id === objCycleCut(this.objects, parent, child.id));
    if (cut) cut.parent = null;
    child.parent = parent;
  }
  /** wire `from` -> input `input` of `to`. A wire that closes a loop is kept (Houdini parity): the loop's nodes and
   *  the two below them show the red ! (`broken`) and the cook reports the cycle */
  private connect(from: number, to: number, input: number): void {
    this.cmd(() => api.edgeConnect(from, to, input));
  }
  /** Houdini network box: a translucent backdrop in the box's colour behind its nodes, a title bar with the comment
   *  (e.g. "Ground cover"). Minimized = the title bar only. */
  private netBoxEl(b: BoxView): SVGElement {
    const rgb = `rgb(${b.color.map((c) => Math.round(Math.min(1, Math.max(0, c)) * 255)).join(",")})`;
    const head = 24, h = b.minimized ? head : Math.max(head, b.h);
    return s("g", { class: "netbox", transform: `translate(${b.x},${b.y})` },
      s("rect", { class: "netbox-body", width: Math.max(0, b.w), height: h, fill: rgb }),
      s("rect", { class: "netbox-head", width: Math.max(0, b.w), height: head, fill: rgb }),
      s("text", { class: "netbox-title", x: 8, y: 17 }, b.title || b.name),
      s("title", {}, `${b.title || b.name} (network box, ${b.nodes.length} node${b.nodes.length === 1 ? "" : "s"})`),
    );
  }
  /** Houdini sticky note: body in the note's colour, a darker title strip with the minimize bar, wrapped text cut with
   *  an ellipsis when it overflows (behind wires and nodes). Minimized = the strip only. */
  private noteEl(n: NoteView): SVGElement {
    const rgb = (k: number) => `rgb(${n.color.map((c) => Math.round(Math.min(1, Math.max(0, c)) * k * 255)).join(",")})`;
    const head = 18, h = n.minimized ? head : Math.max(head, n.h);
    const g = s("g", { class: "note", transform: `translate(${n.x},${n.y})` },
      s("rect", { class: "note-body", width: n.w, height: h, rx: 8, fill: rgb(1) }),
      s("path", { class: "note-head", d: `M0,${head} V8 a8,8 0 0 1 8,-8 H${n.w - 8} a8,8 0 0 1 8,8 V${head} Z`, fill: rgb(1.08) }),
      s("rect", { class: "note-min", x: 10, y: head / 2 - 1.5, width: 16, height: 3 }),
      s("title", {}, n.text),
    );
    if (!n.minimized) {
      const fo = s("foreignObject", { x: 6, y: head + 4, width: Math.max(0, n.w - 12), height: Math.max(0, h - head - 8) });
      const div = Object.assign(document.createElement("div"), { className: "note-text", textContent: n.text });
      div.style.setProperty("--lines", String(Math.max(1, Math.floor((h - head - 8) / (17 * 1.35))))); // 17 px / 1.35 = .note-text
      fo.append(div);
      g.append(fo);
    }
    return g;
  }
  /** red ! marks: node id -> hops below the nearest root (0 = the root itself: failed cook or on a wired loop) */
  private broken = new Map<number, number>();
  private loops = new Set<number>();
  private cookError: { node: number; text: string } | null = null;
  /** the display cook failed on `node` (null = cooked fine): it and the 2 nodes below it get the red ! */
  setCookError(node: number | null, text = ""): void {
    this.cookError = node === null ? null : { node, text };
    this.markBroken();
    this.render();
  }
  private markBroken(): void {
    const g = this.graph;
    this.loops = loopNodes(g.edges);
    const live = new Set(g.nodes.map((n) => n.id));
    const roots = [...this.loops, ...(this.cookError && live.has(this.cookError.node) ? [this.cookError.node] : [])];
    this.broken = brokenNodes(g.edges, roots, (id) => this.isDotId(id));
  }
  /** tooltip on the red ! */
  private brokenWhy(id: number): string {
    if (this.loops.has(id)) return "Error: this node is part of a wired loop (its output feeds back into its own input).";
    if (this.cookError?.node === id) return `Error: ${this.cookError.text}`;
    return "Error: a node above this one has an error.";
  }
  /** end an /obj wire at `p`: `st.up = false` = `st.id` becomes the parent of what is there; `true` = `st.id` takes it as parent */
  private finishObjWire(st: { up: boolean; id: number }, p: { x: number; y: number }): void {
    this.pending.setAttribute("d", "");
    this.svg.querySelectorAll(".hot").forEach((q) => q.classList.remove("hot"));
    if (!st.up) {
      const to = objParentDrop(this.shownObjects, st.id, p, DROP_PX / this.view.k);
      const child = to === null ? undefined : this.objects.find((o) => o.id === to);
      if (child) this.parentObj(child, st.id);
    } else {
      const parent = objChildDrop(this.shownObjects, st.id, p, DROP_PX / this.view.k);
      const child = this.objects.find((o) => o.id === st.id);
      if (child && parent !== null) this.parentObj(child, parent);
    }
    this.objWireSel = new Set([...this.objWireSel].filter((id) => this.objects.find((o) => o.id === id)?.parent != null));
    this.render();
    this.hooks.objects();
  }
  private cancelObjSticky(): void {
    this.objSticky = null;
    this.pending.setAttribute("d", "");
    this.svg.querySelectorAll(".hot").forEach((q) => q.classList.remove("hot"));
    this.showInfo("");
    this.render();
    this.hooks.objects(); // a wire grabbed off an input stays cut
  }
  /** pending /obj parenting wire from `from`'s output ball to the pointer; the object it would land on lights its input */
  private trackObjWire(from: number): void {
    const a = this.objects.find((o) => o.id === from);
    if (a) this.pending.setAttribute("d", wirePath(objOutPort(a), [this.mouse.x, this.mouse.y]));
    this.svg.querySelectorAll(".hot").forEach((p) => p.classList.remove("hot"));
    const hit = objParentDrop(this.shownObjects, from, this.mouse, DROP_PX / this.view.k);
    if (hit !== null) this.svg.querySelector(`[data-objin="${hit}"]`)?.classList.add("hot");
  }
  /** pending wire from the pointer down to `child`'s input ball; the object it would parent to lights its output */
  private trackObjWireUp(child: number): void {
    const c = this.objects.find((o) => o.id === child);
    if (c) this.pending.setAttribute("d", wirePath([this.mouse.x, this.mouse.y], objInPort(c)));
    this.svg.querySelectorAll(".hot").forEach((p) => p.classList.remove("hot"));
    const hit = objChildDrop(this.shownObjects, child, this.mouse, DROP_PX / this.view.k);
    if (hit !== null) this.svg.querySelector(`[data-objout="${hit}"]`)?.classList.add("hot");
  }
  /** pending wire from the pointer down to input `input` of `to`; the node that would feed it lights its output */
  private trackWireUp(to: number, input: number): void {
    const b = this.node(to);
    const m: [number, number] = [this.mouse.x, this.mouse.y];
    if (b) this.pending.setAttribute("d", wirePath(m, nodeIn(b, input)));
    this.svg.querySelectorAll(".hot").forEach((p) => p.classList.remove("hot"));
    const hit = resolveDropUp(this.graph.nodes, to, this.mouse, DROP_PX / this.view.k);
    if (hit !== null) this.svg.querySelector(this.isDotId(hit) ? `[data-node="${hit}"]` : `[data-out="${hit}"]`)?.classList.add("hot");
  }
  /** box select in progress: thin rectangle, touched nodes get the pending (pink) outline */
  private trackBox(d: { p0: { x: number; y: number } }): void {
    const a = graphToHost(d.p0.x, d.p0.y, this.view), b = graphToHost(this.mouse.x, this.mouse.y, this.view);
    const st = this.boxEl.style;
    st.display = "block";
    st.left = `${Math.min(a.x, b.x)}px`; st.top = `${Math.min(a.y, b.y)}px`;
    st.width = `${Math.abs(b.x - a.x)}px`; st.height = `${Math.abs(b.y - a.y)}px`;
    const hits = new Set(this.boxHits(d.p0, this.mouse));
    const attr = this.level === "obj" ? "data-obj" : "data-node";
    for (const g of this.nodes.querySelectorAll<SVGElement>(`[${attr}]`)) g.classList.toggle("pre", hits.has(Number(g.getAttribute(attr))));
    const wires = new Set(this.wireHits(d.p0, this.mouse).map(([g]) => g));
    for (const g of this.wires.children) g.querySelector(".edge")?.classList.toggle("pre", wires.has(g));
  }
  /** nodes (or, at /obj, object ids) touched by the box */
  private boxHits(a: { x: number; y: number }, b: { x: number; y: number }): number[] {
    if (this.level === "obj") return nodesInBox(this.shownObjects.flatMap((o) => (o.pos ? [{ id: o.id, ...o.pos }] : [])), a, b);
    return nodesInBox(this.graph.nodes, a, b);
  }
  /** wires the box touches, as drawn (each curve sampled along its path, graph coords): [element, selection key]
   *  with key = `edgeKey` (SOP) or the child object id (/obj) */
  private wireHits(a: { x: number; y: number }, b: { x: number; y: number }): [Element, string | number][] {
    const out: [Element, string | number][] = [];
    for (const g of this.wires.children) {
      const path = g.querySelector<SVGPathElement>(".edge");
      if (!path) continue;
      const len = path.getTotalLength(), n = Math.max(2, Math.min(48, Math.ceil(len / 20)));
      const pts: [number, number][] = [];
      for (let i = 0; i <= n; i++) { const p = path.getPointAtLength((len * i) / n); pts.push([p.x, p.y]); }
      if (!polylineInBox(pts, a, b)) continue;
      const ow = g.getAttribute("data-objwire");
      if (ow !== null) out.push([g, Number(ow)]);
      else out.push([g, edgeKey(Number(g.getAttribute("data-edge-to")), Number(g.getAttribute("data-edge-input")))]);
    }
    return out;
  }
  private up(e: MouseEvent): void {
    const d = this.drag;
    this.drag = null;
    if (d?.kind === "box") {
      this.boxEl.style.display = "none";
      const p1 = this.toGraph(e.clientX, e.clientY), hits = this.boxHits(d.p0, p1);
      // wires the box touches join the wire selection (the press already cleared it unless shift was held)
      const wires = this.wireHits(d.p0, p1).map(([, k]) => k);
      if (this.level === "obj") {
        this.objSelIds = new Set([...(d.add ? d.base : []), ...hits]);
        this.objWireSel = new Set([...this.objWireSel, ...wires.filter((k): k is number => typeof k === "number")]);
        this.render();
        this.hooks.select(null);
        this.hooks.objects();
        return;
      }
      if (!hits.length && !wires.length && !d.add) return; // a plain click on the background already cleared the selection
      this.sel = new Set([...(d.add ? d.base : []), ...hits]);
      this.primary = hits.length ? hits[hits.length - 1] : this.primary;
      this.edgeSel = new Set([...this.edgeSel, ...wires.filter((k): k is string => typeof k === "string")]);
      this.render();
      this.hooks.select(this.selectedNode);
      return;
    }
    if (d?.kind === "objwire" || d?.kind === "objwireup") {
      const st = d.kind === "objwire" ? { up: false, id: d.from } : { up: true, id: d.child };
      // a click (no travel) on a ball: the wire follows the pointer until the next click
      if (Math.hypot(e.clientX - d.sx, e.clientY - d.sy) < 4) {
        this.objSticky = st;
        this.showInfo("Click a node to finish wire, or press ESC to cancel", true);
        return;
      }
      this.finishObjWire(st, this.toGraph(e.clientX, e.clientY));
      return;
    }
    if (!d || d.kind === "obj") return;
    if (d.kind === "move") {
      if (d.moved) {
        const moves = d.ids.flatMap((id): [number, number, number][] => { const n = this.node(id); return n ? [[id, n.x, n.y]] : []; });
        api.nodesMove(moves).catch(this.fail);
      } else if (this.isDotId(d.click)) {
        // a click on a dot: a wire out of it follows the cursor until the next click (nothing there = cancel)
        this.sticky = d.click;
        this.trackWire(d.click);
        this.showInfo("Click a connector to finish wire, Alt-click to drop a dot, ESC to cancel", true);
      } else if (this.sel.size > 1) {
        // plain click on a member of a multi-selection collapses to that node
        this.sel = new Set([d.click]);
        this.primary = d.click;
        this.render();
        this.hooks.select(d.click);
      }
    } else if (d.kind === "wireup") {
      this.pending.setAttribute("d", "");
      this.svg.querySelectorAll(".hot").forEach((p) => p.classList.remove("hot"));
      const from = resolveDropUp(this.graph.nodes, d.to, this.toGraph(e.clientX, e.clientY), DROP_PX / this.view.k);
      if (from !== null) this.connect(from, d.to, d.input);
    } else if (d.kind === "wire") {
      // a click (no travel) on the output starts a STICKY wire: it follows the cursor until a connector is clicked / Esc
      if (Math.hypot(e.clientX - d.sx, e.clientY - d.sy) < 4) {
        this.sticky = d.from;
        this.showInfo("Click a connector to finish wire, Alt-click to drop a dot, ESC to cancel", true);
        return;
      }
      if (e.altKey && !resolveDrop(this.graph.nodes, this.graph.edges, d.from, this.toGraph(e.clientX, e.clientY), DROP_PX / this.view.k)) {
        this.sticky = d.from;
        this.dropDot(d.from, this.toGraph(e.clientX, e.clientY));
        return;
      }
      this.finishWire(d.from, e);
    }
  }
  private isDotId(id: number): boolean {
    const n = this.node(id);
    return !!n && isDot(n);
  }
  /** Alt while drawing a wire from `from`: a dot at `at` fed by it (one undo step); the wire carries on out of the dot
   *  (a held drag keeps dragging, a sticky wire keeps following). Dropping on a connector is a normal connect instead. */
  private dropDot(from: number, at: { x: number; y: number }): void {
    if (this.dropping) return;
    this.dropping = true;
    api.dotAdd(at.x, at.y, from)
      .then(async (id) => {
        await this.hooks.refresh();
        if (this.drag?.kind === "wire" && this.drag.from === from) this.drag.from = id;
        else if (this.sticky === from) this.sticky = id;
        else return;
        this.trackWire(id);
        this.showInfo("Click a connector to finish wire, Alt-click to drop a dot, ESC to cancel", true);
      })
      .catch(this.fail)
      .finally(() => (this.dropping = false));
  }
  /** end a wire at the pointer: nearest free input there (or the node body) connects, nowhere = cancel */
  private finishWire(from: number, e: MouseEvent): void {
    this.pending.setAttribute("d", "");
    this.svg.querySelectorAll(".hot").forEach((p) => p.classList.remove("hot"));
    const p = this.toGraph(e.clientX, e.clientY);
    const hit = resolveDrop(this.graph.nodes, this.graph.edges, from, p, DROP_PX / this.view.k);
    if (hit) this.connect(from, hit.to, hit.input);
    else this.hooks.refresh().catch(this.fail); // dropped nowhere: re-sync (a grabbed wire stays cut)
  }
  private cancelSticky(): void {
    if (this.sticky === null) return;
    this.sticky = null;
    this.pending.setAttribute("d", "");
    this.svg.querySelectorAll(".hot").forEach((p) => p.classList.remove("hot"));
    this.showInfo("");
    this.hooks.refresh().catch(this.fail);
  }
  private wheel(e: WheelEvent): void {
    e.preventDefault();
    const p = this.toGraph(e.clientX, e.clientY);
    const k = Math.min(8, Math.max(0.05, this.view.k * Math.exp(-e.deltaY * 0.0015))); // zoom about the cursor
    const r = this.svg.getBoundingClientRect();
    this.view = { k, x: e.clientX - r.left - p.x * k, y: e.clientY - r.top - p.y * k };
    this.applyView();
  }
  /** `F`: pan + zoom so every node and sticky note (or, at /obj, every object) is in view, names included (Houdini frame all) */
  frameAll(): void {
    const NAME_W = 110; // node names are drawn right of the body
    const rects: [number, number, number, number][] = this.level === "obj"
      ? this.shownObjects.flatMap((o): [number, number, number, number][] => (o.pos ? [[o.pos.x, o.pos.y - 10, o.pos.x + NODE_W + NAME_W, o.pos.y + NODE_H + 10]] : []))
      : [...this.graph.nodes.map((n): [number, number, number, number] => { const r = nodeRect(n); return isDot(n) ? r : [r[0], r[1], r[2] + NAME_W, r[3]]; }),
        ...(this.graph.notes ?? []).map((n): [number, number, number, number] => [n.x, n.y, n.x + n.w, n.y + (n.minimized ? 18 : n.h)])];
    const r = this.svg.getBoundingClientRect(), v = fitView(rects, r.width, r.height);
    if (!v) return;
    this.view = v;
    this.applyView();
  }
  private key(e: KeyboardEvent): void {
    const t = e.target as HTMLElement;
    const typing = (t.tagName === "INPUT" && !t.matches('[type="checkbox"],[type="range"],[type="button"]')) || t.tagName === "SELECT" || t.tagName === "TEXTAREA";
    if (this.menu || this.renaming || typing) return;
    const mod = e.metaKey || e.ctrlKey;
    const k = e.key.toLowerCase();
    if (e.key === "Alt" && this.level === "sop" && !e.repeat) {
      const from = this.drag?.kind === "wire" ? this.drag.from : this.sticky;
      if (from !== null) {
        e.preventDefault(); // keep the webview / native menu bar from taking the Alt press
        this.dropDot(from, this.mouse);
        return;
      }
    }
    if (e.key === "Escape" && this.ghost) { e.preventDefault(); this.cancelGhost(); return; }
    if (e.key === "Escape" && this.objSticky) {
      e.preventDefault();
      this.cancelObjSticky();
      return;
    }
    if (e.key === "Escape" && this.sticky !== null) {
      e.preventDefault();
      this.cancelSticky();
    } else if (e.key === "Tab") {
      e.preventDefault();
      this.openMenu();
    } else if (!mod && !e.altKey && k === "f" && this.over) {
      e.preventDefault();
      this.frameAll();
    } else if (!mod && k === "u" && this.over && (this.level === "sop" || this.inside !== null)) {
      e.preventDefault();
      this.goUp(); // Houdini `u`: up one level
    } else if (this.level === "obj" && !mod) {
      // Delete / Backspace removes the selected objects and cuts the selected parenting wires; the other SOP keys
      // (D, F2) do nothing at /obj
      if ((e.key === "Delete" || e.key === "Backspace") && (this.objSelIds.size || this.objWireSel.size)) {
        e.preventDefault();
        void this.deleteObjSelection();
      }
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
      if (id !== null && !this.isDotId(id)) this.toggleDisplay(id);
    } else if (e.key === "F2" && this.selectedNode !== null) {
      e.preventDefault();
      this.startRename(this.selectedNode);
    } else if ((e.key === "Delete" || e.key === "Backspace") && !mod) {
      // selected nodes and selected wires go together; a wire into a node being removed goes with the node
      if (this.sel.size || this.edgeSel.size) {
        e.preventDefault();
        const ids = [...this.sel];
        const cuts = this.graph.edges.filter((x) => this.edgeSel.has(edgeKey(x.to, x.input)) && !this.sel.has(x.to) && !this.sel.has(x.from));
        this.edgeSel = new Set();
        this.cmd(async () => {
          for (const x of cuts) await api.edgeDisconnect(x.to, x.input);
          if (ids.length) await api.nodesRemove(ids);
        });
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
    this.edgeSel = new Set();
    await this.hooks.refresh();
  }
  /** inline rename box over the node's name; Enter / blur commits (unique names enforced in Rust), Esc cancels */
  /** F2 / click on a node's name: rename it in place (unique names enforced in Rust) */
  startRename(id: number): void {
    const n = this.node(id);
    if (!n || isDot(n)) return;
    this.renameBox(n.x + NODE_W + 6, n.y + NODE_H / 2, n.name, labelOf(n.type), (name) => this.cmd(() => api.nodeRename(id, name)));
  }
  /** click on an object's name at /obj: rename it (editor state; the Geometry object's name is the `/obj/<name>` path) */
  private startObjRename(o: ObjNode): void {
    if (!o.pos) return;
    const taken = new Set(this.objects.filter((q) => q.id !== o.id).map((q) => q.name));
    const done = () => { this.render(); this.hooks.objects(); this.hooks.level(this.level); }; // path bars show the new name
    this.renameBox(o.pos.x + NODE_W + 6, o.pos.y + NODE_H / 2, o.name, OBJ_KIND_LABEL[o.kind], (name) => {
      // every object's name is its Rust object (numbered up there when taken, same rule)
      const old = o.name;
      api.objectRename(old, uniqueName(name, taken)).then((n) => {
        o.name = n;
        if (this.current === old) this.current = n;
        done();
      }).catch(this.fail);
    });
  }
  /** Houdini rename box: the type in grey above, the name field over the node's name (left edge at graph `x`, centred
   *  on `y`). Enter / blur commits a changed, non-empty name; Esc cancels. */
  private renameBox(x: number, y: number, value: string, type: string, commit: (name: string) => void): void {
    this.renaming?.remove();
    const at = graphToHost(x, y, this.view);
    const box = document.createElement("div");
    box.id = "renamebox";
    box.style.left = `${at.x}px`;
    box.style.top = `${at.y}px`;
    const input = document.createElement("input");
    input.value = value;
    input.spellcheck = false;
    input.size = Math.max(10, value.length + 4);
    box.append(Object.assign(document.createElement("div"), { className: "rn-type", textContent: type }), input);
    this.host.append(box);
    this.renaming = box;
    let done = false;
    const finish = (ok: boolean) => {
      if (done) return;
      done = true;
      const name = input.value.trim();
      box.remove();
      this.renaming = null;
      this.host.focus();
      if (ok && name && name !== value) commit(name);
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
  /** TAB menu (Tab key, or right-click on empty canvas): title, search box, category submenus (hover = flyout), All, History.
   *  Typing switches to a flat ranked result list (arrows + Enter). The node lands under the cursor (pane centre if elsewhere). */
  openMenu(at?: { x: number; y: number }): void {
    this.closeMenu(false);
    const r = this.host.getBoundingClientRect();
    const where = at ?? (this.over ? { ...this.mouse } : this.toGraph(r.left + r.width / 2, r.top + r.height / 2));
    const m = graphToHost(where.x, where.y, this.view);
    const W = 230;
    const mk = <K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] => {
      const e = document.createElement(tag);
      if (cls) e.className = cls;
      if (text) e.textContent = text;
      return e;
    };
    const box = mk("div");
    box.id = "tabmenu";
    const search = mk("div", "tm-search");
    const input = mk("input");
    input.placeholder = "(Type to search)";
    input.spellcheck = false;
    search.append(s("svg", { viewBox: "0 0 16 16", class: "tm-mag" }, s("circle", { cx: 6.5, cy: 6.5, r: 4.5 }), s("path", { d: "M10 10l4.5 4.5" })), input);
    const browse = mk("div", "tm-browse");
    const results = mk("ul", "tm-list tm-results");
    box.append(mk("div", "tm-title", "TAB Menu"), search, browse, results);
    this.host.append(box);
    this.menu = box;
    // /obj = Houdini's OBJ context: object types (all OBJ categories listed), own history; else SOP types
    const obj = this.level === "obj", label = obj ? objLabelOf : labelOf, histKey = obj ? HIST_KEY_OBJ : HIST_KEY;
    const all = obj ? OBJ_TYPES : this.types.map((t) => t.type).filter((t) => t !== "dot"); // dots come from Alt while wiring
    let history = obj ? tabHistoryObj : tabHistory;
    // one wire selected (SOP level): the picked node is spliced into it (Houdini TAB on a wire)
    const sw = !obj && this.edgeSel.size === 1 ? [...this.edgeSel][0] : null;
    const into = sw === null ? undefined : this.graph.edges.find((x) => edgeKey(x.to, x.input) === sw);
    const pick = (t: string | undefined) => {
      this.closeMenu();
      if (!t) return;
      history = pushHistory(history, t);
      if (obj) tabHistoryObj = history; else tabHistory = history;
      try { localStorage.setItem(histKey, JSON.stringify(history)); } catch { /* storage off: history is session-only */ }
      const p = centred(where);
      // hexe has ONE Geometry object (the whole SOP graph): picking Geometry goes inside it; a Null lands under the cursor
      if (obj) {
        if (t === "geo") void this.placeGeo();
        else void this.addNull([0, 0, 0], p).then((o) => { if (o) this.hooks.objects(); });
        return;
      }
      if (into) this.edgeSel = new Set();
      this.hooks.add(t, p.x, p.y, into && { from: into.from, to: into.to, input: into.input }).catch(this.fail);
    };
    const item = (t: string): HTMLLIElement => {
      const li = mk("li", "tm-item", label(t));
      li.dataset.type = t;
      li.title = t;
      li.addEventListener("mousedown", (ev) => { ev.preventDefault(); ev.stopPropagation(); pick(t); });
      return li;
    };
    // flyout submenu: a window-level layer (not clipped by the pane) beside the menu with a small gap, so it never covers
    // the menu; it cascades the same way as the menu (right of it, or left when the menu itself opened left of the cursor)
    // and only takes the other side when the window has no room
    const GAP = 4;
    const fitsRight = m.x + GAP + W <= r.width - 2;
    const flyout = (row: HTMLElement, ts: string[]) => {
      this.sub?.remove();
      const sub = mk("ul", "tm-list tm-sub");
      sub.append(...(ts.length ? ts.map(item) : [mk("li", "tm-none", "(no entries)")]));
      document.body.append(sub);
      this.sub = sub;
      for (const li of browse.querySelectorAll(".tm-cat.on")) li.classList.remove("on");
      row.classList.add("on");
      const br = box.getBoundingClientRect(), rr = row.getBoundingClientRect(), sw = sub.offsetWidth, sh = sub.offsetHeight;
      const right = br.right + GAP, left = br.left - GAP - sw;
      const x = fitsRight ? (right + sw <= window.innerWidth ? right : Math.max(0, left)) : left >= 0 ? left : right;
      const y = Math.min(Math.max(0, rr.top - 4), Math.max(0, window.innerHeight - sh));
      sub.style.left = `${x}px`;
      sub.style.top = `${y}px`;
    };
    const cat = (label: string, ts: string[]): HTMLLIElement => {
      const li = mk("li", "tm-cat", label);
      li.append(mk("span", "tm-arrow", "▶"));
      // open on real pointer MOVEMENT over the row (not the mouseenter a freshly opened menu gets under a still cursor), after a short hover
      let timer = 0;
      li.addEventListener("mousemove", () => { if (this.sub && li.classList.contains("on")) return; clearTimeout(timer); timer = window.setTimeout(() => { if (this.menu && li.matches(":hover")) flyout(li, ts); }, 120); });
      li.addEventListener("mouseleave", () => clearTimeout(timer));
      li.addEventListener("mousedown", (ev) => { ev.preventDefault(); ev.stopPropagation(); flyout(li, ts); });
      return li;
    };
    const cats = mk("ul", "tm-list");
    cats.append(...(obj ? groupedObj(all) : grouped(all)).map(([c, ts]) => cat(c, ts)));
    const allList = mk("ul", "tm-list");
    allList.append(cat("All", [...all].sort((a, b) => label(a).localeCompare(label(b)))));
    const hist = mk("ul", "tm-list tm-hist");
    const live = history.filter((t) => all.includes(t));
    if (live.length) hist.append(...live.map(item));
    else hist.append(mk("li", "tm-none", "(no entries)"));
    browse.append(cats, mk("div", "tm-sep"), allList, mk("div", "tm-head", "History"), hist);
    // hovering a plain history row closes any open flyout
    for (const li of hist.querySelectorAll("li")) li.addEventListener("mouseenter", () => { this.sub?.remove(); this.sub = null; });

    let shown: string[] = [], on = 0;
    const paint = () => {
      const q = input.value.trim();
      box.classList.toggle("searching", q !== "");
      if (!q) return;
      this.sub?.remove();
      this.sub = null;
      // labels match too: `transform` finds Transform (type `xform`), `geom` finds Geometry (type `geo`)
      shown = searchTypes(all, q, label);
      on = Math.min(on, Math.max(0, shown.length - 1));
      results.replaceChildren(...(shown.length ? shown.map((t, i) => { const li = item(t); if (i === on) li.classList.add("on"); return li; }) : [mk("li", "tm-none", "(no matches)")]));
      results.querySelector(".on")?.scrollIntoView({ block: "nearest" });
    };
    input.addEventListener("input", () => { on = 0; paint(); });
    input.addEventListener("keydown", (ev) => {
      ev.stopPropagation();
      if (ev.key === "Escape" || ev.key === "Tab") { ev.preventDefault(); this.closeMenu(); }
      else if (ev.key === "Enter") { ev.preventDefault(); pick(shown[on]); }
      else if (ev.key === "ArrowDown") { ev.preventDefault(); on = Math.min(on + 1, shown.length - 1); paint(); }
      else if (ev.key === "ArrowUp") { ev.preventDefault(); on = Math.max(on - 1, 0); paint(); }
    });
    // place after layout: keep the whole menu inside the pane
    box.style.width = `${W}px`;
    box.style.maxHeight = `${Math.max(120, r.height - 4)}px`;
    const h = box.offsetHeight;
    // never under the cursor (a row there would hover-open its submenu at once): just RIGHT of it, or just LEFT when the
    // pane has no room on the right
    box.style.left = `${fitsRight ? Math.max(m.x + GAP, 0) : Math.max(0, m.x - GAP - W)}px`;
    box.style.top = `${Math.min(Math.max(m.y, 0), Math.max(0, r.height - h - 2))}px`;
    input.focus();
  }
  private closeMenu(refocus = true): void {
    this.menu?.remove();
    this.menu = null;
    this.sub?.remove();
    this.sub = null;
    if (refocus) this.host.focus();
  }
}

const HIST_KEY = "hexe.tabmenu.history", HIST_KEY_OBJ = "hexe.tabmenu.history.obj";
const loadHistory = (key: string): string[] => {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(key) ?? "[]");
    return Array.isArray(v) ? v.filter((t): t is string => typeof t === "string").slice(0, 8) : [];
  } catch { return []; }
};
/** recent TAB menu picks, most recent first, SOP and /obj kept apart (per viewer; survives restarts when storage works) */
let tabHistory = loadHistory(HIST_KEY), tabHistoryObj = loadHistory(HIST_KEY_OBJ);
