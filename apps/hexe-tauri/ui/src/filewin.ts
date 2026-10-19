// Houdini-style file browser (File > Open): Locations column, Look in bar (back / forward / up, $HIP shown as such),
// Name / Last Modified / Size table, File field, Show files matching, Accept / Cancel. Model in filedlg.ts, fs in files.rs.
import { api, type FsEntry, type FsPlaces } from "./api";
import { baseName, dirOf, FILTERS, fmtModified, fmtSize, hipDir, parentDir, resolvePath, expandVars, visibleEntries, type SortCol } from "./filedlg";

const BOOKMARKS_KEY = "hexe.filedlg.places";
const loadBookmarks = (): string[] => {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(BOOKMARKS_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((t): t is string => typeof t === "string").slice(0, 50) : [];
  } catch { return []; }
};

const FOLDER = '<svg viewBox="0 0 20 16" class="fd-ico"><path d="M1 3h6l2 2h10v10H1z" fill="#e8b628" stroke="#8a6a10"/><path d="M1 6h18v9H1z" fill="#f7cf45"/></svg>';
const FILE = '<svg viewBox="0 0 20 16" class="fd-ico"><path d="M5 1h7l4 4v10H5z" fill="#d8d8d8" stroke="#777"/><path d="M12 1v4h4" fill="none" stroke="#777"/></svg>';
const DRIVE = '<svg viewBox="0 0 20 16" class="fd-ico"><path d="M2 8h16v6H2z" fill="#cfcfcf" stroke="#777"/><path d="M4 3h12l2 5H2z" fill="#e6e6e6" stroke="#777"/><circle cx="15" cy="11" r="1" fill="#555"/></svg>';
const HOME = '<svg viewBox="0 0 20 16" class="fd-ico"><path d="M10 1l9 7h-3v7H4V8H1z" fill="#e6e6e6" stroke="#777"/></svg>';

/** Open the browser at $HIP (the folder of `current`, home when untitled); resolves to the accepted path, null = cancel. */
export async function openFileDialog(current: string, title = "Open"): Promise<string | null> {
  let places: FsPlaces;
  try { places = await api.fsPlaces(); } catch { places = { home: "/", desktop: "/", temp: "/", roots: ["/"] }; }
  const hip = hipDir(current, places.home);
  const vars: Record<string, string> = { HIP: hip, JOB: hip, TEMP: places.temp, HOME: places.home };
  const mk = <K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text) e.textContent = text;
    return e;
  };
  const btn = (cls: string, html: string, tip: string) => { const b = mk("button", cls); b.innerHTML = html; b.title = tip; return b; };

  // ---- layout
  const veil = mk("div", "fd-veil");
  veil.tabIndex = -1;
  const win = mk("div", "fd-win");
  win.setAttribute("role", "dialog");
  win.setAttribute("aria-label", title);
  const back = btn("fd-nav", "&#9664;", "Back"), fwd = btn("fd-nav", "&#9654;", "Forward"), up = btn("fd-nav fd-up", "&#9650;", "Up one folder");
  const look = mk("input", "fd-look");
  look.spellcheck = false;
  const lookList = mk("datalist");
  lookList.id = "fd-look-list";
  look.setAttribute("list", lookList.id);
  const newFolder = mk("button", "fd-btn", "New Folder");
  const top = mk("div", "fd-top");
  top.append(mk("span", "fd-navs"), mk("span", "fd-lbl", "Look in"), look, lookList, newFolder);
  top.firstElementChild!.append(back, fwd, up);

  const locs = mk("div", "fd-locs");
  const locList = mk("ul", "fd-loclist");
  const addPlace = btn("fd-sq", "+", "Add the current folder to Locations"), delPlace = btn("fd-sq", "&#10005;", "Remove the selected location");
  const locBtns = mk("div", "fd-locbtns");
  locBtns.append(addPlace, delPlace);
  locs.append(mk("div", "fd-head", "Locations"), locList, locBtns);

  const table = mk("div", "fd-table");
  const head = mk("div", "fd-row fd-thead");
  const cols: [SortCol, string][] = [["name", "Name"], ["modified", "Last Modified"], ["size", "Size"]];
  const heads = cols.map(([c, t]) => { const h = mk("span", `fd-c fd-c-${c}`, t); h.dataset.col = c; head.append(h); return h; });
  const body = mk("div", "fd-tbody");
  table.append(head, body);

  const mid = mk("div", "fd-mid");
  mid.append(locs, table);

  const file = mk("input", "fd-file");
  file.spellcheck = false;
  file.value = baseName(current) || "untitled.hip";
  const filter = mk("input", "fd-filter");
  filter.spellcheck = false;
  filter.value = FILTERS[0];
  const filterPick = mk("select", "fd-filterpick");
  filterPick.title = "Show files matching";
  for (const f of FILTERS) filterPick.append(Object.assign(mk("option", "", f), { value: f }));
  const status = mk("span", "fd-status");
  const accept = mk("button", "fd-btn fd-accept", "Accept"), cancel = mk("button", "fd-btn", "Cancel");
  const row1 = mk("div", "fd-bottom");
  row1.append(mk("span", "fd-lbl", "File"), file, mk("span", "fd-lbl", "Show files matching"), filter, filterPick);
  const row2 = mk("div", "fd-bottom");
  row2.append(status, mk("span", "grow"), accept, cancel);
  win.append(top, mid, row1, row2);
  veil.append(win);
  document.body.append(veil);

  // ---- state
  let dir = "", entries: FsEntry[] = [], col: SortCol = "name", asc = true, sel: string | null = null;
  const hist: string[] = [], fut: string[] = [], recent: string[] = [];
  let bookmarks = loadBookmarks(), selPlace: string | null = null;
  const shown = (d: string) => (d === vars.HIP ? "$HIP/" : d);

  const paintPlaces = () => {
    locList.replaceChildren();
    const item = (icon: string, label: string, target: string, user = false) => {
      const li = mk("li", `fd-loc${dirOf(target) === dir ? " on" : ""}${user && selPlace === target ? " sel" : ""}`);
      li.innerHTML = icon;
      li.append(mk("span", "", label));
      li.title = target;
      li.addEventListener("click", () => { if (user) selPlace = target; void go(target); });
      locList.append(li);
    };
    item(HOME, "Home Folder", places.home);
    item(DRIVE.replace("#cfcfcf", "#9ab"), "Desktop", places.desktop);
    for (const r of places.roots) item(DRIVE, r, r);
    item(DRIVE, places.home, places.home);
    locList.append(mk("li", "fd-sep"));
    item(FOLDER, "$HIP/", vars.HIP);
    item(FOLDER, "$JOB/", vars.JOB);
    item(FOLDER, "$TEMP/", vars.TEMP);
    if (bookmarks.length) locList.append(mk("li", "fd-sep"));
    for (const b of bookmarks) item(FOLDER, b, b, true);
    delPlace.disabled = !selPlace;
  };
  const paintRows = () => {
    for (const h of heads) h.dataset.sort = h.dataset.col === col ? (asc ? "asc" : "desc") : "";
    body.replaceChildren();
    const row = (e: FsEntry | null) => {
      const name = e ? e.name : "..";
      const r = mk("div", `fd-row${sel === name ? " on" : ""}`);
      r.dataset.name = name;
      const n = mk("span", "fd-c fd-c-name");
      n.innerHTML = !e || e.dir ? FOLDER : FILE;
      n.append(mk("span", "", name));
      r.append(n, mk("span", "fd-c fd-c-modified", e ? fmtModified(e.modified) : ""), mk("span", "fd-c fd-c-size", e ? fmtSize(e.size) : ""));
      r.addEventListener("click", () => { sel = name; if (e && !e.dir) file.value = name; paintRows(); });
      r.addEventListener("dblclick", () => { if (!e) void go(parentDir(dir)); else if (e.dir) void go(dir + e.name); else { file.value = e.name; finish(dir + e.name); } });
      body.append(r);
    };
    if (parentDir(dir) !== dir) row(null);
    for (const e of visibleEntries(entries, filter.value, col, asc)) row(e);
  };
  /** browse to `target` (`push` = record in back history); a failed listing keeps the current folder */
  const go = async (target: string, push = true) => {
    const d = dirOf(expandVars(target, vars));
    let es: FsEntry[];
    try { es = await api.fsList(d); } catch (x) { status.textContent = String(x); status.classList.add("bad"); look.value = shown(dir); return; }
    if (push && dir && d !== dir) { hist.push(dir); fut.length = 0; }
    dir = d; entries = es; sel = null;
    status.textContent = ""; status.classList.remove("bad");
    look.value = shown(dir);
    if (!recent.includes(dir)) { recent.unshift(dir); recent.length = Math.min(recent.length, 12); }
    lookList.replaceChildren(...recent.map((r) => Object.assign(mk("option"), { value: r })));
    back.disabled = !hist.length; fwd.disabled = !fut.length; up.disabled = parentDir(dir) === dir;
    paintPlaces();
    paintRows();
    body.scrollTop = 0;
  };

  // ---- result
  let done: (v: string | null) => void = () => {};
  const result = new Promise<string | null>((r) => (done = r));
  const close = (v: string | null) => { veil.remove(); done(v); };
  const finish = (path: string) => close(path);
  const acceptTyped = () => {
    const typed = file.value.trim();
    if (!typed) return;
    const p = resolvePath(dir, typed, vars);
    // a folder typed (or picked) in File browses into it instead of accepting
    const hit = entries.find((e) => e.name === typed && e.dir);
    if (hit || p.endsWith("/")) { void go(hit ? dir + hit.name : p); file.value = ""; return; }
    finish(p);
  };

  // ---- wiring
  back.addEventListener("click", () => { const d = hist.pop(); if (d) { fut.push(dir); void go(d, false); } });
  fwd.addEventListener("click", () => { const d = fut.pop(); if (d) { hist.push(dir); void go(d, false); } });
  up.addEventListener("click", () => void go(parentDir(dir)));
  look.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); void go(look.value); } });
  look.addEventListener("change", () => { if (recent.includes(look.value)) void go(look.value); });
  for (const h of heads) h.addEventListener("click", () => { const c = h.dataset.col as SortCol; asc = c === col ? !asc : true; col = c; paintRows(); });
  filter.addEventListener("input", paintRows);
  filterPick.addEventListener("change", () => { filter.value = filterPick.value; paintRows(); });
  newFolder.addEventListener("click", async () => {
    try { const name = await api.fsMkdir(dir); await go(dir, false); sel = name; paintRows(); } catch (x) { status.textContent = String(x); status.classList.add("bad"); }
  });
  addPlace.addEventListener("click", () => {
    if (!bookmarks.includes(dir)) bookmarks = [...bookmarks, dir];
    try { localStorage.setItem(BOOKMARKS_KEY, JSON.stringify(bookmarks)); } catch { /* storage off: session only */ }
    paintPlaces();
  });
  delPlace.addEventListener("click", () => {
    bookmarks = bookmarks.filter((b) => b !== selPlace);
    selPlace = null;
    try { localStorage.setItem(BOOKMARKS_KEY, JSON.stringify(bookmarks)); } catch { /* storage off: session only */ }
    paintPlaces();
  });
  accept.addEventListener("click", acceptTyped);
  cancel.addEventListener("click", () => close(null));
  file.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); acceptTyped(); } });
  // keys stay inside the dialog (the network / viewport hotkeys listen on window: Delete must not delete nodes here)
  veil.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.key === "Escape") { e.preventDefault(); close(null); return; }
    if (e.target === file || e.target === look || e.target === filter) return;
    const rows = visibleEntries(entries, filter.value, col, asc).map((x) => x.name);
    if (parentDir(dir) !== dir) rows.unshift("..");
    const i = sel === null ? -1 : rows.indexOf(sel);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      sel = rows[Math.min(rows.length - 1, Math.max(0, i + (e.key === "ArrowDown" ? 1 : -1)))] ?? null;
      const ent = entries.find((x) => x.name === sel);
      if (ent && !ent.dir) file.value = ent.name;
      paintRows();
      body.querySelector(".fd-row.on")?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter" && sel !== null) {
      e.preventDefault();
      body.querySelector<HTMLElement>(".fd-row.on")?.dispatchEvent(new MouseEvent("dblclick"));
    } else if (e.key === "Backspace") {
      e.preventDefault();
      void go(parentDir(dir));
    }
  });
  veil.addEventListener("mousedown", (e) => { if (e.target === veil) e.preventDefault(); });

  await go(hip, false);
  file.focus();
  file.select();
  return result;
}
