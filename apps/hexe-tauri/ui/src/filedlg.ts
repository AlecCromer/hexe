// File browser model (no DOM: unit-tested with `bun test`): `/` paths, $HIP / $JOB / $TEMP, filters, sorting, formatting.
import type { FsEntry } from "./api";

/** Show-files-matching presets: scene files hexe opens (Houdini .hip family imports; .json scenes; Endorphin files). */
export const FILTERS = ["*.hip,*.hiplc,*.hipnc,*.json,*.ens,*.nmc,*.nmb,*.nma", "*.hip,*.hiplc,*.hipnc", "*.json", "*.ens,*.nmc,*.nmb,*.nma", "*"];

/** `C:\a\b` / `C:/a/b` -> `C:/a/b/` (directory form: forward slashes, one trailing `/`). */
export function dirOf(p: string): string {
  const s = p.trim().replace(/\\/g, "/").replace(/\/{2,}/g, "/");
  return s.endsWith("/") ? s : `${s}/`;
}

/** Parent directory (`C:/a/b/` -> `C:/a/`); a root (`C:/`, `/`) is its own parent. */
export function parentDir(dir: string): string {
  const d = dirOf(dir);
  if (/^[A-Za-z]:\/$/.test(d) || d === "/") return d;
  const i = d.slice(0, -1).lastIndexOf("/");
  return i < 0 ? d : d.slice(0, i + 1);
}

/** Directory of a file path (`C:/x/scene.hip` -> `C:/x/`); "" when it has none. */
export function fileDir(path: string): string {
  const s = path.trim().replace(/\\/g, "/");
  const i = s.lastIndexOf("/");
  return i < 0 ? "" : s.slice(0, i + 1);
}

export const baseName = (path: string): string => path.trim().replace(/\\/g, "/").split("/").pop() ?? "";

/** $HIP = the open file's folder; untitled (no path / no folder) = the home folder. */
export const hipDir = (current: string, home: string): string => {
  const d = fileDir(current);
  return d ? dirOf(d) : dirOf(home);
};

/** Expand a leading `$HIP` / `$JOB` / `$TEMP` / `~` (Houdini variables) against `vars` (dirs ending in `/`). */
export function expandVars(p: string, vars: Record<string, string>): string {
  const s = p.trim().replace(/\\/g, "/");
  const m = /^(\$[A-Z]+|~)(\/|$)(.*)$/.exec(s);
  if (!m) return s;
  const base = vars[m[1] === "~" ? "HOME" : m[1].slice(1)];
  return base === undefined ? s : dirOf(base) + m[3];
}

/** A typed File entry: absolute (`C:/..`, `/..`, `$HIP/..`, `~/..`) stays, a bare name joins `dir`. */
export function resolvePath(dir: string, typed: string, vars: Record<string, string>): string {
  const e = expandVars(typed, vars);
  return /^([A-Za-z]:\/|\/)/.test(e) ? e : dirOf(dir) + e;
}

/** Does `name` match a comma/space separated glob list (`*.hip,*.json`; case-insensitive)? Empty list = everything. */
export function matchesFilter(name: string, filter: string): boolean {
  const globs = filter.split(/[,\s]+/).filter(Boolean);
  if (!globs.length) return true;
  return globs.some((g) => new RegExp("^" + g.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".") + "$", "i").test(name));
}

export type SortCol = "name" | "modified" | "size";

/** Rows to show: folders always, files through `filter`; folders first, then by `col` (`asc`), names case-insensitive. */
export function visibleEntries(entries: FsEntry[], filter: string, col: SortCol = "name", asc = true): FsEntry[] {
  const byName = (a: FsEntry, b: FsEntry) => a.name.localeCompare(b.name, undefined, { sensitivity: "base", numeric: true });
  const key = (e: FsEntry) => (col === "modified" ? e.modified ?? -Infinity : e.size ?? -Infinity);
  return entries
    .filter((e) => e.dir || matchesFilter(e.name, filter))
    .sort((a, b) => {
      if (a.dir !== b.dir) return a.dir ? -1 : 1;
      const c = col === "name" ? byName(a, b) : key(a) - key(b) || byName(a, b);
      return asc ? c : -c;
    });
}

const MON = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/** Houdini browser date: `29 Sep 2026 6:36 PM` (local time); "" when unknown. */
export function fmtModified(ms: number | null): string {
  if (ms === null || !Number.isFinite(ms)) return "";
  const d = new Date(ms), h = d.getHours();
  return `${d.getDate()} ${MON[d.getMonth()]} ${d.getFullYear()} ${h % 12 || 12}:${String(d.getMinutes()).padStart(2, "0")} ${h < 12 ? "AM" : "PM"}`;
}

/** `5 B`, `12.3 KB`, `4.0 MB`, ... ; "" for folders / unknown. */
export function fmtSize(bytes: number | null): string {
  if (bytes === null || !Number.isFinite(bytes) || bytes < 0) return "";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let v = bytes, i = 0;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return i === 0 ? `${v} B` : `${v.toFixed(1)} ${u[i]}`;
}
