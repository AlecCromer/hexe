// Pure geometry helpers for the viewport (no three.js import: unit-tested with `bun test`).

/** Fan-triangulate closed prims (v0,v[i],v[i+1]); prims with <3 points or any out-of-range index are skipped. */
export function fanTriangulate(prims: number[][], closed: boolean[], pointCount: number): Uint32Array {
  const out: number[] = [];
  prims.forEach((p, k) => {
    if (!closed[k] || p.length < 3 || p.some((i) => !(i >= 0 && i < pointCount))) return;
    for (let i = 1; i + 1 < p.length; i++) out.push(p[0], p[i], p[i + 1]);
  });
  return Uint32Array.from(out);
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
