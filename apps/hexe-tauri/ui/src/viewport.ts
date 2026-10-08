// three.js viewport: points (Points), open prims (LineSegments), closed polys (fan-triangulated flat-shaded Mesh) +
// optional wireframe overlay. OrbitControls: left = orbit, right/middle = pan, wheel = zoom. Fit on every cook.
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { Cooked } from "./api";
import { bounds, edgeSegments, shapeWire, triangulate } from "./geom";

export class Viewport {
  private renderer: THREE.WebGLRenderer;
  private scene = new THREE.Scene();
  private camera = new THREE.PerspectiveCamera(45, 1, 0.01, 10000);
  private controls: OrbitControls;
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
    this.camera.position.set(3, 2.5, 4);
    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.enableDamping = false;
    this.controls.addEventListener("change", () => this.draw());
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

  /** Frame the point cloud's bounding sphere, keeping the current view direction. */
  fit(positions: ArrayLike<number>): void {
    const b = bounds(positions);
    if (!b) return;
    const r = Math.max(b.radius, 1e-3);
    const dir = this.camera.position.clone().sub(this.controls.target);
    if (dir.lengthSq() < 1e-12) dir.set(1, 0.8, 1.2);
    dir.normalize();
    const dist = r / Math.sin((this.camera.fov * Math.PI) / 360) * 1.15;
    this.controls.target.set(...b.center);
    this.camera.position.copy(this.controls.target).addScaledVector(dir, dist);
    this.camera.near = Math.max(dist / 1000, 1e-4);
    this.camera.far = dist * 100;
    this.camera.updateProjectionMatrix();
    this.controls.update();
    this.draw();
  }

  /** Re-fit to the currently shown geometry (toolbar Fit button). */
  fitCurrent(): void {
    const p = this.points?.geometry.getAttribute("position");
    if (p) this.fit(p.array as Float32Array);
  }
}
