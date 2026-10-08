// App shell hooks (T3): the `--script` proof runner and the native Edit menu talk to the live editor through Tauri events.
// Imported once from main.ts for its side effects; owns no state. Boot is detected by `window.__hexe` (set at the end of main()).
import { listen } from "@tauri-apps/api/event";
import { api, msg } from "./api";
import { currentNetwork } from "./network";

interface ScriptEvent { select: string[] | null }

async function boot(): Promise<void> {
  for (let i = 0; i < 600 && !(window as unknown as Record<string, unknown>).__hexe; i++) await new Promise((r) => setTimeout(r, 50));
  // Rust applied an op to the session: re-read the graph (+ cook), select the named nodes, then ack so the next op / shot can run
  await listen<ScriptEvent>("hexe://script", async (ev) => {
    try { await currentNetwork?.resync(ev.payload.select); } catch (e) { console.error(msg(e)); } finally { await api.scriptAck(); }
  });
  // native Edit > Undo / Redo (WKWebView would otherwise swallow Cmd+Z as a text-edit command)
  await listen<string>("hexe://menu", (ev) => {
    const a = document.activeElement;
    if (a instanceof HTMLInputElement || a instanceof HTMLTextAreaElement) { document.execCommand(ev.payload === "redo" ? "redo" : "undo"); return; }
    currentNetwork?.undo(ev.payload === "redo").catch((e) => console.error(msg(e)));
  });
  await api.scriptReady();
}
boot().catch((e) => console.error(msg(e)));
