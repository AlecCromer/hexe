use crate::*;
use std::path::{Path, PathBuf};
fn walk(d: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(d) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_symlink() {
        } else if p.is_dir() {
            walk(&p, out)
        } else if p.extension().is_some_and(|x| x == "bgeo") {
            out.push(p)
        }
    }
}
#[test]
fn probe() {
    let mut v = Vec::new();
    walk(Path::new("/Applications/Houdini/Houdini20.5.487"), &mut v);
    v.sort();
    let (mut ok, mut val) = (0, 0);
    for p in &v {
        let b = std::fs::read(p).unwrap();
        if b.starts_with(&[0x7f]) {
            match bjson_to_value(&b) {
                Ok(_) => val += 1,
                Err(e) => eprintln!("VALUE-ERR {} {e}", p.display()),
            }
            match read_geo_bjson(&b) {
                Ok(_) => ok += 1,
                Err(e) => eprintln!("DETAIL-ERR {} {e}", p.file_name().unwrap().to_string_lossy()),
            }
        } else {
            eprintln!("NOTBJSON {} {:02x?}", p.display(), &b[..8.min(b.len())])
        }
    }
    eprintln!("total {} value-ok {val} detail-ok {ok}", v.len());
}
#[test]
fn dump() {
    let Ok(f) = std::env::var("BGEO") else { return };
    let v = bjson_to_value(&std::fs::read(f).unwrap());
    match v {
        Ok(v) => eprintln!("{}", serde_json::to_string(&v).unwrap()),
        Err(e) => eprintln!("ERR {e}"),
    }
}
#[test]
fn survey() {
    let mut v = Vec::new();
    walk(Path::new("/Applications/Houdini/Houdini20.5.487"), &mut v);
    v.sort();
    let mut types = std::collections::BTreeMap::<String, usize>::new();
    let mut keys = std::collections::BTreeMap::<String, usize>::new();
    for p in &v {
        let b = std::fs::read(p).unwrap();
        let Ok(val) = bjson_to_value(&b) else { continue };
        let Value::Array(top) = &val else { continue };
        let mut it = top.chunks(2);
        while let Some([Value::String(k), x]) = it.next() {
            if k == "primitives" {
                for run in x.as_array().unwrap() {
                    let r = run.as_array().unwrap();
                    let ty = r[0].as_array().unwrap()[1].as_str().unwrap().to_string();
                    *types.entry(ty.clone()).or_default() += 1;
                    for kk in r[1].as_array().unwrap().chunks(2) {
                        *keys.entry(format!("{ty}/{}", kk[0].as_str().unwrap_or("?"))).or_default() += 1;
                    }
                }
            }
        }
    }
    eprintln!("TYPES {types:?}\nKEYS {keys:?}");
}
use serde_json::Value;
#[test]
fn survey2() {
    let mut v = Vec::new();
    walk(Path::new("/Applications/Houdini/Houdini20.5.487"), &mut v);
    v.sort();
    let mut seen = std::collections::BTreeSet::<String>::new();
    for p in &v {
        let b = std::fs::read(p).unwrap();
        let Ok(val) = bjson_to_value(&b) else { continue };
        let Value::Array(top) = &val else { continue };
        for c in top.chunks(2) {
            if c[0] == "primitives" {
                for run in c[1].as_array().unwrap() {
                    let r = run.as_array().unwrap();
                    let s = serde_json::to_string(run).unwrap();
                    let ty = r[0].as_array().unwrap()[1].as_str().unwrap().to_string();
                    let key =
                        if ty == "run" || s.contains("n_v") { format!("{ty}{}", s.contains("n_v")) } else { continue };
                    if seen.insert(key) {
                        eprintln!("EX {} :: {}", p.file_name().unwrap().to_string_lossy(), &s[..s.len().min(400)]);
                    }
                }
            }
        }
    }
}
