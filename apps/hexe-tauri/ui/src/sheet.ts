// Geometry spreadsheet pane: class tabs (point/vertex/prim/detail), ptnum + P + every attribute as columns, rows virtualised
// (only the visible window is in the DOM; pages of PAGE rows are fetched lazily via the `spreadsheet` command and dropped
// on every cook).
import type { SheetData, SheetClass } from "./api";
import { pagesFor, showing, visibleRange } from "./sheetwin";

const PAGE = 200;
const ROW_H = 18;
const TABS: [SheetClass, string][] = [["point", "Points"], ["vertex", "Vertices"], ["prim", "Primitives"], ["detail", "Detail"]];

export type Fetch = (cls: SheetClass, offset: number, limit: number) => Promise<SheetData>;

const el = <K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text;
  return e;
};

export class Spreadsheet {
  private cls: SheetClass = "point";
  private tabs = new Map<SheetClass, HTMLButtonElement>();
  private info = el("span", "sh-info");
  private scroller = el("div", "sh-scroll");
  private table = el("table");
  private head = el("thead");
  private body = el("tbody");
  private pages = new Map<number, unknown[][]>();
  private columns: string[] = [];
  private shown = 0;
  private gen = 0; // bumped on invalidate: stale fetches are dropped
  private pending = new Set<number>();

  constructor(host: HTMLElement, private fetch: Fetch) {
    const bar = el("div", "sh-bar");
    for (const [c, label] of TABS) {
      const b = el("button", "sh-tab", label);
      b.dataset.class = c;
      b.addEventListener("click", () => this.select(c));
      this.tabs.set(c, b);
      bar.append(b);
    }
    bar.append(this.info);
    this.table.append(this.head, this.body);
    this.scroller.append(this.table);
    this.scroller.addEventListener("scroll", () => this.render());
    new ResizeObserver(() => this.render()).observe(this.scroller);
    host.replaceChildren(bar, this.scroller);
    this.mark();
  }

  private mark(): void {
    this.tabs.forEach((b, c) => b.classList.toggle("on", c === this.cls));
  }

  select(c: SheetClass): void {
    this.cls = c;
    this.mark();
    this.scroller.scrollTop = 0;
    this.invalidate();
  }

  /** Called after every cook: geometry changed (or failed -> `null`), refetch from the top of the current class. */
  invalidate(ok = true): void {
    const gen = ++this.gen;
    this.pages.clear();
    this.pending.clear();
    if (!ok) { this.clearTable("no geometry (cook failed)"); return; }
    this.fetchPage(0, gen).then(() => this.render());
  }

  private clearTable(msg: string): void {
    this.columns = [];
    this.shown = 0;
    this.head.replaceChildren();
    this.body.replaceChildren();
    this.info.textContent = msg;
    this.tabs.forEach((b, c) => { b.textContent = TABS.find((t) => t[0] === c)?.[1] ?? c; });
  }

  private async fetchPage(p: number, gen: number): Promise<void> {
    if (this.pages.has(p) || this.pending.has(p)) return;
    this.pending.add(p);
    try {
      const s = await this.fetch(this.cls, p * PAGE, PAGE);
      if (gen !== this.gen) return;
      this.pages.set(p, s.rows);
      if (p === 0 || this.columns.length === 0) {
        this.head.replaceChildren();
        this.columns = s.columns;
        this.shown = s.shown;
        this.info.textContent = showing(s.total);
        const c = s.counts;
        const n: Record<SheetClass, number> = { point: c.point, vertex: c.vertex, prim: c.prim, detail: c.detail };
        this.tabs.forEach((b, k) => { b.textContent = `${TABS.find((t) => t[0] === k)?.[1]} ${n[k]}`; });
      }
    } catch (e) {
      if (gen === this.gen) this.clearTable(typeof e === "string" ? e : String(e));
    } finally {
      this.pending.delete(p);
    }
  }

  private render(): void {
    if (!this.columns.length) return;
    const gen = this.gen;
    const [a, b] = visibleRange(this.scroller.scrollTop, this.scroller.clientHeight - ROW_H, ROW_H, this.shown);
    const need = pagesFor(a, b, PAGE).filter((p) => !this.pages.has(p));
    if (need.length) Promise.all(need.map((p) => this.fetchPage(p, gen))).then(() => { if (gen === this.gen) this.render(); });
    if (!this.head.firstChild) {
      const tr = el("tr");
      this.columns.forEach((c) => tr.append(el("th", "", c)));
      this.head.append(tr);
    }
    const rows: HTMLElement[] = [];
    const pad = (h: number) => { const tr = el("tr", "pad"); tr.style.height = `${h}px`; return tr; };
    rows.push(pad(a * ROW_H));
    for (let i = a; i < b; i++) {
      const cells = this.pages.get(Math.floor(i / PAGE))?.[i % PAGE];
      const tr = el("tr");
      tr.style.height = `${ROW_H}px`;
      if (cells) cells.forEach((v, k) => tr.append(el("td", k === 0 ? "idx" : typeof v === "number" ? "num" : "", v === null ? "" : String(v))));
      rows.push(tr);
    }
    rows.push(pad(Math.max(0, this.shown - b) * ROW_H));
    this.body.replaceChildren(...rows);
  }
}
