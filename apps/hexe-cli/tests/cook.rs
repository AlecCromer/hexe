//! End-to-end: the real binary cooks a scene file to a `.geo` file; failures exit 1 with a message.
use std::path::PathBuf;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hexe-cli"))
}

fn work(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cli-cook");
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

#[test]
fn cook_null_scene_writes_empty_geo() {
    let (scene, out) = (work("null.json"), work("null.geo"));
    std::fs::write(&scene, r#"{"nodes":[{"name":"n","type":"null"}],"display":"n"}"#).unwrap();
    let _ = std::fs::remove_file(&out);
    let r = bin().arg("cook").arg(&scene).arg("--out").arg(&out).arg("--time").arg("0").output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let d = hexe_geo::read_geo(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!((d.point_count, d.prim_count(), d.p().len()), (0, 0, 0));
}

#[test]
fn bad_scene_exits_1_with_message() {
    let scene = work("bad.json");
    std::fs::write(&scene, "{\"nodes\":[{\"name\":\"a\",\"type\":\"nope\"}],\"display\":\"a\"}").unwrap();
    let r = bin().arg("cook").arg(&scene).arg("--out").arg(work("bad.geo")).output().unwrap();
    assert_eq!(r.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&r.stderr).contains("unknown type `nope`"));
}

#[test]
fn missing_file_and_no_args_exit_1() {
    let r = bin().args(["cook", "/nonexistent/x.json", "--out", "y.geo"]).output().unwrap();
    assert_eq!(r.status.code(), Some(1));
    assert_eq!(bin().output().unwrap().status.code(), Some(1));
}

#[test]
fn nodes_prints_registry() {
    let r = bin().arg("nodes").output().unwrap();
    assert!(r.status.success());
    assert!(String::from_utf8_lossy(&r.stdout).contains("null"));
}

#[test]
fn nodes_lists_the_motion_node_with_its_modes() {
    let r = bin().arg("nodes").output().unwrap();
    let out = String::from_utf8_lossy(&r.stdout);
    let motion = out.split("\n\n").find(|b| b.trim_start().starts_with("motion")).unwrap_or(&out);
    assert!(out.contains("motion") && motion.contains("duration_s") && motion.contains("seed"), "{out}");
}

/// The shipped `demo/endorphin-chain.hexe` shape, with the fake python stub (hermetic): motion(chain) -> output cooks the
/// skeleton frame picked by `--time` (frame param 0 = follow time; stub frame k has pelvis y = k/100).
#[test]
fn cook_motion_chain_scene_gives_the_skeleton_frame() {
    let root = env!("CARGO_MANIFEST_DIR");
    let cache = work("motion-cache");
    let _ = std::fs::remove_dir_all(&cache);
    let scene = work("chain.hexe");
    let json = format!(
        r#"{{"nodes":[{{"name":"m","type":"motion","params":{{"mode":1,"seed":8,"duration_s":2.0,"fps":4.0,"python_bin":"{root}/../../fixtures/motion/stub.sh","motion_root":"{root}","cache_dir":"{}"}}}},{{"name":"o","type":"output"}}],"edges":[["m",0,"o",0]],"display":"o"}}"#,
        cache.display()
    );
    std::fs::write(&scene, json).unwrap();
    let out = work("chain.geo");
    let r = bin().arg("cook").arg(&scene).arg("--out").arg(&out).args(["--time", "0.5"]).output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert!(String::from_utf8_lossy(&r.stdout).contains("cooked 24 points, 23 prims"));
    let d = hexe_geo::read_geo(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!((d.point_count, d.prim_count()), (24, 23));
    assert_eq!(d.p()[0], [0.0, 0.02, 2.0]); // 0.5 s * 4 fps = frame idx 2
}
