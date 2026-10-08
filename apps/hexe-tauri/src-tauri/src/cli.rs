//! Command line: `hexe [scene.json] [--set node.param=value]... [--script ops] [--version]`.
use crate::model::split_set;

pub const USAGE: &str = "usage: hexe [scene.json] [--set node.param=value]... [--script ops|file] [--version]";

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Cli {
    /// Scene to open at startup (`HEXE_OPEN` is the fallback, see `startup_scene`).
    pub scene: Option<String>,
    /// `--set node.param=value` overrides, re-applied on every reload.
    pub sets: Vec<String>,
    /// `--script` argument (op text or a file path), parsed by `script::parse`.
    pub script: Option<String>,
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
            "--script" => cli.script = Some(it.next().ok_or("--script needs ops or a file")?),
            // Launch Services / debuggers may add their own flags (`-psn_0_123`): not ours, skip.
            f if f.starts_with("-psn_") => {}
            f if f.starts_with("--") => return Err(format!("unknown option `{f}`")),
            _ if cli.scene.is_some() => return Err(format!("more than one scene given (`{a}`)")),
            _ => cli.scene = Some(a),
        }
    }
    Ok(Parsed::Run(cli))
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
    }
}
