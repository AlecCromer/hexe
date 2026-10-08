// Cd (colour) plumbing: the cook payload's `cd` is one rgb per prim CORNER (prim order, then corner order), so the surface is
// built corner-indexed. Pure functions (no three.js) so bun test can pin them.
import type { Cooked } from "./api";

/** Number of corners over all prims. */
export function cornerCount(prims: number[][]): number {
  let t = 0;
  for (const p of prims) t += p.length;
  return t;
}

/** Per-corner rgb as a Float32Array, or null when the cook carries no (or a mis-sized) `cd`. */
export function cornerColors(c: Pick<Cooked, "cd" | "prims">): Float32Array | null {
  if (!c.cd || c.cd.length === 0 || c.cd.length !== cornerCount(c.prims) * 3) return null;
  return Float32Array.from(c.cd);
}

/** The cook re-indexed per corner: positions[k] = position of corner k's point, prims = consecutive corner ids. A prim with an
 * out-of-range point is marked open (so triangulate skips it). */
export function cornerGeometry(c: Pick<Cooked, "positions" | "prims" | "closed">): { positions: Float32Array; prims: number[][]; closed: boolean[] } {
  const n = c.positions.length / 3;
  const positions = new Float32Array(cornerCount(c.prims) * 3);
  const prims: number[][] = [];
  const closed: boolean[] = [];
  let k = 0;
  c.prims.forEach((p, pi) => {
    const ids: number[] = [];
    let ok = true;
    for (const i of p) {
      if (!(i >= 0 && i < n)) ok = false;
      else positions.set(c.positions.slice(i * 3, i * 3 + 3), k * 3);
      ids.push(k++);
    }
    prims.push(ids);
    closed.push(ok && !!c.closed[pi]);
  });
  return { positions, prims, closed };
}
