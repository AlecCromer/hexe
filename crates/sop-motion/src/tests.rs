use super::*;
use hexe_geo::{Detail, PrimKind};

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
    let mode = &spec()[0];
    assert_eq!(mode.name, "mode");
    assert_eq!(mode.default, ParamValue::Menu(0)); // push
    assert_eq!(
        mode.menu.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(),
        ["push", "chain", "behaviour", "hitreact"]
    );
    let scene = spec().into_iter().find(|s| s.name == "scene").expect("scene param");
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
/// Chain-mode params on top of the stub defaults: 2 s at 4 fps = 9 frames, seed 8, base shove 100.
fn chain_params(cache: &str, over: &[(&str, ParamValue)]) -> ParamSet {
    let mut o: Vec<(&str, ParamValue)> = vec![
        ("mode", ParamValue::Menu(1)),
        ("seed", ParamValue::I(8)),
        ("impulse_ns", ParamValue::F(100.0)),
        ("duration_s", ParamValue::F(2.0)),
        ("fps", ParamValue::F(4.0)),
    ];
    o.extend(over.iter().cloned());
    params(cache, &o)
}
fn pelvis_z(d: &Detail) -> f32 {
    match d.attr(AttrClass::Detail, "pelvis_z").map(|a| &a.data) {
        Some(AttrData::F32(v)) => v[0],
        other => panic!("pelvis_z: {other:?}"),
    }
}
#[test]
fn chain_cooks_a_skeleton_per_frame() {
    let sc = Scratch::new("chain");
    let p = chain_params(&sc.s(), &[]);
    let d = cook(&p, 0.5).expect("chain frame"); // 0.5 s * 4 fps = idx 2 -> file 3
    assert_eq!(d.p().len(), 24);
    assert_eq!(d.prim_count(), 23, "23 bones");
    assert_eq!(d.vertex_count(), 46);
    assert_eq!(d.p()[0], [0.0, 0.02, 2.0]);
    assert_eq!(pelvis_z(&d), 0.02);
    assert!(d.attr(AttrClass::Point, "name").is_some());
    // bone 0 = L_Hip(1) -> Pelvis(0), last bone = R_Hand(23) -> R_Wrist(22)
    assert_eq!(d.vertex_point[0..2], [1, 0]);
    assert_eq!(d.vertex_point[44..46], [23, 22]);
    assert!(d.prims.iter().all(|pr| pr.kind == PrimKind::Poly { closed: false }));
    // clamp at the clip end (9 frames -> idx 8) and below 0
    assert_eq!(pelvis_z(&cook(&p, 99.0).expect("hi")), 0.08);
    assert_eq!(pelvis_z(&cook(&p, -3.0).expect("lo")), 0.0);
    // `frame` override (1-based, clamped) beats time
    let f4 = chain_params(&sc.s(), &[("frame", ParamValue::I(4))]);
    assert_eq!(pelvis_z(&cook(&f4, 99.0).expect("frame 4")), 0.03);
    let f99 = chain_params(&sc.s(), &[("frame", ParamValue::I(99))]);
    assert_eq!(pelvis_z(&cook(&f99, 0.0).expect("frame clamp")), 0.08);
}
#[test]
fn chain_passes_the_cli_chain_flags_and_caches_per_hash() {
    let sc = Scratch::new("chainargv");
    let p = chain_params(&sc.s(), &[]);
    cook(&p, 0.0).expect("cooks");
    let cfg = read_cfg(&p).expect("cfg");
    let dir = sc.0.join(param_hash(&cfg));
    let argv = std::fs::read_to_string(dir.join("argv.txt")).expect("argv.txt");
    let mut got: Vec<&str> = argv.lines().collect();
    let oi = got.iter().position(|a| *a == "--out").expect("--out") + 1;
    got[oi] = "<out>";
    assert_eq!(
        got,
        [
            "-m",
            "gaia_motion.cli",
            "chain",
            "--out",
            "<out>",
            "--ns",
            "100",
            "--run-s",
            "2",
            "--fps",
            "4",
            "--seed",
            "8"
        ]
    );
    assert!(dir.join(".ok").is_file() && dir.join("chain.json").is_file());
    // seed -1 -> no --seed flag
    let ns = chain_params(&sc.s(), &[("seed", ParamValue::I(-1))]);
    cook(&ns, 0.0).expect("no seed");
    let a2 =
        std::fs::read_to_string(sc.0.join(param_hash(&read_cfg(&ns).expect("cfg"))).join("argv.txt")).expect("argv");
    assert!(!a2.contains("--seed"), "{a2}");
    // python gone: same params are a cache hit; `frame` / python / timeout do not change the hash
    let gone = chain_params(
        &sc.s(),
        &[("python_bin", ParamValue::S("/nonexistent/python".into())), ("frame", ParamValue::I(2))],
    );
    assert_eq!(pelvis_z(&cook(&gone, 0.0).expect("cache hit")), 0.01);
    let h = |o: &[(&str, ParamValue)]| param_hash(&read_cfg(&chain_params(&sc.s(), o)).expect("cfg"));
    let base = h(&[]);
    assert_eq!(base.len(), 32);
    assert_eq!(base, h(&[("timeout_s", ParamValue::I(7)), ("frame", ParamValue::I(3)), ("frames", ParamValue::I(9))]));
    for o in [
        [("seed", ParamValue::I(9))],
        [("impulse_ns", ParamValue::F(101.0))],
        [("duration_s", ParamValue::F(3.0))],
        [("fps", ParamValue::F(5.0))],
    ] {
        assert_ne!(base, h(&o), "{o:?}");
    }
    // different from the push clip of the same params
    assert_ne!(base, param_hash(&read_cfg(&params(&sc.s(), &[])).expect("push")));
}
#[test]
fn hitreact_mode_passes_the_hitreact_flags_cooks_a_skeleton_and_has_its_own_hash() {
    let sc = Scratch::new("hitreact");
    let over = [("mode", ParamValue::Menu(3))];
    let p = chain_params(&sc.s(), &over);
    let d = cook(&p, 0.5).expect("hitreact frame");
    assert_eq!((d.p().len(), d.prim_count(), d.vertex_count()), (24, 23, 46));
    assert_eq!(pelvis_z(&d), 0.02);
    let cfg = read_cfg(&p).expect("cfg");
    assert!(cfg.hitreact && !cfg.chain);
    let argv = std::fs::read_to_string(sc.0.join(param_hash(&cfg)).join("argv.txt")).expect("argv.txt");
    let got: Vec<&str> = argv.lines().collect();
    assert_eq!(&got[0..3], ["-m", "gaia_motion.cli", "hitreact"]);
    assert!(got.windows(2).any(|w| w == ["--seed", "8"]) && got.windows(2).any(|w| w == ["--ns", "100"]));
    assert_ne!(param_hash(&cfg), param_hash(&read_cfg(&chain_params(&sc.s(), &[])).expect("chain")));
    let f = chain_params(&sc.s(), &[("mode", ParamValue::Menu(3)), ("frame", ParamValue::I(4))]);
    assert_eq!(pelvis_z(&cook(&f, 99.0).expect("frame 4")), 0.03);
    let bad = chain_params(&sc.s(), &[("mode", ParamValue::Menu(4))]);
    assert!(matches!(cook(&bad, 0.0), Err(CookError::BadParam(_))));
}
/// Real hit-reaction run (gaia-motion `hitreact --seed 12`, ~3 s); skipped when the checkout has no `hitreact` yet (`GAIA_MOTION_HITREACT_ROOT` overrides the worktree path).
#[test]
fn integration_real_hitreact_cooks_a_moving_skeleton() {
    let home = std::env::var_os("HOME").map(PathBuf::from).expect("HOME");
    let root = std::env::var_os("GAIA_MOTION_HITREACT_ROOT")
        .map_or_else(|| home.join("projects/gaia-motion-wt/endo"), PathBuf::from);
    let py = home.join(".gaia/cache/gaia-motion/venv/bin/python");
    let assets = home.join(".gaia/cache/gaia-motion/assets/masked_mimic.onnx");
    let usable = root.join("gaia_motion/hitreact.py").is_file()
        && root.join("models/masked_mimic_smpl/masked_mimic.onnx").is_file()
        && assets.is_file()
        && py.is_file();
    if !usable {
        eprintln!("SKIP integration_real_hitreact: no gaia-motion checkout with hitreact + venv + assets");
        return;
    }
    let sc = Scratch::new("realhit");
    let over = [
        ("mode", ParamValue::Menu(3)),
        ("python_bin", ParamValue::S(py.to_string_lossy().into_owned())),
        ("motion_root", ParamValue::S(root.to_string_lossy().into_owned())),
        ("seed", ParamValue::I(12)),
        ("duration_s", ParamValue::F(3.0)),
        ("fps", ParamValue::F(30.0)),
    ];
    let at = |f: i32| {
        let mut o = over.to_vec();
        o.push(("frame", ParamValue::I(f)));
        cook(&chain_params(&sc.s(), &o), 0.0).expect("real hitreact cooks")
    };
    let (d1, d30) = (at(1), at(30));
    assert_eq!((d30.p().len(), d30.prim_count()), (24, 23));
    assert!(d30.p().iter().flatten().all(|c| c.is_finite()));
    assert!((pelvis_z(&d1) - 0.95).abs() < 0.02, "standing at t=0: {}", pelvis_z(&d1));
    assert_ne!(d1.p()[1], d30.p()[1], "the body moves between frame 1 and 30");
    eprintln!("hitreact frame 30: pelvis_z = {}", pelvis_z(&d30));
}
#[test]
fn chain_failure_surfaces_stderr_and_hostile_chain_params_are_bad_param() {
    let sc = Scratch::new("chainfail");
    let e = cook(&chain_params(&sc.s(), &[("impulse_ns", ParamValue::F(13.0))]), 0.0).expect_err("stub exits 4");
    let CookError::Failed(m) = &e else { panic!("{e:?}") };
    assert!(m.contains("exited 4") && m.contains("stub: chain boom (ns 13)"), "{m}");
    assert_eq!(std::fs::read_dir(&sc.0).expect("ls").count(), 0, "failed run must leave no cache entry");
    let bad: Vec<(&str, ParamValue)> = vec![
        ("mode", ParamValue::Menu(4)),
        ("seed", ParamValue::I(-2)),
        ("seed", ParamValue::I(i32::MAX)),
        ("duration_s", ParamValue::F(f32::NAN)),
        ("duration_s", ParamValue::F(0.5)),
        ("duration_s", ParamValue::F(1.0e9)),
        ("frame", ParamValue::I(-1)),
        ("frame", ParamValue::I(i32::MAX)),
    ];
    for (k, v) in bad {
        let e = cook(&chain_params(&sc.s(), &[(k, v.clone())]), 0.0).expect_err(&format!("{k}={v:?}"));
        assert!(matches!(e, CookError::BadParam(_)), "{k}={v:?}: {e:?}");
    }
}
#[test]
fn default_python_falls_back_to_the_cached_venv_only_when_missing() {
    let sc = Scratch::new("pyfallback");
    let mut p = params(&sc.s(), &[]);
    p = p.with("python_bin", ParamValue::S(DEFAULT_PYTHON.into()));
    let cfg = read_cfg(&p).expect("cfg");
    let want = if expand(DEFAULT_PYTHON).is_file() { expand(DEFAULT_PYTHON) } else { expand(FALLBACK_PYTHON) };
    assert_eq!(cfg.python, want);
    let custom = params(&sc.s(), &[("python_bin", ParamValue::S("/nonexistent/python".into()))]);
    assert_eq!(read_cfg(&custom).expect("cfg").python, PathBuf::from("/nonexistent/python"));
}
/// Real chain run (gaia-motion `chain --seed 8`, ~5 s) vs the sealed Houdini-chain .geo frames; skipped when gaia-motion is absent.
/// Committed reference frames: 1, 20, 39, 58, 77, 96, 115, ... (every 19th); frame 90 is checked too when present on disk (gitignored).
#[test]
fn parity_real_chain_matches_sealed_houdini_chain_geo() {
    let home = std::env::var_os("HOME").map(PathBuf::from).expect("HOME");
    let root = home.join("projects/gaia-motion");
    let refs = root.join("proof/houdini-chain/geo");
    let pythons = [root.join(".venv/bin/python"), home.join(".gaia/cache/gaia-motion/venv/bin/python")];
    let assets = home.join(".gaia/cache/gaia-motion/assets/masked_mimic.onnx");
    let Some(py) = pythons.iter().find(|p| p.is_file()) else {
        eprintln!("SKIP parity_real_chain: no gaia-motion python");
        return;
    };
    let importable = refs.join("frame.0096.geo").is_file()
        && assets.is_file()
        && Command::new(py)
            .current_dir(&root)
            .args(["-c", "import gaia_motion.cli, mujoco, onnxruntime"])
            .output()
            .is_ok_and(|o| o.status.success());
    if !importable {
        eprintln!("SKIP parity_real_chain: gaia_motion/mujoco/onnx/reference frames not usable");
        return;
    }
    let sc = Scratch::new("realchain");
    let over = [
        ("python_bin", ParamValue::S(py.to_string_lossy().into_owned())),
        ("motion_root", ParamValue::S(root.to_string_lossy().into_owned())),
    ];
    let mut worst = 0.0_f32;
    for frame in [1, 90, 96, 190, 200, 360] {
        let reference = refs.join(format!("frame.{frame:04}.geo"));
        if !reference.is_file() {
            eprintln!("parity: frame {frame} reference not on disk, skipped");
            continue;
        }
        let want = hexe_geo::read_any(&reference).expect("reference .geo");
        let d = cook(
            &chain_params(
                &sc.s(),
                &[
                    over.clone().to_vec(),
                    vec![
                        ("duration_s", ParamValue::F(12.0)),
                        ("fps", ParamValue::F(30.0)),
                        ("frame", ParamValue::I(frame)),
                    ],
                ]
                .concat(),
            ),
            0.0,
        )
        .expect("real chain cooks");
        assert_eq!(d.p().len(), 24);
        assert_eq!(d.prim_count(), 23);
        let diff = d
            .p()
            .iter()
            .zip(want.p())
            .flat_map(|(a, b)| (0..3).map(move |k| (a[k] - b[k]).abs()))
            .fold(0.0_f32, f32::max);
        eprintln!("parity frame {frame}: max |dP| = {diff:e}, pelvis_z = {}", pelvis_z(&d));
        assert!(diff <= 1.0e-4, "frame {frame}: max |dP| {diff}");
        assert_eq!(pelvis_z(&d), want.p()[0][1]);
        worst = worst.max(diff);
    }
    eprintln!("parity worst max |dP| = {worst:e}");
}

/// A fake gaia-motion checkout: `<scratch>/root/behaviours/{stagger,balance}.json` (+ a data table and a non-json file the listing must skip).
fn fake_root(sc: &Scratch) -> PathBuf {
    let b = sc.0.join("root/behaviours");
    std::fs::create_dir_all(&b).expect("behaviours dir");
    std::fs::write(b.join("stagger.json"), r#"{"name":"stagger","impulses":[]}"#).expect("w");
    std::fs::write(b.join("balance.json"), r#"{"name":"balance"}"#).expect("w");
    std::fs::write(b.join("table.json"), r#"{"schema":1,"dofs":[]}"#).expect("w");
    std::fs::write(b.join("notes.txt"), "name").expect("w");
    sc.0.join("root")
}
fn behaviour_params(sc: &Scratch, root: &Path, over: &[(&str, ParamValue)]) -> ParamSet {
    let cache = sc.0.join("cache");
    let mut o: Vec<(&str, ParamValue)> = vec![
        ("mode", ParamValue::Menu(2)),
        ("behaviour", ParamValue::S("stagger".into())),
        ("seed", ParamValue::I(8)),
        ("duration_s", ParamValue::F(2.0)),
        ("fps", ParamValue::F(4.0)),
        ("motion_root", ParamValue::S(root.to_string_lossy().into_owned())),
    ];
    o.extend(over.iter().cloned());
    params(&cache.to_string_lossy(), &o)
}
#[test]
fn behaviour_listing_is_the_json_objects_with_a_name_and_is_cached_per_dir_mtime() {
    let sc = Scratch::new("beh-list");
    let root = fake_root(&sc);
    assert_eq!(behaviours(&root), ["balance", "stagger"]);
    assert_eq!(behaviours(&sc.0.join("nonexistent")), Vec::<String>::new());
    // cached: an in-place edit of an existing file is not re-read (dir mtime unchanged) ...
    let b = root.join("behaviours");
    let before = std::fs::metadata(&b).and_then(|m| m.modified()).expect("mtime");
    std::fs::write(b.join("stagger.json"), r#"{"nameless":1}"#).expect("w");
    if std::fs::metadata(&b).and_then(|m| m.modified()).expect("mtime") == before {
        assert_eq!(behaviours(&root), ["balance", "stagger"], "served from cache");
    }
    // ... but a new file changes the dir mtime and is picked up
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(b.join("zeta.json"), r#"{"name":"zeta"}"#).expect("w");
    let names = behaviours(&root);
    assert!(names.contains(&"zeta".to_string()) && names.contains(&"balance".to_string()), "{names:?}");
}
#[test]
fn behaviour_param_is_a_dropdown_over_the_real_gaia_motion_directory() {
    let spec = spec();
    let b = spec.iter().find(|s| s.name == "behaviour").expect("behaviour param");
    assert_eq!(b.choices, behaviours(&expand(DEFAULT_ROOT)));
    assert!(matches!(&b.default, ParamValue::S(_)) && b.default.is_acceptable());
    for n in ["seed", "fps", "duration_s", "frame"] {
        assert!(spec.iter().any(|s| s.name == n), "{n}");
    }
    if expand(DEFAULT_ROOT).join("behaviours/stagger.json").is_file() {
        for want in ["balance", "catch-fall", "stagger", "get-up", "protect-head"] {
            assert!(b.choices.iter().any(|c| c == want), "{want} in {:?}", b.choices);
        }
        assert!(!b.choices.iter().any(|c| c == "smpl_pd_params"), "data table is not a behaviour");
        assert_eq!(b.default, ParamValue::S("balance".into()));
    }
}
#[test]
fn behaviour_mode_cooks_a_skeleton_passes_flags_and_caches_per_preset_seed() {
    let sc = Scratch::new("beh-cook");
    let root = fake_root(&sc);
    let p = behaviour_params(&sc, &root, &[]);
    let d = cook(&p, 0.5).expect("behaviour frame"); // 0.5 s * 4 fps = idx 2 -> file 3
    assert_eq!((d.p().len(), d.prim_count()), (24, 23));
    assert_eq!(pelvis_z(&d), 0.02);
    let cfg = read_cfg(&p).expect("cfg");
    let dir = sc.0.join("cache").join(param_hash(&cfg));
    let argv = std::fs::read_to_string(dir.join("argv.txt")).expect("argv.txt");
    let mut got: Vec<&str> = argv.lines().collect();
    let oi = got.iter().position(|a| *a == "--out").expect("--out") + 1;
    got[oi] = "<out>";
    assert_eq!(
        got,
        [
            "-m",
            "gaia_motion.cli",
            "behaviour",
            "--name",
            "stagger",
            "--out",
            "<out>",
            "--run-s",
            "2",
            "--fps",
            "4",
            "--seed",
            "8"
        ]
    );
    let h = |o: &[(&str, ParamValue)]| param_hash(&read_cfg(&behaviour_params(&sc, &root, o)).expect("cfg"));
    let base = h(&[]);
    assert_eq!(
        base,
        h(&[("frame", ParamValue::I(3)), ("timeout_s", ParamValue::I(9)), ("impulse_ns", ParamValue::F(5.0))])
    );
    for o in [
        [("behaviour", ParamValue::S("balance".into()))],
        [("seed", ParamValue::I(9))],
        [("duration_s", ParamValue::F(3.0))],
        [("fps", ParamValue::F(5.0))],
    ] {
        assert_ne!(base, h(&o), "{o:?}");
    }
    // editing the preset file (size/mtime) is a new clip
    std::fs::write(root.join("behaviours/stagger.json"), r#"{"name":"stagger","impulses":[1]}"#).expect("w");
    assert_ne!(base, h(&[]), "preset edit re-runs the clip");
    // seed -1 -> no --seed
    let ns = behaviour_params(&sc, &root, &[("seed", ParamValue::I(-1))]);
    cook(&ns, 0.0).expect("no seed");
    let a2 =
        std::fs::read_to_string(sc.0.join("cache").join(param_hash(&read_cfg(&ns).expect("cfg"))).join("argv.txt"))
            .expect("argv");
    assert!(!a2.contains("--seed"), "{a2}");
}
#[test]
fn behaviour_scrubs_with_the_timeline_and_the_frame_override_wins() {
    let sc = Scratch::new("beh-scrub");
    let root = fake_root(&sc);
    let p = behaviour_params(&sc, &root, &[]); // 2 s at 4 fps = 9 frames, pelvis_z = (frame-1)/100
    // hexe timeline: $F -> time = (F - 1) / timeline_fps; here timeline fps == clip fps (4)
    for (f, want) in [(1, 0.0), (3, 0.02), (9, 0.08), (40, 0.08)] {
        let t = (f as f32 - 1.0) / 4.0;
        assert_eq!(pelvis_z(&cook(&p, t).expect("scrub")), want, "F{f}");
    }
    let over = behaviour_params(&sc, &root, &[("frame", ParamValue::I(4))]);
    assert_eq!(pelvis_z(&cook(&over, 0.0).expect("override")), 0.03);
}
#[test]
fn behaviour_errors_are_bad_param_or_surface_stderr() {
    let sc = Scratch::new("beh-bad");
    let root = fake_root(&sc);
    let e =
        cook(&behaviour_params(&sc, &root, &[("behaviour", ParamValue::S("nope".into()))]), 0.0).expect_err("unknown");
    let CookError::BadParam(m) = &e else { panic!("{e:?}") };
    assert!(m.contains("nope") && m.contains("balance, stagger"), "{m}");
    for bad in ["", "../x", "table", "notes"] {
        let e = cook(&behaviour_params(&sc, &root, &[("behaviour", ParamValue::S(bad.into()))]), 0.0).expect_err(bad);
        assert!(matches!(e, CookError::BadParam(_)), "{bad:?}: {e:?}");
    }
    std::fs::write(root.join("behaviours/FAIL.json"), r#"{"name":"FAIL"}"#).expect("w");
    let e = cook(&behaviour_params(&sc, &root, &[("behaviour", ParamValue::S("FAIL".into()))]), 0.0)
        .expect_err("stub exits 5");
    let CookError::Failed(m) = &e else { panic!("{e:?}") };
    assert!(m.contains("exited 5") && m.contains("behaviour boom"), "{m}");
}
/// Real `behaviour` runs (MuJoCo, ~3 s each) against a gaia-motion checkout that has the `behaviour` subcommand (`HEXE_MOTION_ROOT`, else
/// `~/projects/gaia-motion-wt/contact`, else `~/projects/gaia-motion`); skipped when absent. Two behaviours cook to different, finite poses.
#[test]
fn integration_real_behaviours_differ() {
    let home = std::env::var_os("HOME").map(PathBuf::from).expect("HOME");
    let candidates = [
        std::env::var_os("HEXE_MOTION_ROOT").map(PathBuf::from).unwrap_or_default(),
        home.join("projects/gaia-motion-wt/contact"),
        home.join("projects/gaia-motion"),
    ];
    let py = home.join(".gaia/cache/gaia-motion/venv/bin/python");
    let Some(root) = candidates.iter().find(|r| r.join("gaia_motion/behaviour.py").is_file()) else {
        eprintln!("SKIP integration_real_behaviours: no gaia-motion checkout with the `behaviour` subcommand");
        return;
    };
    if !py.is_file() || !home.join(".gaia/cache/gaia-motion/assets/masked_mimic.onnx").is_file() {
        eprintln!("SKIP integration_real_behaviours: no gaia-motion python / onnx");
        return;
    }
    let sc = Scratch::new("beh-real");
    let mut poses = Vec::new();
    for name in ["balance-ankles", "catch-fall"] {
        let p = behaviour_params(
            &sc,
            root,
            &[
                ("behaviour", ParamValue::S(name.into())),
                ("seed", ParamValue::I(-1)),
                ("duration_s", ParamValue::F(3.0)),
                ("fps", ParamValue::F(30.0)),
                ("python_bin", ParamValue::S(py.to_string_lossy().into_owned())),
            ],
        );
        let d = cook(&p, 89.0 / 30.0).expect("real behaviour cook");
        assert_eq!((d.p().len(), d.prim_count()), (24, 23), "{name}");
        assert!(d.p().iter().all(|q| q.iter().all(|c| c.is_finite())), "{name}");
        poses.push((pelvis_z(&d), d.p().to_vec()));
    }
    assert_ne!(poses[0].1, poses[1].1, "balance-ankles vs catch-fall at F90 must differ");
    eprintln!("real behaviours at F90: balance-ankles pelvis_y {} | catch-fall pelvis_y {}", poses[0].0, poses[1].0);
}
