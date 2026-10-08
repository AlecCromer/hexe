// Viewport point picking maths (pure, unit-tested). Screen points are CSS px relative to the viewport; `NaN` = not visible.

/** Pick radius (CSS px): a click selects the nearest point within this distance. */
export const PICK_PX = 8;
/** Pointer travel (CSS px) below which a press-release counts as a click, not a drag. */
export const CLICK_SLOP = 4;

/** Project `n` points (flat xyz) with a column-major view-projection matrix `m` into viewport px `[x0, y0, x1, y1, ...]`. */
export function projectAll(positions: ArrayLike<number>, m: ArrayLike<number>, w: number, h: number): Float32Array {
  const n = Math.floor(positions.length / 3), out = new Float32Array(n * 2);
  for (let i = 0; i < n; i++) {
    const x = positions[i * 3], y = positions[i * 3 + 1], z = positions[i * 3 + 2];
    const cw = m[3] * x + m[7] * y + m[11] * z + m[15];
    const cx = m[0] * x + m[4] * y + m[8] * z + m[12], cy = m[1] * x + m[5] * y + m[9] * z + m[13], cz = m[2] * x + m[6] * y + m[10] * z + m[14];
    if (!(cw > 1e-9) || Math.abs(cz) > cw) { out[i * 2] = out[i * 2 + 1] = NaN; continue; } // behind the eye / outside near-far
    out[i * 2] = ((cx / cw + 1) / 2) * w;
    out[i * 2 + 1] = ((1 - cy / cw) / 2) * h;
  }
  return out;
}

/** Index of the point nearest to (mx, my) within `radius` px, or -1. Ties: lowest index. */
export function nearest(screen: ArrayLike<number>, mx: number, my: number, radius = PICK_PX): number {
  let best = -1, bd = radius * radius;
  for (let i = 0; i < screen.length / 2; i++) {
    const dx = screen[i * 2] - mx, dy = screen[i * 2 + 1] - my;
    const d = dx * dx + dy * dy;
    if (d <= bd) { if (best < 0 || d < bd) { best = i; bd = d; } }
  }
  return best;
}

/** Points inside the (unordered) box corners, ascending. */
export function inBox(screen: ArrayLike<number>, ax: number, ay: number, bx: number, by: number): number[] {
  const [x0, x1, y0, y1] = [Math.min(ax, bx), Math.max(ax, bx), Math.min(ay, by), Math.max(ay, by)];
  const out: number[] = [];
  for (let i = 0; i < screen.length / 2; i++) {
    const x = screen[i * 2], y = screen[i * 2 + 1];
    if (x >= x0 && x <= x1 && y >= y0 && y <= y1) out.push(i);
  }
  return out;
}

/** New selection after picking `hit`: replace, or (`add`) union - a single already-selected point toggles off. Ascending. */
export function combine(cur: readonly number[], hit: readonly number[], add: boolean): number[] {
  if (!add) return [...hit].sort((a, b) => a - b);
  const set = new Set(cur);
  if (hit.length === 1 && set.has(hit[0])) set.delete(hit[0]);
  else for (const h of hit) set.add(h);
  return [...set].sort((a, b) => a - b);
}

/** Status text: one point = `pt N  P (x, y, z)`; several = count + the first ptnums. */
export function describe(sel: readonly number[], positions: ArrayLike<number>): string {
  if (!sel.length) return "selection cleared";
  if (sel.length === 1) {
    const i = sel[0], f = (k: number) => Number(positions[i * 3 + k]).toFixed(3);
    return `pt ${i}  P (${f(0)}, ${f(1)}, ${f(2)})`;
  }
  const head = sel.slice(0, 12).join(" ");
  return `${sel.length} points selected: ${head}${sel.length > 12 ? " ..." : ""}`;
}

/** Drop ids that no longer exist after a re-cook. */
export const clampSel = (sel: readonly number[], count: number): number[] => sel.filter((i) => i < count);
