// Object-level transform (Houdini OBJ Transform tab: xOrd, rOrd, t, r, s, p, pr, scale) as a 4x4 matrix. Pure, no DOM:
// unit-tested against hython worldTransform() recordings (fixtures/obj-null-xform). Houdini's convention: row vectors
// (p' = p · M), so the row-major array of M is exactly three.js `Matrix4.fromArray` (column-major, column vectors).
export type M4 = number[];

const ident = (): M4 => [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
/** a · b (row-major) */
export function mul(a: M4, b: M4): M4 {
  const o = new Array<number>(16).fill(0);
  for (let i = 0; i < 4; i++) for (let j = 0; j < 4; j++) {
    let s = 0;
    for (let k = 0; k < 4; k++) s += a[i * 4 + k] * b[k * 4 + j];
    o[i * 4 + j] = s;
  }
  return o;
}
const trans = (v: readonly number[]): M4 => [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, v[0], v[1], v[2], 1];
const scl = (v: readonly number[]): M4 => [v[0], 0, 0, 0, 0, v[1], 0, 0, 0, 0, v[2], 0, 0, 0, 0, 1];
function rot(axis: number, deg: number): M4 {
  const a = (deg * Math.PI) / 180, c = Math.cos(a), s = Math.sin(a);
  if (axis === 0) return [1, 0, 0, 0, 0, c, s, 0, 0, -s, c, 0, 0, 0, 0, 1];
  if (axis === 1) return [c, 0, -s, 0, 0, 1, 0, 0, s, 0, c, 0, 0, 0, 0, 1];
  return [c, s, 0, 0, -s, c, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
}
/** rOrd menu order (xyz xzy yxz yzx zxy zyx): the axis applied first comes first */
const R_ORDERS = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
/** xOrd menu order (srt str rst rts tsr trs) */
const X_ORDERS = ["srt", "str", "rst", "rts", "tsr", "trs"];
export function rotation(r: readonly number[], rOrd: number): M4 {
  const ord = R_ORDERS[rOrd] ?? R_ORDERS[0];
  return ord.reduce((m, ax) => mul(m, rot(ax, r[ax])), ident());
}

export interface ObjXform {
  xOrd?: number; rOrd?: number;
  t?: readonly number[]; r?: readonly number[]; s?: readonly number[]; p?: readonly number[]; pr?: readonly number[];
  scale?: number;
}
const v3 = (v: readonly number[] | undefined, d: number): number[] =>
  [0, 1, 2].map((i) => (v && Number.isFinite(v[i]) ? v[i] : d));

/** The object's local transform: pivot frame (translate `p`, rotated by `pr`, always Rx Ry Rz) around the xOrd product
 *  of S (= s * scale), R (rOrd) and T. Non-finite inputs fall back to the defaults. */
export function objectMatrix(x: ObjXform): M4 {
  const k = Number.isFinite(x.scale) ? (x.scale as number) : 1;
  const S = scl(v3(x.s, 1).map((c) => c * k));
  const R = rotation(v3(x.r, 0), x.rOrd ?? 0);
  const T = trans(v3(x.t, 0));
  const parts: Record<string, M4> = { s: S, r: R, t: T };
  const core = [...(X_ORDERS[x.xOrd ?? 0] ?? "srt")].reduce((m, c) => mul(m, parts[c]), ident());
  const p = v3(x.p, 0);
  const pr = rotation(v3(x.pr, 0), 0);
  const prInv = [0, 1, 2, 3].flatMap((i) => [0, 1, 2, 3].map((j) => (i < 3 && j < 3 ? pr[j * 4 + i] : i === j ? 1 : 0)));
  return mul(mul(mul(mul(trans(p.map((c) => -c)), prInv), core), pr), trans(p));
}
