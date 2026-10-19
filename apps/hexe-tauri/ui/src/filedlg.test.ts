import { expect, test } from "bun:test";
import { baseName, dirOf, expandVars, fileDir, fmtModified, fmtSize, hipDir, matchesFilter, parentDir, resolvePath, visibleEntries } from "./filedlg";

test("paths: directory form, parent stops at roots, file dir / base name", () => {
  expect(dirOf("C:\\Users\\me")).toBe("C:/Users/me/");
  expect(dirOf("C://a//b/")).toBe("C:/a/b/");
  expect(parentDir("C:/Users/me/")).toBe("C:/Users/");
  expect(parentDir("C:/")).toBe("C:/");
  expect(parentDir("/")).toBe("/");
  expect(parentDir("/home/x")).toBe("/home/");
  expect(fileDir("C:\\x\\scene.hip")).toBe("C:/x/");
  expect(fileDir("scene.hip")).toBe("");
  expect(baseName("C:/x/scene.hip")).toBe("scene.hip");
});

test("$HIP: the open file's folder, untitled = home", () => {
  expect(hipDir("C:/proj/shots/a.hip", "C:/Users/me/")).toBe("C:/proj/shots/");
  expect(hipDir("", "C:/Users/me")).toBe("C:/Users/me/");
  expect(hipDir("untitled.hip", "C:/Users/me/")).toBe("C:/Users/me/");
});

test("variables and typed paths", () => {
  const vars = { HIP: "C:/proj/", JOB: "C:/proj/", TEMP: "C:/Temp/", HOME: "C:/Users/me/" };
  expect(expandVars("$HIP/geo/a.json", vars)).toBe("C:/proj/geo/a.json");
  expect(expandVars("$HIP", vars)).toBe("C:/proj/");
  expect(expandVars("~/x", vars)).toBe("C:/Users/me/x");
  expect(expandVars("$NOPE/x", vars)).toBe("$NOPE/x");
  expect(expandVars("$HIPX/x", vars)).toBe("$HIPX/x");
  expect(resolvePath("C:/proj/", "a.hip", vars)).toBe("C:/proj/a.hip");
  expect(resolvePath("C:/proj/", "D:\\b.hip", vars)).toBe("D:/b.hip");
  expect(resolvePath("C:/proj/", "$TEMP/c.hip", vars)).toBe("C:/Temp/c.hip");
});

test("filters: glob lists, case-insensitive, regex chars literal, empty = all", () => {
  expect(matchesFilter("A.HIP", "*.hip,*.json")).toBe(true);
  expect(matchesFilter("a.json", "*.hip, *.json")).toBe(true);
  expect(matchesFilter("a.hipx", "*.hip")).toBe(false);
  expect(matchesFilter("a+b.hip", "a+b.*")).toBe(true);
  expect(matchesFilter("anything", "")).toBe(true);
});

test("visible rows: folders always and first, files filtered, sorted by column", () => {
  const es = [
    { name: "b.hip", dir: false, modified: 1, size: 300 },
    { name: "Zed", dir: true, modified: 5, size: null },
    { name: "a.txt", dir: false, modified: 2, size: 10 },
    { name: "A.json", dir: false, modified: 3, size: 20 },
    { name: "docs", dir: true, modified: 9, size: null },
  ];
  expect(visibleEntries(es, "*.hip,*.json").map((e) => e.name)).toEqual(["docs", "Zed", "A.json", "b.hip"]);
  expect(visibleEntries(es, "*", "name", false).map((e) => e.name)).toEqual(["Zed", "docs", "b.hip", "a.txt", "A.json"]);
  expect(visibleEntries(es, "*", "size").map((e) => e.name)).toEqual(["docs", "Zed", "a.txt", "A.json", "b.hip"]);
});

test("formatting: Houdini date, sizes, unknowns blank", () => {
  expect(fmtModified(new Date(2026, 8, 29, 18, 36).getTime())).toBe("29 Sep 2026 6:36 PM");
  expect(fmtModified(new Date(2026, 0, 2, 0, 5).getTime())).toBe("2 Jan 2026 12:05 AM");
  expect(fmtModified(null)).toBe("");
  expect(fmtSize(5)).toBe("5 B");
  expect(fmtSize(12_595)).toBe("12.3 KB");
  expect(fmtSize(null)).toBe("");
  expect(fmtSize(-1)).toBe("");
});
