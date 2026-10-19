// Add SOP `points` (whitespace-separated `x,y,z` triples, the engine's grammar) <-> the param pane's point list
// (Houdini: Number of Points + one `Point N` row of x y z and a weight per point). Pure: unit-tested with `bun test`.

export type Pt = [number, number, number];

/** `"0,0,0 1,2,3"` -> points; a malformed triple (not three finite numbers) is skipped, never invented. */
export function parsePoints(text: string): Pt[] {
  return text.split(/\s+/).filter(Boolean).flatMap((tok): Pt[] => {
    const v = tok.split(",").map(Number);
    return v.length === 3 && v.every(Number.isFinite) ? [[v[0], v[1], v[2]]] : [];
  });
}

/** points -> the engine's `points` text */
export const formatPoints = (pts: Pt[]): string => pts.map((p) => p.join(",")).join(" ");

/** Number of Points changed to `n`: extra rows go, new rows start at the origin (Houdini). Capped at `max`. */
export function resizePoints(pts: Pt[], n: number, max = 10_000): Pt[] {
  const k = Math.max(0, Math.min(max, Math.floor(Number.isFinite(n) ? n : 0)));
  return Array.from({ length: k }, (_, i): Pt => pts[i] ?? [0, 0, 0]);
}
