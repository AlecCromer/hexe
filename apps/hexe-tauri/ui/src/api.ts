// Typed wrappers over the Tauri commands in src-tauri/src/lib.rs (all engine state lives in Rust).
import { invoke } from "@tauri-apps/api/core";

export type Kind = "float" | "int" | "toggle" | "menu" | "string" | "vector3";
export type PVal = number | boolean | string | [number, number, number];

export interface MenuEntry { value: number; token: string; label: string }
export interface ParamInfo {
  name: string; label: string; kind: Kind; default: PVal; min: number | null; max: number | null; menu: MenuEntry[];
  /** "vex" = source-code string (script pane editor) */
  language: string | null;
  /** allowed values of a string param (dropdown, e.g. wrangle Run Over) */
  choices: string[];
}
export interface NodeType { type: string; label: string; inputs: string[]; /** inputs 0..required must be wired; the rest are optional */ required: number; params: ParamInfo[] }
export interface NodeView {
  id: number; type: string; name: string; x: number; y: number; inputs: string[];
  /** inputs 0..required must be wired (solid wire; red arrow when unwired); the rest are optional (dashed wire) */
  required: number;
  /** bypass flag: the node passes its input 0 through unchanged */
  bypass: boolean;
  params: Record<string, PVal>;
}
export interface EdgeView { from: number; to: number; input: number }
/** Sticky note of a network (editor px: top-left x, y, size w, h; colour rgb 0..1), from the scene / .hip file */
export interface NoteView { name: string; text: string; color: [number, number, number]; x: number; y: number; w: number; h: number; minimized: boolean }
/** /obj object kinds: SOP network / Null (control + point1 inside) / object-level container (subnet, digital asset) */
export type ObjKind = "geo" | "null" | "subnet";
export interface ObjInfo { name: string; kind: ObjKind; net: string | null; pos: [number, number] | null; display: boolean; selectable: boolean; notes: NoteView[] }
/** Network box (titled backdrop; editor px top-left, size; colour rgb 0..1; `nodes` = the names inside) */
export interface BoxView { name: string; title: string; color: [number, number, number]; x: number; y: number; w: number; h: number; minimized: boolean; nodes: string[] }
export interface GraphView { nodes: NodeView[]; edges: EdgeView[]; display: number | null; notes?: NoteView[]; boxes?: BoxView[]; /** node name -> user colour */ colors?: Record<string, [number, number, number]> }
import type { Shape } from "./geom";
export interface Cooked {
positions: number[]; prims: number[][]; closed: boolean[];
  /** prim group name -> prim indices (viewport Fit exclusion) */
  prim_groups?: Record<string, number[]>;
  /** rgb per prim corner (prim order, corner order); absent when the geometry has no Cd */
  cd?: number[] | null;
  /** rgb per point, only when a point Cd exists (colours the points layer) */
  cd_point?: number[] | null;
  /** edge groups: name -> flat point pairs [a0, b0, a1, b1, ...] */
  edge_groups?: Record<string, number[]>;
  shapes: Shape[];
  counts: { points: number; prims: number; closed: number; open: number; unsupported: number; shapes: number };
  error: string | null;
  /** id of the node whose cook failed (red border), if tied to one */
  error_node: number | null;
}
export interface RenderReq { out: string; width: number; height: number; wire: boolean }
export type SheetClass = "point" | "vertex" | "prim" | "detail";
export interface SheetData {
  class: SheetClass; columns: string[]; rows: (number | string | null)[][]; offset: number; total: number; shown: number;
  counts: { point: number; vertex: number; prim: number; detail: number };
}

/** File browser entry (`modified` = ms since epoch, `size` = bytes; null when unreadable / a folder) */
export interface FsEntry { name: string; dir: boolean; modified: number | null; size: number | null }
/** File browser places: directories with a trailing `/` */
export interface FsPlaces { home: string; desktop: string; temp: string; roots: string[] }

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
  /** network dot centred at (x, y): `from` -> dot, dot -> input `input` of `to` (each optional); returns its id */
  dotAdd: (x: number, y: number, from: number | null, to: number | null = null, input = 0) => invoke<number>("dot_add", { x, y, from, to, input }),
  /** TAB menu with a wire selected: a `type` node spliced into `from` -> `to`:`input` (a generator only feeds `to`); one undo step */
  nodeAddWired: (type: string, x: number, y: number, from: number, to: number, input: number) => invoke<number>("node_add_wired", { type, x, y, from, to, input }),
  setDisplay: (id: number | null) => invoke<void>("set_display", { id }),
  setBypass: (id: number, on: boolean) => invoke<void>("set_bypass", { id, on }),
  nodesRemove: (ids: number[]) => invoke<void>("nodes_remove", { ids }),
  nodesMove: (moves: [number, number, number][]) => invoke<void>("nodes_move", { moves }),
  nodeRename: (id: number, name: string) => invoke<void>("node_rename", { id, name }),
  undo: () => invoke<boolean>("undo"),
  redo: () => invoke<boolean>("redo"),
  copyNodes: (ids: number[]) => invoke<number>("copy_nodes", { ids }),
  pasteNodes: () => invoke<number[]>("paste_nodes"),
  scriptReady: () => invoke<void>("script_ready"),
  scriptAck: (error: string | null = null) => invoke<void>("script_ack", { error }),
  cook: () => invoke<Cooked>("cook"),
  /** cook a non-display node (template overlay); the spreadsheet keeps the display cook */
  cookNode: (id: number) => invoke<Cooked>("cook_node", { id }),
  /** timeline: `$F` (1-based) + rate; the display cook runs at `($F - 1) / fps` */
  timelineGet: () => invoke<[number, number]>("timeline_get"),
  timelineSet: (frame: number) => invoke<void>("timeline_set", { frame }),
  spreadsheet: (cls: SheetClass, offset: number, limit: number) => invoke<SheetData>("spreadsheet", { class: cls, offset, limit }),
    startupScene: () => invoke<string | null>("startup_scene"),
    selectionSet: (ids: number[]) => invoke<void>("selection_set", { ids }),
    selectionGet: () => invoke<number[]>("selection_get"),
    groupFromSelection: () => invoke<number>("group_from_selection"),
    renderRequest: () => invoke<RenderReq | null>("render_request"),
    renderSave: (dataUrl: string) => invoke<void>("render_save", { dataUrl }),
    renderFail: (message: string) => invoke<void>("render_fail", { message }),
    /** File > Open browser (files.rs): directory entries, standard places, New Folder (returns the created name) */
    /** /obj Geometry objects (one SOP network each): names (sorted) + the current one; enter = make current (returns its network) */
    objectsList: () => invoke<[string[], string]>("objects_list"),
    objectEnter: (name: string) => invoke<GraphView>("object_enter", { name }),
    /** new empty object / rename: a taken name is numbered up; both return the final name */
    objectAdd: (name: string, kind: ObjKind = "geo", net: string | null = null) => invoke<string>("object_add", { name, kind, net }),
    /** every object: kind, container (`net`), file position, flags, a container's sticky notes; + the current one */
    objectsInfo: () => invoke<[ObjInfo[], string]>("objects_info"),
    objectRemove: (name: string) => invoke<void>("object_remove", { name }),
    objectRename: (old: string, name: string) => invoke<string>("object_rename", { old, new: name }),
    cookObject: (name: string) => invoke<Cooked>("cook_object", { name }),
    fsList: (dir: string) => invoke<FsEntry[]>("fs_list", { dir }),
    fsPlaces: () => invoke<FsPlaces>("fs_places"),
    fsMkdir: (dir: string) => invoke<string>("fs_mkdir", { dir }),
};

export const msg = (e: unknown): string => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
