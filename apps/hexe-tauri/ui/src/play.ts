// Timeline playback clock (pure, no DOM: unit-tested with `bun test`). Play advances `$F` at `fps` against wall-clock time,
// so a slow cook drops frames instead of slowing the clip; Stop halts. The caller drives it from the frame scheduler.
export class Player {
  running = false;
  private f0 = 1;
  private t0 = 0;
  private fps = 30;
  /** Start playing from `frame` at `now` (ms, e.g. performance.now()) at `fps` frames per second. */
  start(frame: number, now: number, fps: number): void {
    this.running = true;
    this.f0 = Math.max(1, Math.round(frame));
    this.t0 = now;
    this.fps = Number.isFinite(fps) && fps > 0 ? fps : 30;
  }
  stop(): void {
    this.running = false;
  }
  /** The `$F` that should be showing at `now` (1-based); the start frame while stopped. */
  frameAt(now: number): number {
    if (!this.running) return this.f0;
    return this.f0 + Math.floor((Math.max(0, now - this.t0) * this.fps) / 1000);
  }
}
