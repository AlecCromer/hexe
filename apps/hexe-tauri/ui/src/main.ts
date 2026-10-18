// hexe UI: viewport (three.js) + param pane + network editor over the Tauri engine commands. State lives in Rust;
// every edit = command -> graph_get -> render, every graph/param change -> cook -> viewport.
import "./style.css";
import "./t4.css";
import { api, msg, type Cooked, type GraphView, type NodeType, type PVal, type RenderReq } from "./api";
import { Network } from "./network";
import { renderParams, showCookError } from "./params";
import { Spreadsheet } from "./sheet";
import { Viewport } from "./viewport";
import { frameScheduler } from "./frame";
import { Player } from "./play";
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
let lastCook: Cooked | null = null;
/** one-shot suffix for the next status line (e.g. `grouped 4 points -> selection`) */
let note = "";
const viewport = new Viewport($("viewport"));
const sheet = new Spreadsheet($("sheet"), (c, o, l) => api.spreadsheet(c, o, l));

/** `display: <name> · pts · prims · verts · cook ms` (+ undrawn / cap warnings) */
function statusLine(c: Cooked, g: GraphView, ms: string, warn: string[]): string {
  const d = g.nodes.find((n) => n.id === g.display);
  if (!d) return "no display node (right-click a node or press D)";
  const verts = c.prims.reduce((a, p) => a + p.length, 0) + c.shapes.length;
  const un = c.counts.unsupported ? ` · ${c.counts.unsupported} undrawn` : "";
    const n = note ? ` · ${note}` : "";
    note = "";
    return `display: ${d.name} · ${c.counts.points} pts · ${c.counts.prims} prims · ${verts} verts · ${ms} ms${un}${warn.length ? ` · WARNING: ${warn.join("; ")}` : ""}${n}`;
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
            lastCook = c;
            const warn = viewport.show(c);
            showCookError(c.error && c.error !== "no display node" ? c.error_node : null, c.error && c.error !== "no display node" ? c.error : null);
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
$<HTMLInputElement>("fitx").addEventListener("change", (e) => { viewport.setFitExclude((e.target as HTMLInputElement).value); viewport.fitCurrent(); });
    // viewport picking: selection lives here + in Rust (`selection_get`); the group button writes it into a new group SOP
    const grp = $<HTMLButtonElement>("grpsel"), pickBtn = $("pick");
    const syncPick = () => pickBtn.classList.toggle("on", viewport.pickMode);
    viewport.onState = syncPick;
    viewport.onPick = (sel, info) => {
      grp.disabled = sel.length === 0;
      api.selectionSet(sel).catch((e) => say(msg(e), true));
      say(info);
    };
    pickBtn.addEventListener("click", () => viewport.setPickMode(!viewport.pickMode));
    grp.addEventListener("click", structural(async () => {
      const n = viewport.selection.length;
      const id = await api.groupFromSelection();
      await refresh(false);
      net.selectNode(id);
      note = `grouped ${n} point${n === 1 ? "" : "s"} -> group node #${id}`;
    }));
  const path = $<HTMLInputElement>("path");
  const open = async () => {
    try {
      const p = path.value.trim();
      if (/\.(ens|nmc|nmb|nma)$/i.test(p)) {
        // Endorphin file: one `file` SOP over it (hexe-geo read_any opens .ens/.nmc as rest-pose geometry).
        await api.graphNew();
        const id = await api.nodeAdd("file", 0, 0);
        await api.nodeSetParam(id, "file", p);
        await api.setDisplay(id);
        graph = await api.graphGet();
      } else {
        graph = await api.graphLoad(p);
      }
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
            else if (k === "s") viewport.setPickMode(!viewport.pickMode);
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
  // timeline: `$F` lives in Rust (`--frame`/`--fps` seed it); number box + slider scrub it, every change re-cooks at ($F - 1) / fps
  const frameBox = $<HTMLInputElement>("frame"), frameSlider = $<HTMLInputElement>("frameslider");
  const showFrame = (f: number, fps: number) => {
    frameBox.value = String(f);
    frameSlider.max = String(Math.max(360, f));
    frameSlider.value = String(f);
    $("fpslab").textContent = `${fps} fps`;
  };
  const scrub = async (raw: string) => {
    const f = Math.round(Number(raw));
    if (!Number.isFinite(f) || f < 1) return;
    try { await api.timelineSet(f); } catch (e) { say(msg(e), true); return; }
    const [cur, fps] = await api.timelineGet();
    showFrame(cur, fps);
    await recook();
  };
  // Play / Stop: advance `$F` at the timeline fps against the wall clock (frames are dropped when a cook is slower than 1/fps);
  // the frame scheduler is rAF with a timer fallback, so playback also runs in an occluded window. Touching the slider/box stops it.
  const playBtn = $<HTMLButtonElement>("play");
  const player = new Player();
  let playBusy = false;
  const playTick = frameScheduler(() => void (async () => {
    if (!player.running) return;
    const [cur] = await api.timelineGet();
    const want = player.frameAt(performance.now());
    if (want !== cur && !playBusy) {
      playBusy = true;
      try { await scrub(String(want)); } finally { playBusy = false; }
    }
    if (player.running) playTick();
  })());
  const setPlaying = async (on: boolean) => {
    if (on === player.running) return;
    if (on) {
      const [cur, fps] = await api.timelineGet();
      player.start(cur, performance.now(), fps);
      playBtn.textContent = "Stop";
      playTick();
    } else {
      player.stop();
      playBtn.textContent = "Play";
    }
  };
  playBtn.addEventListener("click", () => void setPlaying(!player.running));
  frameSlider.addEventListener("input", () => { void setPlaying(false); void scrub(frameSlider.value); });
  frameBox.addEventListener("change", () => { void setPlaying(false); void scrub(frameBox.value); });
  { const [f, fps] = await api.timelineGet(); showFrame(f, fps); }
  const start = await api.startupScene();
  if (start) { path.value = start; await open(); } else { await refresh(); showParams(); }
  // proof / test door (read by the T1 proof script): drive the real param pane through the DOM
    (window as unknown as Record<string, unknown>).__hexe = { select: (name: string) => { const n = graph.nodes.find((q) => q.name === name); if (n) net.selectNode(n.id); }, recook, viewport, setFrame: scrub, play: setPlaying, open: async (p: string) => { path.value = p; await open(); } };
    const job = await api.renderRequest();
    if (job) await headlessRender(job);
}

/** `--render`: the scene is loaded + cooked; draw it at the requested size, hand the PNG to Rust (which writes it and exits). */
async function headlessRender(job: RenderReq): Promise<void> {
  try {
        if (!lastCook || lastCook.error) throw new Error(lastCook?.error ?? (status.textContent || "nothing cooked"));
    viewport.setWireframe(job.wire);
    await api.renderSave(viewport.snapshot(job.width, job.height));
  } catch (e) {
    await api.renderFail(msg(e));
  }
}

main().catch((e) => say(msg(e), true));
