// Click-drag value scrubbing for number fields (no DOM: unit-tested with `bun test`).
import type { ParamInfo } from "./api";

type Spec = Pick<ParamInfo, "kind" | "min" | "max">;
export const isIntKind = (p: Spec): boolean => p.kind === "int" || p.kind === "menu";
/** Pixels of horizontal travel before a press becomes a scrub (below it = a plain click that focuses the field). */
export const SCRUB_THRESHOLD = 3;
/** A full registry range is covered by this many px of drag. */
const RANGE_PX = 200;
/** Slowest int scrub: one unit per this many px. */
const INT_PX = 12;

/** Value units per px of drag: the registry range / 200 px; ints never slower than 1 unit per 12 px; no range = 0.01 (float) / 1 per 12 px (int). */
export function scrubSpeed(p: Spec): number {
  const span = p.min !== null && p.max !== null ? p.max - p.min : 0;
  const base = span > 0 ? span / RANGE_PX : isIntKind(p) ? 1 / INT_PX : 0.01;
  return isIntKind(p) ? Math.max(base, 1 / INT_PX) : base;
}

/** New value after dragging `dx` px from `start`. Shift = 10x finer, Alt/Ctrl = 10x coarser; ints round; clamped to the registry range. */
export function scrubbed(p: Spec, start: number, dx: number, mods: { fine?: boolean; coarse?: boolean } = {}): number {
  const m = mods.fine ? 0.1 : mods.coarse ? 10 : 1;
  let v = start + dx * scrubSpeed(p) * m;
  if (isIntKind(p)) v = Math.round(v);
  else v = Number(v.toPrecision(7)); // no 0.30000000000000004 in the field
  if (p.min !== null && v < p.min) v = p.min;
  if (p.max !== null && v > p.max) v = p.max;
  return v;
}

/** Attach scrubbing to a number `<input>`: press-drag changes the value (live `commit`), a click without travel focuses it. */
export function attachScrub(input: HTMLInputElement, p: Spec, commit: (v: number) => void, current: () => number = () => Number(input.value)): void {
  input.addEventListener("mousedown", (e) => {
    if (e.button !== 0 || document.activeElement === input) return; // an already-focused field edits as text
    const x0 = e.clientX;
    const start = current();
    if (!Number.isFinite(start)) return;
    let on = false;
    e.preventDefault(); // no focus / text selection until we know it is a click
    const move = (m: MouseEvent) => {
      const dx = m.clientX - x0;
      if (!on && Math.abs(dx) < SCRUB_THRESHOLD) return;
      on = true;
      document.body.classList.add("scrubbing");
      const v = scrubbed(p, start, dx, { fine: m.shiftKey, coarse: m.altKey || m.ctrlKey });
      input.value = String(v);
      input.dispatchEvent(new Event("input", { bubbles: true })); // keeps the range slider in sync
      commit(v);
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.classList.remove("scrubbing");
      if (!on) { input.focus(); input.select(); }
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  });
}
