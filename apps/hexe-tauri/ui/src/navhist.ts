// Network path history (no DOM: unit-tested with `bun test`): the pane's back / forward arrows, like a browser.

export interface Nav<L> { back: L[]; cur: L; fwd: L[] }

/** Bounded: the oldest entries drop off past `max`. */
export const MAX_NAV = 50;

export const navStart = <L>(cur: L): Nav<L> => ({ back: [], cur, fwd: [] });

/** Went somewhere new: `cur` joins the back list, the forward list is dropped. Same place = unchanged. */
export function navVisit<L>(h: Nav<L>, to: L, max = MAX_NAV): Nav<L> {
  if (to === h.cur) return h;
  return { back: [...h.back, h.cur].slice(-max), cur: to, fwd: [] };
}

/** Back arrow (unchanged when there is nothing behind). */
export function navBack<L>(h: Nav<L>): Nav<L> {
  if (!h.back.length) return h;
  return { back: h.back.slice(0, -1), cur: h.back[h.back.length - 1], fwd: [h.cur, ...h.fwd] };
}

/** Forward arrow (unchanged when there is nothing ahead). */
export function navForward<L>(h: Nav<L>): Nav<L> {
  if (!h.fwd.length) return h;
  return { back: [...h.back, h.cur], cur: h.fwd[0], fwd: h.fwd.slice(1) };
}
