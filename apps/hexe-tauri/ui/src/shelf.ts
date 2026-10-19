// Shelf (Houdini layout): two shelf sets side by side, each a tab strip + a row of tool buttons.
// Tool spec "Label=type" maps a Houdini shelf tool onto a hexe node type: clicking calls `add(type)` (main.ts places / wires it).
// Tools without a hexe type (or whose type this engine build lacks) are shown dimmed and do nothing but say so.
import { labelOf } from "./tabcats";

type Shelf = [name: string, tools: string[]];

const LEFT: Shelf[] = [
  ["Create", ["Box=box", "Sphere=sphere", "Tube=tube", "Torus=torus", "Grid=grid", "Null=null", "Line=line", "Circle=circle", "Curve Bezier=curve", "Draw Curve", "Path", "Spray Paint", "Font", "Platonic Solids=platonic", "L-System", "Metaball", "File=file", "Spiral", "Helix", "Quick Shapes"]],
  ["Modify", ["Combine=merge", "Extract", "Mirror=mirror", "Copy to Points=copytopoints", "Duplicate", "Align Components", "Match Transform", "Move Pivot to Center", "Move Center to Pivot", "Freeze", "Bake Animation", "Delete=delete", "Hide", "Parent"]],
  ["Model", ["Revolve=revolve", "Loft", "Skin=skin", "Rails", "Sweep=sweep", "Creep", "Ray=ray", "Boolean=boolean", "Shatter", "TopoBuild", "Carve=carve", "Convert=convert", "Refine", "Reverse=reverse", "Sculpt", "Scatter=scatter", "Cluster Points", "Group=group", "Edit", "Visibility"]],
  ["Polygon", ["TopoBuild", "PolyDraw", "Curve Polygon", "PolyExtrude=polyextrude", "PolyBevel=polybevel", "PolyBridge", "PolySplit=polysplit", "Edge Loop", "PolyReduce=polyreduce", "PolyFill=polyfill", "PolyExpand2D", "Edge Collapse", "Edge Cusp", "Edge Flip", "Edge Divide=edgedivide", "Dissolve=dissolve", "Knife=knife", "Clip=clip"]],
  ["Deform", ["Twist=twist", "Bend=bend", "Linear Taper", "Squash and Stretch", "Bulge", "Lattice=lattice", "Fractal", "Mountain=mountain", "Waves", "Ripple", "Morph"]],
  ["Texture", ["UV Project=uvproject", "UV Edit", "UV Brush", "UV Fuse", "UV Flatten", "UV Pelt", "UV Texture", "UV Transform", "UV Unwrap=uvunwrap", "UV Quick Shade", "UV Smooth", "UV Layout=uvlayout", "Clear Texture Cache"]],
  ["Rigging", ["Bones", "Bones from Curve", "Pose", "IK from Objects", "IK from Bones", "Capture Geometry", "Edit Capture Regions", "Edit Capture Weights", "Edit Capture Blends", "Mirror Capture Weights", "Paint Capture Layer", "Align Capture Pose", "Delta Mush", "Comb"]],
  ["Characters", ["Simple Biped", "Simple Female", "Simple Male", "Mocap Biped 1", "Mocap Biped 2", "Mocap Biped 3"]],
  ["Constraints", ["Parent Blend", "Blend", "Look At", "Follow Path", "Points", "Surface", "Transform Constraint", "BlendPose", "Lag", "Jiggle", "Gamepad", "Constraints Network", "Toggle Constraint", "Bake Constraint", "Delete Constraint"]],
  ["Hair Utils", ["Add Fur", "Create Empty Guide Groom", "Create Guides", "Merge Groom Objects", "Deform Guides", "Simulate Guides", "Generate Hair", "Isolate Groom Parts", "Toggle Display", "Generate Hair Cards", "Hair Card Texture"]],
  ["Guide Process", ["Initialize Guides", "Curve Advect", "Groom", "Reguide", "Set Guide Direction", "Set Guide Length", "Lift Guides", "Straighten Guides", "Smooth Guides", "Frizz Guides", "Bend Guides", "Clump Guides", "Part Guides", "Add White Hairs"]],
  ["Terrain FX", ["Terrain: Hills", "Terrain: Mountain", "Terrain: Valley", "Terrain: Moonscape", "Terrain: Islands", "Terrain: Canyon", "Terrain: Dunes"]],
  ["Simple FX", ["Simple Cloth", "Simple Softbody", "Simple Fracture", "Simple Guided RBDs", "Simple Muzzle Flash", "Simple Billowy Smoke", "Simple GPU Ground Explosion", "Simple GPU Torch", "Simple Fireball", "Simple Aerial Explosion", "Simple Ground Explosion", "Simple Shockwave", "Simple Bonfire", "Simple Spreading Fire"]],
  ["Volume", ["Volume", "SDF Polys", "SDF Particles", "Volume Paint Color", "Volume Paint Fog", "Volume Paint SDF", "Fog Points", "Fog Particles", "Fog Curve", "Convert VDB"]],
];

const RIGHT: Shelf[] = [
  ["Lights and Cameras", ["Camera", "Point Light", "Spot Light", "Area Light", "Geometry Light", "Volume Light", "Distant Light", "Environment Light", "Sky Light", "GI Light", "Caustic Light", "Portal Light", "Ambient Light", "Stereo Camera", "VR Camera", "Switcher", "Gamepad Camera"]],
  ["Collisions", ["Static Object", "Deforming Object", "Surface Collider", "Ground Plane", "Terrain Object"]],
  ["Particles", ["Fireworks", "Location Particle Emitter", "Source Particle Emitter", "Replicate Particles", "Axis Force Particles", "Point Attract Particles", "Curve Attract Particles", "Curve Force Particles", "Metaball Force Particles", "Color Particles", "Drag Particles", "Fan Particles", "Flock Particles", "Force Particles"]],
  ["Grains", ["Dry Sand", "Wet Sand", "Sand Box", "RBD Grains", "Upres Sand", "Granular Solid", "Granular Sheet", "Granular Strand", "Auto-Sleep Particles", "Awaken By Geometry", "Slice", "Slice Along Line", "Distribute Particle Fluid"]],
  ["Vellum", ["Surface Collider", "Vellum Cloth", "Vellum Hair", "Vellum Strut Softbody", "Vellum Tetrahedral Softbody", "Vellum Balloon", "Vellum Grains"]],
  ["Rigid Bodies", ["RBD Objects", "RBD Hero Object", "RBD Glued Objects", "RBD Instanced Objects", "RBD Ragdolls", "RBD Grains", "RBD Convex Proxy", "RBD Sphere Proxy", "RBD Fractured Object", "Make Breakable", "RBD Impacts", "Debris", "RBD Auto Freeze", "RBD Pin Constraint", "RBD Twist Constraint"]],
  ["Particle Fluids", ["FLIP Fluid from Object", "FLIP Tank", "Emit Particle Fluid", "Sink from Objects", "Sculpted Particle Fluid", "Crown Splash Particle Fluid", "Drip Particle Fluid", "Suction Fluid", "Condensation", "Make Viscous", "Whitewater", "Slice", "Slice Along Line", "Distribute Particle Fluid"]],
  ["Viscous Fluids", ["Melt Object", "Lava From Object", "Emit Lava", "Heat Within Object", "Cool Within Object", "Emit Steam On Collision", "Sink from Objects", "Slice", "Slice Along Line", "Distribute Particle Fluid"]],
  ["Oceans", ["Small Ocean", "Large Ocean", "Guided Ocean Layer", "Wave Tank", "Beach Tank", "Flat Tank", "Ocean Flat Tank", "Whitewater"]],
  ["Pyro FX", ["Sparse Billowy Smoke", "Sparse Fireball", "Aerial Explosion", "Ground Explosion", "Shockwave", "Bonfire", "Spreading Fire"]],
  ["FEM", ["Solid Object", "Hybrid Object", "Surface Collider", "Volume Collider", "Constrain Points to Target", "Attach Points to Target Object", "Slide Points against Target", "Constrain Region", "Fuse Point Pairs"]],
  ["Wires", ["Wire Object", "Set Initial Pose", "Glue Wire to Animation", "Attach Wire to Surface", "Wire Pin Constraint", "Wire Spring Constraint", "Wire Angular Constraint", "Wire Angular Spring Constraint", "Wire Glue Constraint", "Attach Constraint to Geometry", "Attach Constraint to Object", "Attach Constraint to Point", "Detach Constraint", "Delete Constraint"]],
  ["Crowds", ["Agent", "Add Clip", "Collision Layer", "Configure Joints", "Populate", "Paint Density", "Simulate", "Terrain Foot Planting", "Obstacle", "Path", "Look At", "Target Position", "Agent Cam", "Stadium Example", "Street Example", "Formation Example", "Fuzzy Obstacle"]],
  ["Drive Simulation", ["New Simulation", "Group Dynamic Objects", "Gravity Force", "Drag Force", "Uniform Force", "Fan Force", "Fluid Force", "Wind Force", "Magnet Force", "Vortex Force"]],
];

/** "Label=type" -> [label, type | null] */
export function parseTool(spec: string): [string, string | null] {
  const i = spec.lastIndexOf("=");
  return i > 0 ? [spec.slice(0, i), spec.slice(i + 1) || null] : [spec, null];
}

const NS = "http://www.w3.org/2000/svg";
/** 24px glyphs for the tools hexe can run (by node type). */
const GLYPH: Record<string, string> = {
  box: '<path d="M4 8l8-4 8 4v9l-8 4-8-4z" fill="#c9a26b"/><path d="M4 8l8 4 8-4M12 12v9" stroke="#6b4d22" fill="none"/>',
  sphere: '<circle cx="12" cy="12" r="8" fill="#7fb0e0"/><ellipse cx="9.5" cy="9" rx="3" ry="2" fill="#d8ecff"/>',
  tube: '<ellipse cx="12" cy="6" rx="6" ry="2.5" fill="#cbe6a9"/><path d="M6 6v12a6 2.5 0 0012 0V6" fill="#a5c97f"/>',
  torus: '<ellipse cx="12" cy="12" rx="9" ry="6" fill="#e0a64f"/><ellipse cx="12" cy="11" rx="4" ry="2" fill="#333"/>',
  grid: '<path d="M3 8l9-4 9 4-9 4zM6 6.7l9 4M9 5.3l9 4M15 5.3l-9 4M18 6.7l-9 4" stroke="#dfe8f1" stroke-width=".8" fill="#9fb3c8"/>',
  circle: '<circle cx="12" cy="12" r="7.5" fill="none" stroke="#e39a9a" stroke-width="2.5"/>',
  line: '<path d="M4 19L20 5" stroke="#e6d27a" stroke-width="2.5"/><circle cx="4" cy="19" r="2" fill="#fff6c4"/><circle cx="20" cy="5" r="2" fill="#fff6c4"/>',
  curve: '<path d="M3 18C7 2 15 22 21 6" stroke="#e98fbf" stroke-width="2.5" fill="none"/>',
  file: '<path d="M6 3h8l4 4v14H6z" fill="#d6d6d6"/><path d="M14 3v4h4" fill="#9a9a9a"/>',
  merge: '<path d="M5 4l7 8 7-8M12 12v9" stroke="#8fc0e9" stroke-width="2.5" fill="none"/>',
  null: '<circle cx="12" cy="12" r="7" fill="none" stroke="#bdbdbd" stroke-width="2.5"/><path d="M6 18L18 6" stroke="#bdbdbd" stroke-width="2.5"/>',
  scatter: '<circle cx="6" cy="7" r="1.8" fill="#d9b96b"/><circle cx="15" cy="5" r="1.8" fill="#d9b96b"/><circle cx="11" cy="12" r="1.8" fill="#d9b96b"/><circle cx="18" cy="14" r="1.8" fill="#d9b96b"/><circle cx="7" cy="18" r="1.8" fill="#d9b96b"/>',
  mirror: '<path d="M12 3v18" stroke="#ddd" stroke-dasharray="2 2"/><path d="M10 6L4 18h6zM14 6l6 12h-6z" fill="#8fb5d6"/>',
  bend: '<path d="M6 20V12a6 6 0 016-6h6" stroke="#d58fd5" stroke-width="4" fill="none"/>',
  twist: '<path d="M8 3c8 4-8 6 0 9s-8 5 0 9M16 3c-8 4 8 6 0 9s8 5 0 9" stroke="#e0a64f" stroke-width="2" fill="none"/>',
  boolean: '<circle cx="9" cy="12" r="6" fill="#e0a64f"/><circle cx="15" cy="12" r="6" fill="#7fb0e0" fill-opacity=".85"/>',
  polyextrude: '<path d="M3 16l9-4 9 4-9 4z" fill="#6f8f50"/><path d="M7 14V7l5-2 5 2v7l-5 2z" fill="#9fc77a"/>',
  polybevel: '<path d="M5 9l3-4h8l3 4v10H5z" fill="#e0a64f"/>',
  copytopoints: '<rect x="3" y="3" width="7" height="7" fill="#c9a26b"/><rect x="14" y="3" width="7" height="7" fill="#c9a26b"/><rect x="8.5" y="14" width="7" height="7" fill="#c9a26b"/>',
  group: '<rect x="3" y="3" width="18" height="18" rx="3" fill="none" stroke="#e6d27a" stroke-width="2" stroke-dasharray="3 2"/><circle cx="9" cy="10" r="2" fill="#e6d27a"/><circle cx="15" cy="14" r="2" fill="#e6d27a"/>',
  mountain: '<path d="M2 20l6-10 4 5 3-7 7 12z" fill="#e0a64f"/>',
  lattice: '<path d="M4 4h16v16H4zM4 12h16M12 4v16" stroke="#7fb0e0" stroke-width="1.6" fill="none"/>',
  revolve: '<ellipse cx="12" cy="12" rx="8" ry="3" fill="none" stroke="#e07a5f" stroke-width="2"/><path d="M12 3v18" stroke="#ddd"/>',
  sweep: '<path d="M3 18c5-12 13-12 18 0" stroke="#7fb0e0" stroke-width="4" fill="none"/>',
  skin: '<path d="M4 6c4-3 12-3 16 0M4 18c4 3 12 3 16 0M4 6v12M20 6v12" stroke="#a5c97f" stroke-width="2" fill="none"/>',
  delete: '<path d="M6 7h12l-1 14H7zM4 7h16M9 4h6" stroke="#d6d6d6" stroke-width="2" fill="none"/>',
  uvproject: '<rect x="4" y="4" width="16" height="16" fill="#333" stroke="#d58fd5" stroke-width="1.5"/><path d="M4 12h16M12 4v16" stroke="#d58fd5"/>',
  knife: '<path d="M4 20L18 6l2 2L6 22z" fill="#d6d6d6"/>',
  clip: '<path d="M4 14l8-8 8 8" stroke="#e98fbf" stroke-width="2.5" fill="none"/><path d="M2 16h20" stroke="#ddd" stroke-dasharray="2 2"/>',
};
/** dimmed placeholder glyph for tools hexe cannot run yet: shelf-tinted disc with the label's initials */
function initials(label: string): string {
  return label.replace(/[^A-Za-z0-9 ]/g, " ").split(/\s+/).filter(Boolean).map((w) => w[0]).join("").slice(0, 2).toUpperCase();
}
const TINT = ["#c9a26b", "#7fb0e0", "#a5c97f", "#e0a64f", "#d58fd5", "#e07a5f", "#87d1a8", "#e6d27a"];

function icon(label: string, type: string | null, tint: string): SVGSVGElement {
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.innerHTML = (type && GLYPH[type]) ??
    `<circle cx="12" cy="12" r="9" fill="${tint}"/><text x="12" y="15.5" text-anchor="middle" font-size="9" font-weight="700" fill="#2a2a2a">${initials(label)}</text>`;
  return svg;
}

/** Network-node icon markup (24x24 viewBox) for a node type: its shelf glyph, else a grey disc with the type's initials. */
export function nodeIcon(type: string): string {
  return GLYPH[type] ?? `<circle cx="12" cy="12" r="9" fill="#9aa4ae"/><text x="12" y="15.5" text-anchor="middle" font-size="9" font-weight="700" fill="#22272c">${initials(labelOf(type))}</text>`;
}

/** One shelf set (tab strip + tools) into `host`. `say` reports clicks on tools hexe cannot run. */
function shelfSet(host: HTMLElement, key: string, shelves: Shelf[], have: Set<string>, add: (type: string) => void, say: (m: string) => void): void {
  const tabs = document.createElement("div");
  tabs.className = "shelftabs";
  const tools = document.createElement("div");
  tools.className = "shelftools";
  host.replaceChildren(tabs, tools);
  let active = shelves[0][0];
  try { const v = localStorage.getItem(key); if (v && shelves.some(([n]) => n === v)) active = v; } catch { /* default tab */ }
  const show = () => {
    for (const b of tabs.querySelectorAll<HTMLElement>("button")) b.classList.toggle("on", b.dataset.shelf === active);
    const idx = Math.max(0, shelves.findIndex(([n]) => n === active));
    tools.replaceChildren(...shelves[idx][1].map((spec) => {
      const [label, t] = parseTool(spec);
      const live = t !== null && have.has(t);
      const b = document.createElement("button");
      b.className = live ? "tool" : "tool off";
      b.title = live ? `${label}  (${t})` : `${label}: not in hexe yet`;
      if (t) b.dataset.type = t;
      const lab = document.createElement("span");
      lab.textContent = label;
      b.append(icon(label, live ? t : null, TINT[idx % TINT.length]), lab);
      b.addEventListener("click", () => (live && t ? add(t) : say(`${label}: not in hexe yet (try the TAB menu for what is)`)));
      return b;
    }));
  };
  tabs.append(...shelves.map(([n]) => {
    const b = document.createElement("button");
    b.textContent = n;
    b.dataset.shelf = n;
    b.addEventListener("click", () => {
      active = n;
      try { localStorage.setItem(key, n); } catch { /* not remembered */ }
      show();
    });
    return b;
  }));
  show();
}

/** Both shelf sets (left: modelling, right: lights + dynamics) built against the live node types. */
export function buildShelf(left: HTMLElement, right: HTMLElement, live: string[], add: (type: string) => void, say: (m: string) => void): void {
  const have = new Set(live);
  shelfSet(left, "hexe.shelf.left", LEFT, have, add, say);
  shelfSet(right, "hexe.shelf.right", RIGHT, have, add, say);
}

/** every node type a shelf tool maps onto (for tests: each must be a real type name, no typos) */
export const shelfTypes = (): string[] => [...new Set([...LEFT, ...RIGHT].flatMap(([, ts]) => ts.map((s) => parseTool(s)[1]).filter((t): t is string => !!t)))];
export { labelOf };
