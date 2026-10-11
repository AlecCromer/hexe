//! `motion` SOP (hexe-native, no Houdini counterpart): GAIA Motion as a node.
//! Cook = run `python -m gaia_motion.cli run …` once per distinct parameter set into
//! `<cache>/<hash>/geo/frame.%04d.geo` (default cache `~/.gaia/cache/hexe/motion`), then read the `.geo` frame for the
//! current time (`round(time * fps)`, clamped to the clip) as the output `Detail`. Python failures surface their stderr.
//! Flags are the real ones of `~/projects/gaia-motion/gaia_motion/cli.py` (lines 20-38); output layout `export.py:48-54`.
//! `mode = chain`: the Endorphin chain (shove -> protect reflex -> limp -> settle -> get-up, ONE MuJoCo run) via
//! `python -m gaia_motion.cli chain` -> `<cache>/<hash>/{geo/frame.%04d.geo, chain.npz (qpos), chain.json}`; the frame is cooked as a
//! skeleton: 24 joint points (`P`, `name`), 23 bone polylines, detail attr `pelvis_z` (m, = `P.y` of the pelvis, Y-up geo).
//! `mode = behaviour`: one `~/projects/gaia-motion/behaviours/<name>.json` preset (the `behaviour` string param, dropdown = the directory listing,
//! read once per directory mtime and cached) run in MuJoCo via `python -m gaia_motion.cli behaviour --name N --run-s D --fps F [--seed S]`,
//! cooked as the same skeleton. Scrub: the node follows the timeline (`ctx.time` = `($F - 1) / fps`) unless `frame` > 0 overrides it.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

/// Scene tokens, menu order. Mirror of `SCENES` in `gaia_motion/scenes.py:57`.
pub const SCENES: [&str; 4] = ["stand", "push", "leg_hit", "fall_from_height"];
/// Hard cap on `frames` (clip size / run time bound).
pub const MAX_FRAMES: i32 = 100_000;
const DEFAULT_PYTHON: &str = "~/projects/gaia-motion/.venv/bin/python";
/// Used when the default `python_bin` does not exist (the cached venv the gaia-motion setup creates).
const FALLBACK_PYTHON: &str = "~/.gaia/cache/gaia-motion/venv/bin/python";
/// Bones of the 24-point SMPL body cloud as `(child, parent)` joint names; mirror of `PAR` in `gaia-motion/scripts/houdini_chain.py`.
pub const BONES: [(&str, &str); 23] = [
    ("L_Hip", "Pelvis"),
    ("L_Knee", "L_Hip"),
    ("L_Ankle", "L_Knee"),
    ("L_Toe", "L_Ankle"),
    ("R_Hip", "Pelvis"),
    ("R_Knee", "R_Hip"),
    ("R_Ankle", "R_Knee"),
    ("R_Toe", "R_Ankle"),
    ("Torso", "Pelvis"),
    ("Spine", "Torso"),
    ("Chest", "Spine"),
    ("Neck", "Chest"),
    ("Head", "Neck"),
    ("L_Thorax", "Chest"),
    ("L_Shoulder", "L_Thorax"),
    ("L_Elbow", "L_Shoulder"),
    ("L_Wrist", "L_Elbow"),
    ("L_Hand", "L_Wrist"),
    ("R_Thorax", "Chest"),
    ("R_Shoulder", "R_Thorax"),
    ("R_Elbow", "R_Shoulder"),
    ("R_Wrist", "R_Elbow"),
    ("R_Hand", "R_Wrist"),
];
const DEFAULT_ROOT: &str = "~/projects/gaia-motion";
/// `behaviour` param default when the directory has no `balance.json` (or cannot be read).
const FALLBACK_BEHAVIOUR: &str = "balance";
type NamesCache = Mutex<HashMap<PathBuf, (Option<SystemTime>, Vec<String>)>>;
/// Behaviour names of a gaia-motion checkout: sorted stems of `<root>/behaviours/*.json` that are JSON objects with a string `name`
/// (data tables such as `smpl_pd_params.json` have none). Cached per directory, re-read only when the directory's mtime changes.
pub fn behaviours(root: &Path) -> Vec<String> {
    static CACHE: OnceLock<NamesCache> = OnceLock::new();
    let dir = root.join("behaviours");
    let stamp = std::fs::metadata(&dir).and_then(|m| m.modified()).ok();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, names)) = g.get(&dir).filter(|(st, _)| *st == stamp) {
        return names.clone();
    }
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter(|p| {
            std::fs::read_to_string(p)
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .is_some_and(|v| v.get("name").is_some_and(serde_json::Value::is_string))
        })
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .collect();
    names.sort();
    g.insert(dir, (stamp, names.clone()));
    names
}
/// `size + mtime` of a preset file: part of the cache key so editing `behaviours/<name>.json` re-runs the clip.
fn preset_stamp(root: &Path, name: &str) -> String {
    std::fs::metadata(root.join("behaviours").join(format!("{name}.json"))).map_or_else(
        |_| String::new(),
        |m| {
            let t = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            format!("{}:{t}", m.len())
        },
    )
}
const STDERR_TAIL: usize = 2000;

/// Registry token of the node.
pub const TYPE_NAME: &str = "motion";

pub fn spec() -> Vec<ParamSpec> {
    let names = behaviours(&expand(DEFAULT_ROOT));
    let choices: Vec<&str> = names.iter().map(String::as_str).collect();
    vec![
        ParamSpec::new("mode", ParamValue::Menu(0)).label("Mode").menu(&[
            ("push", "Push (scene)"),
            ("chain", "Endorphin Chain"),
            ("behaviour", "Behaviour (preset)"),
        ]),
        ParamSpec::new("scene", ParamValue::Menu(1)).label("Scene").menu(&[
            ("stand", "Stand"),
            ("push", "Push"),
            ("leg_hit", "Leg Hit"),
            ("fall_from_height", "Fall From Height"),
        ]),
        ParamSpec::new(
            "behaviour",
            ParamValue::S(if names.iter().any(|n| n == FALLBACK_BEHAVIOUR) {
                FALLBACK_BEHAVIOUR.into()
            } else {
                names.first().cloned().unwrap_or_else(|| FALLBACK_BEHAVIOUR.into())
            }),
        )
        .label("Behaviour (behaviours/*.json)")
        .choices(&choices),
        ParamSpec::new("impulse_ns", ParamValue::F(90.0)).label("Impulse (N*s; chain: base shove)").range(0.0, 300.0),
        ParamSpec::new("impulse_dir", ParamValue::V3([1.0, 0.0, 0.0])).label("Impulse Direction (world Z-up)"),
        ParamSpec::new("impulse_body", ParamValue::S("Chest".into())).label("Impulse Body"),
        ParamSpec::new("seed", ParamValue::I(-1))
            .label("Chain Shove Seed (-1 = none: +X at 0.5 s)")
            .range(-1.0, 1_000_000.0),
        ParamSpec::new("duration_s", ParamValue::F(12.0)).label("Chain Duration (s)").range(1.0, 60.0),
        ParamSpec::new("frame", ParamValue::I(0))
            .label("Frame Override (0 = follow timeline $F; else 1-based clip frame)")
            .range(0.0, MAX_FRAMES as f32),
        ParamSpec::new("frames", ParamValue::I(120)).label("Frames (push)").range(1.0, 600.0),
        ParamSpec::new("fps", ParamValue::F(30.0)).label("Clip FPS").range(1.0, 120.0),
        ParamSpec::new("python_bin", ParamValue::S(DEFAULT_PYTHON.into())).label("Python"),
        ParamSpec::new("motion_root", ParamValue::S(DEFAULT_ROOT.into())).label("gaia-motion Checkout"),
        ParamSpec::new("cache_dir", ParamValue::S(String::new())).label("Cache Dir (empty = default)"),
        ParamSpec::new("timeout_s", ParamValue::I(900)).label("Timeout (s)").range(1.0, 3600.0),
    ]
}

pub fn new() -> Box<dyn Node<Detail>> {
    Box::new(Motion { specs: spec() })
}

struct Motion {
    specs: Vec<ParamSpec>,
}

/// Validated parameters.
#[derive(Debug, Clone, PartialEq)]
struct Cfg {
    chain: bool,
    /// `mode = behaviour`: preset stem + `size:mtime` of its file (cache key).
    behaviour: Option<(String, String)>,
    ns: f32,
    seed: i32,
    duration: f32,
    frame: i32,
    scene: &'static str,
    impulse: [f32; 3],
    body: String,
    frames: i32,
    fps: f32,
    python: PathBuf,
    root: PathBuf,
    cache: PathBuf,
    timeout: Duration,
}

fn bad<T>(m: impl Into<String>) -> Result<T, CookError> {
    Err(CookError::BadParam(m.into()))
}

/// `~` / `~/x` -> `$HOME/x`; anything else verbatim.
fn expand(s: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match (s.strip_prefix("~/"), s == "~", home) {
        (Some(rest), _, Some(h)) => h.join(rest),
        (_, true, Some(h)) => h,
        _ => PathBuf::from(s),
    }
}

fn string<'a>(p: &'a ParamSet, name: &str) -> Result<&'a str, CookError> {
    p.string(name).map_or_else(|| bad(format!("missing string param `{name}`")), Ok)
}

fn read_cfg(p: &ParamSet) -> Result<Cfg, CookError> {
    let Some(mode_i) = p.menu("mode") else { return bad("missing menu param `mode`") };
    if mode_i > 2 {
        return bad(format!("mode menu index {mode_i} out of range (0..3)"));
    }
    let Some(scene_i) = p.menu("scene") else { return bad("missing menu param `scene`") };
    let Some(scene) = SCENES.get(scene_i as usize).copied() else {
        return bad(format!("scene menu index {scene_i} out of range (0..{})", SCENES.len()));
    };
    let Some(ns) = p.float("impulse_ns") else { return bad("missing `impulse_ns`") };
    let Some(dir) = p.vec3("impulse_dir") else { return bad("missing `impulse_dir`") };
    if !ns.is_finite() || ns.abs() > 1.0e6 {
        return bad(format!("impulse_ns {ns} must be finite and |x| <= 1e6"));
    }
    let len = dir.iter().map(|c| f64::from(*c) * f64::from(*c)).sum::<f64>().sqrt();
    if !dir.iter().all(|c| c.is_finite()) || (ns != 0.0 && len < 1.0e-9) {
        return bad("impulse_dir must be finite and non-zero when impulse_ns != 0");
    }
    let k = if ns == 0.0 { 0.0 } else { f64::from(ns) / len };
    let impulse = [(f64::from(dir[0]) * k) as f32, (f64::from(dir[1]) * k) as f32, (f64::from(dir[2]) * k) as f32];
    let body = string(p, "impulse_body")?.trim().to_string();
    if body.is_empty() || body.starts_with('-') || body.contains('\0') {
        return bad(format!("impulse_body {body:?} must be a body name (non-empty, not starting with '-')"));
    }
    let Some(seed) = p.int("seed") else { return bad("missing `seed`") };
    if !(-1..=1_000_000).contains(&seed) {
        return bad(format!("seed {seed} outside -1..=1000000"));
    }
    let Some(duration) = p.float("duration_s") else { return bad("missing `duration_s`") };
    if !duration.is_finite() || !(1.0..=60.0).contains(&duration) {
        return bad(format!("duration_s {duration} must be in 1..=60"));
    }
    let Some(frame) = p.int("frame") else { return bad("missing `frame`") };
    if !(0..=MAX_FRAMES).contains(&frame) {
        return bad(format!("frame {frame} outside 0..={MAX_FRAMES}"));
    }
    let Some(frames) = p.int("frames") else { return bad("missing `frames`") };
    if !(1..=MAX_FRAMES).contains(&frames) {
        return bad(format!("frames {frames} outside 1..={MAX_FRAMES}"));
    }
    let Some(fps) = p.float("fps") else { return bad("missing `fps`") };
    if !fps.is_finite() || fps <= 0.0 || fps > 1000.0 {
        return bad(format!("fps {fps} must be in (0, 1000]"));
    }
    let python = string(p, "python_bin")?.trim();
    let root = string(p, "motion_root")?.trim();
    if python.is_empty() || root.is_empty() {
        return bad("python_bin and motion_root must not be empty");
    }
    let cache = match string(p, "cache_dir")?.trim() {
        "" => expand("~/.gaia/cache/hexe/motion"),
        c => expand(c),
    };
    let Some(t) = p.int("timeout_s") else { return bad("missing `timeout_s`") };
    if !(1..=86_400).contains(&t) {
        return bad(format!("timeout_s {t} outside 1..=86400"));
    }
    let mut python = expand(python);
    if python == expand(DEFAULT_PYTHON) && !python.is_file() {
        python = expand(FALLBACK_PYTHON);
    }
    let root = expand(root);
    let behaviour = if mode_i == 2 {
        let name = p.string("behaviour").unwrap_or(FALLBACK_BEHAVIOUR).trim().to_string();
        let known = behaviours(&root);
        if !known.contains(&name) {
            return bad(format!(
                "behaviour {name:?} is not in {}/behaviours (have: {})",
                root.display(),
                known.join(", ")
            ));
        }
        let stamp = preset_stamp(&root, &name);
        Some((name, stamp))
    } else {
        None
    };
    Ok(Cfg {
        chain: mode_i == 1,
        behaviour,
        ns,
        seed,
        duration,
        frame,
        scene,
        impulse,
        body,
        frames,
        fps,
        python,
        root,
        cache,
        timeout: Duration::from_secs(t.unsigned_abs().into()),
    })
}

fn fnv64(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64 ^ seed;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Cache key: 32 hex chars over everything that changes the clip. `python_bin` / `motion_root` / `fps` / `timeout` /
/// `cache_dir` are location or playback choices, not simulation inputs, so they are NOT part of it.
fn param_hash(c: &Cfg) -> String {
    // chain: the sim inputs are base shove, seed, duration; `fps` too because the .geo sequence is resampled at it
    let key = if let Some((name, stamp)) = &c.behaviour {
        // sim inputs: preset (name + file stamp), seed, duration; `fps` too because the .geo sequence is resampled at it
        format!("behaviour-v1|{name}|{stamp}|{}|{:08x}|{:08x}", c.seed, c.duration.to_bits(), c.fps.to_bits())
    } else if c.chain {
        format!("chain-v1|{:08x}|{}|{:08x}|{:08x}", c.ns.to_bits(), c.seed, c.duration.to_bits(), c.fps.to_bits())
    } else {
        push_key(c)
    };
    format!("{:016x}{:016x}", fnv64(0, key.as_bytes()), fnv64(0x9e37_79b9_7f4a_7c15, key.as_bytes()))
}
fn push_key(c: &Cfg) -> String {
    format!(
        "v1|{}|{:08x},{:08x},{:08x}|{}|{}",
        c.scene,
        c.impulse[0].to_bits(),
        c.impulse[1].to_bits(),
        c.impulse[2].to_bits(),
        c.body,
        c.frames
    )
}

/// Number of `frame.NNNN.geo` files of a finished clip (frames are contiguous from 1).
fn count_frames(geo_dir: &Path) -> Result<i32, CookError> {
    let rd = std::fs::read_dir(geo_dir).map_err(|e| CookError::Failed(format!("ls {}: {e}", geo_dir.display())))?;
    let n = rd
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("frame.") && n.ends_with(".geo"))
        .count();
    i32::try_from(n)
        .ok()
        .filter(|n| *n > 0)
        .map_or_else(|| Err(CookError::Failed(format!("{}: no frames", geo_dir.display()))), Ok)
}
/// 24-point body cloud -> skeleton: bones (`BONES`, by joint `name`) as 2-point open polylines + detail `pelvis_z` (= `P.y` of Pelvis).
fn skeleton(mut d: Detail) -> Result<Detail, CookError> {
    let names: Vec<String> = match d.attr(AttrClass::Point, "name").map(|a| &a.data) {
        Some(AttrData::Str { idx, table }) => idx
            .iter()
            .map(|i| usize::try_from(*i).ok().and_then(|i| table.get(i)).cloned().unwrap_or_default())
            .collect(),
        _ => return Err(CookError::Failed("chain frame has no string point attribute `name`".into())),
    };
    let find = |n: &str| -> Result<u32, CookError> {
        names
            .iter()
            .position(|x| x == n)
            .and_then(|i| u32::try_from(i).ok())
            .map_or_else(|| Err(CookError::Failed(format!("chain frame has no joint `{n}`"))), Ok)
    };
    for (child, parent) in BONES {
        d.add_poly(&[find(child)?, find(parent)?], false);
    }
    let pelvis = usize::try_from(find("Pelvis")?).unwrap_or(0);
    let Some(p) = d.p().get(pelvis).copied() else {
        return Err(CookError::Failed("chain frame has no P for Pelvis".into()));
    };
    d.add_attr(Attribute {
        name: "pelvis_z".into(),
        class: AttrClass::Detail,
        data: AttrData::F32(vec![p[1]]),
        type_info: TypeInfo::None,
    });
    Ok(d)
}
fn frame_path(geo_dir: &Path, one_based: i64) -> PathBuf {
    geo_dir.join(format!("frame.{one_based:04}.geo"))
}

/// Frame index (0-based) for `time` seconds at `fps`, clamped to `0..frames`.
fn frame_index(time: f32, fps: f32, frames: i32) -> i64 {
    let f = (f64::from(time) * f64::from(fps)).round();
    let last = i64::from(frames) - 1;
    if f.is_nan() { 0 } else { (f.clamp(0.0, last as f64)) as i64 }
}

fn tail(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_default();
    let start = bytes.len().saturating_sub(STDERR_TAIL);
    String::from_utf8_lossy(&bytes[start..]).trim().to_string()
}

/// Make sure `<cache>/<hash>/` holds the finished clip; returns its `geo/` dir.
fn ensure_clip(c: &Cfg) -> Result<PathBuf, CookError> {
    let dir = c.cache.join(param_hash(c));
    if dir.join(".ok").is_file() {
        return Ok(dir.join("geo"));
    }
    if !c.root.is_dir() {
        return bad(format!("motion_root {} is not a directory", c.root.display()));
    }
    let run = c.cache.join(format!(
        "{}.run-{}-{}",
        param_hash(c),
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos())
    ));
    std::fs::create_dir_all(&run).map_err(|e| CookError::Failed(format!("create {}: {e}", run.display())))?;
    let result = run_python(c, &run);
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&run);
        return Err(e);
    }
    let finish = (|| -> Result<(), CookError> {
        std::fs::write(run.join(".ok"), b"ok\n").map_err(|e| CookError::Failed(format!("write marker: {e}")))?;
        match std::fs::rename(&run, &dir) {
            Ok(()) => Ok(()),
            // lost a race with another cook of the same params: theirs is complete, drop ours
            Err(_) if dir.join(".ok").is_file() => {
                let _ = std::fs::remove_dir_all(&run);
                Ok(())
            }
            Err(e) => Err(CookError::Failed(format!("publish {}: {e}", dir.display()))),
        }
    })();
    if finish.is_err() {
        let _ = std::fs::remove_dir_all(&run);
    }
    finish.map(|()| dir.join("geo"))
}

/// `python -m gaia_motion.cli run …` with cwd = `motion_root`, stdout/stderr to files in `run`, bounded by the timeout.
fn run_python(c: &Cfg, run: &Path) -> Result<(), CookError> {
    let (out_log, err_log) = (run.join("stdout.log"), run.join("stderr.log"));
    let open =
        |p: &Path| std::fs::File::create(p).map_err(|e| CookError::Failed(format!("create {}: {e}", p.display())));
    let mut cmd = Command::new(&c.python);
    cmd.current_dir(&c.root);
    if let Some((name, _)) = &c.behaviour {
        cmd.args(["-m", "gaia_motion.cli", "behaviour", "--name"])
            .arg(name)
            .arg("--out")
            .arg(run)
            .arg("--run-s")
            .arg(c.duration.to_string())
            .arg("--fps")
            .arg(c.fps.to_string());
        if c.seed >= 0 {
            cmd.arg("--seed").arg(c.seed.to_string());
        }
    } else if c.chain {
        cmd.args(["-m", "gaia_motion.cli", "chain", "--out"])
            .arg(run)
            .arg("--ns")
            .arg(c.ns.to_string())
            .arg("--run-s")
            .arg(c.duration.to_string())
            .arg("--fps")
            .arg(c.fps.to_string());
        if c.seed >= 0 {
            cmd.arg("--seed").arg(c.seed.to_string());
        }
    } else {
        cmd.args(["-m", "gaia_motion.cli", "run", "--scene", c.scene, "--frames"])
            .arg(c.frames.to_string())
            .arg("--out")
            .arg(run)
            .arg("--push-body")
            .arg(&c.body)
            .arg("--push-impulse")
            .args(c.impulse.iter().map(|v| v.to_string()))
            .args(["--up-axis", "y"]);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::from(open(&out_log)?)).stderr(Stdio::from(open(&err_log)?));
    let mut child = cmd.spawn().map_err(|e| CookError::Failed(format!("spawn {}: {e}", c.python.display())))?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > c.timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CookError::Failed(format!(
                    "gaia-motion timed out after {}s; stderr: {}",
                    c.timeout.as_secs(),
                    tail(&err_log)
                )));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(CookError::Failed(format!("wait on {}: {e}", c.python.display()))),
        }
    };
    if !status.success() {
        let code = status.code().map_or_else(|| "signal".to_string(), |c| c.to_string());
        return Err(CookError::Failed(format!("gaia-motion exited {code}: {}", tail(&err_log))));
    }
    let last = frame_path(&run.join("geo"), if c.chain || c.behaviour.is_some() { 1 } else { i64::from(c.frames) });
    if !last.is_file() {
        return Err(CookError::Failed(format!(
            "gaia-motion exited 0 but {} is missing; stderr: {}",
            last.display(),
            tail(&err_log)
        )));
    }
    Ok(())
}

impl Node<Detail> for Motion {
    fn type_name(&self) -> &'static str {
        TYPE_NAME
    }
    fn inputs(&self) -> &[InputSpec] {
        &[]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn time_dependent(&self) -> bool {
        true
    }
    fn cook(&self, ctx: &CookCtx, _inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let cfg = read_cfg(ctx.params)?;
        let geo_dir = ensure_clip(&cfg)?;
        let sequence = cfg.chain || cfg.behaviour.is_some();
        let frames = if sequence { count_frames(&geo_dir)? } else { cfg.frames };
        let idx =
            if cfg.frame > 0 { i64::from(cfg.frame.min(frames) - 1) } else { frame_index(ctx.time, cfg.fps, frames) };
        let path = frame_path(&geo_dir, idx + 1);
        let what = cfg.behaviour.as_ref().map_or(if cfg.chain { "chain" } else { cfg.scene }, |(n, _)| n.as_str());
        (ctx.log)(&format!("motion: {what} frame {} of {frames}", idx + 1));
        let d = hexe_geo::read_any(&path).map_err(|e| CookError::Failed(format!("read {}: {e}", path.display())))?;
        if sequence { skeleton(d).map(Arc::new) } else { Ok(Arc::new(d)) }
    }
}

#[cfg(test)]
mod tests;
