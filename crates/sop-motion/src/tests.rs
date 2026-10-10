use super::*;
use hexe_geo::Detail;

fn stub() -> String {
    format!("{}/../../fixtures/motion/stub.sh", env!("CARGO_MANIFEST_DIR"))
}

/// Per-test scratch under `~/.gaia/cache/hexe/motion-test/` (never /tmp); removed on drop.
struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Scratch {
        let home = std::env::var_os("HOME").map(PathBuf::from).expect("HOME");
        let p = home.join(".gaia/cache/hexe/motion-test").join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("scratch");
        Scratch(p)
    }
    fn s(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// All spec defaults, then stub-friendly overrides.
fn params(cache: &str, over: &[(&str, ParamValue)]) -> ParamSet {
    let mut p = ParamSet::new();
    for s in spec() {
        p = p.with(&s.name, s.default);
    }
    p = p
        .with("python_bin", ParamValue::S(stub()))
        .with("motion_root", ParamValue::S(env!("CARGO_MANIFEST_DIR").into()))
        .with("cache_dir", ParamValue::S(cache.into()))
        .with("frames", ParamValue::I(3));
    for (k, v) in over {
        p = p.with(k, v.clone());
    }
    p
}

fn cook(p: &ParamSet, time: f32) -> Result<Arc<Detail>, CookError> {
    let ctx = CookCtx { time, fps: 24.0, params: p, log: &|_| {} };
    new().cook(&ctx, &[])
}

fn p0x(d: &Detail) -> f32 {
    d.p()[0][0]
}

#[test]
fn registry_contract_and_menu_matches_gaia_motion_scenes() {
    let n = new();
    assert_eq!(n.type_name(), "motion");
    assert_eq!(n.params(), spec().as_slice());
    assert!(n.inputs().is_empty() && n.time_dependent());
    assert!(spec().iter().all(|s| s.default.is_acceptable()));
    let scene = &spec()[0];
    let toks: Vec<&str> = scene.menu.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(toks, SCENES);
    assert_eq!(scene.default, ParamValue::Menu(1)); // push
}

#[test]
fn stub_clip_cooks_the_frame_at_time_and_clamps() {
    let sc = Scratch::new("frames");
    let p = params(&sc.s(), &[]);
    let d0 = cook(&p, 0.0).expect("frame 0");
    assert_eq!(d0.p().len(), 2);
    assert_eq!(p0x(&d0), 0.0);
    assert_eq!(p0x(&cook(&p, 1.0 / 30.0).expect("f1")), 1.0);
    assert_eq!(p0x(&cook(&p, 2.0 / 30.0).expect("f2")), 2.0);
    assert_eq!(p0x(&cook(&p, 99.0).expect("clamped high")), 2.0);
    assert_eq!(p0x(&cook(&p, -5.0).expect("clamped low")), 0.0);
    // fps param (clip fps) drives time -> frame, not the scene fps
    let p60 = params(&sc.s(), &[("fps", ParamValue::F(60.0))]);
    assert_eq!(p0x(&cook(&p60, 1.0 / 60.0).expect("f1@60")), 1.0);
}

#[test]
fn passes_the_real_cli_flags() {
    let sc = Scratch::new("argv");
    let p = params(
        &sc.s(),
        &[
            ("scene", ParamValue::Menu(2)),
            ("impulse_ns", ParamValue::F(100.0)),
            ("impulse_dir", ParamValue::V3([0.0, -2.0, 0.0])),
            ("impulse_body", ParamValue::S("L_Knee".into())),
        ],
    );
    cook(&p, 0.0).expect("cooks");
    let cfg = read_cfg(&p).expect("cfg");
    let argv = std::fs::read_to_string(sc.0.join(param_hash(&cfg)).join("argv.txt")).expect("argv.txt");
    let got: Vec<&str> = argv.lines().collect();
    let out = sc.0.join(param_hash(&cfg)).to_string_lossy().into_owned();
    // out dir is the pre-rename run dir; compare everything but it
    let mut flags: Vec<&str> = got.clone();
    let oi = flags.iter().position(|a| *a == "--out").expect("--out") + 1;
    assert!(flags[oi].contains(&param_hash(&cfg)) && flags[oi].contains(".run-"), "{out} vs {}", flags[oi]);
    flags[oi] = "<out>";
    assert_eq!(
        flags,
        [
            "-m",
            "gaia_motion.cli",
            "run",
            "--scene",
            "leg_hit",
            "--frames",
            "3",
            "--out",
            "<out>",
            "--push-body",
            "L_Knee",
            "--push-impulse",
            "0",
            "-100",
            "0",
            "--up-axis",
            "y"
        ]
    );
}

#[test]
fn cache_hit_skips_python_and_params_change_the_hash() {
    let sc = Scratch::new("cache");
    let p = params(&sc.s(), &[]);
    cook(&p, 0.0).expect("first cook runs the stub");
    // python gone: a second cook of the same sim params must come from the cache
    let gone = params(&sc.s(), &[("python_bin", ParamValue::S("/nonexistent/python".into()))]);
    assert_eq!(p0x(&cook(&gone, 1.0 / 30.0).expect("cache hit")), 1.0);
    // a different sim param is a miss -> needs python -> spawn error
    let miss = params(
        &sc.s(),
        &[("python_bin", ParamValue::S("/nonexistent/python".into())), ("impulse_ns", ParamValue::F(5.0))],
    );
    let e = cook(&miss, 0.0).expect_err("miss");
    assert!(matches!(&e, CookError::Failed(m) if m.contains("spawn /nonexistent/python")), "{e}");
    let (a, b) = (read_cfg(&p).expect("a"), read_cfg(&miss).expect("b"));
    assert_ne!(param_hash(&a), param_hash(&b));
    assert_eq!(param_hash(&a).len(), 32);
    // hash is stable across processes (pinned) and ignores location/playback params
    let moved = params(&sc.s(), &[("fps", ParamValue::F(12.0)), ("timeout_s", ParamValue::I(5))]);
    assert_eq!(param_hash(&a), param_hash(&read_cfg(&moved).expect("moved")));
    assert_eq!(param_hash(&a), "2ab6f5cdc8a414c470da52eba4698845");
    // no half-written run dirs left behind
    let leftovers: Vec<String> = std::fs::read_dir(&sc.0)
        .expect("ls")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.contains(".run-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn python_failure_surfaces_stderr_and_caches_nothing() {
    let sc = Scratch::new("fail");
    let p = params(&sc.s(), &[("impulse_body", ParamValue::S("FAIL".into()))]);
    let e = cook(&p, 0.0).expect_err("stub exits 3");
    let CookError::Failed(m) = &e else { panic!("{e:?}") };
    assert!(m.contains("exited 3") && m.contains("stub: boom (push-body FAIL)"), "{m}");
    assert_eq!(std::fs::read_dir(&sc.0).expect("ls").count(), 0, "failed run must leave no cache entry");
}

#[test]
fn hostile_params_are_bad_param_not_panic() {
    let sc = Scratch::new("hostile");
    let bad: Vec<(&str, ParamValue)> = vec![
        ("frames", ParamValue::I(0)),
        ("frames", ParamValue::I(i32::MAX)),
        ("frames", ParamValue::I(-4)),
        ("impulse_ns", ParamValue::F(f32::NAN)),
        ("impulse_ns", ParamValue::F(f32::INFINITY)),
        ("impulse_dir", ParamValue::V3([0.0, 0.0, 0.0])),
        ("impulse_dir", ParamValue::V3([f32::NAN, 0.0, 1.0])),
        ("impulse_body", ParamValue::S("--onnx=evil".into())),
        ("impulse_body", ParamValue::S("  ".into())),
        ("fps", ParamValue::F(0.0)),
        ("fps", ParamValue::F(f32::NAN)),
        ("scene", ParamValue::Menu(99)),
        ("timeout_s", ParamValue::I(0)),
        ("python_bin", ParamValue::S(String::new())),
    ];
    for (k, v) in bad {
        let e = cook(&params(&sc.s(), &[(k, v.clone())]), 0.0).expect_err(&format!("{k}={v:?}"));
        assert!(matches!(e, CookError::BadParam(_)), "{k}={v:?}: {e:?}");
    }
    // missing param (empty set) and missing motion_root are errors too
    assert!(matches!(cook(&ParamSet::new(), 0.0), Err(CookError::BadParam(_))));
    let e = cook(&params(&sc.s(), &[("motion_root", ParamValue::S("/nonexistent/root".into()))]), 0.0);
    assert!(matches!(e, Err(CookError::BadParam(m)) if m.contains("motion_root")));
    // zero impulse needs no direction
    let z = params(&sc.s(), &[("impulse_ns", ParamValue::F(0.0)), ("impulse_dir", ParamValue::V3([0.0; 3]))]);
    assert!(cook(&z, 0.0).is_ok());
}

#[test]
fn frame_index_clamps_and_rounds() {
    assert_eq!(frame_index(0.0, 30.0, 120), 0);
    assert_eq!(frame_index(0.5, 30.0, 120), 15);
    assert_eq!(frame_index(1000.0, 30.0, 120), 119);
    assert_eq!(frame_index(-1.0, 30.0, 120), 0);
    assert_eq!(frame_index(f32::NAN, 30.0, 120), 0);
    assert_eq!(frame_index(f32::INFINITY, 30.0, 7), 6);
}

/// Real gaia-motion: skipped (printed) when the venv / MuJoCo assets / checkout are absent.
#[test]
fn integration_real_gaia_motion_push() {
    let home = std::env::var_os("HOME").map(PathBuf::from).expect("HOME");
    let root = home.join("projects/gaia-motion");
    let pythons = [root.join(".venv/bin/python"), home.join(".gaia/cache/gaia-motion/venv/bin/python")];
    let assets = home.join(".gaia/cache/gaia-motion/assets/masked_mimic.onnx");
    let Some(py) = pythons.iter().find(|p| p.is_file()) else {
        eprintln!("SKIP integration_real_gaia_motion_push: no gaia-motion python");
        return;
    };
    let importable = root.is_dir()
        && assets.is_file()
        && Command::new(py)
            .current_dir(&root)
            .args(["-c", "import gaia_motion.cli, mujoco, onnxruntime"])
            .output()
            .is_ok_and(|o| o.status.success());
    if !importable {
        eprintln!("SKIP integration_real_gaia_motion_push: gaia_motion/mujoco/onnx assets not usable");
        return;
    }
    let sc = Scratch::new("real");
    let p = params(
        &sc.s(),
        &[
            ("python_bin", ParamValue::S(py.to_string_lossy().into_owned())),
            ("motion_root", ParamValue::S(root.to_string_lossy().into_owned())),
            ("frames", ParamValue::I(12)),
        ],
    );
    let d = cook(&p, 5.0 / 30.0).expect("real clip cooks");
    assert_eq!(d.p().len(), 24, "SMPL body cloud = 24 points");
    assert!(d.p()[0].iter().all(|c| c.is_finite()));
    // a later frame differs from frame 0 (the body moves)
    let d0 = cook(&p, 0.0).expect("frame 0");
    let d11 = cook(&p, 11.0 / 30.0).expect("frame 11");
    assert_ne!(d0.p()[1], d11.p()[1]);
}
