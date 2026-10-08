// Pure windowing math for the virtualised geometry spreadsheet (no DOM: unit-tested with `bun test`).

/** Rows the sheet may expose at most (mirrors `MAX_ROWS` in src-tauri/src/sheet.rs). */
export const MAX_ROWS = 10_000;

/** Visible row range `[start, end)` for a scroll position, with `overscan` extra rows each side, clamped to `total`. */
export function visibleRange(scrollTop: number, viewH: number, rowH: number, total: number, overscan = 6): [number, number] {
  if (!(rowH > 0) || total <= 0) return [0, 0];
  const first = Math.floor(Math.max(0, scrollTop) / rowH);
  const last = Math.ceil((Math.max(0, scrollTop) + Math.max(0, viewH)) / rowH);
  const end = Math.min(total, last + overscan);
  return [Math.min(Math.max(0, first - overscan), end), end];
}

/** Page indices covering rows `[start, end)`. */
export function pagesFor(start: number, end: number, pageSize: number): number[] {
  if (end <= start || pageSize <= 0) return [];
  const out: number[] = [];
  for (let p = Math.floor(start / pageSize); p <= Math.floor((end - 1) / pageSize); p++) out.push(p);
  return out;
}

/** The `showing N of M` caption: N = rows addressable (cap 10k), M = elements in the geometry. */
export function showing(total: number): string {
  const n = Math.min(total, MAX_ROWS);
  return n < total ? `showing ${n} of ${total}` : `showing ${total} of ${total}`;
}
