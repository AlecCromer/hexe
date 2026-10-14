//! Endorphin reader tests: synthetic hostile cases always run; the corpus test is a no-op when the bottle is absent.
use super::endorphin::*;
use std::path::{Path, PathBuf};

#[test]
fn bfloat_decodes_hex_ieee() {
    assert_eq!(bfloat("c11cf5c3"), Some(-9.81));
    assert_eq!(bfloat("3f800000"), Some(1.0));
    assert_eq!(bfloat("00000000"), Some(0.0));
    assert_eq!(bfloat("7f800000"), Some(f32::INFINITY));
    assert_eq!(bfloat(""), None);
    assert_eq!(bfloat("3f8000000"), None);
    assert_eq!(bfloat("zz"), None);
}

#[test]
fn xml_attrs_text_entities_and_unknowns_survive() {
    let n = parse_xml("<?xml version=\"1.0\"?><!-- c --><A k='v&amp;w' bx=\"3f800000\" x=\"9\"><B/><Weird q=\"1\">t&lt;x<![CDATA[<raw>]]></Weird></A>").unwrap();
    assert_eq!(n.attr("k"), Some("v&w"));
    assert_eq!(n.f32("x"), Some(1.0), "hex twin wins over decimal");
    assert_eq!(n.children.len(), 2);
    assert_eq!(n.child("Weird").unwrap().text, "t<x<raw>");
    assert_eq!(n.child("Weird").unwrap().attr("q"), Some("1"));
    assert_eq!(n.count(), 3);
}

#[test]
fn hostile_xml_errors_not_panics() {
    for s in [
        "",
        "<",
        "<a",
        "<a b>",
        "<a b=1/>",
        "<a></b>",
        "<a><b></a>",
        "<a b=\"x/>",
        "text",
        "<a><!-- x </a>",
        "<a>&#xFFFFFFFF;&#;&</a>",
    ] {
        let _ = parse_xml(s); // must return, Ok or Err
    }
    let deep = format!("{}{}", "<a>".repeat(5000), "</a>".repeat(5000));
    assert!(parse_xml(&deep).is_err(), "depth cap");
    assert!(read_tree(&[0x78, 0x01, 0xff, 0xff]).is_err(), "bad zlib");
    assert!(read_tree(&[0xff, 0xfe]).is_err(), "not utf8");
}

#[test]
fn inflate_round_trip_via_stored_block() {
    // zlib header 78 01 + one stored deflate block holding `<a/>` + adler32.
    let payload = b"<a/>";
    let mut z = vec![0x78, 0x01, 0x01, payload.len() as u8, 0, !(payload.len() as u8), 0xff];
    z.extend_from_slice(payload);
    let (mut a, mut b) = (1u32, 0u32);
    for &c in payload {
        a = (a + u32::from(c)) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    assert_eq!(read_tree(&z).unwrap().tag, "a");
}

#[test]
fn character_world_composes_joint_and_body_chain() {
    let xml = r#"<Character name="c"><MetaBodies><MetaBody name="M"><TransformMatrix><Vi bx="3f800000"/><Vj by="3f800000"/><Vk bz="3f800000"/><Vp by="3f800000"/></TransformMatrix>
<CollisionSurface name="cs" material="skin"><Geometry type="Sphere" bparam0="3f000000"/></CollisionSurface>
<InertialBody><Geometry type="Cuboid" bparam0="40000000" bparam1="3f800000" bparam2="3f800000"/><Inertia density="3.0"/></InertialBody></MetaBody></MetaBodies>
<Skeleton name="root" metaBody="M"><TransformMatrix><Vi bx="3f800000"/><Vj by="3f800000"/><Vk bz="3f800000"/><Vp by="40000000"/></TransformMatrix>
<ChildJoints><SkeletalJoint name="j1"><TransformMatrix><Vi bx="3f800000"/><Vj by="3f800000"/><Vk bz="3f800000"/><Vp bx="40400000"/></TransformMatrix></SkeletalJoint><Null name="n"/><Odd name="o"/></ChildJoints></Skeleton></Character>"#;
    let Document::Character(c) = Document::from_node(parse_xml(xml).unwrap()) else { panic!("character") };
    assert_eq!(
        c.joints.iter().map(|j| j.kind.as_str()).collect::<Vec<_>>(),
        ["Skeleton", "SkeletalJoint", "Null", "Odd"]
    );
    assert_eq!(c.joints[1].parent, Some(0));
    let w = c.world(None);
    assert_eq!(w[1].p, [3.0, 2.0, 0.0]);
    assert_eq!(c.world(Some([0.0, 5.0, 0.0]))[1].p, [3.0, 5.0, 0.0]);
    let b = c.body("M").unwrap();
    assert_eq!(b.mass(), Some(6.0));
    assert_eq!(b.collisions[0].material, "skin");
    assert_eq!(b.collisions[0].geom.as_ref().unwrap().params[0], 0.5);
    assert!(c.joints[3].raw.attr("name") == Some("o"), "unknown joint elements kept");
}

fn resources() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let p = std::env::var_os("ENDORPHIN_RESOURCES").map(PathBuf::from).unwrap_or_else(|| {
        Path::new(&home).join("Library/Application Support/CrossOver/Bottles/endorphin/drive_c/Program Files (x86)/NaturalMotion/endorphin 2.5.2/Resources")
    });
    p.is_dir().then_some(p)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Every shipped sample file parses; roots are the expected kinds; scenes have characters. No-op without the bottle.
#[test]
fn endorphin_corpus_parses_completely() {
    let Some(root) = resources() else { return };
    let mut files = Vec::new();
    walk(&root, &mut files);
    let (mut n, mut fail) = (0, Vec::new());
    for f in &files {
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if !matches!(ext.as_str(), "ens" | "nmc" | "nmb" | "nma" | "nmp" | "nmw" | "nmm" | "nme") {
            continue;
        }
        n += 1;
        let bytes = std::fs::read(f).unwrap();
        match read_endorphin(&bytes) {
            Ok(Document::Unknown(r)) => fail.push(format!("{}: unknown root {}", f.display(), r.tag)),
            Ok(Document::Scene(s)) => assert!(!s.characters.is_empty() || s.environment.is_some(), "{}", f.display()),
            Ok(_) => {}
            Err(e) => fail.push(format!("{}: {e}", f.display())),
        }
    }
    assert!(fail.is_empty(), "{} of {n} failed: {fail:#?}", fail.len());
    assert!(n >= 100, "corpus smaller than expected: {n}");
}

#[test]
fn endorphin_gravity_and_ground_read_from_a_real_scene() {
    let Some(root) = resources() else { return };
    let f = root.join("Scenes/Sample Scenes/Balcony.ens");
    let Ok(bytes) = std::fs::read(&f) else { return };
    let Document::Scene(s) = read_endorphin(&bytes).unwrap() else { panic!("scene") };
    assert_eq!(s.gravity, [0.0, -9.81, 0.0]);
    assert!(s.use_default_ground);
    assert_eq!(s.characters.len(), 4);
    assert_eq!(s.characters[0].joints.iter().filter(|j| j.kind == "SkeletalJoint").count(), 29);
    assert_eq!(s.characters[0].joints[0].kind, "Skeleton");
    assert!(s.characters[0].bodies.iter().all(|b| b.mass().is_some()));
    assert!(!s.behaviours.is_empty() && !s.channels.is_empty());
}

#[test]
fn character_detail_has_skeleton_bodies_mass_and_round_trips() {
    use super::endorphin_geo::document_detail;
    use crate::{AttrClass, AttrData, read_geo, write_geo_json};
    let xml = r#"<Character name="c"><MetaBodies><MetaBody name="M"><TransformMatrix><Vi bx="3f800000"/><Vj by="3f800000"/><Vk bz="3f800000"/></TransformMatrix>
<CollisionSurface name="cs" material="steel"><Geometry type="Sphyl" bparam0="3dcccccd" bparam1="3f000000"/></CollisionSurface>
<InertialBody><Geometry type="Cuboid" bparam0="40000000" bparam1="3f800000" bparam2="3f800000"/><Inertia density="3.0"/></InertialBody></MetaBody></MetaBodies>
<Skeleton name="root" metaBody="M"><ChildJoints><SkeletalJoint name="j1"><SoftLimits btwistSize="3f800000"/></SkeletalJoint></ChildJoints></Skeleton></Character>"#;
    let doc = Document::from_node(parse_xml(xml).unwrap());
    let d = document_detail(&doc);
    assert!(d.prim_count() > 10 && d.point_count > 10);
    let kind = d.attr(AttrClass::Prim, "kind").unwrap();
    let AttrData::Str { idx, table } = &kind.data else { panic!("str") };
    assert_eq!(idx.len(), d.prim_count() as usize);
    assert!(table.contains(&"bone".to_string()) && table.contains(&"collision".to_string()));
    let AttrData::F32(m) = &d.attr(AttrClass::Prim, "mass").unwrap().data else { panic!("f32") };
    assert!(m.iter().filter(|&&x| x == 6.0).count() > 10, "mass = 3 x 2 x 1 x 1 on every collision prim");
    let AttrData::F32(ts) = &d.attr(AttrClass::Point, "twist_soft").unwrap().data else { panic!("f32") };
    assert_eq!(ts[1], 1.0);
    assert!(d.attr(AttrClass::Detail, "endo_kind").is_some());
    let back = read_geo(write_geo_json(&d).as_slice()).unwrap();
    assert_eq!((back.point_count, back.prim_count()), (d.point_count, d.prim_count()));
}

#[test]
fn scene_detail_for_real_scenes_is_non_empty_and_finite() {
    use super::endorphin_geo::document_detail;
    use crate::AttrClass;
    let Some(root) = resources() else { return };
    for n in ["Balcony", "Car Hit", "Gymnast Rings", "Sword Play"] {
        let Ok(bytes) = std::fs::read(root.join(format!("Scenes/Sample Scenes/{n}.ens"))) else { continue };
        let d = document_detail(&read_endorphin(&bytes).unwrap());
        assert!(d.prim_count() > 500, "{n}: {}", d.prim_count());
        assert!(d.p().iter().all(|p| p.iter().all(|v| v.is_finite())), "{n}: non-finite P");
        assert!(
            d.attr(AttrClass::Detail, "endo_behaviours").is_some()
                && d.attr(AttrClass::Detail, "endo_gravity").is_some()
        );
    }
}

#[test]
fn scene_detail_groups_split_kinds_and_characters() {
    use super::endorphin_geo::document_detail;
    use crate::AttrClass;
    let Some(root) = resources() else { return };
    let Ok(bytes) = std::fs::read(root.join("Scenes/Sample Scenes/Balcony.ens")) else { return };
    let d = document_detail(&read_endorphin(&bytes).unwrap());
    for g in ["bone", "collision", "ground", "char_character01", "char_background"] {
        let grp = d.group(AttrClass::Prim, g).unwrap_or_else(|| panic!("group {g}"));
        assert_eq!(grp.mask.len(), d.prim_count() as usize);
        assert!(grp.mask.iter().any(|&m| m), "{g} empty");
    }
}

// ---------------------------------------------------------------- h43: scene pose, placement, timeline
fn joint_world(d: &crate::Detail, name: &str, ch: &str) -> [f32; 3] {
    use crate::{AttrClass, AttrData};
    let str_of = |n: &str| match &d.attr(AttrClass::Point, n).unwrap().data {
        AttrData::Str { idx, table } => idx.iter().map(|&i| table[i as usize].clone()).collect::<Vec<_>>(),
        _ => panic!("str"),
    };
    let (j, c) = (str_of("joint"), str_of("character"));
    let i = (0..j.len()).find(|&i| j[i] == name && c[i] == ch).unwrap_or_else(|| panic!("joint {ch}.{name}"));
    d.p()[i]
}
fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= tol)
}
const XF_I: &str = r#"<Vi x="1" y="0" z="0"/><Vj x="0" y="1" z="0"/><Vk x="0" y="0" z="1"/>"#;

/// Hand-computed: place = Ry(90) at (10,0,5); root pose = Rz(90) at (0,2,0); child A rest (0.5,0,0).
/// A_local_world = Rz90*(0.5,0,0)+(0,2,0) = (0,2.5,0); Ry90*(0,2.5,0)+(10,0,5) = (10,2.5,5).
#[test]
fn scene_pose_and_character_placement_apply_by_hand() {
    use super::endorphin_geo::document_detail;
    let xml = format!(
        r#"<Scene useDefaultGround="false"><Characters><Character name="c1">
<TransformMatrix><Vi x="0" y="0" z="-1"/><Vj x="0" y="1" z="0"/><Vk x="1" y="0" z="0"/><Vp x="10" y="0" z="5"/></TransformMatrix>
<Skeleton name="root"><TransformMatrix>{XF_I}<Vp x="0" y="1" z="0"/></TransformMatrix><ChildJoints>
<SkeletalJoint name="A"><TransformMatrix>{XF_I}<Vp x="0.5" y="0" z="0"/></TransformMatrix></SkeletalJoint></ChildJoints></Skeleton></Character></Characters>
<Poses><Pose character="c1" refFrame="Parent"><Relationships><Relationship name="root"><TransformMatrix>
<Vi x="0" y="1" z="0"/><Vj x="-1" y="0" z="0"/><Vk x="0" y="0" z="1"/><Vp x="0" y="2" z="0"/></TransformMatrix></Relationship></Relationships></Pose></Poses></Scene>"#
    );
    let doc = Document::from_node(parse_xml(&xml).unwrap());
    let d = document_detail(&doc);
    assert!(close(joint_world(&d, "root", "c1"), [10.0, 2.0, 5.0], 1e-5), "root = place*pose.p");
    assert!(close(joint_world(&d, "A", "c1"), [10.0, 2.5, 5.0], 1e-5), "{:?}", joint_world(&d, "A", "c1"));
    // without the Pose the rest chain gives root (10,1,5), A = root + Ry90*(0.5,0,0) = (10,1,4.5)
    let Document::Scene(mut s) = doc else { panic!("scene") };
    s.poses.clear();
    let d = document_detail(&Document::Scene(s));
    assert!(close(joint_world(&d, "A", "c1"), [10.0, 1.0, 4.5], 1e-5));
}

/// Keyed channels: A.tx 0.5 -> 1.5 over t 0..1, root.quat identity -> Rz(90). At t=0.5: root rot 45 deg, A.tx = 1.
/// A world = (0,1,0) + Rz45*(1,0,0) = (cos45, 1+sin45, 0).
#[test]
fn keyed_channels_sample_by_time() {
    use super::endorphin_geo::document_detail_at;
    let h = std::f32::consts::FRAC_1_SQRT_2;
    let xml = format!(
        r#"<Scene useDefaultGround="false"><Characters><Character name="c1"><Skeleton name="root"><TransformMatrix>{XF_I}<Vp x="0" y="1" z="0"/></TransformMatrix><ChildJoints>
<SkeletalJoint name="A"><TransformMatrix>{XF_I}<Vp x="0.5" y="0" z="0"/></TransformMatrix></SkeletalJoint></ChildJoints></Skeleton></Character></Characters>
<Animation refFrame="parent"><Channel name="c1.Joints.A.tx" type="linear"><Key x="0" y="0.5"/><Key x="1" y="1.5"/></Channel>
<QuatChannel name="c1.root.quat" type="linear"><Key t="0" x="1" y="0" z="0" w="0"/><Key t="1" x="{h}" y="0" z="0" w="{h}"/></QuatChannel></Animation></Scene>"#
    );
    let doc = Document::from_node(parse_xml(&xml).unwrap());
    let at = |t| joint_world(&document_detail_at(&doc, t), "A", "c1");
    assert!(close(at(0.0), [0.5, 1.0, 0.0], 1e-5), "{:?}", at(0.0));
    assert!(close(at(0.5), [h, 1.0 + h, 0.0], 1e-4), "{:?}", at(0.5));
    assert!(close(at(1.0), [0.0, 2.5, 0.0], 1e-5), "{:?}", at(1.0));
    assert!(close(at(9.0), at(1.0), 1e-6), "clamped past the last key");
    let Document::Scene(s) = &doc else { panic!("scene") };
    assert_eq!(s.duration(), 1.0);
}

/// Real Balcony: every character at its file placement; character02 root by hand from the XML:
/// place Vp (0.6737,0,-3.7131) + Ry(-90)-ish basis (0,0,1),(0,1,0),(-1,0,0) applied to the pose root p (0.0027,0.5531,-0.444).
#[test]
fn balcony_characters_are_separated_and_posed() {
    use super::endorphin_geo::document_detail;
    let Some(root) = resources() else { return };
    let Ok(bytes) = std::fs::read(root.join("Scenes/Sample Scenes/Balcony.ens")) else { return };
    let d = document_detail(&read_endorphin(&bytes).unwrap());
    let r: Vec<[f32; 3]> = (1..=4).map(|i| joint_world(&d, "root", &format!("character0{i}"))).collect();
    assert!(close(r[1], [1.1177, 0.5531, -3.7104], 2e-3), "character02 root {:?}", r[1]);
    for a in 0..4 {
        for b in a + 1..4 {
            let dist = (0..3).map(|k| (r[a][k] - r[b][k]).powi(2)).sum::<f32>().sqrt();
            assert!(dist > 0.8, "character0{} and 0{} stacked: {dist}", a + 1, b + 1);
        }
    }
    // character01 is placed 3 m up (placement y=3.0) + root ty 0.848
    assert!((r[0][1] - 3.848).abs() < 5e-3, "{:?}", r[0]);
    // posed, not T: character02's left knee is not at the rest offset below the hip
    let (hip, knee) = (joint_world(&d, "LeftHipJoint", "character02"), joint_world(&d, "LeftKneeJoint", "character02"));
    assert!((knee[1] - hip[1]).abs() < 0.3, "sitting thigh is ~horizontal: dy {}", knee[1] - hip[1]);
}
