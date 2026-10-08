// hexe UI: viewport (three.js) + param pane + network editor over the Tauri engine commands. State lives in Rust;
// every edit = command -> graph_get -> render, every graph/param change -> cook -> viewport.
import "./style.css";
import { api, msg, type GraphView, type NodeType, type PVal } from "./api";
import { Network } from "./network";
import { renderParams } from "./params";
import { Spreadsheet } from "./sheet";
import { Viewport } from "./viewport";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const status = $("status");
const say = (m: string, err = false) => { status.textContent = m; status.className = err ? "err" : ""; };

let types: NodeType[] = [];
let graph: GraphView = { nodes: [], edges: [], display: null };
let net: Network;
const viewport = new Viewport($("viewport"));
const sheet = new Spreadsheet($("sheet"), (c, o, l) => api.spreadsheet(c, o, l));

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
      viewport.show(c);
      sheet.invalidate(!c.error);
      const ms = (performance.now() - t0).toFixed(0);
      if (c.error) say(c.error, true);
      else say(`${c.counts.points} pts · ${c.counts.prims} prims (${c.counts.closed} poly, ${c.counts.open} open${c.counts.unsupported ? `, ${c.counts.unsupported} undrawn` : ""}) · ${ms} ms`);
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

  $("wire").addEventListener("change", (e) => viewport.setWireframe((e.target as HTMLInputElement).checked));
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

  const start = await api.startupScene();
  if (start) { path.value = start; await open(); } else { await refresh(); showParams(); }
  // proof / test door (read by the T1 proof script): drive the real param pane through the DOM
  (window as unknown as Record<string, unknown>).__hexe = { select: (name: string) => { const n = graph.nodes.find((q) => q.name === name); if (n) net.selectNode(n.id); }, recook };
}

main().catch((e) => say(msg(e), true));
