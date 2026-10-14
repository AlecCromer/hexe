// Frame scheduler. WKWebView pauses requestAnimationFrame while the window is occluded / not frontmost (document.hidden):
// a rAF-only draw never fires -> canvas stays blank although the scene is loaded (h42). A timer fallback runs the same
// callback; whichever fires first wins, the other is cancelled. `fallbackMs` is a param (default 50).
export interface FrameEnv {
  raf: (cb: () => void) => number; caf: (id: number) => void;
  after: (cb: () => void, ms: number) => number; cancelAfter: (id: number) => void;
}
export const domEnv = (): FrameEnv => ({
  raf: (cb) => requestAnimationFrame(cb), caf: (id) => cancelAnimationFrame(id),
  after: (cb, ms) => window.setTimeout(cb, ms), cancelAfter: (id) => clearTimeout(id),
});
export function frameScheduler(run: () => void, env: FrameEnv = domEnv(), fallbackMs = 50): () => void {
  let r = 0, t = 0, pending = false;
  return () => {
    if (pending) { env.caf(r); env.cancelAfter(t); }
    pending = true;
    const fire = () => { if (!pending) return; pending = false; env.caf(r); env.cancelAfter(t); run(); };
    r = env.raf(fire);
    t = env.after(fire, fallbackMs);
  };
}
