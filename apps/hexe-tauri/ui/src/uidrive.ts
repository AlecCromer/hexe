// `--script` op `ui <verb> ...`: drives the REAL network / param handlers with synthetic DOM events (proof + CI only; nothing
// here is reachable without the CLI flag). Verbs: click NAME [shift] · rclick NAME · dblclick NAME [name|body] · rename NAME NEW ·
// wire FROM TO [body|N] · drag NAME DX DY · hover NAME · key COMBO (meta+shift+z, delete, d, ...) · scrub PARAM DX · addat TYPE X Y · assert SELECTOR TEXT · wait MS.
const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
const centre = (el: Element): { x: number; y: number } => { const r = el.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; };
function mouse(target: EventTarget, type: string, p: { x: number; y: number }, extra: MouseEventInit = {}): void {
  target.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, clientX: p.x, clientY: p.y, button: 0, view: window, ...extra }));
}
function nodeEl(name: string): SVGGElement {
  for (const t of document.querySelectorAll<SVGTextElement>("#network text.nm")) if (t.textContent === name) return t.closest<SVGGElement>("g.node") as SVGGElement;
  throw new Error(`ui: no node named ${name}`);
}
const body = (name: string): Element => nodeEl(name).querySelector("rect.body") as Element;
function key(combo: string): void {
  const parts = combo.toLowerCase().split("+");
  const k = parts[parts.length - 1];
  const named: Record<string, string> = { delete: "Delete", backspace: "Backspace", tab: "Tab", f2: "F2", enter: "Enter", escape: "Escape" };
  document.body.dispatchEvent(new KeyboardEvent("keydown", {
    key: named[k] ?? k, bubbles: true, cancelable: true,
    metaKey: parts.includes("meta"), ctrlKey: parts.includes("ctrl"), shiftKey: parts.includes("shift"), altKey: parts.includes("alt"),
  }));
}
async function dragTo(el: Element, to: { x: number; y: number }, extra: MouseEventInit = {}): Promise<void> {
  const a = centre(el);
  mouse(el, "mousedown", a, extra);
  for (const f of [0.3, 0.7, 1]) { mouse(window, "mousemove", { x: a.x + (to.x - a.x) * f, y: a.y + (to.y - a.y) * f }); await sleep(16); }
  mouse(window, "mouseup", to);
}

export async function drive(args: string[]): Promise<void> {
  const [verb, ...a] = args;
  const net = document.getElementById("network") as HTMLElement;
  switch (verb) {
    case "click": {
      const p = centre(body(a[0]));
      mouse(body(a[0]), "mousedown", p, { shiftKey: a[1] === "shift" });
      mouse(window, "mouseup", p);
      break;
    }
    case "rclick": mouse(body(a[0]), "contextmenu", centre(body(a[0])), { button: 2 }); break;
    case "dblclick": {
      const t = a[1] === "body" ? body(a[0]) : (nodeEl(a[0]).querySelector("text.nm") as Element);
      mouse(t, "dblclick", centre(t));
      break;
    }
    case "rename": {
      const t = nodeEl(a[0]).querySelector("text.nm") as Element;
      mouse(t, "dblclick", centre(t));
      await sleep(60);
      const box = document.getElementById("renamebox") as HTMLInputElement | null;
      if (!box) throw new Error("ui: rename box did not open");
      box.value = a[1];
      box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      break;
    }
    case "wire": {
      const out = nodeEl(a[0]).querySelector(".port.out") as Element;
      const port = a[2] && a[2] !== "body" ? nodeEl(a[1]).querySelector(`.port.in[data-input="${a[2]}"]`) : null;
      const to = port ? centre(port) : centre(body(a[1]));
      // "near" drop: 6 px off the port dot (inside the 14 px radius, not on the dot)
      await dragTo(out, port ? { x: to.x + 6, y: to.y - 6 } : to);
      break;
    }
    case "drag": {
      const p = centre(body(a[0]));
      await dragTo(body(a[0]), { x: p.x + Number(a[1]), y: p.y + Number(a[2]) });
      break;
    }
    case "hover": mouse(net, "mouseenter", centre(body(a[0]))); mouse(window, "mousemove", centre(body(a[0]))); break;
    case "key": key(a[0]); break;
    case "scrub": {
      const input = document.querySelector<HTMLInputElement>(`input[data-param="${a[0]}"]`);
      if (!input) throw new Error(`ui: no param field ${a[0]}`);
      const p = centre(input), dx = Number(a[1]);
      mouse(input, "mousedown", p);
      for (const f of [0.25, 0.5, 1]) { mouse(window, "mousemove", { x: p.x + dx * f, y: p.y }); await sleep(30); }
      mouse(window, "mouseup", { x: p.x + dx, y: p.y });
      break;
    }
    case "addat": {
      const r = (net.querySelector("svg") as Element).getBoundingClientRect();
      mouse(net, "mouseenter", { x: r.left + Number(a[1]), y: r.top + Number(a[2]) });
      mouse(window, "mousemove", { x: r.left + Number(a[1]), y: r.top + Number(a[2]) });
      key("tab");
      await sleep(60);
      const input = document.querySelector<HTMLInputElement>("#tabmenu input");
      if (!input) throw new Error("ui: Tab menu did not open");
      input.value = a[0];
      input.dispatchEvent(new Event("input", { bubbles: true }));
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      break;
    }
    case "assert": { // assert SELECTOR TEXT...: the element exists and its text contains TEXT (fails the script otherwise)
      const el = document.querySelector(a[0]);
      const want = a.slice(1).join(" ");
      if (!el || !(el.textContent ?? "").includes(want)) throw new Error(`ui assert ${a[0]} ~ "${want}": got ${el ? JSON.stringify(el.textContent) : "no such element"}`);
      return;
    }
    case "wait": await sleep(Number(a[0])); return;
    default: throw new Error(`ui: unknown verb ${verb}`);
  }
  await sleep(350); // let the async command + graph refresh the handler fired settle
}
