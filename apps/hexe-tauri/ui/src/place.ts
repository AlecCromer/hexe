// Shelf placement (Houdini "Select position for Box"): which generators place interactively, the red preview
// wireframe each shows under the cursor, and the cursor -> construction plane (y = 0) hit. No DOM: unit-tested.
import type { PVal } from "./api";

/** Generators that place in the viewport: type -> the vector param that takes the clicked position. */
export const PLACE: Record<string, string> = { box: "t", sphere: "t", tube: "t", torus: "t", grid: "t", platonic: "t", circle: "center", line: "origin" };

type V3 = [number, number, number];
const v3 = (p: PVal | undefined, d: V3): V3 =>
  Array.isArray(p) && p.length === 3 && p.every((x) => Number.isFinite(x)) ? [p[0], p[1], p[2]] : d;
const num = (p: PVal | undefined, d: number): number => (typeof p === "number" && Number.isFinite(p) ? p : d);

/** Ring of `n` segments, radius r, in plane `plane` (xz | xy), centred at c. */
function ring(out: number[], c: V3, r: number, plane: "xz" | "xy" | "yz", n = 32): void {
  for (let i = 0; i < n; i++) {
    const a0 = (i / n) * Math.PI * 2, a1 = ((i + 1) / n) * Math.PI * 2;
    for (const a of [a0, a1]) {
      const u = Math.cos(a) * r, w = Math.sin(a) * r;
      if (plane === "xz") out.push(c[0] + u, c[1], c[2] + w);
      else if (plane === "xy") out.push(c[0] + u, c[1] + w, c[2]);
      else out.push(c[0], c[1] + u, c[2] + w);
    }
  }
}

/** Preview line segments (xyz pairs) around the origin for `type`, sized from its param defaults. Sizes are clamped to
 *  [1e-3, 1e4] so a hostile default can neither vanish nor blow up the preview. */
export function previewSegments(type: string, d: Record<string, PVal>): number[] {
  const cl = (x: number) => Math.min(1e4, Math.max(1e-3, Math.abs(x)));
  const o: number[] = [], z: V3 = [0, 0, 0];
  switch (type) {
    case "box": {
      const s = v3(d.size, [1, 1, 1]), k = cl(num(d.scale, 1)), [x, y, w] = s.map((q) => (cl(q) * k) / 2);
      const c = [[-x, -y, -w], [x, -y, -w], [x, y, -w], [-x, y, -w], [-x, -y, w], [x, -y, w], [x, y, w], [-x, y, w]];
      for (const [a, b] of [[0, 1], [1, 2], [2, 3], [3, 0], [4, 5], [5, 6], [6, 7], [7, 4], [0, 4], [1, 5], [2, 6], [3, 7]]) o.push(...c[a], ...c[b]);
      break;
    }
    case "sphere": case "platonic": {
      const r = type === "sphere" ? cl(v3(d.rad, [1, 1, 1])[0]) * cl(num(d.scale, 1)) : cl(num(d.radius, 1));
      ring(o, z, r, "xz"); ring(o, z, r, "xy"); ring(o, z, r, "yz");
      break;
    }
    case "tube": {
      const r = v3(d.rad, [1, 1, 0]), k = cl(num(d.radscale, 1)), h = cl(num(d.height, 1)) / 2;
      const r0 = cl(r[0]) * k, r1 = cl(r[1]) * k;
      ring(o, [0, -h, 0], r0, "xz"); ring(o, [0, h, 0], r1, "xz");
      for (let i = 0; i < 4; i++) { const a = (i * Math.PI) / 2; o.push(Math.cos(a) * r0, -h, Math.sin(a) * r0, Math.cos(a) * r1, h, Math.sin(a) * r1); }
      break;
    }
    case "torus": {
      const r = v3(d.rad, [1, 0.5, 0]), k = cl(num(d.scale, 1)), a = cl(r[0]) * k, b = cl(r[1]) * k;
      ring(o, z, a + b, "xz"); ring(o, z, Math.max(1e-3, a - b), "xz"); ring(o, [0, b, 0], a, "xz"); ring(o, [0, -b, 0], a, "xz");
      break;
    }
    case "grid": {
      const s = v3(d.size, [10, 10, 0]), x = cl(s[0]) / 2, w = cl(s[1]) / 2;
      const rows = Math.min(100, Math.max(2, Math.round(num(d.rows, 10)))), cols = Math.min(100, Math.max(2, Math.round(num(d.cols, 10))));
      for (let i = 0; i < rows; i++) { const t = -w + (2 * w * i) / (rows - 1); o.push(-x, 0, t, x, 0, t); }
      for (let j = 0; j < cols; j++) { const t = -x + (2 * x * j) / (cols - 1); o.push(t, 0, -w, t, 0, w); }
      break;
    }
    case "circle": ring(o, z, cl(v3(d.radius, [1, 1, 0])[0]), "xy"); break;
    case "line": {
      const dir = v3(d.dir, [0, 1, 0]), n = Math.hypot(...dir) || 1, L = cl(num(d.dist, 1));
      o.push(0, 0, 0, (dir[0] / n) * L, (dir[1] / n) * L, (dir[2] / n) * L);
      break;
    }
    case "nullobj": // Null object: axes cross (half length 0.5) + a small ring on the ground
      o.push(-0.5, 0, 0, 0.5, 0, 0, 0, 0, 0, 0, 0.5, 0, 0, 0, -0.5, 0, 0, 0.5);
      ring(o, z, 0.15, "xz", 16);
      break;
    default: ring(o, z, 1, "xz");
  }
  return o;
}

/** Ray (origin, direction) hit on the construction plane y = 0; null when parallel, behind the eye, or absurdly far. */
export function groundHit(o: V3, dir: V3): V3 | null {
  if (!o.every(Number.isFinite) || !dir.every(Number.isFinite) || Math.abs(dir[1]) < 1e-9) return null;
  const t = -o[1] / dir[1];
  if (!(t > 0) || t > 1e6) return null;
  return [o[0] + dir[0] * t, 0, o[2] + dir[2] * t];
}

/** Houdini-style coordinate readout: `(x, y, z)` with up to 6 significant digits. */
export const fmtPos = (p: V3): string => `(${p.map((x) => String(Number(x.toPrecision(6)))).join(", ")})`;
