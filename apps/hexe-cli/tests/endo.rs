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

#[allow(dead_code)]
fn corpus() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("HOME")?)
        .join("Library/Application Support/CrossOver/Bottles/endorphin/drive_c/Program Files (x86)/NaturalMotion/endorphin 2.5.2/Resources");
    p.is_dir().then_some(p)
}
