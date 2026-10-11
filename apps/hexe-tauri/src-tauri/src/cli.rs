//! Command line: `hexe [scene.json] [--set node.param=value]... [--frame N] [--fps R] [--script ops] [--render out.png [--size WxH] [--wire]] [--version]`.
use crate::model::split_set;

pub const USAGE: &str = "usage: hexe [scene.json] [--set node.param=value]... [--frame N] [--fps R] [--script ops|file] [--render out.png [--size WxH] [--wire]] [--version]";

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Cli {
    /// Scene to open at startup (`HEXE_OPEN` is the fallback, see `startup_scene`).
    pub scene: Option<String>,
    /// `--set node.param=value` overrides, re-applied on every reload.
    pub sets: Vec<String>,
    /// `--frame N`: the timeline `$F` (1-based) the display cook / render runs at (default 1).
    pub frame: Option<i32>,
    /// `--fps R`: timeline rate; `time = ($F - 1) / R` (default 30, the gaia-motion clip rate).
    pub fps: Option<f32>,
    /// `--script` argument (op text or a file path), parsed by `script::parse`.
    pub script: Option<String>,
    /// `--render out.png`: headless render of the scene (window cooks, canvas -> PNG, exit); see `render.rs`.
    pub render: Option<String>,
    /// `--size WxH` (render only).
    pub size: Option<(u32, u32)>,
    /// `--wire` (render only): edges only, no shaded surface.
    pub wire: bool,
}

#[derive(Debug, PartialEq)]
pub enum Parsed {
    Version,
    Help,
    Run(Cli),
}

/// `hexe --version` text.
pub fn version() -> String {
    format!("hexe {}", env!("CARGO_PKG_VERSION"))
}

/// Arguments (without argv[0]) -> what to do. Unknown flags and malformed `--set` are errors (exit 2), never ignored.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Parsed, String> {
    let mut cli = Cli::default();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--version" | "-V" => return Ok(Parsed::Version),
            "--help" | "-h" => return Ok(Parsed::Help),
            "--set" => {
                let v = it.next().ok_or("--set needs node.param=value")?;
                split_set(&v)?;
                cli.sets.push(v);
            }
            "--frame" => {
                let v = it.next().ok_or("--frame needs N (1-based)")?;
                let ok = v.trim().parse().ok().filter(|f| (1..=crate::model::MAX_FRAME).contains(f));
                cli.frame = Some(ok.ok_or(format!("--frame {v}: want an integer 1..={}", crate::model::MAX_FRAME))?);
            }
            "--fps" => {
                let v = it.next().ok_or("--fps needs a rate")?;
                let ok = v.trim().parse::<f32>().ok().filter(|f| f.is_finite() && *f > 0.0 && *f <= 1000.0);
                cli.fps = Some(ok.ok_or(format!("--fps {v}: want a number in (0, 1000]"))?);
            }
            "--script" => cli.script = Some(it.next().ok_or("--script needs ops or a file")?),
            "--render" => cli.render = Some(it.next().ok_or("--render needs an output .png path")?),
            "--size" => cli.size = Some(crate::render::parse_size(&it.next().ok_or("--size needs WxH")?)?),
            "--wire" => cli.wire = true,
            // Launch Services / debuggers may add their own flags (`-psn_0_123`): not ours, skip.
            f if f.starts_with("-psn_") => {}
            f if f.starts_with("--") => return Err(format!("unknown option `{f}`")),
            _ if cli.scene.is_some() => return Err(format!("more than one scene given (`{a}`)")),
            _ => cli.scene = Some(a),
        }
    }
    if cli.render.is_none() && (cli.size.is_some() || cli.wire) {
        return Err("--size / --wire only apply with --render out.png".into());
    }
    if cli.render.is_some() && cli.scene.is_none() {
        return Err("--render needs a scene: hexe scene.json --render out.png".into());
    }
    if cli.render.is_some() && cli.script.is_some() {
        return Err("--render and --script cannot be combined".into());
    }
    Ok(Parsed::Run(cli))
}
/// The window's render job, if `--render` was given.
pub fn render_req(cli: &Cli) -> Option<crate::render::RenderReq> {
    let (width, height) = cli.size.unwrap_or(crate::render::DEFAULT_SIZE);
    cli.render.as_ref().map(|out| crate::render::RenderReq { out: out.clone(), width, height, wire: cli.wire })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &[&str]) -> Result<Parsed, String> {
        parse(a.iter().map(|s| s.to_string()))
    }

    #[test]
    fn positional_set_script() {
        let want = Cli {
            scene: Some("a.json".into()),
            sets: vec!["pe.distance=2".into(), "b.size=1,2,3".into()],
            script: Some("quit".into()),
            ..Cli::default()
        };
        let got = p(&["a.json", "--set", "pe.distance=2", "--set", "b.size=1,2,3", "--script", "quit"]);
        assert_eq!(got, Ok(Parsed::Run(want)));
        assert_eq!(p(&[]), Ok(Parsed::Run(Cli::default())));
    }

    #[test]
    fn version_and_errors() {
        assert_eq!(p(&["--version"]), Ok(Parsed::Version));
        assert_eq!(p(&["x.json", "--version"]), Ok(Parsed::Version));
        assert!(version().starts_with("hexe 0."));
        for bad in [&["--set"][..], &["--set", "nodot=1"], &["--set", "a.b"], &["--bogus"], &["a", "b"], &["--script"]]
        {
            assert!(p(bad).is_err(), "{bad:?}");
        }
        assert_eq!(p(&["-psn_0_1"]), Ok(Parsed::Run(Cli::default())));
        // Houdini-path spelling of a --set key
        assert_eq!(split_set("motion/behaviour=stagger"), Ok(("motion", "behaviour", "stagger")));
        assert_eq!(split_set("motion.behaviour=stagger"), Ok(("motion", "behaviour", "stagger")));
        assert!(p(&["--set", "motion=stagger"]).is_err() && p(&["--set", "/x=1"]).is_err());
    }
    #[test]
    fn frame_and_fps_flags() {
        let Ok(Parsed::Run(c)) = p(&["a.json", "--frame", "90", "--fps", "24", "--render", "o.png"]) else {
            panic!("parse")
        };
        assert_eq!((c.frame, c.fps), (Some(90), Some(24.0)));
        let Ok(Parsed::Run(d)) = p(&["a.json"]) else { panic!("parse") };
        assert_eq!((d.frame, d.fps), (None, None));
        for bad in [
            &["--frame"][..],
            &["--frame", "0"],
            &["--frame", "-3"],
            &["--frame", "1.5"],
            &["--frame", "x"],
            &["--frame", "99999999"],
            &["--fps"],
            &["--fps", "0"],
            &["--fps", "nan"],
            &["--fps", "1001"],
        ] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
    }
    #[test]
    fn render_flags() {
        let Ok(Parsed::Run(c)) = p(&["a.json", "--render", "o.png", "--size", "640x480", "--wire"]) else {
            panic!("parse")
        };
        let r = render_req(&c).expect("render job");
        assert_eq!((r.out.as_str(), r.width, r.height, r.wire), ("o.png", 640, 480, true));
        let Ok(Parsed::Run(d)) = p(&["a.json", "--render", "o.png"]) else { panic!("parse") };
        assert_eq!(render_req(&d).map(|r| (r.width, r.height, r.wire)), Some((1280, 800, false)));
        for bad in [
            &["a.json", "--size", "640x480"][..],
            &["a.json", "--wire"],
            &["--render", "o.png"],
            &["a.json", "--render"],
            &["a.json", "--render", "o.png", "--size", "9x9"],
            &["a.json", "--render", "o.png", "--script", "quit"],
        ] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
    }
}
