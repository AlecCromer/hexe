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
