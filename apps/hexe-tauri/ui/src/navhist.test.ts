import { expect, test } from "bun:test";
import { navBack, navForward, navStart, navVisit } from "./navhist";

test("back / forward walk the visited paths; a new visit drops forward; ends are no-ops; bounded", () => {
  let h = navStart("/obj");
  h = navVisit(h, "/obj/geo1");
  h = navVisit(h, "/obj/geo1"); // same place: no entry
  expect(h).toEqual({ back: ["/obj"], cur: "/obj/geo1", fwd: [] });
  h = navBack(h);
  expect(h).toEqual({ back: [], cur: "/obj", fwd: ["/obj/geo1"] });
  expect(navBack(h)).toBe(h);
  h = navForward(h);
  expect(h.cur).toBe("/obj/geo1");
  expect(navForward(h)).toBe(h);
  h = navVisit(navBack(h), "/obj/other");
  expect(h).toEqual({ back: ["/obj"], cur: "/obj/other", fwd: [] });
  let big = navStart(0);
  for (let i = 1; i <= 120; i++) big = navVisit(big, i, 50);
  expect(big.back.length).toBe(50);
  expect(big.back[0]).toBe(70);
});
