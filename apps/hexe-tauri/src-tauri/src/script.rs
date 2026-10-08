//! `--script`: proof / automation hook (ported from the retired egui app). Ops separated by `;` or newlines, run one at a
//! time against the live session, the window re-syncs after each (see `shell.rs`):
//! `add <type> [name] [@x,y]` · `connect <dst> <port> <src>` · `disconnect <dst> <port>` · `remove <name>...` ·
//! `display <name|none>` · `select <name>...` · `set node.param=value` · `save <path>` · `shot <path>` · `quit`;
//! extras for the T3 proofs: `rename <old> <new>` · `undo` · `redo` · `copy <name>...` · `paste`.
//! The argument is the op list itself, or a path to a file holding it. `#` starts a comment line.
use crate::model::Session;

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Add {
        ty: String,
        name: Option<String>,
        at: Option<(f32, f32)>,
    },
    Connect {
        dst: String,
        port: usize,
        src: String,
    },
    Disconnect {
        dst: String,
        port: usize,
    },
    Remove(Vec<String>),
    /// `None` = clear the display flag.
    Display(Option<String>),
    Select(Vec<String>),
    Set(String),
    Rename {
        old: String,
        new: String,
    },
    Undo,
    Redo,
    Copy(Vec<String>),
    Paste,
    Save(String),
    Shot(String),
    Quit,
}

/// What the shell must do after an op ran against the session.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Effect {
    /// Node names the window should select afterwards (`None` = leave the selection alone).
    pub select: Option<Vec<String>>,
    pub shot: Option<String>,
    pub quit: bool,
}

/// Expand a leading `~/`.
pub fn expand(path: &str) -> String {
    match (path.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(h)) => format!("{h}/{rest}"),
        _ => path.to_string(),
    }
}

/// Op list text (or a path to a file holding it) -> ops. Any bad op is an `Err` naming it.
pub fn parse(arg: &str) -> Result<Vec<Op>, String> {
    let text = match std::fs::read_to_string(expand(arg)) {
        Ok(t) if !arg.contains(';') => t,
        _ => arg.to_string(),
    };
    text.split([';', '\n']).map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(op).collect()
}

fn at(s: &str) -> Option<(f32, f32)> {
    let (x, y) = s.strip_prefix('@')?.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

fn op(line: &str) -> Result<Op, String> {
    let t: Vec<&str> = line.split_whitespace().collect();
    let bad = || format!("script: bad op `{line}`");
    let port = |s: &str| s.parse::<usize>().map_err(|_| bad());
    let names = |r: &[&str]| -> Result<Vec<String>, String> {
        if r.is_empty() { Err(bad()) } else { Ok(r.iter().map(|s| s.to_string()).collect()) }
    };
    Ok(match t.as_slice() {
        ["add", ty] => Op::Add { ty: ty.to_string(), name: None, at: None },
        ["add", ty, a] if a.starts_with('@') => {
            Op::Add { ty: ty.to_string(), name: None, at: Some(at(a).ok_or_else(bad)?) }
        }
        ["add", ty, name] => Op::Add { ty: ty.to_string(), name: Some(name.to_string()), at: None },
        ["add", ty, name, a] => {
            Op::Add { ty: ty.to_string(), name: Some(name.to_string()), at: Some(at(a).ok_or_else(bad)?) }
        }
        ["connect", dst, p, src] => Op::Connect { dst: dst.to_string(), port: port(p)?, src: src.to_string() },
        ["disconnect", dst, p] => Op::Disconnect { dst: dst.to_string(), port: port(p)? },
        ["remove", rest @ ..] => Op::Remove(names(rest)?),
        ["display", "none"] => Op::Display(None),
        ["display", n] => Op::Display(Some(n.to_string())),
        ["select", rest @ ..] => Op::Select(names(rest)?),
        ["set", kv] => Op::Set(kv.to_string()),
        ["rename", old, new] => Op::Rename { old: old.to_string(), new: new.to_string() },
        ["undo"] => Op::Undo,
        ["redo"] => Op::Redo,
        ["copy", rest @ ..] => Op::Copy(names(rest)?),
        ["paste"] => Op::Paste,
        ["save", p] => Op::Save(expand(p)),
        ["shot", p] => Op::Shot(expand(p)),
        ["quit"] => Op::Quit,
        _ => return Err(bad()),
    })
}

fn ids(s: &Session, names: &[String]) -> Result<Vec<u32>, String> {
    names.iter().map(|n| s.id_of(n).ok_or_else(|| format!("script: no node `{n}`"))).collect()
}

fn names_of(s: &Session, ids: &[u32]) -> Vec<String> {
    ids.iter().filter_map(|&i| s.name_of(i)).collect()
}

/// Run one op against the session.
pub fn apply(s: &mut Session, op: &Op) -> Result<Effect, String> {
    let sel = |v: Vec<String>| Effect { select: Some(v), ..Effect::default() };
    Ok(match op {
        Op::Add { ty, name, at } => {
            let n = s.node_count();
            let (x, y) = at.unwrap_or((40.0 + 190.0 * (n % 4) as f32, 30.0 + 90.0 * (n / 4) as f32));
            let id = s.node_add(ty, x, y)?;
            if let Some(name) = name {
                s.node_rename(id, name)?;
            }
            sel(names_of(s, &[id]))
        }
        Op::Connect { dst, port, src } => {
            let (d, sr) = (ids(s, std::slice::from_ref(dst))?[0], ids(s, std::slice::from_ref(src))?[0]);
            s.edge_connect(sr, d, *port)?;
            Effect::default()
        }
        Op::Disconnect { dst, port } => {
            s.edge_disconnect(ids(s, std::slice::from_ref(dst))?[0], *port)?;
            Effect::default()
        }
        Op::Remove(n) => {
            s.nodes_remove(&ids(s, n)?)?;
            sel(Vec::new())
        }
        Op::Display(n) => {
            let id = n.as_ref().map(|n| ids(s, std::slice::from_ref(n)).map(|v| v[0])).transpose()?;
            s.set_display(id)?;
            Effect::default()
        }
        Op::Select(n) => {
            ids(s, n)?;
            sel(n.clone())
        }
        Op::Set(kv) => {
            s.set_arg(kv)?;
            Effect::default()
        }
        Op::Rename { old, new } => {
            let id = ids(s, std::slice::from_ref(old))?[0];
            s.node_rename(id, new)?;
            sel(vec![new.clone()])
        }
        Op::Undo => {
            s.undo()?;
            sel(Vec::new())
        }
        Op::Redo => {
            s.redo()?;
            sel(Vec::new())
        }
        Op::Copy(n) => {
            s.copy(&ids(s, n)?)?;
            Effect::default()
        }
        Op::Paste => {
            let new = s.paste()?;
            sel(names_of(s, &new))
        }
        Op::Save(p) => {
            s.graph_save(p)?;
            Effect::default()
        }
        Op::Shot(p) => Effect { shot: Some(p.clone()), ..Effect::default() },
        Op::Quit => Effect { quit: true, ..Effect::default() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_proof_script() {
        let ops = parse("add box box2; connect pe1 0 box2; display pe1\n# c\nsave /x/o.json; shot /x/o.png; quit")
            .expect("parses");
        assert_eq!(ops.len(), 6);
        assert_eq!(ops[0], Op::Add { ty: "box".into(), name: Some("box2".into()), at: None });
        assert_eq!(ops[1], Op::Connect { dst: "pe1".into(), port: 0, src: "box2".into() });
        assert_eq!(ops[5], Op::Quit);
    }

    #[test]
    fn parses_extras() {
        let ops =
            parse("add box @10,20; select a b; rename a c; undo; redo; copy a b; paste; display none; remove x y")
                .expect("parses");
        assert_eq!(ops[0], Op::Add { ty: "box".into(), name: None, at: Some((10.0, 20.0)) });
        assert_eq!(ops[1], Op::Select(vec!["a".into(), "b".into()]));
        assert_eq!(ops[7], Op::Display(None));
        assert_eq!(ops[8], Op::Remove(vec!["x".into(), "y".into()]));
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["frobnicate", "connect a x b", "connect a 0", "add", "select", "add box @x", "rename a"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn applies_a_build_script() {
        let mut s = Session::new();
        for o in parse("add box b; add polyextrude pe; connect pe 0 b; display pe; set pe.distance=0.5; rename pe ext")
            .expect("parse")
        {
            apply(&mut s, &o).expect("op");
        }
        let g = s.graph_get();
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.edges.len(), 1);
        let e = g.nodes.iter().find(|n| n.name == "ext").expect("renamed");
        assert_eq!(g.display, Some(e.id));
        assert_eq!(e.params["distance"], serde_json::json!(0.5));
        assert!(apply(&mut s, &Op::Select(vec!["nope".into()])).is_err());
        assert_eq!(s.cook().error, None);
    }
}
