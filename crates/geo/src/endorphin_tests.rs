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
