// App shell hooks (T3): the `--script` proof runner and the native Edit menu talk to the live editor through Tauri events.
// Imported once from main.ts for its side effects; owns no state. Boot is detected by `window.__hexe` (set at the end of main()).
import { listen } from "@tauri-apps/api/event";
import { api, msg } from "./api";
import { currentNetwork } from "./network";
import { drive } from "./uidrive";

interface ScriptEvent { select: string[] | null; ui: string[] | null }

async function boot(): Promise<void> {
  for (let i = 0; i < 600 && !(window as unknown as Record<string, unknown>).__hexe; i++) await new Promise((r) => setTimeout(r, 50));
  // Rust applied an op to the session: re-read the graph (+ cook), select the named nodes, then ack so the next op / shot can run
  await listen<ScriptEvent>("hexe://script", async (ev) => {
    let err: string | null = null;
    try {
      // `ui` ops go through the real handlers, which re-sync themselves (a second sync would overwrite a rejection in the status line)
      if (ev.payload.ui) await drive(ev.payload.ui);
      else await currentNetwork?.resync(ev.payload.select);
    } catch (e) { err = msg(e); }
    await api.scriptAck(err);
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
