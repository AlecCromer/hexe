// three.js viewport: points (Points), open prims (LineSegments), closed polys (fan-triangulated flat-shaded Mesh) +
// optional wireframe overlay. Houdini camera (camera.ts): LMB/alt+LMB orbit, shift+LMB/MMB pan, RMB dolly, wheel dolly. Fit on every cook.
import * as THREE from "three";
import { clip, type Cam, defaultCam, dolly, dragDolly, eye, fit as fitCam, FOV, orbit, pan, wheelDolly } from "./camera";
import type { Cooked } from "./api";
import { bounds, edgeSegments, shapeWire, triangulate } from "./geom";

export class Viewport {
  private renderer: THREE.WebGLRenderer;
  private scene = new THREE.Scene();
  private camera = new THREE.PerspectiveCamera((FOV * 180) / Math.PI, 1, 0.01, 10000);
  private cam: Cam = defaultCam();
  private content = new THREE.Group();
  private wire: THREE.LineSegments | null = null;
  private points: THREE.Points | null = null;
  private wireOn = false;
  private pointsOn = true;
  private raf = 0;

  constructor(private host: HTMLElement) {
    this.renderer = new THREE.WebGLRenderer({ antialias: true, preserveDrawingBuffer: true });
    this.renderer.setPixelRatio(window.devicePixelRatio);
    host.appendChild(this.renderer.domElement);
    this.scene.background = new THREE.Color(0x3b3f45);
    this.scene.add(new THREE.HemisphereLight(0xffffff, 0x444a55, 1.4));
    const key = new THREE.DirectionalLight(0xffffff, 1.6);
    key.position.set(3, 5, 4);
    this.scene.add(key);
    const grid = new THREE.GridHelper(20, 20, 0x666d78, 0x4d535c);
    this.scene.add(grid, this.content);
    this.bindInput(this.renderer.domElement);
this.applyCam();
new ResizeObserver(() => this.resize()).observe(host);
    this.resize();
  }

  private resize(): void {
    const w = Math.max(1, this.host.clientWidth), h = Math.max(1, this.host.clientHeight);
    this.renderer.setSize(w, h, false);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.draw();
  }

  private draw(): void {
    cancelAnimationFrame(this.raf);
    this.raf = requestAnimationFrame(() => this.renderer.render(this.scene, this.camera));
  }

  setWireframe(on: boolean): void { this.wireOn = on; if (this.wire) this.wire.visible = on; this.draw(); }
  setPoints(on: boolean): void { this.pointsOn = on; if (this.points) this.points.visible = on; this.draw(); }

  private clear(): void {
    for (const o of [...this.content.children]) {
      this.content.remove(o);
      o.traverse((c) => {
        const m = c as THREE.Mesh;
        m.geometry?.dispose();
        (Array.isArray(m.material) ? m.material : [m.material]).forEach((x) => x?.dispose());
      });
    }
    this.wire = this.points = null;
  }

  /** Replace the displayed geometry with a fresh cook and fit the camera to it. */
  show(c: Cooked): void {
    this.clear();
    const n = c.counts.points;
    if (!c.error && n > 0) {
      const pos = new THREE.Float32BufferAttribute(Float32Array.from(c.positions), 3);
      const geo = () => { const g = new THREE.BufferGeometry(); g.setAttribute("position", pos); return g; };
      const tris = triangulate(c.prims, c.closed, c.positions);
      if (tris.length) {
        const g = geo();
        g.setIndex(new THREE.BufferAttribute(tris, 1));
        const mat = new THREE.MeshStandardMaterial({
          color: 0xb9c2cf, flatShading: true, side: THREE.DoubleSide, roughness: 0.75, metalness: 0.05,
          polygonOffset: true, polygonOffsetFactor: 1, polygonOffsetUnits: 1,
        });
        this.content.add(new THREE.Mesh(g, mat));
      }
      const open = edgeSegments(c.prims, c.closed, n, true);
      if (open.length) {
        const g = geo();
        g.setIndex(new THREE.BufferAttribute(open, 1));
        this.content.add(new THREE.LineSegments(g, new THREE.LineBasicMaterial({ color: 0xffd24a })));
      }
      const all = edgeSegments(c.prims, c.closed, n, false);
      if (all.length) {
        const g = geo();
        g.setIndex(new THREE.BufferAttribute(all, 1));
        this.wire = new THREE.LineSegments(g, new THREE.LineBasicMaterial({ color: 0x14171a }));
        this.wire.visible = this.wireOn;
        this.content.add(this.wire);
      }
      if (c.shapes.length) {
  const w = new THREE.BufferGeometry();
  w.setAttribute("position", new THREE.BufferAttribute(Float32Array.from(c.shapes.flatMap((x) => [...shapeWire(x)])), 3));
  this.content.add(new THREE.LineSegments(w, new THREE.LineBasicMaterial({ color: 0x7fd4ff })));
}
this.points = new THREE.Points(geo(), new THREE.PointsMaterial({ color: 0xffa24a, size: 5, sizeAttenuation: false }));
      this.points.visible = this.pointsOn;
      this.content.add(this.points);
      this.fit(c.positions);
    }
    this.draw();
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
let drag: { mode: "orbit" | "pan" | "dolly"; x: number; y: number } | null = null;
el.addEventListener("contextmenu", (e) => e.preventDefault());
el.addEventListener("pointerdown", (e) => {
const mode = e.button === 2 ? "dolly" : e.button === 1 || (e.button === 0 && e.shiftKey) ? "pan" : e.button === 0 ? "orbit" : null;
if (!mode) return;
drag = { mode, x: e.clientX, y: e.clientY };
el.setPointerCapture(e.pointerId);
e.preventDefault();
});
el.addEventListener("pointermove", (e) => {
if (!drag) return;
const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
drag.x = e.clientX; drag.y = e.clientY;
this.cam = drag.mode === "orbit" ? orbit(this.cam, dx, dy) : drag.mode === "pan" ? pan(this.cam, dx, dy, el.clientHeight) : dolly(this.cam, dragDolly(dx, dy));
this.applyCam();
});
const end = (e: PointerEvent) => { drag = null; if (el.hasPointerCapture(e.pointerId)) el.releasePointerCapture(e.pointerId); };
el.addEventListener("pointerup", end);
el.addEventListener("pointercancel", end);
el.addEventListener("wheel", (e) => {
e.preventDefault();
this.cam = dolly(this.cam, wheelDolly(e.deltaY, e.deltaMode));
this.applyCam();
}, { passive: false });
}
/** Frame the point cloud's bounding sphere, keeping the current view direction. */
fit(positions: ArrayLike<number>): void {
const b = bounds(positions);
if (!b) return;
this.cam = fitCam(this.cam, b);
this.applyCam();
}
/** Re-fit to the currently shown geometry (toolbar Fit / H / F). */
fitCurrent(): void {
const p = this.points?.geometry.getAttribute("position");
if (p) this.fit(p.array as Float32Array);
}
}
