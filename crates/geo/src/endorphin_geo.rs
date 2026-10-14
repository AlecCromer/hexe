//! Endorphin `Document` -> hexe `Detail` (rest pose). Clean-room from data (oracle/endorphin/SURVEY.md).
//! * skeleton = open 2-point polylines (joint -> parent); joints = points with `joint jtype character parent *_soft *_hard`;
//! * bodies = closed polygon meshes of each collision surface (capsule/cylinder/sphere/box), prim attrs
//!   `kind body material shape mass character`; mass = inertial density x volume;
//! * scene ground = quad grid (`kind=ground`); behaviours / animation / poses / gravity = detail attrs (JSON strings).
//!
//! Axis conventions measured off the shipped limbs: Sphyl/Cylinder run along local Z (Sphyl p1 = cylinder length, p0 = radius),
//! Cuboid params are full extents. Rest pose only: joint rotation channels are carried in `endo_animation`, not applied.
use crate::endorphin::{Character, Document, Geometry, Limits, Node, Scene, Xf};
use crate::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use serde_json::Value;

const SIDES: usize = 10;
const RINGS: usize = 3;

type Mesh = (Vec<[f32; 3]>, Vec<Vec<usize>>);

/// Surface of revolution about Z from `(z, rho)` rings (`rho == 0` = pole), `SIDES` segments.
fn revolve(profile: &[(f32, f32)]) -> Mesh {
    let (mut v, mut f) = (Vec::new(), Vec::new());
    let mut ring: Vec<(usize, bool)> = Vec::new(); // (first vertex index, is_pole)
    for &(z, rho) in profile {
        let pole = rho.abs() < 1e-9;
        ring.push((v.len(), pole));
        if pole {
            v.push([0.0, 0.0, z]);
        } else {
            for s in 0..SIDES {
                let a = std::f32::consts::TAU * s as f32 / SIDES as f32;
                v.push([rho * a.cos(), rho * a.sin(), z]);
            }
        }
    }
    for w in ring.windows(2) {
        let ((a, ap), (b, bp)) = (w[0], w[1]);
        for s in 0..SIDES {
            let t = (s + 1) % SIDES;
            match (ap, bp) {
                (false, false) => f.push(vec![a + s, a + t, b + t, b + s]),
                (true, false) => f.push(vec![a, b + t, b + s]),
                (false, true) => f.push(vec![b, a + s, a + t]),
                (true, true) => {}
            }
        }
    }
    (v, f)
}

fn capsule(r: f32, len: f32) -> Mesh {
    let h = len * 0.5;
    let ang = |k: usize| std::f32::consts::FRAC_PI_2 * (1.0 - k as f32 / (RINGS + 1) as f32);
    let mut p = vec![(-h - r, 0.0)];
    for k in 1..=RINGS {
        p.push((-h - r * ang(k).sin(), r * ang(k).cos()));
    }
    for k in (1..=RINGS).rev() {
        p.push((h + r * ang(k).sin(), r * ang(k).cos()));
    }
    p.push((h + r, 0.0));
    revolve(&p)
}

fn cuboid(a: f32, b: f32, c: f32) -> Mesh {
    let (x, y, z) = (a * 0.5, b * 0.5, c * 0.5);
    let v = vec![[-x, -y, -z], [x, -y, -z], [x, y, -z], [-x, y, -z], [-x, -y, z], [x, -y, z], [x, y, z], [-x, y, z]];
    let f = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![2, 3, 7, 6],
        vec![1, 2, 6, 5],
        vec![3, 0, 4, 7],
    ];
    (v, f)
}

/// Local-space mesh of a primitive; `File`/`Mesh`/unknown get a 5 cm placeholder box (the `shape` attr says what it was).
fn mesh_of(g: &Geometry) -> Mesh {
    let [a, b, c, _] = g.params.map(f32::abs);
    match g.kind.as_str() {
        "Sphere" => capsule(a, 0.0),
        "Sphyl" => capsule(a, b),
        "Cylinder" => revolve(&[(-b * 0.5, 0.0), (-b * 0.5, a), (b * 0.5, a), (b * 0.5, 0.0)]),
        "Cuboid" => cuboid(a, b, c),
        _ => cuboid(0.05, 0.05, 0.05),
    }
}

/// Per-prim and per-point attribute columns, pushed in lock-step with the geometry.
#[derive(Default)]
struct Cols {
    kind: Vec<String>,
    body: Vec<String>,
    material: Vec<String>,
    shape: Vec<String>,
    character: Vec<String>,
    mass: Vec<f32>,
    pj: Vec<String>,
    pjtype: Vec<String>,
    pchar: Vec<String>,
    pparent: Vec<i32>,
    plim: [Vec<f32>; 6],
}

impl Cols {
    fn prim(&mut self, kind: &str, body: &str, material: &str, shape: &str, mass: f32, ch: &str) {
        self.kind.push(kind.into());
        self.body.push(body.into());
        self.material.push(material.into());
        self.shape.push(shape.into());
        self.character.push(ch.into());
        self.mass.push(mass);
    }
    fn point(&mut self, joint: &str, jtype: &str, ch: &str, parent: i32, lim: [f32; 6]) {
        self.pj.push(joint.into());
        self.pjtype.push(jtype.into());
        self.pchar.push(ch.into());
        self.pparent.push(parent);
        for (c, v) in self.plim.iter_mut().zip(lim) {
            c.push(v);
        }
    }
}

fn str_attr(name: &str, class: AttrClass, vals: &[String]) -> Attribute {
    let mut table: Vec<String> = Vec::new();
    let idx = vals
        .iter()
        .map(|v| match table.iter().position(|t| t == v) {
            Some(i) => i as i32,
            None => {
                table.push(v.clone());
                table.len() as i32 - 1
            }
        })
        .collect();
    Attribute { name: name.into(), class, data: AttrData::Str { idx, table }, type_info: TypeInfo::None }
}

fn f32_attr(name: &str, class: AttrClass, v: Vec<f32>) -> Attribute {
    Attribute { name: name.into(), class, data: AttrData::F32(v), type_info: TypeInfo::None }
}

#[derive(Default)]
struct B {
    d: Detail,
    c: Cols,
}

/// Label for one placed collision/inertial mesh.
struct Tag<'a> {
    kind: &'a str,
    body: &'a str,
    material: &'a str,
    shape: &'a str,
    mass: f32,
    ch: &'a str,
}

impl B {
    fn mesh(&mut self, m: &Mesh, x: &Xf, t: &Tag) {
        let base = self.d.point_count;
        let r = self.d.add_points(m.0.len() as u32);
        let p = self.d.p_mut();
        for (i, v) in m.0.iter().enumerate() {
            if let Some(s) = p.get_mut(r.start as usize + i) {
                *s = x.apply(*v);
            }
            self.c.point("", "", "", -1, [0.0; 6]);
        }
        for f in &m.1 {
            let ids: Vec<u32> = f.iter().map(|&i| base + i as u32).collect();
            self.d.add_poly(&ids, true);
            self.c.prim(t.kind, t.body, t.material, t.shape, t.mass, t.ch);
        }
    }

    fn character(&mut self, c: &Character, root_p: Option<[f32; 3]>) {
        let w = c.world(root_p);
        let base = self.d.point_count;
        let r = self.d.add_points(c.joints.len() as u32);
        for (i, j) in c.joints.iter().enumerate() {
            if let (Some(s), Some(x)) = (self.d.p_mut().get_mut(r.start as usize + i), w.get(i)) {
                *s = x.p;
            }
            let lim = |l: &Option<Limits>| l.map_or([0.0; 3], |l| [l.twist, l.swing1, l.swing2]);
            let (s, h) = (lim(&j.soft), lim(&j.hard));
            let parent = j.parent.map_or(-1, |p| (base as usize + p) as i32);
            self.c.point(&j.name, &j.kind, &c.name, parent, [s[0], s[1], s[2], h[0], h[1], h[2]]);
        }
        for (i, j) in c.joints.iter().enumerate() {
            if let Some(p) = j.parent {
                self.d.add_poly(&[base + p as u32, base + i as u32], false);
                self.c.prim("bone", &j.name, "", "bone", 0.0, &c.name);
            }
        }
        for (i, j) in c.joints.iter().enumerate() {
            let (Some(bn), Some(jw)) = (j.body.as_deref(), w.get(i)) else { continue };
            let Some(b) = c.body(bn) else { continue };
            let bw = jw.mul(&b.xf);
            let mass = b.mass().unwrap_or(0.0);
            let mut drew = false;
            for cs in &b.collisions {
                if let Some(g) = &cs.geom {
                    let t = Tag {
                        kind: "collision",
                        body: &b.name,
                        material: &cs.material,
                        shape: &g.kind,
                        mass,
                        ch: &c.name,
                    };
                    self.mesh(&mesh_of(g), &bw.mul(&cs.xf), &t);
                    drew = true;
                }
            }
            if let (false, Some(g)) = (drew, &b.inertial) {
                let t = Tag { kind: "inertial", body: &b.name, material: "", shape: &g.kind, mass, ch: &c.name };
                self.mesh(&mesh_of(g), &bw, &t);
            }
        }
    }

    fn ground(&mut self, half: f32, n: usize) {
        let base = self.d.point_count;
        let r = self.d.add_points(((n + 1) * (n + 1)) as u32);
        let p = self.d.p_mut();
        for gz in 0..=n {
            for gx in 0..=n {
                if let Some(s) = p.get_mut(r.start as usize + gz * (n + 1) + gx) {
                    *s = [(gx as f32 / n as f32 * 2.0 - 1.0) * half, 0.0, (gz as f32 / n as f32 * 2.0 - 1.0) * half];
                }
                self.c.point("", "", "", -1, [0.0; 6]);
            }
        }
        let id = |x: usize, z: usize| base + (z * (n + 1) + x) as u32;
        for gz in 0..n {
            for gx in 0..n {
                self.d.add_poly(&[id(gx, gz), id(gx, gz + 1), id(gx + 1, gz + 1), id(gx + 1, gz)], true);
                self.c.prim("ground", "ground", "default", "grid", 0.0, "");
            }
        }
    }

    fn finish(self) -> Detail {
        let (mut d, c) = (self.d, self.c);
        if d.point_count == 0 {
            return d;
        }
        let (pr, pt) = (AttrClass::Prim, AttrClass::Point);
        d.add_attr(str_attr("kind", pr, &c.kind));
        d.add_attr(str_attr("body", pr, &c.body));
        d.add_attr(str_attr("material", pr, &c.material));
        d.add_attr(str_attr("shape", pr, &c.shape));
        d.add_attr(str_attr("character", pr, &c.character));
        d.add_attr(f32_attr("mass", pr, c.mass));
        d.add_attr(str_attr("joint", pt, &c.pj));
        d.add_attr(str_attr("jtype", pt, &c.pjtype));
        d.add_attr(str_attr("character", pt, &c.pchar));
        d.add_attr(Attribute {
            name: "parent".into(),
            class: pt,
            data: AttrData::I32(c.pparent),
            type_info: TypeInfo::None,
        });
        for (name, col) in ["twist_soft", "swing1_soft", "swing2_soft", "twist_hard", "swing1_hard", "swing2_hard"]
            .into_iter()
            .zip(c.plim)
        {
            d.add_attr(f32_attr(name, pt, col));
        }
        d
    }
}

fn detail_str(d: &mut Detail, name: &str, v: &str) {
    d.add_attr(str_attr(name, AttrClass::Detail, &[v.to_string()]));
}

fn nodes_json<'a>(ns: impl Iterator<Item = &'a Node>) -> String {
    Value::Array(ns.map(Node::to_json).collect()).to_string()
}

fn scene_detail(s: &Scene) -> Detail {
    let mut b = B::default();
    if s.use_default_ground {
        b.ground(10.0, 10);
    }
    if let Some(e) = &s.environment {
        b.character(e, None);
    }
    for c in &s.characters {
        let g = |p: &str| s.channel0(&c.name, p);
        let root = match (g("root.tx"), g("root.ty"), g("root.tz")) {
            (Some(x), Some(y), Some(z)) => Some([x, y, z]),
            _ => None,
        };
        b.character(c, root);
    }
    let mut d = b.finish();
    detail_str(&mut d, "endo_kind", "scene");
    d.add_attr(Attribute {
        name: "endo_gravity".into(),
        class: AttrClass::Detail,
        data: AttrData::V3(vec![s.gravity]),
        type_info: TypeInfo::Vector,
    });
    detail_str(&mut d, "endo_behaviours", &nodes_json(s.behaviours.iter().map(|b| &b.raw)));
    let ch = Value::Array(
        s.channels
            .iter()
            .map(|c| serde_json::json!({"name": c.name, "quat": c.quat, "interp": c.interp, "keys": c.keys}))
            .collect(),
    );
    detail_str(&mut d, "endo_animation", &ch.to_string());
    detail_str(&mut d, "endo_poses", &nodes_json(s.poses.iter().map(|p| &p.raw)));
    d
}

/// Rest-pose geometry + detail attrs for any opened Endorphin file.
pub fn document_detail(doc: &Document) -> Detail {
    let mut d = match doc {
        Document::Scene(s) => return scene_detail(s),
        Document::Character(c) => {
            let mut b = B::default();
            b.character(c, None);
            let mut d = b.finish();
            detail_str(&mut d, "endo_kind", "character");
            d
        }
        Document::Behaviour(bh) => {
            let mut d = Detail::default();
            detail_str(&mut d, "endo_kind", "behaviour");
            detail_str(&mut d, "endo_behaviours", &nodes_json(std::iter::once(&bh.raw)));
            d
        }
        Document::Pose(p) => {
            let mut d = Detail::default();
            detail_str(&mut d, "endo_kind", "pose");
            detail_str(&mut d, "endo_poses", &nodes_json(std::iter::once(&p.raw)));
            d
        }
        Document::Unknown(n) => {
            let mut d = Detail::default();
            detail_str(&mut d, "endo_kind", "unknown");
            detail_str(&mut d, "endo_raw", &n.to_json().to_string());
            d
        }
    };
    d.canonicalize();
    d
}
