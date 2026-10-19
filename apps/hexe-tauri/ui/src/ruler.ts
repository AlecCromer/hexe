// Playbar ruler math (no DOM: unit-tested with `bun test`).

/** Frame steps the ruler may label (film-friendly: multiples of 24 once zoomed out). */
const STEPS = [1, 2, 4, 6, 12, 24, 48, 96, 120, 240, 480, 960, 2400, 4800, 9600, 24000, 48000, 96000];

/** Ticks for frames `[a, b]` drawn `widthPx` wide: `[frame, major]`, majors >= 100 px apart and labelled,
 *  minors at a quarter (or half) of the major step. Bounded: at most 4096 ticks. */
export function rulerTicks(a: number, b: number, widthPx: number): [number, boolean][] {
  if (!(b > a) || !(widthPx > 0) || !Number.isFinite(a) || !Number.isFinite(b)) return [];
  const span = b - a;
  const major = STEPS.find((s) => (s / span) * widthPx >= 100) ?? Math.ceil(span / Math.max(1, widthPx / 100));
  const minor = major % 4 === 0 ? major / 4 : major % 2 === 0 ? major / 2 : major;
  const out: [number, boolean][] = [];
  for (let f = Math.ceil(a / minor) * minor; f <= b && out.length < 4096; f += minor) {
    if (f >= a) out.push([f, f % major === 0]);
  }
  return out;
}
