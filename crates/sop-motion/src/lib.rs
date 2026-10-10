//! `motion` SOP (hexe-native, no Houdini counterpart): GAIA Motion as a node.
//! Cook = run `python -m gaia_motion.cli run …` once per distinct parameter set into
//! `<cache>/<hash>/geo/frame.%04d.geo` (default cache `~/.gaia/cache/hexe/motion`), then read the `.geo` frame for the
//! current time (`round(time * fps)`, clamped to the clip) as the output `Detail`. Python failures surface their stderr.
//! Flags are the real ones of `~/projects/gaia-motion/gaia_motion/cli.py` (lines 20-38); output layout `export.py:48-54`.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Scene tokens, menu order. Mirror of `SCENES` in `gaia_motion/scenes.py:57`.
pub const SCENES: [&str; 4] = ["stand", "push", "leg_hit", "fall_from_height"];
/// Hard cap on `frames` (clip size / run time bound).
pub const MAX_FRAMES: i32 = 100_000;
const DEFAULT_PYTHON: &str = "~/projects/gaia-motion/.venv/bin/python";
const DEFAULT_ROOT: &str = "~/projects/gaia-motion";
const STDERR_TAIL: usize = 2000;

/// Registry token of the node.
pub const TYPE_NAME: &str = "motion";

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("scene", ParamValue::Menu(1)).label("Scene").menu(&[
            ("stand", "Stand"),
            ("push", "Push"),
            ("leg_hit", "Leg Hit"),
            ("fall_from_height", "Fall From Height"),
        ]),
        ParamSpec::new("impulse_ns", ParamValue::F(90.0)).label("Impulse (N*s)").range(0.0, 300.0),
        ParamSpec::new("impulse_dir", ParamValue::V3([1.0, 0.0, 0.0])).label("Impulse Direction (world Z-up)"),
        ParamSpec::new("impulse_body", ParamValue::S("Chest".into())).label("Impulse Body"),
        ParamSpec::new("frames", ParamValue::I(120)).label("Frames").range(1.0, 600.0),
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
    Ok(Cfg {
        scene,
        impulse,
        body,
        frames,
        fps,
        python: expand(python),
        root: expand(root),
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
    let key = format!(
        "v1|{}|{:08x},{:08x},{:08x}|{}|{}",
        c.scene,
        c.impulse[0].to_bits(),
        c.impulse[1].to_bits(),
        c.impulse[2].to_bits(),
        c.body,
        c.frames
    );
    format!("{:016x}{:016x}", fnv64(0, key.as_bytes()), fnv64(0x9e37_79b9_7f4a_7c15, key.as_bytes()))
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
    cmd.current_dir(&c.root)
        .args(["-m", "gaia_motion.cli", "run", "--scene", c.scene, "--frames"])
        .arg(c.frames.to_string())
        .arg("--out")
        .arg(run)
        .arg("--push-body")
        .arg(&c.body)
        .arg("--push-impulse")
        .args(c.impulse.iter().map(|v| v.to_string()))
        .args(["--up-axis", "y"])
        .stdin(Stdio::null())
        .stdout(Stdio::from(open(&out_log)?))
        .stderr(Stdio::from(open(&err_log)?));
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
    let last = frame_path(&run.join("geo"), i64::from(c.frames));
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
        let idx = frame_index(ctx.time, cfg.fps, cfg.frames);
        let path = frame_path(&geo_dir, idx + 1);
        (ctx.log)(&format!("motion: {} frame {} of {}", cfg.scene, idx + 1, cfg.frames));
        hexe_geo::read_any(&path).map(Arc::new).map_err(|e| CookError::Failed(format!("read {}: {e}", path.display())))
    }
}

#[cfg(test)]
mod tests;
