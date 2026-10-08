import { expect, test } from "bun:test";
import type { NodeView } from "./api";
import { NODE_H, NODE_W, inPort } from "./geom";
import { centred, clientToGraph, displayToggle, graphToHost, nodeAt, resolveDrop, toggled } from "./netlogic";

const node = (id: number, x: number, y: number, inputs: string[]): NodeView => ({ id, type: "t", name: `n${id}`, x, y, inputs, params: {} });
const A = node(1, 0, 0, []);
const B = node(2, 300, 200, ["a", "b"]);
const C = node(3, 0, 200, ["a"]);

test("drop within 14px of a port picks the nearest port", () => {
  const [px, py] = inPort(B.x, B.y, 1, 2);
  expect(resolveDrop([A, B, C], [], 1, { x: px + 5, y: py - 9 })).toEqual({ to: 2, input: 1 });
  const [qx, qy] = inPort(B.x, B.y, 0, 2);
  expect(resolveDrop([A, B, C], [], 1, { x: qx - 3, y: qy + 3 })).toEqual({ to: 2, input: 0 });
  expect(resolveDrop([A, B, C], [], 1, { x: px + 20, y: py - 20 })).toBeNull();
});

test("drop on a node body = first free input, else input 0", () => {
  const mid = { x: B.x + NODE_W / 2, y: B.y + NODE_H - 4 };
  expect(resolveDrop([A, B], [], 1, mid)).toEqual({ to: 2, input: 0 });
  expect(resolveDrop([A, B], [{ from: 1, to: 2, input: 0 }], 1, mid)).toEqual({ to: 2, input: 1 });
  expect(resolveDrop([A, B], [{ from: 1, to: 2, input: 0 }, { from: 1, to: 2, input: 1 }], 1, mid)).toEqual({ to: 2, input: 0 });
});

test("never drops on the source node or an input-less node", () => {
  expect(resolveDrop([A, B], [], 2, { x: B.x + 70, y: B.y + 20 })).toBeNull();
  expect(resolveDrop([A, B], [], 2, { x: A.x + 70, y: A.y + 20 })).toBeNull();
});

test("radius is in graph px (zoomed-out views widen it)", () => {
  const [px, py] = inPort(C.x, C.y, 0, 1);
  expect(resolveDrop([A, C], [], 1, { x: px + 15, y: py - 15 }, 14 / 0.5)).toEqual({ to: 3, input: 0 });
});

test("Tab-add lands centred on the cursor, through pan + zoom", () => {
  const v = { x: 40, y: -10, k: 2 };
  const rect = { left: 100, top: 50 };
  const g = clientToGraph(500, 450, rect, v);
  expect(g).toEqual({ x: 180, y: 205 });
  expect(graphToHost(g.x, g.y, v)).toEqual({ x: 400, y: 400 });
  const at = centred(g);
  expect(at).toEqual({ x: 180 - NODE_W / 2, y: 205 - NODE_H / 2 });
  // the node box centre maps back onto the cursor
  const centre = graphToHost(at.x + NODE_W / 2, at.y + NODE_H / 2, v);
  expect([centre.x + rect.left, centre.y + rect.top]).toEqual([500, 450]);
});

test("nodeAt picks the topmost; toggled; displayToggle", () => {
  const top = node(9, 10, 10, []);
  expect(nodeAt([A, top], { x: 20, y: 20 })).toBe(9);
  expect(nodeAt([A, top], { x: 500, y: 500 })).toBeNull();
  expect([...toggled(new Set([1, 2]), 2)]).toEqual([1]);
  expect([...toggled(new Set([1]), 5)]).toEqual([1, 5]);
  expect(displayToggle(3, 3)).toBeNull();
  expect(displayToggle(3, 4)).toBe(4);
  expect(displayToggle(null, 4)).toBe(4);
});
