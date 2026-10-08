// Typed wrappers over the Tauri commands in src-tauri/src/lib.rs (all engine state lives in Rust).
import { invoke } from "@tauri-apps/api/core";

export type Kind = "float" | "int" | "toggle" | "menu" | "string" | "vector3";
export type PVal = number | boolean | string | [number, number, number];

export interface MenuEntry { value: number; token: string; label: string }
export interface ParamInfo { name: string; label: string; kind: Kind; default: PVal; min: number | null; max: number | null; menu: MenuEntry[] }
export interface NodeType { type: string; label: string; inputs: string[]; params: ParamInfo[] }
export interface NodeView { id: number; type: string; name: string; x: number; y: number; inputs: string[]; params: Record<string, PVal> }
export interface EdgeView { from: number; to: number; input: number }
export interface GraphView { nodes: NodeView[]; edges: EdgeView[]; display: number | null }
import type { Shape } from "./geom";
export interface Cooked {
  positions: number[]; prims: number[][]; closed: boolean[];
  shapes: Shape[];
  counts: { points: number; prims: number; closed: number; open: number; unsupported: number; shapes: number };
  error: string | null;
  /** id of the node whose cook failed (red border), if tied to one */
  error_node: number | null;
}
export type SheetClass = "point" | "vertex" | "prim" | "detail";
export interface SheetData {
  class: SheetClass; columns: string[]; rows: (number | string | null)[][]; offset: number; total: number; shown: number;
  counts: { point: number; vertex: number; prim: number; detail: number };
}

export const api = {
  listNodeTypes: () => invoke<NodeType[]>("list_node_types"),
  graphNew: () => invoke<void>("graph_new"),
  graphLoad: (path: string) => invoke<GraphView>("graph_load", { path }),
  graphSave: (path: string) => invoke<void>("graph_save", { path }),
  graphGet: () => invoke<GraphView>("graph_get"),
  nodeAdd: (type: string, x: number, y: number) => invoke<number>("node_add", { type, x, y }),
  nodeRemove: (id: number) => invoke<void>("node_remove", { id }),
  nodeMove: (id: number, x: number, y: number) => invoke<void>("node_move", { id, x, y }),
  nodeSetParam: (id: number, name: string, value: PVal) => invoke<void>("node_set_param", { id, name, value }),
  edgeConnect: (from: number, to: number, input: number) => invoke<void>("edge_connect", { from, to, input }),
  edgeDisconnect: (to: number, input: number) => invoke<void>("edge_disconnect", { to, input }),
  setDisplay: (id: number) => invoke<void>("set_display", { id }),
  cook: () => invoke<Cooked>("cook"),
  spreadsheet: (cls: SheetClass, offset: number, limit: number) => invoke<SheetData>("spreadsheet", { class: cls, offset, limit }),
  startupScene: () => invoke<string | null>("startup_scene"),
};

export const msg = (e: unknown): string => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
