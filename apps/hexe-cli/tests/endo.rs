//! End-to-end: the real binary surveys / opens Endorphin files. Corpus parts are no-ops without the bottle.
use std::path::PathBuf;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hexe-cli"))
}

fn work(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cli-endo");
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

#[test]
fn survey_counts_a_synthetic_corpus_and_reports_bad_files() {
    let dir = work("corpus");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.ens"), r#"<Scene><Engine bx="3f800000" x="1"><Gravity bx="0"/></Engine></Scene>"#)
        .unwrap();
    std::fs::write(dir.join("b.nma"), "<Pose character=\"c\"/>").unwrap();
    std::fs::write(dir.join("bad.nmc"), "<Character><oops></Character>").unwrap();
    let md = dir.join("S.md");
    let r = bin().arg("endo-survey").arg(&dir).arg("--out").arg(&md).output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let t = std::fs::read_to_string(&md).unwrap();
    assert!(t.contains("| .ens | 1 | 1/1 |") && t.contains("| .nmc | 1 | 0/1 |"), "{t}");
    assert!(t.contains("x(+hex)"), "hex twin collapsed: {t}");
    assert!(t.contains("FAILED .nmc"));
    assert!(t.contains("**Total: 2/3 parsed."));
}

#[test]
fn survey_of_an_empty_dir_is_an_error() {
    let dir = work("empty");
    std::fs::create_dir_all(&dir).unwrap();
    let r = bin().arg("endo-survey").arg(&dir).output().unwrap();
    assert_eq!(r.status.code(), Some(1));
}

const CHAR: &str = r#"<Character name="c"><MetaBodies><MetaBody name="M"><CollisionSurface name="cs" material="skin"><Geometry type="Sphyl" bparam0="3dcccccd" bparam1="3f000000"/></CollisionSurface><InertialBody><Geometry type="Sphere" bparam0="3f000000"/><Inertia density="2.0"/></InertialBody></MetaBody></MetaBodies><Skeleton name="root" metaBody="M"><ChildJoints><SkeletalJoint name="j1"/></ChildJoints></Skeleton></Character>"#;

#[test]
fn open_writes_a_file_sop_graph_and_geo_that_cook_reads_back() {
    let f = work("toy.nmc");
    std::fs::write(&f, CHAR).unwrap();
    let (scene, geo) = (work("toy.json"), work("toy.geo"));
    let r = bin().arg("open").arg(&f).arg("--out").arg(&scene).arg("--geo").arg(&geo).output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let out = String::from_utf8_lossy(&r.stdout);
    assert!(out.contains("opened character") && out.contains("2 joints, 1 bodies"), "{out}");
    let g = hexe_geo::read_any(&geo).unwrap();
    assert!(g.prim_count() > 20 && g.attr(hexe_geo::AttrClass::Prim, "mass").is_some());
    // the graph file cooks through the normal path
    let cooked = work("toy.cooked.geo");
    let r = bin().arg("cook").arg(&scene).arg("--out").arg(&cooked).output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(hexe_geo::read_any(&cooked).unwrap().prim_count(), g.prim_count());
    // and `cook <file.nmc>` is the same thing in one step
    let direct = work("toy.direct.geo");
    assert!(bin().arg("cook").arg(&f).arg("--out").arg(&direct).status().unwrap().success());
    assert_eq!(hexe_geo::read_any(&direct).unwrap().point_count, g.point_count);
}

#[test]
fn open_rejects_bad_input_with_one_line_and_exit_1() {
    let bad = work("bad.ens");
    std::fs::write(&bad, "<Scene><oops></Scene>").unwrap();
    for args in [
        vec!["open".into()],
        vec!["open".into(), "x.json".into()],
        vec!["open".into(), bad.display().to_string()],
        vec!["open".into(), "/nonexistent/a.ens".into()],
    ] {
        let r = bin().args(&args).output().unwrap();
        assert_eq!(r.status.code(), Some(1), "{args:?}");
        assert_eq!(String::from_utf8_lossy(&r.stderr).lines().count(), 1, "{args:?}");
    }
}

#[test]
fn corpus_scene_opens_when_the_bottle_exists() {
    let Some(home) = std::env::var_os("HOME") else { return };
    let f = PathBuf::from(home).join("Library/Application Support/CrossOver/Bottles/endorphin/drive_c/Program Files (x86)/NaturalMotion/endorphin 2.5.2/Resources/Scenes/Sample Scenes/Balcony.ens");
    if !f.is_file() {
        return;
    }
    let r = bin().arg("open").arg(&f).arg("--out").arg(work("balcony.json")).output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert!(String::from_utf8_lossy(&r.stdout).contains("11 behaviours"));
}

/// `cook <scene.ens> --frame N` samples keyed channels at t = (N-1)/24: the joint moves along the key ramp, frame 1 = t 0.
#[test]
fn cook_frame_scrubs_keyed_scene_channels() {
    let xi = r#"<Vi x="1" y="0" z="0"/><Vj x="0" y="1" z="0"/><Vk x="0" y="0" z="1"/>"#;
    let f = work("keyed.ens");
    std::fs::write(
        &f,
        format!(
            r#"<Scene useDefaultGround="false"><Characters><Character name="c1"><Skeleton name="root"><TransformMatrix>{xi}<Vp x="0" y="1" z="0"/></TransformMatrix><ChildJoints><SkeletalJoint name="A"><TransformMatrix>{xi}<Vp x="0.5" y="0" z="0"/></TransformMatrix></SkeletalJoint></ChildJoints></Skeleton></Character></Characters><Animation refFrame="parent"><Channel name="c1.Joints.A.tx" type="linear"><Key x="0" y="1"/><Key x="2" y="3"/></Channel></Animation></Scene>"#
        ),
    )
    .unwrap();
    let ax = |frame: &str| {
        let out = work(&format!("keyed_{frame}.geo"));
        let r = bin().arg("cook").arg(&f).args(["--frame", frame, "--out"]).arg(&out).output().unwrap();
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        let d = hexe_geo::read_any(&out).unwrap();
        // point 1 = joint A (root = 0)
        d.p()[1][0]
    };
    assert!((ax("1") - 1.0).abs() < 1e-5);
    assert!((ax("25") - 2.0).abs() < 1e-5, "frame 25 = t 1.0 s");
    assert!((ax("49") - 3.0).abs() < 1e-5, "frame 49 = t 2.0 s (last key)");
    assert!((ax("200") - 3.0).abs() < 1e-5, "clamped past the last key");
    let r = bin().arg("cook").arg(&f).args(["--frame", "x", "--out", "/tmp/never.geo"]).output().unwrap();
    assert_eq!(r.status.code(), Some(1));
}
