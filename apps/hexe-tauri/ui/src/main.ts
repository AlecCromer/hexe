// hexe UI: viewport (three.js) + param pane + network editor over the Tauri engine commands. State lives in Rust;
// every edit = command -> graph_get -> render, every graph/param change -> cook -> viewport.
import "./style.css";
import { api, msg, type Cooked, type GraphView, type NodeType, type PVal } from "./api";
import { Network } from "./network";
import { renderParams } from "./params";
import { Spreadsheet } from "./sheet";
import { Viewport } from "./viewport";
import "./shell";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const status = $("status");
const say = (m: string, err = false, warn = false) => {
  status.textContent = m;
  status.className = err ? "err" : warn ? "warn" : "";
  $("bar").classList.toggle("err", err);
};
/** red border on the node whose cook failed: a style rule keyed by node id survives network re-renders */
const errStyle = document.head.appendChild(document.createElement("style"));
const markError = (id: number | null) => { errStyle.textContent = id == null ? "" : `.node[data-node="${id}"] rect.body { stroke: var(--bad); stroke-width: 3.5; }`; };

let types: NodeType[] = [];
let graph: GraphView = { nodes: [], edges: [], display: null };
let net: Network;
const viewport = new Viewport($("viewport"));
const sheet = new Spreadsheet($("sheet"), (c, o, l) => api.spreadsheet(c, o, l));

/** `display: <name> · pts · prims · verts · cook ms` (+ undrawn / cap warnings) */
function statusLine(c: Cooked, g: GraphView, ms: string, warn: string[]): string {
  const d = g.nodes.find((n) => n.id === g.display);
  if (!d) return "no display node (right-click a node or press D)";
  const verts = c.prims.reduce((a, p) => a + p.length, 0) + c.shapes.length;
  const un = c.counts.unsupported ? ` · ${c.counts.unsupported} undrawn` : "";
  return `display: ${d.name} · ${c.counts.points} pts · ${c.counts.prims} prims · ${verts} verts · ${ms} ms${un}${warn.length ? ` · WARNING: ${warn.join("; ")}` : ""}`;
}
// cook requests never overlap: while one is in flight only the latest follow-up is kept
let cooking = false, again = false;
async function recook(): Promise<void> {
  if (cooking) { again = true; return; }
  cooking = true;
  try {
    do {
      again = false;
      const t0 = performance.now();
      const c = await api.cook();
      const ms = (performance.now() - t0).toFixed(0);
      const warn = viewport.show(c);
      sheet.invalidate(!c.error);
      markError(c.error ? c.error_node : null);
      if (c.error === "no display node") say("no display node (right-click a node or press D)");
      else if (c.error) say(c.error, true);
      else say(statusLine(c, graph, ms, warn), false, warn.length > 0);
    } while (again);
  } catch (e) {
    say(msg(e), true);
  } finally {
    cooking = false;
  }
}

function showParams(): void {
  const id = net.selectedNode;
  const node = graph.nodes.find((n) => n.id === id) ?? null;
  renderParams($("params"), node, types.find((t) => t.type === node?.type), setParam);
}

// param edits: serialized + coalesced per param (a slider drag fires many input events)
const pendingParams = new Map<string, [number, string, PVal]>();
let flushing = false;
async function setParam(id: number, name: string, value: PVal): Promise<void> {
  pendingParams.set(`${id}:${name}`, [id, name, value]);
  if (flushing) return;
  flushing = true;
  try {
    while (pendingParams.size) {
      const [k, [i, n, v]] = pendingParams.entries().next().value!;
      pendingParams.delete(k);
      await api.nodeSetParam(i, n, v);
      await recook();
    }
  } catch (e) {
    say(msg(e), true);
    throw e;
  } finally {
    flushing = false;
  }
}

async function refresh(cook = true): Promise<void> {
  graph = await api.graphGet();
  net.setGraph(graph);
  if (cook) await recook();
}

/** run a structural command, then re-sync the graph + viewport; failures land in the status bar */
const structural = (f: () => Promise<unknown>) => async () => {
  try { await f(); } catch (e) { say(msg(e), true); }
  await refresh();
  showParams();
};

async function main(): Promise<void> {
  types = await api.listNodeTypes();
  net = new Network($("network"), types, {
    select: () => showParams(),
    add: async (t, x, y) => { await structural(async () => { const id = await api.nodeAdd(t, x, y); await refresh(false); net.selectNode(id); })(); },
    remove: (id) => structural(() => api.nodeRemove(id))(),
    move: async (id, x, y) => { await api.nodeMove(id, x, y); const n = graph.nodes.find((q) => q.id === id); if (n) { n.x = x; n.y = y; } },
    connect: (f, t, i) => structural(() => api.edgeConnect(f, t, i))(),
    disconnect: (t, i) => structural(() => api.edgeDisconnect(t, i))(),
    display: (id) => structural(() => api.setDisplay(id))(),
    refresh: () => structural(async () => {})(),
    error: (m) => say(m, true),
  });

  const wire = $<HTMLInputElement>("wire");
  wire.addEventListener("change", () => viewport.setWireframe(wire.checked));
  $("ptsize").addEventListener("input", (e) => viewport.setPointSize(Number((e.target as HTMLInputElement).value)));
  $("pts").addEventListener("change", (e) => viewport.setPoints((e.target as HTMLInputElement).checked));
  $("fit").addEventListener("click", () => viewport.fitCurrent());
  const path = $<HTMLInputElement>("path");
  const open = async () => {
    try {
      graph = await api.graphLoad(path.value.trim());
      net.selectNode(null);
      net.setGraph(graph);
      await recook();
      showParams();
    } catch (e) { say(msg(e), true); }
  };
  $("open").addEventListener("click", open);
  path.addEventListener("keydown", (e) => { if (e.key === "Enter") open(); });
  $("save").addEventListener("click", async () => {
    try { await api.graphSave(path.value.trim()); say(`saved ${path.value.trim()}`); } catch (e) { say(msg(e), true); }
  });
  $("new").addEventListener("click", structural(async () => { await api.graphNew(); net.selectNode(null); }));

  $("save").title = "Cmd+S"; $("open").title = "Cmd+O";
  // hotkeys: viewport-hover scoped H/F W P N; network-hover scoped D; Cmd+S / Cmd+O / R global; never while typing
  let mouse = { x: 0, y: 0 };
  window.addEventListener("mousemove", (e) => (mouse = { x: e.clientX, y: e.clientY }));
  const typing = (t: EventTarget | null) => {
    const e = t as HTMLInputElement | null;
    return !!e && (e.tagName === "TEXTAREA" || e.tagName === "SELECT" || (e.tagName === "INPUT" && e.type !== "checkbox" && e.type !== "range"));
  };
  window.addEventListener("keydown", (e) => {
    const k = e.key.toLowerCase();
    if ((e.metaKey || e.ctrlKey) && !e.altKey && (k === "s" || k === "o")) {
      e.preventDefault();
      if (k === "s") $("save").click(); else void open();
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey || typing(e.target)) return;
    if (k === "r") { e.preventDefault(); void recook(); return; }
    if (viewport.hover) {
      if (k === "h" || k === "f") viewport.fitCurrent();
      else if (k === "w") { wire.checked = !viewport.wireframe; viewport.setWireframe(wire.checked); }
      else if (k === "p") viewport.setNumbers("p", !viewport.numbers.p);
      else if (k === "n") viewport.setNumbers("n", !viewport.numbers.n);
      else return;
      e.preventDefault();
      return;
    }
    if (k === "d") {
      const at = document.elementFromPoint(mouse.x, mouse.y);
      const id = at && $("network").contains(at) ? at.closest<SVGElement>("[data-node]")?.dataset.node : undefined;
      if (id !== undefined) { e.preventDefault(); void structural(() => api.setDisplay(Number(id)))(); }
    }
  });
  const start = await api.startupScene();
  if (start) { path.value = start; await open(); } else { await refresh(); showParams(); }
  // proof / test door (read by the T1 proof script): drive the real param pane through the DOM
  (window as unknown as Record<string, unknown>).__hexe = { select: (name: string) => { const n = graph.nodes.find((q) => q.name === name); if (n) net.selectNode(n.id); }, recook };
}

main().catch((e) => say(msg(e), true));
