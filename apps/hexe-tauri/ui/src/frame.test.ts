import { expect, test } from "bun:test";
import { frameScheduler, type FrameEnv } from "./frame";
import { clip, fit } from "./camera";
import { bounds } from "./geom";

/** manual env: queued callbacks are fired by hand; `hidden` = rAF never fires (occluded WKWebView). */
function env() {
  const raf = new Map<number, () => void>(), tm = new Map<number, () => void>(); let n = 0;
  const e: FrameEnv = {
    raf: (cb) => (raf.set(++n, cb), n), caf: (id) => void raf.delete(id),
    after: (cb) => (tm.set(++n, cb), n), cancelAfter: (id) => void tm.delete(id),
  };
  return { e, raf, tm, fire: (m: Map<number, () => void>) => [...m.values()].forEach((f) => f()) };
}

test("hidden webview: rAF never fires, the timer fallback still draws once", () => {
  const x = env(); let draws = 0;
  const draw = frameScheduler(() => draws++, x.e);
  draw();
  x.fire(x.tm); // rAF stalled
  expect(draws).toBe(1);
  expect(x.raf.size).toBe(0);
});

test("visible: rAF draws once, fallback cancelled; repeat calls coalesce", () => {
  const x = env(); let draws = 0;
  const draw = frameScheduler(() => draws++, x.e);
  draw(); draw(); draw();
  expect(x.raf.size).toBe(1);
  expect(x.tm.size).toBe(1);
  x.fire(x.raf);
  expect(draws).toBe(1);
  expect(x.tm.size).toBe(0);
  draw(); x.fire(x.tm); expect(draws).toBe(2);
});

// Balcony.ens bbox as seen live (100 m char_background slab + ~4 m character): fit must give a clip range containing it all
test("Balcony-scale bounds: near/far enclose the whole bbox after fit", () => {
  const bb = bounds([-50, -3, -50, 50, 3.977408, 50, 0, 0.85, 0])!;
  const cam = fit({ target: [0, 0, 0], yaw: 0.6, pitch: 0.45, dist: 6 }, bb);
  const k = clip(cam);
  expect(k.near).toBeGreaterThan(0);
  expect(k.far / k.near).toBeLessThan(1e6);
  expect(k.near).toBeLessThan(cam.dist - bb.radius); // nearest part of the slab is in front of the near plane
  expect(k.far).toBeGreaterThan(cam.dist + bb.radius);
});
