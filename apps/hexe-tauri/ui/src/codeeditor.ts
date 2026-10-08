// Script pane: a small code editor for `language` string params (VEX wrangle snippets): monospace textarea, line-number gutter,
// Tab inserts spaces, Cmd/Ctrl+Enter = apply + cook, inline engine error (line:col + message) with a red strip on the failing line.

/** Fixed metrics (px): gutter, strip and textarea share them, so the strip lands exactly on the textarea line. */
export const LINE_H = 16, PAD_Y = 4;

export interface CodeError { line: number | null; col: number | null; msg: string }

/** Engine error text -> position + message. VEX errors carry `line:col: message` (1-based); other failures keep the whole text. */
export function parseCodeError(text: string | null | undefined): CodeError | null {
  if (!text) return null;
  const m = /(\d+):(\d+): ([\s\S]*)$/.exec(text);
  if (!m) return { line: null, col: null, msg: text };
  return { line: Number(m[1]), col: Number(m[2]), msg: m[3].trim() };
}

/** Number of lines in `text` (an empty text is one line). */
export const lineCount = (text: string): number => text.split("\n").length;

/** Vertical offset (px, within the editor body) of the top of 1-based `line` for a given scrollTop. */
export const lineTop = (line: number, scrollTop: number): number => PAD_Y + (line - 1) * LINE_H - scrollTop;

/** Replace `[start, end)` of `text` with `ins`; returns the new text and caret. */
export function spliceText(text: string, start: number, end: number, ins: string): { text: string; caret: number } {
  return { text: text.slice(0, start) + ins + text.slice(end), caret: start + ins.length };
}

export const TAB = "  ";

export interface CodeEditor {
  el: HTMLElement;
  textarea: HTMLTextAreaElement;
  value(): string;
  setError(e: CodeError | null): void;
}

const h = <K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, text?: string): HTMLElementTagNameMap[K] => {
  const e = document.createElement(tag);
  e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
};

/** Build an editor over `initial`. `apply(text)` runs on Cmd/Ctrl+Enter (always) and on blur when the text changed. */
export function createCodeEditor(initial: string, apply: (text: string) => void): CodeEditor {
  const root = h("div", "codeed");
  const wrap = h("div", "code-wrap");
  const gutter = h("pre", "code-gutter");
  const body = h("div", "code-body");
  const strip = h("div", "code-strip");
  const ta = h("textarea", "code-ta");
  const err = h("div", "code-err");
  const hint = h("div", "code-hint", "\u2318\u21b5 apply + cook \u00b7 Tab indents");
  ta.value = initial;
  ta.spellcheck = false;
  ta.wrap = "off";
  ta.dataset.param = "code";
  ta.setAttribute("autocapitalize", "off");
  body.append(strip, ta);
  wrap.append(gutter, body);
  root.append(wrap, err, hint);

  let cur: CodeError | null = null;
  let committed = initial;
  let shownLines = -1, shownErr: number | null = -2;
  const paintGutter = () => {
    const n = lineCount(ta.value), el = cur?.line ?? null;
    if (n !== shownLines || el !== shownErr) {
      shownLines = n; shownErr = el;
      gutter.replaceChildren(...Array.from({ length: n }, (_, i) => h("span", i + 1 === el ? "ln bad" : "ln", String(i + 1))));
    }
    gutter.scrollTop = ta.scrollTop;
    if (cur?.line) {
      strip.style.display = "block";
      strip.style.top = `${lineTop(Math.min(cur.line, n), ta.scrollTop)}px`;
    } else strip.style.display = "none";
  };
  const paintError = () => {
    root.classList.toggle("has-err", !!cur);
    err.textContent = cur ? `${cur.line ? `line ${cur.line}${cur.col ? `:${cur.col}` : ""} \u2014 ` : ""}${cur.msg}` : "";
    err.style.display = cur ? "block" : "none";
    paintGutter();
  };
  const run = () => { committed = ta.value; apply(ta.value); };

  ta.addEventListener("input", paintGutter);
  ta.addEventListener("scroll", paintGutter);
  ta.addEventListener("change", () => { if (ta.value !== committed) run(); });
  ta.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") { e.preventDefault(); e.stopPropagation(); run(); return; }
    if (e.key === "Tab" && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
      e.preventDefault();
      if (!document.execCommand("insertText", false, TAB)) { // no native undo-aware insert: splice by hand
        const r = spliceText(ta.value, ta.selectionStart, ta.selectionEnd, TAB);
        ta.value = r.text;
        ta.setSelectionRange(r.caret, r.caret);
        ta.dispatchEvent(new Event("input", { bubbles: true }));
      }
    }
  });
  paintError();
  return {
    el: root,
    textarea: ta,
    value: () => ta.value,
    setError(e) { cur = e; paintError(); },
  };
}
