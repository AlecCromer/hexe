// three.js viewport: headlight flat-shaded double-sided surface (earcut), edges, round points, sphere/tube wire proxies,
// ground grid with coloured axes, corner axis gizmo, P/N number overlays, draw caps. Houdini camera (camera.ts).
import * as THREE from "three";
import { clip, type Cam, defaultCam, dolly, dragDolly, eye, fit as fitCam, FOV, orbit, pan, wheelDolly } from "./camera";
import type { Cooked } from "./api";
import { describe } from "./pick";
import { CLICK_SLOP, clampSel, combine, inBox, nearest, projectAll } from "./pick";
import { frameScheduler } from "./frame";
import { cornerColors, cornerGeometry } from "./cd";
import { bounds, fitPositions, capCheck, CAP_LABELS, CAP_LABEL_SCAN, edgeSegments, primCentroids, shapeWire, triangulate, triEstimate, viewState } from "./geom";

/** Viewport tunables (egui `Style` parity). Colours are 0xRRGGBB; light = headlight ambient + diffuse*|N.V|. */
export const STYLE = {
  background: 0x3b3f45, surface: [0.73, 0.76, 0.81] as [number, number, number], ambient: 0.28, diffuse: 0.72,
  point: [1, 0.64, 0.29] as [number, number, number], pointPx: 6, edge: 0x14171a, wire: 0xd6dae0, openEdge: 0xffd24a, shape: 0x7fd4ff,
  gridHalf: 10, gridStep: 1, gridMinor: 0x4d535c, axisX: 0xe0453a, axisY: 0x56c456, axisZ: 0x3a7bf0,   gizmoPx: 84,
   /** picked points: colour, size px */
   pick: [1, 0.95, 0.2] as [number, number, number], pickPx: 11,
 };

// USE_CD (material define): per-corner colour attribute `cd` replaces the flat base colour
const SURFACE_VS = "varying vec3 vP;\n#ifdef USE_CD\nattribute vec3 cd; varying vec3 vC;\n#endif\nvoid main(){\n#ifdef USE_CD\nvC = cd;\n#endif\nvec4 mv = modelViewMatrix * vec4(position, 1.0); vP = mv.xyz; gl_Position = projectionMatrix * mv; }";
// flat per-face normal from screen derivatives (== the Newell normal), headlight: ambient + diffuse * |N.V|
const SURFACE_FS = "varying vec3 vP; uniform vec3 base; uniform float amb; uniform float dif;\n#ifdef USE_CD\nvarying vec3 vC;\n#endif\n" +
  "void main(){ vec3 b = base;\n#ifdef USE_CD\nb = vC;\n#endif\nvec3 n = normalize(cross(dFdx(vP), dFdy(vP))); float d = abs(dot(n, normalize(-vP))); gl_FragColor = vec4(b * (amb + dif * d), 1.0); }";
const POINT_VS = "uniform float size;\n#ifdef USE_CD\nattribute vec3 cd; varying vec3 vC;\n#endif\nvoid main(){\n#ifdef USE_CD\nvC = cd;\n#endif\nvec4 mv = modelViewMatrix * vec4(position, 1.0); gl_Position = projectionMatrix * mv; gl_Position.z -= 2e-4 * gl_Position.w; gl_PointSize = size; }";
const POINT_FS = "uniform vec3 col;\n#ifdef USE_CD\nvarying vec3 vC;\n#endif\nvoid main(){ if (length(gl_PointCoord - vec2(0.5)) > 0.5) discard; vec3 c = col;\n#ifdef USE_CD\nc = vC;\n#endif\ngl_FragColor = vec4(c, 1.0); }";

export class Viewport {
  private renderer: THREE.WebGLRenderer;
  private scene = new THREE.Scene();
  private camera = new THREE.PerspectiveCamera((FOV * 180) / Math.PI, 1, 0.01, 10000);
  private cam: Cam = defaultCam();
  private content = new THREE.Group();
  private mesh: THREE.Mesh | null = null;
  private open: THREE.LineSegments | null = null;
  private wire: THREE.LineSegments | null = null;
  private points: THREE.Points | null = null;
  private pointMat = new THREE.ShaderMaterial({ vertexShader: POINT_VS, fragmentShader: POINT_FS, uniforms: { size: { value: STYLE.pointPx }, col: { value: new THREE.Vector3(...STYLE.point) } } });
  private gizmo = new THREE.Scene();
  private gizmoCam = new THREE.OrthographicCamera(-1.5, 1.5, 1.5, -1.5, 0.1, 10);
  private labels = document.createElement("div");
  private tag = document.createElement("div");
  private axisTags: HTMLElement[] = [];
  private wireOn = false;
  private pointsOn = true;
  private showP = false;
  private showN = false;
  private shown: Cooked | null = null;
  private primC = new Float32Array(0);
private bb: ReturnType<typeof bounds> = null;
  /** Fit-only bounds (displayed geometry minus prims in groups matching `fitExclude`); `bb` stays the full extent. */
  private fitBb: ReturnType<typeof bounds> = null;
  /** Globs of prim groups Fit ignores (comma/space separated, empty = none): a param, not hardcoded. */
  fitExclude = "*background*";
    private schedule = frameScheduler(() => this.render());
    private pickOn = false;
    private sel: number[] = [];
    private hi: THREE.Points | null = null;
    private hiMat = new THREE.ShaderMaterial({ vertexShader: POINT_VS, fragmentShader: POINT_FS, depthTest: false, uniforms: { size: { value: STYLE.pickPx }, col: { value: new THREE.Vector3(...STYLE.pick) } } });
    private boxEl = document.createElement("div");
  /** true while the mouse is over the viewport (hotkeys are hover scoped) */
  hover = false;
  /** fired when P / N / wire state changes (status label) */
    onState: (s: string) => void = () => {};
    /** fired when the point selection changes: ascending ptnums + a status line (`pt 5  P (x, y, z)`) */
    onPick: (sel: number[], info: string) => void = () => {};

  constructor(private host: HTMLElement) {
    this.renderer = new THREE.WebGLRenderer({ antialias: true, preserveDrawingBuffer: true });
    this.renderer.setPixelRatio(window.devicePixelRatio);
    host.appendChild(this.renderer.domElement);
    this.labels.className = "vp-labels";
    this.tag.className = "vp-tag";
        this.boxEl.className = "vp-box";
        host.append(this.labels, this.tag, this.boxEl);
    for (const [t, c] of [["X", STYLE.axisX], ["Y", STYLE.axisY], ["Z", STYLE.axisZ]] as const) {
      const e = document.createElement("span");
      e.className = "vp-axis"; e.textContent = t; e.style.color = `#${c.toString(16).padStart(6, "0")}`;
      this.axisTags.push(e); host.appendChild(e);
    }
    host.addEventListener("mouseenter", () => (this.hover = true));
    host.addEventListener("mouseleave", () => (this.hover = false));
    this.scene.background = new THREE.Color(STYLE.background);
    this.scene.add(this.grid(), this.content);
    this.gizmo.add(this.axes(1, [STYLE.axisX, STYLE.axisY, STYLE.axisZ]));
    this.bindInput(this.renderer.domElement);
    this.applyCam();
    new ResizeObserver(() => this.resize()).observe(host);
    this.resize();
    this.tag.textContent = this.state();
  }
  /** XZ ground grid (axis lines drawn separately, coloured: X red, Z blue, Y green). */
  private grid(): THREE.Group {
    const g = new THREE.Group(), pos: number[] = [], h = STYLE.gridHalf, st = STYLE.gridStep;
    for (let i = -h; i <= h; i += st) {
      if (i === 0) continue;
      pos.push(-h, 0, i, h, 0, i, i, 0, -h, i, 0, h);
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
    g.add(new THREE.LineSegments(geo, new THREE.LineBasicMaterial({ color: STYLE.gridMinor })), this.axes(h, [STYLE.axisX, STYLE.axisY, STYLE.axisZ], true));
    return g;
  }
  /** Three coloured axis lines of length `len` (`ground`: X and Z through both sides, Y from 0 up). */
  private axes(len: number, col: [number, number, number], ground = false): THREE.LineSegments {
    const p = [ground ? -len : 0, 0, 0, len, 0, 0, 0, 0, 0, 0, len, 0, 0, 0, ground ? -len : 0, 0, 0, len];
    const c = col.flatMap((x) => [(x >> 16) / 255, ((x >> 8) & 255) / 255, (x & 255) / 255]);
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.Float32BufferAttribute(p, 3));
    geo.setAttribute("color", new THREE.Float32BufferAttribute([...c.slice(0, 3), ...c.slice(0, 3), ...c.slice(3, 6), ...c.slice(3, 6), ...c.slice(6, 9), ...c.slice(6, 9)], 3));
    return new THREE.LineSegments(geo, new THREE.LineBasicMaterial({ vertexColors: true }));
  }
  private resize(): void {
    const w = Math.max(1, this.host.clientWidth), h = Math.max(1, this.host.clientHeight);
    this.renderer.setSize(w, h, false);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.draw();
  }
  private draw(): void {
    this.schedule();
  }
    private render(w = this.host.clientWidth, h = this.host.clientHeight): void {
      const r = this.renderer, g = STYLE.gizmoPx;
    r.setViewport(0, 0, w, h);
    r.render(this.scene, this.camera);
    // corner axis gizmo: same view rotation, own ortho camera, bottom-left
    const e = eye(this.cam), t = this.cam.target, d = new THREE.Vector3(e[0] - t[0], e[1] - t[1], e[2] - t[2]).normalize();
    this.gizmoCam.position.copy(d.multiplyScalar(4));
    this.gizmoCam.up.set(0, 1, 0);
    this.gizmoCam.lookAt(0, 0, 0);
    this.gizmoCam.updateMatrixWorld();
    r.autoClear = false;
    r.clearDepth();
    r.setViewport(8, 8, g, g);
    r.render(this.gizmo, this.gizmoCam);
    r.autoClear = true;
    r.setViewport(0, 0, w, h);
    [new THREE.Vector3(1.25, 0, 0), new THREE.Vector3(0, 1.25, 0), new THREE.Vector3(0, 0, 1.25)].forEach((v, i) => {
      v.project(this.gizmoCam);
      const s = this.axisTags[i].style;
      s.left = `${8 + ((v.x + 1) / 2) * g - 4}px`; s.top = `${h - 8 - ((v.y + 1) / 2) * g - 7}px`;
    });
    this.updateLabels();
  }
    private state(): string { return viewState(this.wireOn, this.showP, this.showN) + (this.pickOn ? " [pick]" : ""); }
  private changed(): void { this.tag.textContent = this.state(); this.onState(this.state()); this.applyVisibility(); this.draw(); }
  private applyVisibility(): void {
    if (this.mesh) this.mesh.visible = !this.wireOn;
    if (this.open) this.open.visible = !this.wireOn;
    if (this.wire) { this.wire.visible = this.wireOn; (this.wire.material as THREE.LineBasicMaterial).color.set(STYLE.wire); }
    if (this.points) this.points.visible = this.pointsOn;
  }
  /** W: edges only (no shaded surface), like egui. */
  setWireframe(on: boolean): void { this.wireOn = on; this.changed(); }
  get wireframe(): boolean { return this.wireOn; }
  setPoints(on: boolean): void { this.pointsOn = on; this.changed(); }
  setPointSize(px: number): void { this.pointMat.uniforms.size.value = px * this.renderer.getPixelRatio(); this.draw(); }
  setNumbers(kind: "p" | "n", on: boolean): void {
    if (kind === "p") this.showP = on; else this.showN = on;
    if (this.showN && this.shown && !this.primC.length) this.primC = primCentroids(this.shown.prims, this.shown.positions);
    this.changed();
  }
  get numbers(): { p: boolean; n: boolean } { return { p: this.showP, n: this.showN }; }
  /** Point / prim number overlay: first <=500 items inside the frustum (scan <=100k). */
  private updateLabels(): void {
    const c = this.shown, want: [number, number, number, string][] = []; // x, y, 0=point|1=prim, text
    if (c && !c.error && (this.showP || this.showN)) {
      this.camera.updateMatrixWorld();
      const v = new THREE.Vector3(), put = (x: number, y: number, z: number, t: string, kind: number) => {
        if (!Number.isFinite(x)) return;
        v.set(x, y, z).project(this.camera);
        if (v.z < -1 || v.z > 1 || Math.abs(v.x) > 1 || Math.abs(v.y) > 1) return;
        want.push([((v.x + 1) / 2) * this.host.clientWidth, ((1 - v.y) / 2) * this.host.clientHeight, kind, t]);
      };
      if (this.showP) {
        const n = Math.min(c.counts.points, CAP_LABEL_SCAN);
        for (let i = 0, k = 0; i < n && k < CAP_LABELS; i++) { const b = want.length; put(c.positions[i * 3], c.positions[i * 3 + 1], c.positions[i * 3 + 2], `${i}`, 0); k += want.length - b; }
      }
      if (this.showN) {
        const n = this.primC.length / 3;
        for (let i = 0, k = 0; i < n && k < CAP_LABELS; i++) { const b = want.length; put(this.primC[i * 3], this.primC[i * 3 + 1], this.primC[i * 3 + 2], `${i}`, 1); k += want.length - b; }
      }
    }
    // pool: reuse spans
    while (this.labels.childElementCount < want.length) this.labels.appendChild(document.createElement("span"));
    const kids = this.labels.children;
    for (let i = 0; i < kids.length; i++) {
      const s = kids[i] as HTMLElement;
      if (i >= want.length) { s.style.display = "none"; continue; }
      s.style.display = ""; s.className = want[i][2] ? "n" : "p"; s.textContent = want[i][3]; s.style.left = `${want[i][0]}px`; s.style.top = `${want[i][1]}px`;
    }
  }
  private clear(): void {
    for (const o of [...this.content.children]) {
      this.content.remove(o);
      o.traverse((c) => {
        const m = c as THREE.Mesh;
        m.geometry?.dispose();
        if (m.material !== this.pointMat) (Array.isArray(m.material) ? m.material : [m.material]).forEach((x) => x?.dispose());
      });
    }
    this.mesh = this.open = this.wire = this.points = null;
this.bb = this.fitBb = null;
this.primC = new Float32Array(0);
  }
  /** Replace the displayed geometry with a fresh cook and fit the camera. Returns draw-cap warnings (layers skipped, not frozen). */
  show(c: Cooked): string[] {
        this.clear();
        const sameGeo = !!this.shown && !this.shown.error && !c.error && this.shown.counts.points === c.counts.points;
        this.shown = c;
    const warnings: string[] = [];
    const n = c.counts.points;
    if (!c.error && n > 0) {
      const pos = new THREE.Float32BufferAttribute(Float32Array.from(c.positions), 3);
      const geo = () => { const g = new THREE.BufferGeometry(); g.setAttribute("position", pos); return g; };
      const segEst = c.prims.reduce((a, p, k) => a + (p.length < 2 ? 0 : p.length - 1 + (c.closed[k] && p.length > 2 ? 1 : 0)), 0);
      const caps = capCheck(triEstimate(c.prims, c.closed), segEst * 2, n);
      warnings.push(...caps.warnings);
      if (caps.mesh) {
        const cd = cornerColors(c);
        // with colours the mesh is built per CORNER (a point can carry a different colour on each face)
        const cg = cd ? cornerGeometry(c) : null;
        const tris = cg ? triangulate(cg.prims, cg.closed, cg.positions) : triangulate(c.prims, c.closed, c.positions);
        if (tris.length) {
          const g = cg ? new THREE.BufferGeometry() : geo();
          if (cg && cd) {
            g.setAttribute("position", new THREE.BufferAttribute(cg.positions, 3));
            g.setAttribute("cd", new THREE.BufferAttribute(cd, 3));
          }
          g.setIndex(new THREE.BufferAttribute(tris, 1));
          const mat = new THREE.ShaderMaterial({
            defines: cd ? { USE_CD: "" } : {},
            vertexShader: SURFACE_VS, fragmentShader: SURFACE_FS, side: THREE.DoubleSide,
            uniforms: { base: { value: new THREE.Vector3(...STYLE.surface) }, amb: { value: STYLE.ambient }, dif: { value: STYLE.diffuse } },
            polygonOffset: true, polygonOffsetFactor: 1, polygonOffsetUnits: 1,
          });
          this.mesh = new THREE.Mesh(g, mat);
          this.content.add(this.mesh);
        }
      }
      if (caps.edges) {
        const open = edgeSegments(c.prims, c.closed, n, true);
        if (open.length) {
          const g = geo();
          g.setIndex(new THREE.BufferAttribute(open, 1));
          this.open = new THREE.LineSegments(g, new THREE.LineBasicMaterial({ color: STYLE.openEdge }));
          this.content.add(this.open);
        }
        const all = edgeSegments(c.prims, c.closed, n, false);
        if (all.length) {
          const g = geo();
          g.setIndex(new THREE.BufferAttribute(all, 1));
          this.wire = new THREE.LineSegments(g, new THREE.LineBasicMaterial({ color: STYLE.edge }));
          this.content.add(this.wire);
        }
      }
      const verts: number[] = [];
      for (const s of c.shapes) verts.push(...shapeWire(s));
      if (verts.length) {
        const w = new THREE.BufferGeometry();
        w.setAttribute("position", new THREE.BufferAttribute(Float32Array.from(verts), 3));
        this.content.add(new THREE.LineSegments(w, new THREE.LineBasicMaterial({ color: STYLE.shape })));
      }
      if (caps.points) {
        const cdp = c.cd_point && c.cd_point.length === n * 3 ? Float32Array.from(c.cd_point) : null;
          const pg = geo();
          if (cdp) pg.setAttribute("cd", new THREE.BufferAttribute(cdp, 3));
          // the coloured variant shares the size/col uniforms of pointMat
          const pm = cdp ? new THREE.ShaderMaterial({ defines: { USE_CD: "" }, vertexShader: POINT_VS, fragmentShader: POINT_FS, uniforms: this.pointMat.uniforms }) : this.pointMat;
          this.points = new THREE.Points(pg, pm);
        this.content.add(this.points);
      }
      if (this.showN) this.primC = primCentroids(c.prims, c.positions);
this.bb = bounds(verts.length ? [...c.positions, ...verts] : c.positions);
      this.computeFitBb();
      this.applyVisibility();
      this.frame();
    }
        // the selection survives a re-cook of the same point count (param tweaks), else it is dropped
        if (this.sel.length) { if (sameGeo) this.setSelection(this.sel, false); else this.setSelection([]); }
        this.draw();
        return warnings;
  }
    /** S: pick mode - LMB drag = box select (shift adds), click = nearest point; off = LMB orbits (a click still picks). */
    setPickMode(on: boolean): void {
      this.pickOn = on;
      this.host.classList.toggle("picking", on);
      this.changed();
    }
    get pickMode(): boolean { return this.pickOn; }
    get selection(): number[] { return [...this.sel]; }
    /** Replace the selection (ids beyond the point count are dropped), rebuild the highlight, notify. */
    setSelection(ids: number[], notify = true): void {
      const n = this.shown?.counts.points ?? 0;
      const next = clampSel(ids, n);
      const same = next.length === this.sel.length && next.every((v, i) => v === this.sel[i]);
      this.sel = next;
      this.buildHighlight();
      this.draw();
      if (notify && !same) this.onPick([...next], describe(next, this.shown?.positions ?? []));
    }
    private buildHighlight(): void {
      if (this.hi) { this.scene.remove(this.hi); this.hi.geometry.dispose(); this.hi = null; }
      const c = this.shown;
      if (!c || !this.sel.length) return;
      const pos = new Float32Array(this.sel.length * 3);
      this.sel.forEach((p, k) => pos.set(c.positions.slice(p * 3, p * 3 + 3), k * 3));
      const g = new THREE.BufferGeometry();
      g.setAttribute("position", new THREE.BufferAttribute(pos, 3));
      this.hi = new THREE.Points(g, this.hiMat);
      this.hi.renderOrder = 10;
      this.scene.add(this.hi);
    }
    /** All displayed points in viewport CSS px (`NaN` = off screen / behind the eye). */
    private screenPoints(): Float32Array {
      const c = this.shown;
      if (!c || c.error) return new Float32Array(0);
      this.camera.updateMatrixWorld();
      this.camera.updateProjectionMatrix();
      const m = new THREE.Matrix4().multiplyMatrices(this.camera.projectionMatrix, this.camera.matrixWorldInverse);
      return projectAll(c.positions, m.elements, this.host.clientWidth, this.host.clientHeight);
    }
    /** Viewport px of point `i` (proof driver + tests), or null when not visible. */
    pointScreen(i: number): { x: number; y: number } | null {
      const s = this.screenPoints();
      return i >= 0 && i * 2 < s.length && Number.isFinite(s[i * 2]) ? { x: s[i * 2], y: s[i * 2 + 1] } : null;
    }
    /** Click pick at viewport px: nearest point within 8 px; nothing there = clear (unless `add`). */
    pickAt(x: number, y: number, add: boolean): void {
      const hit = nearest(this.screenPoints(), x, y);
      this.setSelection(combine(this.sel, hit < 0 ? [] : [hit], add));
    }
    /** Box pick between two viewport px corners. */
    pickBox(x0: number, y0: number, x1: number, y1: number, add: boolean): void {
      this.setSelection(combine(this.sel, inBox(this.screenPoints(), x0, y0, x1, y1), add));
    }
    /** Draw the scene at exactly w x h px (pixel ratio 1, camera refitted) and return it as a PNG data URL: the headless render. */
    snapshot(w: number, h: number): string {
      const r = this.renderer, ratio = r.getPixelRatio();
      r.setPixelRatio(1);
      r.setSize(w, h, false);
      this.camera.aspect = w / h;
      this.fitCurrent();
      this.camera.updateMatrixWorld();
      this.render(w, h);
      const url = r.domElement.toDataURL("image/png");
      r.setPixelRatio(ratio);
      this.resize();
      return url;
    }
    /** Push the orbit state into the three.js camera. */
  private applyCam(): void {
    const e = eye(this.cam), k = clip(this.cam);
    this.camera.position.set(...e);
    this.camera.up.set(0, 1, 0);
    this.camera.lookAt(...this.cam.target);
    this.camera.near = k.near;
    this.camera.far = k.far;
    this.camera.updateProjectionMatrix();
    this.draw();
  }
  private bindInput(el: HTMLElement): void {
    type Drag = { mode: "orbit" | "pan" | "dolly" | "box"; x: number; y: number; x0: number; y0: number; moved: boolean; shift: boolean };
    let drag: Drag | null = null;
    const local = (e: PointerEvent) => { const r = el.getBoundingClientRect(); return { x: e.clientX - r.left, y: e.clientY - r.top }; };
    el.addEventListener("contextmenu", (e) => e.preventDefault());
    el.addEventListener("pointerdown", (e) => {
      const lmb = e.button === 0;
      // pick mode: LMB drag = box select (shift adds, so shift+LMB no longer pans there; MMB still does)
      const mode = e.button === 2 ? "dolly" : e.button === 1 ? "pan" : lmb && this.pickOn ? "box" : lmb && e.shiftKey ? "pan" : lmb ? "orbit" : null;
      if (!mode) return;
      const p = local(e);
      drag = { mode, x: e.clientX, y: e.clientY, x0: p.x, y0: p.y, moved: false, shift: e.shiftKey };
      try { el.setPointerCapture(e.pointerId); } catch { /* synthetic pointer (proof driver): no capture needed */ }
      e.preventDefault();
    });
    el.addEventListener("pointermove", (e) => {
      if (!drag) return;
      const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
      const p = local(e);
      if (!drag.moved && Math.hypot(p.x - drag.x0, p.y - drag.y0) <= CLICK_SLOP) return; // still a click
      drag.moved = true;
      if (drag.mode === "box") {
        const b = this.boxEl.style;
        b.display = "block"; b.left = `${Math.min(drag.x0, p.x)}px`; b.top = `${Math.min(drag.y0, p.y)}px`;
        b.width = `${Math.abs(p.x - drag.x0)}px`; b.height = `${Math.abs(p.y - drag.y0)}px`;
        return;
      }
      drag.x = e.clientX; drag.y = e.clientY;
      this.cam = drag.mode === "orbit" ? orbit(this.cam, dx, dy) : drag.mode === "pan" ? pan(this.cam, dx, dy, el.clientHeight) : dolly(this.cam, dragDolly(dx, dy));
      this.applyCam();
    });
    const end = (e: PointerEvent) => {
      const d = drag;
      drag = null;
      this.boxEl.style.display = "none";
      if (el.hasPointerCapture(e.pointerId)) el.releasePointerCapture(e.pointerId);
      if (!d || e.type === "pointercancel") return;
      const p = local(e);
      if (d.mode === "box" && d.moved) this.pickBox(d.x0, d.y0, p.x, p.y, d.shift);
      else if (!d.moved && e.button === 0) this.pickAt(p.x, p.y, d.shift); // a click without travel picks in either mode
    };
    el.addEventListener("pointerup", end);
    el.addEventListener("pointercancel", end);
    el.addEventListener("wheel", (e) => {
      e.preventDefault();
      this.cam = dolly(this.cam, wheelDolly(e.deltaY, e.deltaMode));
      this.applyCam();
    }, { passive: false });
  }
/** Recompute the Fit bounds from the shown cook + `fitExclude`. */
  private computeFitBb(): void {
    const c = this.shown;
    this.fitBb = c && !c.error ? bounds(fitPositions(c.positions, c.prims, c.prim_groups, this.fitExclude)) : null;
  }
  /** Set the Fit exclusion globs and refit. */
  setFitExclude(globs: string): void { this.fitExclude = globs; this.computeFitBb(); }
  private frame(): void {
    const b = this.fitBb ?? this.bb;
    if (!b) return;
    this.cam = fitCam(this.cam, b);
    this.applyCam();
  }
  /** H / F / toolbar Fit: frame the displayed geometry (default camera when nothing is displayed). */
  fitCurrent(): void {
    if (this.bb) this.frame();
    else { this.cam = defaultCam(); this.applyCam(); }
  }
}
