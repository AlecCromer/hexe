import { expect, test } from "bun:test";
import { Player } from "./play";

test("play advances $F at fps against the clock and stop halts", () => {
  const p = new Player();
  expect(p.running).toBe(false);
  p.start(1, 1000, 30);
  expect(p.running).toBe(true);
  expect(p.frameAt(1000)).toBe(1);
  expect(p.frameAt(1000 + 1000 / 30 - 1)).toBe(1); // not yet one frame
  expect(p.frameAt(1000 + 1000 / 30 + 1)).toBe(2);
  expect(p.frameAt(2000)).toBe(31); // one second at 30 fps
  p.stop();
  expect(p.running).toBe(false);
  expect(p.frameAt(9999)).toBe(1); // halted: the start frame, no drift
});

test("play resumes from the frame it is started on and survives bad input", () => {
  const p = new Player();
  p.start(45.4, 500, 24);
  expect(p.frameAt(500 + 1000)).toBe(45 + 24);
  p.start(0, 0, Number.NaN); // frame clamps to 1, fps falls back to 30
  expect(p.frameAt(1000)).toBe(31);
  expect(p.frameAt(-5)).toBe(1); // clock going backwards never rewinds before the start
});
