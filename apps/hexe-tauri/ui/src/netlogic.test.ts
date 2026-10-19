import { expect, test } from "bun:test";
import type { NodeView } from "./api";
import { DOT_R, NODE_H, NODE_W, inPort, linePath, nodeIn, nodeOut, outPort, wireArrow } from "./geom";
import { centred, clientToGraph, displayToggle, graphToHost, nodeAt, nodesInBox, brokenNodes, loopNodes, objChildDrop, objCycleCut, objInPort, objOutPort, fitView, objParentDrop, polylineInBox, resolveDrop, resolveDropUp, toggled, uniqueName, missingInputs, optionalInput, blockPairs } from "./netlogic";

const node = (id: number, x: number, y: number, inputs: string[]): NodeView => ({ id, type: "t", name: `n${id}`, x, y, inputs, required: inputs.length, bypass: false, params: {} });
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

test("box select: touching nodes, any drag direction, hostile corners", () => {
  const ns = [{ id: 1, x: 0, y: 0 }, { id: 2, x: 300, y: 0 }, { id: 3, x: 0, y: 300 }];
  expect(nodesInBox(ns, { x: -10, y: -10 }, { x: 5, y: 5 })).toEqual([1]);
  expect(nodesInBox(ns, { x: 500, y: 50 }, { x: 50, y: -50 })).toEqual([1, 2]);
  expect(nodesInBox(ns, { x: NODE_W + 1, y: NODE_H + 1 }, { x: 200, y: 200 })).toEqual([]);
  expect(nodesInBox(ns, { x: Number.NaN, y: 0 }, { x: 10, y: 10 })).toEqual([]);
});

// /obj parenting wires: output (bottom ball) -> another object's input (top ball) or body = parent it
const ob = (id: number, kind: "geo" | "null", x: number, y: number, parent: number | null = null) => ({ id, kind, pos: { x, y }, parent });

test("obj ports: geo on the box edge, null 10 px off its bow-tie", () => {
  expect(objInPort(ob(0, "geo", 0, 0))).toEqual([NODE_W / 2, 0]);
  expect(objOutPort(ob(0, "geo", 0, 0))).toEqual([NODE_W / 2, NODE_H]);
  expect(objInPort(ob(1, "null", 100, 50))).toEqual([100 + NODE_W / 2, 40]);
  expect(objOutPort(ob(1, "null", 100, 50))).toEqual([100 + NODE_W / 2, 50 + NODE_H + 10]);
});

test("obj drop: near an input ball or on the body parents to the source", () => {
  const g = ob(0, "geo", 0, 0), n = ob(1, "null", 300, 200);
  const [ix, iy] = objInPort(n);
  expect(objParentDrop([g, n], 0, { x: ix + 5, y: iy - 5 })).toBe(1);
  expect(objParentDrop([g, n], 0, { x: n.pos.x + 10, y: n.pos.y + 10 })).toBe(1);
  expect(objParentDrop([g, n], 0, { x: ix + 40, y: iy - 40 })).toBeNull();
});

test("obj drop: never onto itself; a loop is allowed (the old link gets cut); hostile points", () => {
  const a = ob(1, "null", 0, 0), b = ob(2, "null", 300, 0, 1), c = ob(3, "null", 600, 0, 2);
  expect(objParentDrop([a, b, c], 1, { x: 10, y: 10 })).toBeNull(); // self
  const [ax, ay] = objInPort(a);
  expect(objParentDrop([a, b, c], 3, { x: ax, y: ay })).toBe(1); // a is c's grandparent: c -> a closes a loop...
  expect(objCycleCut([a, b, c], 3, 1)).toBe(3); // ...so c leaves its parent b (result: c -> a -> b)
  expect(objCycleCut([a, b, c], 1, 3)).toBeNull(); // a -> c is no loop: nothing cut
  expect(objParentDrop([a, b, c], 1, { x: 610, y: 10 })).toBe(3); // re-parenting down the chain is fine
  expect(objParentDrop([a, b, c], 1, { x: Number.NaN, y: 0 })).toBeNull();
  // a broken parent chain (already looping) must not hang
  const p = ob(4, "null", 0, 300, 5), q = ob(5, "null", 300, 300, 4), r = ob(6, "null", 600, 300);
  expect(objParentDrop([p, q, r], 4, { x: 610, y: 310 })).toBe(6);
});

test("reverse obj drop (dragged up off an input ball): output ball or body becomes the parent", () => {
  const a = ob(1, "null", 0, 0), b = ob(2, "null", 300, 0, 1), c = ob(3, "null", 600, 0);
  const [cx, cy] = objOutPort(c);
  expect(objChildDrop([a, b, c], 1, { x: cx - 4, y: cy + 6 })).toBe(3);
  expect(objChildDrop([a, b, c], 2, { x: 10, y: 10 })).toBe(1); // body
  expect(objChildDrop([a, b, c], 1, { x: 310, y: 10 })).toBe(2); // b is a's child: a under b closes a loop...
  expect(objCycleCut([a, b, c], 2, 1)).toBe(2); // ...so b leaves a (result: b -> a)
  expect(objChildDrop([a, b, c], 1, { x: 10, y: 10 })).toBeNull(); // self
  expect(objChildDrop([a, b, c], 1, { x: cx + 40, y: cy + 40 })).toBeNull();
  expect(objChildDrop([a, b, c], 1, { x: Number.POSITIVE_INFINITY, y: 0 })).toBeNull();
});

test("wire dragged up off an input: nearest output port, else the node body; never the input's own node", () => {
  const [ox, oy] = outPort(A.x, A.y);
  expect(resolveDropUp([A, B, C], 2, { x: ox + 6, y: oy + 5 })).toBe(1);
  expect(resolveDropUp([A, B, C], 2, { x: C.x + 20, y: C.y + 10 })).toBe(3);
  expect(resolveDropUp([A, B, C], 2, { x: B.x + 20, y: B.y + 10 })).toBeNull();
  expect(resolveDropUp([A, B, C], 2, { x: ox + 30, y: oy + 30 })).toBeNull();
  expect(resolveDropUp([A, B, C], 2, { x: Number.NaN, y: oy })).toBeNull();
});

test("loopNodes: members of every wired loop, nothing for a tree, long chains do not recurse", () => {
  const e = (from: number, to: number) => ({ from, to });
  expect([...loopNodes([e(1, 2), e(2, 1), e(2, 3)])].sort()).toEqual([1, 2]); // null1 <-> null2, 3 hangs below
  expect([...loopNodes([e(1, 2), e(2, 3), e(3, 1), e(4, 5)])].sort()).toEqual([1, 2, 3]);
  expect(loopNodes([e(1, 2), e(1, 3), e(2, 4), e(3, 4)]).size).toBe(0); // diamond: no loop
  expect(loopNodes([]).size).toBe(0);
  const chain = Array.from({ length: 50_000 }, (_, i) => e(i, i + 1));
  expect(loopNodes(chain).size).toBe(0);
  expect(loopNodes([...chain, e(50_000, 0)]).size).toBe(50_001);
});

test("brokenNodes: roots + 2 hops downstream (dots pass through), smallest hop wins, loops terminate", () => {
  const e = (from: number, to: number) => ({ from, to });
  const chain = [e(1, 2), e(2, 3), e(3, 4), e(4, 5)];
  expect([...brokenNodes(chain, [2])]).toEqual([[2, 0], [3, 1], [4, 2]]);
  expect(brokenNodes(chain, [2], (id) => id === 3).get(5)).toBe(2); // 3 is a dot: 4 = hop 1, 5 = hop 2
  expect(brokenNodes(chain, []).size).toBe(0);
  const loop = brokenNodes([e(1, 2), e(2, 1), e(2, 3)], loopNodes([e(1, 2), e(2, 1)]));
  expect([...loop].sort()).toEqual([[1, 0], [2, 0], [3, 1]]);
  expect(brokenNodes([e(1, 2), e(1, 3), e(3, 2)], [1]).get(2)).toBe(1);
});

// network dots (type `dot`): (x, y) is the ball's centre; its one input and its output both sit there
const dot = (id: number, x: number, y: number): NodeView => ({ id, type: "dot", name: `__dot${id}`, x, y, inputs: ["input"], required: 1, bypass: false, params: {} });

test("dot ports sit on its centre; box nodes keep their edge ports", () => {
  const d = dot(7, 50, 60);
  expect(nodeIn(d, 0)).toEqual([50, 60]);
  expect(nodeOut(d)).toEqual([50, 60]);
  expect(nodeIn(B, 1)).toEqual(inPort(B.x, B.y, 1, 2));
  expect(nodeOut(B)).toEqual(outPort(B.x, B.y));
});

test("dots as drop targets, sources and selection", () => {
  const d = dot(7, 600, 600);
  expect(resolveDrop([A, d], [], 1, { x: 603, y: 598 })).toEqual({ to: 7, input: 0 }); // on its input (= centre)
  expect(resolveDrop([A, d], [], 1, { x: 600 + DOT_R + 2, y: 600 })).toEqual({ to: 7, input: 0 }); // ball rim
  expect(resolveDrop([A, d], [], 1, { x: 640, y: 640 })).toBeNull();
  expect(resolveDropUp([A, B, d], 2, { x: 601, y: 602 })).toBe(7); // wire dragged up off B lands on the dot's output
  expect(nodeAt([A, d], { x: 604, y: 596 })).toBe(7);
  expect(nodeAt([A, d], { x: 600 + NODE_W / 2, y: 600 + NODE_H / 2 })).toBeNull(); // not the box a node would cover
  expect(nodesInBox([d], { x: 590, y: 590 }, { x: 595, y: 595 })).toEqual([7]);
  expect(nodesInBox([d], { x: 520, y: 520 }, { x: 580, y: 580 })).toEqual([]);
});

test("wire arrows: only on segments running up the network, pointing along the flow", () => {
  expect(wireArrow([0, 0], [0, 100], true)).toBeNull(); // downward = the normal flow
  expect(wireArrow([0, 0], [100, 0], true)).toBeNull(); // level
  const up = wireArrow([0, 100], [0, 0], true) as string;
  const pts = (up.match(/-?\d+(\.\d+)?/g) as string[]).map(Number);
  expect(pts.length).toBe(6);
  expect(pts[1]).toBeLessThan(50); // tip is above the midpoint: points up
  expect(pts[3]).toBeGreaterThan(50);
  expect(wireArrow([0, 100], [200, 40], false)).not.toBeNull(); // a bezier that climbs gets one too
  expect(wireArrow([0, Number.NaN], [0, 0], true)).toBeNull();
  expect(linePath([1, 2], [3, 4])).toBe("M1,2 L3,4");
});

test("polylineInBox: a vertex inside, or a segment crossing with both ends outside; misses and junk are false", () => {
  const box = [{ x: 0, y: 0 }, { x: 10, y: 10 }] as const;
  expect(polylineInBox([[5, 5], [50, 50]], ...box)).toBe(true);
  expect(polylineInBox([[-5, 5], [15, 5]], ...box)).toBe(true); // straight through
  expect(polylineInBox([[-5, 20], [20, -5]], ...box)).toBe(true); // diagonal corner cut
  expect(polylineInBox([[-5, 30], [30, 25]], ...box)).toBe(false);
  expect(polylineInBox([[20, 0], [20, 10]], { x: 10, y: 10 }, { x: 0, y: 0 })).toBe(false);
  expect(polylineInBox([], ...box)).toBe(false);
  expect(polylineInBox([[5, 5]], { x: NaN, y: 0 }, { x: 10, y: 10 })).toBe(false);
});

test("fitView: centres the union of rects, zooms out to fit far-away nodes, caps zoom-in, ignores junk", () => {
  // a node 10000 px away: both fit inside the pane, centred
  const v = fitView([[0, 0, 100, 40], [10000, 5000, 10100, 5040]], 800, 600)!;
  const toHost = (x: number, y: number) => [x * v.k + v.x, y * v.k + v.y];
  for (const [x, y] of [[0, 0], [10100, 5040]]) { const [hx, hy] = toHost(x, y); expect(hx).toBeGreaterThanOrEqual(39.9); expect(hx).toBeLessThanOrEqual(760.1); expect(hy).toBeGreaterThanOrEqual(39.9); expect(hy).toBeLessThanOrEqual(560.1); }
  const [cx, cy] = toHost(5050, 2520);
  expect(cx).toBeCloseTo(400); expect(cy).toBeCloseTo(300);
  expect(fitView([[0, 0, 10, 10]], 800, 600)!.k).toBe(1.25);
  expect(fitView([], 800, 600)).toBeNull();
  expect(fitView([[NaN, 0, 1, 1]], 800, 600)).toBeNull();
  expect(fitView([[0, 0, 1, 1]], 0, 600)).toBeNull();
});

test("uniqueName: free kept, taken -> next free number on the base", () => {
  const t = new Set(["box", "box1", "null1"]);
  expect(uniqueName("box", t)).toBe("box2");
  expect(uniqueName("box1", t)).toBe("box2");
  expect(uniqueName("sphere", t)).toBe("sphere");
  expect(uniqueName("null1", t)).toBe("null2");
  expect(uniqueName("7", new Set(["7"]))).toBe("71");
});

test("required inputs: unwired ones get the red arrow, wires past them are optional (dashed)", () => {
  // polyreduce: Triangular Mesh required, Feature Polygons / Reference Mesh optional
  const pr = { id: 7, inputs: ["Triangular Mesh", "Feature Polygons", "Reference Mesh"], required: 1 };
  expect(missingInputs(pr, [])).toEqual([0]);
  expect(missingInputs(pr, [{ to: 7, input: 0 }])).toEqual([]);
  expect(missingInputs(pr, [{ to: 8, input: 0 }, { to: 7, input: 2 }])).toEqual([0]); // another node's wire / optional wire don't count
  expect([0, 1, 2].map((i) => optionalInput(pr, i))).toEqual([false, true, true]);
  // a generator has nothing to miss; a node claiming more required than it has inputs never indexes past them
  expect(missingInputs({ id: 1, inputs: [], required: 0 }, [])).toEqual([]);
  expect(missingInputs({ id: 1, inputs: ["a"], required: 5 }, [])).toEqual([0]);
});

test("For-Each pairs: by the End's begin name, else the nearest Begin upstream; loops and strays are safe", () => {
  const n = (id: number, type: string, name: string, begin?: string) => ({ id, type, name, params: begin === undefined ? {} : { begin } });
  const nodes = [n(1, "block_begin", "foreachA"), n(2, "xform", "x"), n(3, "block_end", "end1", "foreachA"), n(4, "block_begin", "b2"), n(5, "block_end", "end2", ""), n(6, "block_end", "lost", "nope"), n(7, "block_end", "loop", "")];
  const edges = [{ from: 1, to: 2 }, { from: 2, to: 3 }, { from: 4, to: 2 }, { from: 2, to: 5 }, { from: 7, to: 7 }];
  expect(blockPairs(nodes, edges)).toEqual([[1, 3], [1, 5]]); // end2: nearest upstream (x's first wire is from foreachA)
  expect(blockPairs(nodes, [{ from: 4, to: 5 }])).toEqual([[1, 3], [4, 5]]);
});
