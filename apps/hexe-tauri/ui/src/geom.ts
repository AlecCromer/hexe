// Pure geometry helpers for the viewport (no three.js import: unit-tested with `bun test`).
import earcut from "earcut";

/** Fan-triangulate closed prims (v0,v[i],v[i+1]); prims with <3 points or any out-of-range index are skipped. */
export function fanTriangulate(prims: number[][], closed: boolean[], pointCount: number): Uint32Array {
  const out: number[] = [];
  prims.forEach((p, k) => {
    if (!closed[k] || p.length < 3 || p.some((i) => !(i >= 0 && i < pointCount))) return;
    for (let i = 1; i + 1 < p.length; i++) out.push(p[0], p[i], p[i + 1]);
  });
  return Uint32Array.from(out);
}
/** Newell normal (unnormalised; length = 2x area) of a polygon given as point indices into flat xyz `pos`. */
export function newell(p: number[], pos: ArrayLike<number>): [number, number, number] {
  let x = 0, y = 0, z = 0;
  for (let i = 0; i < p.length; i++) {
    const a = p[i] * 3, b = p[(i + 1) % p.length] * 3;
    x += (pos[a + 1] - pos[b + 1]) * (pos[a + 2] + pos[b + 2]);
    y += (pos[a + 2] - pos[b + 2]) * (pos[a] + pos[b]);
    z += (pos[a] - pos[b]) * (pos[a + 1] + pos[b + 1]);
  }
  return [x, y, z];
}
/** Triangulate closed prims with earcut on the plane of each prim's Newell normal (drop the dominant axis); concave
 * polygons fill correctly. Triangles/quads degenerate to the same result as a fan. A degenerate prim (earcut yields
 * nothing, e.g. zero area) falls back to the fan so nothing silently vanishes. Skips open/short/out-of-range prims. */
export function triangulate(prims: number[][], closed: boolean[], pos: ArrayLike<number>): Uint32Array {
  const out: number[] = [];
  const n = pos.length / 3;
  prims.forEach((p, k) => {
    if (!closed[k] || p.length < 3 || p.some((i) => !(i >= 0 && i < n))) return;
    if (p.length === 3) { out.push(p[0], p[1], p[2]); return; }
    const [nx, ny, nz] = newell(p, pos).map(Math.abs);
    const [u, v] = nx >= ny && nx >= nz ? [1, 2] : ny >= nz ? [2, 0] : [0, 1];
    const flat: number[] = [];
    for (const i of p) flat.push(pos[i * 3 + u], pos[i * 3 + v]);
    const t = earcut(flat);
    if (t.length >= 3) for (const i of t) out.push(p[i]);
    else for (let i = 1; i + 1 < p.length; i++) out.push(p[0], p[i], p[i + 1]);
  });
  return Uint32Array.from(out);
}
export const CAP_TRIS = 1_000_000, CAP_EDGE_VERTS = 2_000_000, CAP_POINTS = 500_000, CAP_LABELS = 500, CAP_LABEL_SCAN = 100_000;
/** Triangle count `triangulate` will emit for these prims (cheap pre-check so a huge cook never reaches earcut). */
export function triEstimate(prims: number[][], closed: boolean[]): number {
  let t = 0;
  for (let k = 0; k < prims.length; k++) if (closed[k] && prims[k].length >= 3) t += prims[k].length - 2;
  return t;
}
export interface Caps { mesh: boolean; edges: boolean; points: boolean; warnings: string[] }
/** Draw caps (egui parity): 1M tris / 2M edge verts / 500k points. Over a cap -> that layer is skipped + a warning. */
export function capCheck(tris: number, edgeVerts: number, points: number): Caps {
  const c: Caps = { mesh: tris <= CAP_TRIS, edges: edgeVerts <= CAP_EDGE_VERTS, points: points <= CAP_POINTS, warnings: [] };
  if (!c.mesh) c.warnings.push(`${tris} tris > ${CAP_TRIS}: surface not drawn`);
  if (!c.edges) c.warnings.push(`${edgeVerts} edge verts > ${CAP_EDGE_VERTS}: edges not drawn`);
  if (!c.points) c.warnings.push(`${points} pts > ${CAP_POINTS}: points not drawn`);
  return c;
}
/** Mean point of each prim (flat xyz, NaN for prims without points); only the first `limit` prims. */
export function primCentroids(prims: number[][], pos: ArrayLike<number>, limit = CAP_LABEL_SCAN): Float32Array {
  const n = Math.min(prims.length, limit);
  const out = new Float32Array(n * 3).fill(NaN);
  for (let k = 0; k < n; k++) {
    const p = prims[k];
    if (!p.length) continue;
    let x = 0, y = 0, z = 0;
    for (const i of p) { x += pos[i * 3]; y += pos[i * 3 + 1]; z += pos[i * 3 + 2]; }
    out[k * 3] = x / p.length; out[k * 3 + 1] = y / p.length; out[k * 3 + 2] = z / p.length;
  }
  return out;
}
/** Status text for the viewport view state, e.g. `shaded +P +N`. */
export const viewState = (wire: boolean, p: boolean, n: boolean): string => `${wire ? "wire" : "shaded"}${p ? " +P" : ""}${n ? " +N" : ""}`;
export interface Shape { kind: "sphere" | "tube"; center: [number, number, number]; xform: number[]; taper: number }
/** Wire proxy of a sphere / tube prim as flat xyz segment pairs. Sphere = 3 great circles of the unit sphere; tube = 2 rings
 * (y=+-0.5, top radius = taper) + 4 side lines. Unit shape is mapped by the row-major 3x3 `xform` (column-vector M*p) then
 * offset by `center`. UNVERIFIED vs Houdini: tube axis = Y, xform convention. */
export function shapeWire(s: Shape, seg = 32): Float32Array {
  const out: number[] = [];
  const m = s.xform;
  const put = (x: number, y: number, z: number) => out.push(
    s.center[0] + m[0] * x + m[1] * y + m[2] * z, s.center[1] + m[3] * x + m[4] * y + m[5] * z, s.center[2] + m[6] * x + m[7] * y + m[8] * z);
  const ring = (f: (c: number, sn: number) => [number, number, number]) => {
    for (let i = 0; i < seg; i++) {
      const a = (i / seg) * Math.PI * 2, b = ((i + 1) / seg) * Math.PI * 2;
      put(...f(Math.cos(a), Math.sin(a))); put(...f(Math.cos(b), Math.sin(b)));
    }
  };
  if (s.kind === "sphere") {
    ring((c, sn) => [c, sn, 0]); ring((c, sn) => [c, 0, sn]); ring((c, sn) => [0, c, sn]);
  } else {
    const rt = s.taper;
    ring((c, sn) => [c, -0.5, sn]); ring((c, sn) => [c * rt, 0.5, sn * rt]);
    for (let q = 0; q < 4; q++) { const c = Math.cos((q * Math.PI) / 2), sn = Math.sin((q * Math.PI) / 2); put(c, -0.5, sn); put(c * rt, 0.5, sn * rt); }
  }
  return Float32Array.from(out);
}
/** Edge segments (index pairs): every open prim's chain, every closed prim's loop (the wireframe overlay). */
export function edgeSegments(prims: number[][], closed: boolean[], pointCount: number, onlyOpen = false): Uint32Array {
  const out: number[] = [];
  prims.forEach((p, k) => {
    if (p.length < 2 || p.some((i) => !(i >= 0 && i < pointCount))) return;
    if (onlyOpen && closed[k]) return;
    for (let i = 0; i + 1 < p.length; i++) out.push(p[i], p[i + 1]);
    if (closed[k] && p.length > 2) out.push(p[p.length - 1], p[0]);
  });
  return Uint32Array.from(out);
}

export interface Bounds { min: [number, number, number]; max: [number, number, number]; center: [number, number, number]; radius: number }

/** Bounding box + enclosing radius of a flat xyz array; `null` for an empty/non-finite cloud. */
export function bounds(pos: ArrayLike<number>): Bounds | null {
  if (pos.length < 3) return null;
  const min: [number, number, number] = [Infinity, Infinity, Infinity];
  const max: [number, number, number] = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i + 2 < pos.length; i += 3)
    for (let a = 0; a < 3; a++) {
      const v = pos[i + a];
      if (!Number.isFinite(v)) return null;
      if (v < min[a]) min[a] = v;
      if (v > max[a]) max[a] = v;
    }
  const center: [number, number, number] = [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2];
  const radius = Math.hypot(max[0] - min[0], max[1] - min[1], max[2] - min[2]) / 2;
  return { min, max, center, radius };
}

/** Network editor geometry shared by render + hit-testing. */
export const NODE_W = 150;
export const NODE_H = 40;
export const inPort = (x: number, y: number, i: number, n: number): [number, number] => [x + (NODE_W * (i + 1)) / (n + 1), y];
export const outPort = (x: number, y: number): [number, number] => [x + NODE_W / 2, y + NODE_H];
/** Cubic bezier from an output (bottom) to an input (top). */
export function wirePath(a: [number, number], b: [number, number]): string {
  const d = Math.max(30, Math.abs(b[1] - a[1]) / 2);
  return `M${a[0]},${a[1]} C${a[0]},${a[1] + d} ${b[0]},${b[1] - d} ${b[0]},${b[1]}`;
}
/** Fuzzy type filter for the Tab menu: prefix matches first, then substring, then subsequence. */
export function filterTypes(all: string[], q: string): string[] {
  const s = q.trim().toLowerCase();
  if (!s) return all;
  const sub = (t: string) => { let i = 0; for (const c of t) if (c === s[i]) i++; return i === s.length; };
  const rank = (t: string) => (t.startsWith(s) ? 0 : t.includes(s) ? 1 : sub(t) ? 2 : 3);
  return all.filter((t) => rank(t) < 3).sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
}
