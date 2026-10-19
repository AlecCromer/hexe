// Pure network-editor logic (no DOM): wire-drop resolution, coordinate transforms, selection. Unit-tested with `bun test`.
import type { EdgeView, NodeView } from "./api";
import { NODE_H, NODE_W, nodeIn, nodeOut, nodeRect } from "./geom";

/** Wire drop radius around an input port, in screen px (the egui app's DROP). */
export const DROP_PX = 14;

export interface View { x: number; y: number; k: number }

/** client px -> graph coords (`rect` = the svg's bounding box). */
export function clientToGraph(cx: number, cy: number, rect: { left: number; top: number }, v: View): { x: number; y: number } {
  return { x: (cx - rect.left - v.x) / v.k, y: (cy - rect.top - v.y) / v.k };
}
/** graph coords -> px inside the svg host (menu / rename-box placement). */
export function graphToHost(gx: number, gy: number, v: View): { x: number; y: number } {
  return { x: gx * v.k + v.x, y: gy * v.k + v.y };
}
/** Top-left of a node dropped/added so that its box is centred on graph point `p` (Tab menu: node lands under the cursor). */
export const centred = (p: { x: number; y: number }): { x: number; y: number } => ({ x: p.x - NODE_W / 2, y: p.y - NODE_H / 2 });

export interface Drop { to: number; input: number }

/**
 * Where a wire dragged from `from`'s output and released at graph point `p` connects (egui `resolve`):
 * the nearest input port within `radius` graph px (any node but `from`); else, released on a node body, that
 * node's first FREE input (unwired), else input 0. `null` = connect nowhere.
 */
export function resolveDrop(nodes: NodeView[], edges: EdgeView[], from: number, p: { x: number; y: number }, radius = DROP_PX): Drop | null {
  let best: { d: number; to: number; input: number } | null = null;
  for (const n of nodes) {
    if (n.id === from) continue;
    n.inputs.forEach((_, i) => {
      const [px, py] = nodeIn(n, i);
      const d = Math.hypot(px - p.x, py - p.y);
      if (d < radius && (!best || d < best.d)) best = { d, to: n.id, input: i };
    });
  }
  if (best) return { to: (best as { to: number }).to, input: (best as { input: number }).input };
  for (const n of nodes) {
    if (n.id === from || !n.inputs.length) continue;
    const [x0, y0, x1, y1] = nodeRect(n);
    if (p.x >= x0 && p.x <= x1 && p.y >= y0 && p.y <= y1) {
      const wired = new Set(edges.filter((e) => e.to === n.id).map((e) => e.input));
      const free = n.inputs.findIndex((_, i) => !wired.has(i));
      return { to: n.id, input: free < 0 ? 0 : free };
    }
  }
  return null;
}

/**
 * A wire dragged UP off node `to`'s free input, released at `p`: the node whose output port is nearest within `radius`,
 * else the node whose body is under `p`, feeds that input (any node but `to`; a wire that closes a loop is allowed: the nodes on it show as broken, `loopNodes`). `null` = nowhere.
 */
export function resolveDropUp(nodes: NodeView[], to: number, p: { x: number; y: number }, radius = DROP_PX): number | null {
  if (!Number.isFinite(p.x) || !Number.isFinite(p.y)) return null;
  let best: { d: number; id: number } | null = null;
  for (const n of nodes) {
    if (n.id === to) continue;
    const [px, py] = nodeOut(n);
    const d = Math.hypot(px - p.x, py - p.y);
    if (d < radius && (!best || d < best.d)) best = { d, id: n.id };
  }
  if (best) return (best as { id: number }).id;
  return nodeAt(nodes.filter((n) => n.id !== to), p);
}

/** Nodes on a wired loop (Houdini lets the editor close one; the cook then errors): every strongly connected component
 *  of 2+ nodes (Kosaraju, iterative: no recursion depth on a long chain). Self loops cannot be wired. */
export function loopNodes(edges: { from: number; to: number }[]): Set<number> {
  const fwd = new Map<number, number[]>(), rev = new Map<number, number[]>();
  const add = (m: Map<number, number[]>, a: number, b: number) => { const l = m.get(a); if (l) l.push(b); else m.set(a, [b]); };
  for (const e of edges) { add(fwd, e.from, e.to); add(rev, e.to, e.from); }
  const ids = [...new Set(edges.flatMap((e) => [e.from, e.to]))];
  // pass 1: finish order on the forward graph
  const seen = new Set<number>(), order: number[] = [];
  for (const s of ids) {
    if (seen.has(s)) continue;
    seen.add(s);
    const stack: [number, number][] = [[s, 0]];
    while (stack.length) {
      const top = stack[stack.length - 1], next = fwd.get(top[0]) ?? [];
      if (top[1] < next.length) {
        const n = next[top[1]++];
        if (!seen.has(n)) { seen.add(n); stack.push([n, 0]); }
      } else { order.push(top[0]); stack.pop(); }
    }
  }
  // pass 2: components on the reversed graph, latest finisher first
  const comp = new Set<number>(), out = new Set<number>();
  for (let i = order.length - 1; i >= 0; i--) {
    if (comp.has(order[i])) continue;
    const members: number[] = [], stack = [order[i]];
    comp.add(order[i]);
    while (stack.length) {
      const n = stack.pop() as number;
      members.push(n);
      for (const p of rev.get(n) ?? []) if (!comp.has(p)) { comp.add(p); stack.push(p); }
    }
    if (members.length > 1) for (const m of members) out.add(m);
  }
  return out;
}

/** Broken-node marks (red !): each root (a node whose cook failed, or on a loop) and the nodes up to `depth` hops
 *  downstream of it. Dots pass the mark through without counting as a hop. Node id -> hops from the nearest root. */
export function brokenNodes(edges: { from: number; to: number }[], roots: Iterable<number>, isDot: (id: number) => boolean = () => false, depth = 2): Map<number, number> {
  const fwd = new Map<number, number[]>();
  for (const e of edges) { const l = fwd.get(e.from); if (l) l.push(e.to); else fwd.set(e.from, [e.to]); }
  const out = new Map<number, number>();
  const queue: [number, number][] = [];
  for (const r of roots) if (!out.has(r)) { out.set(r, 0); queue.push([r, 0]); }
  // breadth first: a node keeps its smallest hop count; nothing past `depth` is marked
  for (let qi = 0; qi < queue.length; qi++) {
    const [n, h] = queue[qi];
    for (const m of fwd.get(n) ?? []) {
      const hm = isDot(m) ? h : h + 1;
      if (hm > depth || (out.has(m) && (out.get(m) as number) <= hm)) continue;
      out.set(m, hm);
      queue.push([m, hm]);
    }
  }
  return out;
}

/** Snap reach while dragging a node, in SCREEN px (divide by the zoom for graph units): small, so moving on frees it. */
export const SNAP_PX = 8;
const centre = (n: { type?: string; x: number; y: number }): [number, number] => {
  const [x0, y0, x1, y1] = nodeRect(n);
  return [(x0 + x1) / 2, (y0 + y1) / 2];
};
/** Vertical-line snap for a dragged node `m` at its RAW (unsnapped) position: among `others` that sit wholly above or
 *  below it (no vertical overlap), the vertically nearest one whose centre x is within `tol` graph units of `m`'s.
 *  Returns how far to shift `m` in x so the centres line up, plus that node's id (for a guide line); `null` = no snap.
 *  Being decided from the raw position each move, the snap lets go as soon as the pointer moves `tol` further. */
export function snapCentreX(
  m: { type?: string; x: number; y: number },
  others: readonly { id: number; type?: string; x: number; y: number }[],
  tol: number,
): { dx: number; to: number } | null {
  if (!(tol > 0)) return null;
  const [mx, my] = centre(m);
  const [, my0, , my1] = nodeRect(m);
  let best: { dx: number; to: number; dy: number } | null = null;
  for (const o of others) {
    const [ox, oy] = centre(o);
    const [, oy0, , oy1] = nodeRect(o);
    if (!(oy1 <= my0 || oy0 >= my1)) continue; // side by side: lining up would stack them
    const dx = ox - mx;
    if (!(Math.abs(dx) <= tol)) continue; // also drops NaN
    const dy = Math.abs(oy - my);
    if (!best || dy < best.dy) best = { dx, to: o.id, dy };
  }
  return best && { dx: best.dx, to: best.to };
}

/** Node under graph point `p` (topmost = last drawn), `null` if none. */
export function nodeAt(nodes: NodeView[], p: { x: number; y: number }): number | null {
  for (let i = nodes.length - 1; i >= 0; i--) {
    const [x0, y0, x1, y1] = nodeRect(nodes[i]);
    if (p.x >= x0 && p.x <= x1 && p.y >= y0 && p.y <= y1) return nodes[i].id;
  }
  return null;
}

/** Shift-click: toggle `id` in the selection. Returns the new set. */
export function toggled<T>(sel: ReadonlySet<T>, id: T): Set<T> {
  const out = new Set(sel);
  if (!out.delete(id)) out.add(id);
  return out;
}

/** Display flag toggle: clicking the current display node clears it (`null`), any other node becomes the display. */
export const displayToggle = (current: number | null, id: number): number | null => (current === id ? null : id);


/** Box select: ids of nodes whose body rectangle (`nodeRect`) touches the box spanned by graph points `a`, `b` (any corner order). */
export function nodesInBox(nodes: { id: number; type?: string; x: number; y: number }[], a: { x: number; y: number }, b: { x: number; y: number }): number[] {
  const x0 = Math.min(a.x, b.x), x1 = Math.max(a.x, b.x), y0 = Math.min(a.y, b.y), y1 = Math.max(a.y, b.y);
  if (![x0, x1, y0, y1].every(Number.isFinite)) return [];
  return nodes.filter((n) => { const r = nodeRect(n); return r[0] <= x1 && r[2] >= x0 && r[1] <= y1 && r[3] >= y0; }).map((n) => n.id);
}

/** Box select of wires: does the polyline `pts` (a wire sampled along its curve) touch the box spanned by `a`, `b`?
 *  A vertex inside counts, so does a segment crossing the box with both ends outside (Liang-Barsky clip). */
export function polylineInBox(pts: [number, number][], a: { x: number; y: number }, b: { x: number; y: number }): boolean {
  const x0 = Math.min(a.x, b.x), x1 = Math.max(a.x, b.x), y0 = Math.min(a.y, b.y), y1 = Math.max(a.y, b.y);
  if (![x0, x1, y0, y1].every(Number.isFinite)) return false;
  const inside = ([x, y]: [number, number]) => x >= x0 && x <= x1 && y >= y0 && y <= y1;
  const crosses = ([px, py]: [number, number], [qx, qy]: [number, number]) => {
    const dx = qx - px, dy = qy - py;
    let t0 = 0, t1 = 1;
    for (const [p, q] of [[-dx, px - x0], [dx, x1 - px], [-dy, py - y0], [dy, y1 - py]]) {
      if (p === 0) { if (q < 0) return false; continue; }
      const r = q / p;
      if (p < 0) t0 = Math.max(t0, r); else t1 = Math.min(t1, r);
      if (t0 > t1) return false;
    }
    return true;
  };
  return pts.some(inside) || pts.some((p, i) => i > 0 && crosses(pts[i - 1], p));
}

/** An /obj object as the parenting wires see it (network position + parent link). */
export interface ObjLink { id: number; kind: "geo" | "null" | "subnet"; pos: { x: number; y: number } | null; parent: number | null }
/** /obj connector balls: geo on its box edge, Null 10 px off its bow-tie (input on top, output at the bottom). */
export const objInPort = (o: ObjLink): [number, number] => { const p = o.pos ?? { x: 0, y: 0 }; return [p.x + NODE_W / 2, p.y - (o.kind === "null" ? 10 : 0)]; };
export const objOutPort = (o: ObjLink): [number, number] => { const p = o.pos ?? { x: 0, y: 0 }; return [p.x + NODE_W / 2, p.y + NODE_H + (o.kind === "null" ? 10 : 0)]; };
/** `id`'s ancestors (parent chain); stops at a loop or a missing parent. */
function ancestors(objs: ObjLink[], id: number): Set<number> {
  const seen = new Set<number>();
  let p = objs.find((o) => o.id === id)?.parent ?? null;
  while (p !== null && !seen.has(p)) { seen.add(p); p = objs.find((o) => o.id === p)?.parent ?? null; }
  return seen;
}
/** nearest connector (`port` of each candidate) within `radius` of `p`, else the topmost candidate body under `p` */
function objDrop(ok: ObjLink[], port: (o: ObjLink) => [number, number], p: { x: number; y: number }, radius: number): number | null {
  if (!Number.isFinite(p.x) || !Number.isFinite(p.y)) return null;
  let best: { d: number; id: number } | null = null;
  for (const o of ok) {
    const [x, y] = port(o);
    const d = Math.hypot(x - p.x, y - p.y);
    if (d < radius && (!best || d < best.d)) best = { d, id: o.id };
  }
  if (best) return best.id;
  for (let i = ok.length - 1; i >= 0; i--) {
    const q = ok[i].pos as { x: number; y: number };
    if (p.x >= q.x && p.x <= q.x + NODE_W && p.y >= q.y && p.y <= q.y + NODE_H) return ok[i].id;
  }
  return null;
}
/**
 * Where a parenting wire dragged from object `from`'s output and released at `p` lands: the object whose input ball is
 * nearest within `radius`, else the object whose body is under `p`. The target becomes `from`'s child (never `from`
 * itself; one of `from`'s ancestors closes a loop, which `objCycleCut` resolves). `null` = nowhere.
 */
export function objParentDrop(objs: ObjLink[], from: number, p: { x: number; y: number }, radius = DROP_PX): number | null {
  return objDrop(objs.filter((o) => o.pos && o.id !== from), objInPort, p, radius);
}
/**
 * The reverse: a wire dragged UP off `child`'s (free) input ball, released at `p`: the object whose output ball is nearest
 * within `radius`, else whose body is under `p`, becomes `child`'s parent. Never `child` itself (a descendant closes a
 * loop, which `objCycleCut` resolves).
 */
export function objChildDrop(objs: ObjLink[], child: number, p: { x: number; y: number }, radius = DROP_PX): number | null {
  return objDrop(objs.filter((o) => o.pos && o.id !== child), objOutPort, p, radius);
}
/** New link `parent` -> `child` where `child` is already above `parent`: `parent` must leave its own parent (returned) to
 *  break the loop. `null` = no loop. */
export const objCycleCut = (objs: ObjLink[], parent: number, child: number): number | null => (ancestors(objs, parent).has(child) ? parent : null);

/** Selection key of a SOP wire: the input it feeds (`to:input` is unique; an input takes one wire). */
export const edgeKey = (to: number, input: number): string => `${to}:${input}`;

/** Frame-all view (network `F`): pan + zoom so every rect `[x0, y0, x1, y1]` fits a `w` x `h` pane with `pad` px all
 *  round, centred. Zoom clamped to [`kMin`, `kMax`] (a lone node is not blown up); null when there is nothing to frame. */
export function fitView(rects: [number, number, number, number][], w: number, h: number, pad = 40, kMin = 0.05, kMax = 1.25): { x: number; y: number; k: number } | null {
  const ok = rects.filter((r) => r.every(Number.isFinite));
  if (!ok.length || !(w > 0) || !(h > 0)) return null;
  const x0 = Math.min(...ok.map((r) => r[0])), y0 = Math.min(...ok.map((r) => r[1]));
  const x1 = Math.max(...ok.map((r) => r[2])), y1 = Math.max(...ok.map((r) => r[3]));
  const k = Math.min(kMax, Math.max(kMin, Math.min((w - 2 * pad) / Math.max(1e-9, x1 - x0), (h - 2 * pad) / Math.max(1e-9, y1 - y0))));
  return { k, x: w / 2 - ((x0 + x1) / 2) * k, y: h / 2 - ((y0 + y1) / 2) * k };
}

/** Rename to a name already `taken`: the next free number on its base (Houdini; same rule as Rust `node_rename`):
 *  box + box1 taken, `box` -> box2, `box1` -> box2. A free name is kept as typed. */
export function uniqueName(name: string, taken: ReadonlySet<string>): string {
  if (!taken.has(name)) return name;
  const base = name.replace(/\d+$/, "") || name;
  for (let i = 1; ; i++) if (!taken.has(`${base}${i}`)) return `${base}${i}`;
}

/** Inputs of `n` that must be wired (index < `n.required`) but are not: each gets the red down-arrow. */
export function missingInputs(n: { id: number; inputs: string[]; required: number }, edges: { to: number; input: number }[]): number[] {
  const wired = new Set(edges.filter((e) => e.to === n.id).map((e) => e.input));
  return n.inputs.map((_, i) => i).filter((i) => i < n.required && !wired.has(i));
}
/** A wire into input `input` of `n` is dashed when that input is optional (past the required ones). */
export const optionalInput = (n: { required: number }, input: number): boolean => input >= n.required;

/** For-Each pairs `[begin id, end id]`: each `block_end` with the `block_begin` its `begin` param names, else (empty)
 *  the nearest Block Begin upstream (breadth first over the wires), like the engine. Unpaired ends are left out. */
export function blockPairs(nodes: { id: number; type: string; name: string; params: Record<string, unknown> }[], edges: { from: number; to: number }[]): [number, number][] {
  const up = new Map<number, number[]>();
  for (const e of edges) { const l = up.get(e.to); if (l) l.push(e.from); else up.set(e.to, [e.from]); }
  const type = new Map(nodes.map((n) => [n.id, n.type]));
  const out: [number, number][] = [];
  for (const end of nodes.filter((n) => n.type === "block_end")) {
    const want = String(end.params.begin ?? "");
    let begin: number | undefined;
    if (want) begin = nodes.find((n) => n.name === want && n.type === "block_begin")?.id;
    else {
      const seen = new Set([end.id]), queue = [...(up.get(end.id) ?? [])];
      for (let i = 0; i < queue.length && begin === undefined; i++) {
        const q = queue[i];
        if (seen.has(q)) continue;
        seen.add(q);
        if (type.get(q) === "block_begin") begin = q; else queue.push(...(up.get(q) ?? []));
      }
    }
    if (begin !== undefined) out.push([begin, end.id]);
  }
  return out;
}
