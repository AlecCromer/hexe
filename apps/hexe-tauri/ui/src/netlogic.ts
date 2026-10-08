// Pure network-editor logic (no DOM): wire-drop resolution, coordinate transforms, selection. Unit-tested with `bun test`.
import type { EdgeView, NodeView } from "./api";
import { NODE_H, NODE_W, inPort } from "./geom";

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
      const [px, py] = inPort(n.x, n.y, i, n.inputs.length);
      const d = Math.hypot(px - p.x, py - p.y);
      if (d < radius && (!best || d < best.d)) best = { d, to: n.id, input: i };
    });
  }
  if (best) return { to: (best as { to: number }).to, input: (best as { input: number }).input };
  for (const n of nodes) {
    if (n.id === from || !n.inputs.length) continue;
    if (p.x >= n.x && p.x <= n.x + NODE_W && p.y >= n.y && p.y <= n.y + NODE_H) {
      const wired = new Set(edges.filter((e) => e.to === n.id).map((e) => e.input));
      const free = n.inputs.findIndex((_, i) => !wired.has(i));
      return { to: n.id, input: free < 0 ? 0 : free };
    }
  }
  return null;
}

/** Node under graph point `p` (topmost = last drawn), `null` if none. */
export function nodeAt(nodes: NodeView[], p: { x: number; y: number }): number | null {
  for (let i = nodes.length - 1; i >= 0; i--) {
    const n = nodes[i];
    if (p.x >= n.x && p.x <= n.x + NODE_W && p.y >= n.y && p.y <= n.y + NODE_H) return n.id;
  }
  return null;
}

/** Shift-click: toggle `id` in the selection. Returns the new set. */
export function toggled(sel: ReadonlySet<number>, id: number): Set<number> {
  const out = new Set(sel);
  if (!out.delete(id)) out.add(id);
  return out;
}

/** Display flag toggle: clicking the current display node clears it (`null`), any other node becomes the display. */
export const displayToggle = (current: number | null, id: number): number | null => (current === id ? null : id);

