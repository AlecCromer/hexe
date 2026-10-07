//! `--script`: proof/automation hook. Ops separated by `;` or newlines, executed one per frame:
//! `add <type> [name]` · `connect <dst> <port> <src>` · `disconnect <dst> <port>` · `remove <name>` ·
//! `display <name>` · `select <name>` · `set node.param=value` · `view w|p|n` · `save <path>` · `shot <path>` · `quit`.
//! The argument is the op list itself, or a path to a file holding it.

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Add { ty: String, name: Option<String> },
    Connect { dst: String, port: usize, src: String },
    Disconnect { dst: String, port: usize },
    Remove(String),
    Display(String),
    Select(String),
    Set(String),
    View(char),
    Save(String),
    Shot(String),
    Quit,
}

/// Expand a leading `~/`.
pub fn expand(path: &str) -> String {
    match (path.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(h)) => format!("{h}/{rest}"),
        _ => path.to_string(),
    }
}

/// Op list text (or a file path holding it) -> ops. Any bad op is an `Err` naming it.
pub fn parse(arg: &str) -> Result<Vec<Op>, String> {
    let text = match std::fs::read_to_string(expand(arg)) {
        Ok(t) if !arg.contains(';') => t,
        _ => arg.to_string(),
    };
    text.split([';', '\n']).map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(op).collect()
}

fn op(line: &str) -> Result<Op, String> {
    let t: Vec<&str> = line.split_whitespace().collect();
    let bad = || format!("script: bad op `{line}`");
    let port = |s: &str| s.parse::<usize>().map_err(|_| bad());
    Ok(match t.as_slice() {
        ["add", ty] => Op::Add { ty: ty.to_string(), name: None },
        ["add", ty, name] => Op::Add { ty: ty.to_string(), name: Some(name.to_string()) },
        ["connect", dst, p, src] => Op::Connect { dst: dst.to_string(), port: port(p)?, src: src.to_string() },
        ["disconnect", dst, p] => Op::Disconnect { dst: dst.to_string(), port: port(p)? },
        ["remove", n] => Op::Remove(n.to_string()),
        ["display", n] => Op::Display(n.to_string()),
        ["select", n] => Op::Select(n.to_string()),
        ["set", kv] => Op::Set(kv.to_string()),
        ["view", v @ ("w" | "p" | "n")] => Op::View(v.chars().next().unwrap_or('w')),
        ["save", p] => Op::Save(expand(p)),
        ["shot", p] => Op::Shot(expand(p)),
        ["quit"] => Op::Quit,
        _ => return Err(bad()),
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
        assert_eq!(ops[0], Op::Add { ty: "box".into(), name: Some("box2".into()) });
        assert_eq!(ops[1], Op::Connect { dst: "pe1".into(), port: 0, src: "box2".into() });
        assert_eq!(ops[5], Op::Quit);
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["frobnicate", "connect a x b", "connect a 0", "view z", "add"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
