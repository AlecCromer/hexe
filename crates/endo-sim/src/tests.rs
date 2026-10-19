use super::*;
use hexe_geo::endorphin::{Document, parse_xml, read_endorphin};
use std::path::{Path, PathBuf};

fn resources() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let p = std::env::var_os("ENDORPHIN_RESOURCES").map(PathBuf::from).unwrap_or_else(|| {
        Path::new(&home).join("Library/Application Support/CrossOver/Bottles/endorphin/drive_c/Program Files (x86)/NaturalMotion/endorphin 2.5.2/Resources")
    });
    p.is_dir().then_some(p)
}

fn balcony() -> Option<Scene> {
    let bytes = std::fs::read(resources()?.join("Scenes/Sample Scenes/Balcony.ens")).ok()?;
    match read_endorphin(&bytes).ok()? {
        Document::Scene(s) => Some(*s),
        _ => None,
    }
}

fn root_ys(sim: &Sim) -> Vec<(String, f32)> {
    sim.dynamic_characters().into_iter().filter_map(|n| Some((n.to_string(), sim.root_y(n)?))).collect()
}

/// Two-joint character (root + child with limits) 2 m above a default ground.
const MINI: &str = r#"<Scene useDefaultGround="true"><Characters><Character name="c">
<Skeleton name="root" metaBody="pelvis"><TransformMatrix><Vp x="0" y="2" z="0"/></TransformMatrix>
<ChildJoints><SkeletalJoint name="knee" metaBody="shin"><TransformMatrix><Vp x="0" y="0" z="0.4"/></TransformMatrix>
<SoftLimits twistSize="0.5" swing1Size="1.0" swing2Size="1.0"/><HardLimits twistSize="0.6" swing1Size="1.2" swing2Size="1.2"/>
</SkeletalJoint></ChildJoints></Skeleton>
<MetaBodies>
<MetaBody name="pelvis"><CollisionSurface><Geometry type="Sphyl" param0="0.1" param1="0.3"/></CollisionSurface><InertialBody><Geometry type="Sphere" param0="0.1"/><Inertia density="1000"/></InertialBody></MetaBody>
<MetaBody name="shin"><CollisionSurface><Geometry type="Cuboid" param0="0.1" param1="0.1" param2="0.4"/></CollisionSurface></MetaBody>
</MetaBodies></Character></Characters></Scene>"#;

#[test]
fn mini_scene_falls_onto_the_ground_and_stays_finite() {
    let n = parse_xml(MINI).unwrap();
    let s = Scene::from_node(&n);
    let mut sim = Sim::from_scene(&s);
    assert_eq!(sim.counts().0, 2, "two bodies");
    assert_eq!(sim.counts().1, 1, "one joint");
    let y0 = sim.root_y("c").unwrap();
    assert!((y0 - 2.0).abs() < 1e-4, "{y0}");
    for _ in 0..90 {
        sim.step(1.0 / 30.0);
    }
    let y = sim.root_y("c").unwrap();
    assert!(y < 0.5 && y > -0.01, "rests on the ground: {y}");
    for cp in sim.pose() {
        assert!(cp.world.iter().all(|x| [x.i, x.j, x.k, x.p].iter().flatten().all(|v| v.is_finite())));
    }
}

#[test]
fn bbox_fallback_is_a_capsule_for_unknown_geometry() {
    let g = Geometry {
        kind: "File".into(),
        params: [0.0; 4],
        file: Some("x.mesh".into()),
        raw: parse_xml("<Geometry/>").unwrap(),
    };
    assert!(shape_of(&g).volume > 0.0);
    let c = bbox_capsule([0.1, 0.2, 0.5]);
    assert!((c.volume - (std::f32::consts::PI * 0.01 * 0.8 + 4.0 / 3.0 * std::f32::consts::PI * 0.001)).abs() < 1e-5);
}

/// Balcony: character01 stands on the balcony slab (root y 3.85 m, slab top 3.0 m); 02..04 are seated (root ~0.55 m, supported by
/// the set). Passive ragdoll: the standing one collapses (monotone descent to touchdown, then settles within 5 cm), the seated
/// ones stay put-ish; nothing is NaN.
#[test]
fn balcony_standing_characters_fall_monotonically_without_nan() {
    let Some(scene) = balcony() else { return };
    let mut sim = Sim::from_scene(&scene);
    let start = root_ys(&sim);
    assert_eq!(start.len(), 4, "four dynamic characters");
    let mut hist: Vec<Vec<(String, f32)>> = vec![start.clone()];
    for _ in 0..90 {
        sim.step(1.0 / 30.0);
        for cp in &sim.pose() {
            for x in &cp.world {
                assert!([x.i, x.j, x.k, x.p].iter().flatten().all(|v| v.is_finite()), "NaN in {}", cp.name);
            }
        }
        hist.push(root_ys(&sim));
    }
    let mut standing = 0;
    for (i, (name, y0)) in start.iter().enumerate() {
        let ys: Vec<f32> = hist.iter().map(|h| h[i].1).collect();
        if *y0 > 1.0 {
            standing += 1;
            let low = (0..ys.len()).min_by(|&a, &b| ys[a].total_cmp(&ys[b])).unwrap();
            assert!(low > 3 && low < 60, "{name}: touchdown at frame {low}");
            assert!(
                ys.windows(2).take(low).all(|w| w[1] <= w[0]),
                "{name}: y not monotone down to touchdown: {:?}",
                &ys[..=low]
            );
            assert!(ys[0] - ys[low] > 0.5, "{name}: dropped only {}", ys[0] - ys[low]);
            assert!(ys[low..].iter().all(|y| *y - ys[low] < 0.05), "{name}: bounces back up");
        } else {
            assert!(ys.iter().all(|y| (y - y0).abs() < 0.3), "{name}: seated character left its seat: {ys:?}");
        }
    }
    assert_eq!(standing, 1, "character01 only");
}
