// Parameter right-click menu (Houdini's, entry for entry): Revert to Previous Value, Expression, Copy / Paste (values,
// expressions, relative / absolute references), Delete Channels, Lock / Unlock, Revert to Defaults, Make Current Value Default. Paste Relative References writes `ch("../null1/p")` channel
// expressions (Rust evaluates them before every cook: hexe_sop::chans).
import type { PVal } from "./api";

/** Component suffixes Houdini gives tuple parameters (`tx ty tz`, `newparameterx ...`). */
const SUFFIX = ["x", "y", "z", "w"];
/** Channel names of parameter `name` with `size` components. */
export function channelNames(name: string, size: number): string[] {
  return size <= 1 ? [name] : SUFFIX.slice(0, Math.min(4, size)).map((s) => `${name}${s}`);
}

/** What Copy Parameter keeps: where it came from (network path like `/obj/geo1`, node name) and its channels + value. */
export interface ParmClip { net: string; node: string; parm: string; chans: string[]; value: PVal | number[]; /** its channel expressions (Paste Expressions / Channels) */ exprs?: Record<string, string> }
let clip: ParmClip | null = null;
export const copyParm = (c: ParmClip): void => { clip = c; };
export const clipboard = (): ParmClip | null => clip;

/** Path from the node at `toNet/toNode` to `fromNet/fromNode`, Houdini style: the node itself = "", a sibling = `../n`,
 *  another network = `../../<obj>/n` (relative from the node, up through its network). */
export function relPath(toNet: string, toNode: string, fromNet: string, fromNode: string): string {
  if (toNet === fromNet) return toNode === fromNode ? "" : `../${fromNode}`;
  const a = toNet.split("/").filter(Boolean), b = fromNet.split("/").filter(Boolean);
  let k = 0;
  while (k < a.length && k < b.length && a[k] === b[k]) k++;
  return [...Array(a.length - k + 1).fill(".."), ...b.slice(k), fromNode].join("/");
}

/** Paste Relative / Absolute References: target channel -> expression, component by component (a scalar source feeds
 *  only the first target channel; extra target components without a source are left alone). */
export function referenceExprs(src: ParmClip, toNet: string, toNode: string, targetChans: string[], absolute = false): [string, string][] {
  const base = absolute ? `${src.net}/${src.node}` : relPath(toNet, toNode, src.net, src.node);
  return targetChans.flatMap((t, i): [string, string][] => {
    const s = src.chans[i];
    return s === undefined ? [] : [[t, `ch("${base ? `${base}/` : ""}${s}")`]];
  });
}

/** What the menu can do for the parameter under the pointer; a missing handler = that entry is greyed out. */
export interface MenuHandlers {
  copy: () => void;
  revertPrevious?: () => void;
  editExpression?: () => void;
  deleteExpression?: () => void;
  pasteValues?: () => void;
  pasteExpressions?: () => void;
  pasteRelative?: () => void;
  pasteAbsolute?: () => void;
  deleteChannels?: () => void;
  lock?: () => void;
  unlock?: () => void;
  revertDefaults?: () => void;
  makeDefault?: () => void;
}

/** One menu entry: a label, its Houdini shortcut, an action or a submenu (`sub`); neither = not in hexe yet. */
export interface MenuEntry { label: string; key?: string; run?: () => void; sub?: MenuEntry[] }
/** Houdini's parameter right-click menu, in order (`null` = separator). */
export function parmMenuEntries(h: MenuHandlers): (MenuEntry | null)[] {
  return [
    { label: "Revert to Previous Value", key: "Shift+RMB", run: h.revertPrevious },
    { label: "Channels", sub: [] },
    { label: "Keyframes", sub: [] },
    { label: "Expression", sub: [{ label: "Edit Expression...", run: h.editExpression }, { label: "Delete Expression", run: h.deleteExpression }] },
    { label: "Motion FX", sub: [] },
    { label: "Reference", sub: [] },
    null,
    { label: "Copy Parameter", run: h.copy },
    { label: "Paste Values", run: h.pasteValues },
    { label: "Paste Expressions / Channels", run: h.pasteExpressions },
    { label: "Paste Relative References", run: h.pasteRelative },
    { label: "Paste Other", sub: [{ label: "Paste Absolute References", run: h.pasteAbsolute }] },
    { label: "Delete Channels", key: "Ctrl+Shift+LMB", run: h.deleteChannels },
    { label: "Include in Take" },
    { label: "Lock Parameter", run: h.lock },
    { label: "Unlock Parameter", run: h.unlock },
    { label: "Revert to Defaults", key: "Ctrl+MMB", run: h.revertDefaults },
    { label: "Revert to and Restore Factory Defaults" },
    { label: "Make Current Value Default", run: h.makeDefault },
    { label: "Export Parameter to Type Properties", key: "Alt+MMB" },
    { label: "More", sub: [] },
  ];
}
const live = (e: MenuEntry): boolean => !!e.run || (e.sub ?? []).some(live);

let open: HTMLElement[] = [];
export function closeParmMenu(): void { for (const m of open) m.remove(); open = []; }
if (typeof document !== "undefined") {
  document.addEventListener("mousedown", (e) => { if (open.length && !open.some((m) => m.contains(e.target as Node))) closeParmMenu(); });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") closeParmMenu(); });
}
/** Build one menu level at client (x, y); `depth` 0 = the root (closes deeper levels on hover). */
function level(entries: (MenuEntry | null)[], x: number, y: number, depth: number): HTMLElement {
  const m = document.createElement("div");
  m.className = "mdrop parmmenu";
  for (const e of entries) {
    if (!e) { const hr = document.createElement("hr"); hr.className = "msep"; m.append(hr); continue; }
    const b = document.createElement("button");
    b.className = "mi";
    b.dataset.pm = e.label;
    b.disabled = !live(e);
    if (b.disabled) b.title = "Not in hexe yet";
    b.append(e.label);
    if (e.key) { const k = document.createElement("kbd"); k.textContent = e.key; b.append(k); }
    if (e.sub) { const s = document.createElement("span"); s.className = "msub"; s.textContent = "▶"; b.append(s); }
    b.addEventListener("mouseenter", () => {
      for (const deeper of open.slice(depth + 1)) deeper.remove();
      open = open.slice(0, depth + 1);
      if (e.sub && live(e)) {
        const r = b.getBoundingClientRect();
        open.push(level(e.sub, r.right - 2, r.top - 3, depth + 1));
      }
    });
    b.addEventListener("click", () => { if (e.run) { closeParmMenu(); e.run(); } });
    m.append(b);
  }
  m.style.left = `${x}px`;
  m.style.top = `${y}px`;
  document.body.append(m);
  const r = m.getBoundingClientRect();
  if (r.bottom > window.innerHeight) m.style.top = `${Math.max(0, window.innerHeight - r.height)}px`;
  if (r.right > window.innerWidth) m.style.left = `${Math.max(0, x - r.width - (depth ? r.width : 0))}px`;
  return m;
}
/** Show the menu at client (x, y). */
export function showParmMenu(x: number, y: number, h: MenuHandlers): void {
  closeParmMenu();
  open = [level(parmMenuEntries(h), x, y, 0)];
}
