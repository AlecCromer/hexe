//! `.bgeo` writer: round trips through this crate's reader and (when installed) the Houdini 20.5.487 shipped corpus.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use crate::io_tests::cube;
use crate::*;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn rich() -> Detail {
    let mut d = cube();
    let attr = |name: &str, class: AttrClass, data: AttrData| Attribute {
        name: name.into(),
        class,
        data,
        type_info: TypeInfo::None,
    };
    d.add_attr(attr("w", AttrClass::Point, AttrData::F32((0..8).map(|i| i as f32 * 0.1).collect())));
    d.add_attr(attr("id", AttrClass::Point, AttrData::I32((0..8).map(|i| i * 1_000_003).collect())));
    d.add_attr(attr("Cd", AttrClass::Point, AttrData::V3((0..8).map(|i| [i as f32, 0.5, -1.25]).collect())));
    d.add_attr(attr("uv", AttrClass::Vertex, AttrData::V3((0..24).map(|i| [i as f32 / 24.0, 0.25, 0.0]).collect())));
    d.add_attr(attr("cls", AttrClass::Prim, AttrData::F32(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0])));
    d.add_attr(attr(
        "name",
        AttrClass::Prim,
        AttrData::Str { idx: vec![0, 1, 0, 1, 0, -1], table: vec!["a".into(), "bee".into()] },
    ));
    d.groups.push(Group { name: "top".into(), class: AttrClass::Point, mask: (0..8).map(|i| i >= 4).collect() });
    d
}
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::var("HEXE_SCRATCH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/hexe-scratch"));
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name)
}

#[test]
fn written_bgeo_reads_back_identical_to_the_json_path() {
    for d in [cube(), rich(), Detail::default()] {
        let bytes = write_geo_bjson(&d).unwrap();
        assert!(bytes.starts_with(&[0x7f, b'N', b'S', b'J', b'b']));
        let via_json = read_geo_json(&write_geo_json(&d)).unwrap();
        assert_eq!(read_geo_bjson(&bytes).unwrap(), via_json);
        assert_eq!(read_geo(&bytes).unwrap(), via_json, "sniffed");
        // value level: the binary tree equals the ASCII tree
        let ascii: Value = serde_json::from_slice(&write_geo_json(&d)).unwrap();
        assert_eq!(bjson_to_value(&bytes).unwrap(), ascii);
    }
    // write_any picks it by extension
    let d = rich();
    let p = scratch("bjson_w_rich.bgeo");
    write_any(&p, &d).unwrap();
    assert_eq!(read_any(&p).unwrap(), read_geo_json(&write_geo_json(&d)).unwrap());
}

#[test]
fn value_encoder_covers_every_token_and_length_form() {
    let big: Vec<Value> = (0..300).map(|i| Value::from(i64::from(i) * 7)).collect(); // uniform i32, 2-byte length prefix
    let wide: Vec<Value> = vec![Value::from(1_i64 << 40), Value::from(-5), Value::from(7), Value::from(9)]; // uniform i64
    let f32s: Vec<Value> = [0.5, -2.25, 8.0, 0.125].iter().map(|&x| Value::from(x)).collect();
    let f64s: Vec<Value> = [0.1, 0.2, 0.3, 1e-300].iter().map(|&x| Value::from(x)).collect();
    let mixed = Value::Array(vec![Value::from(1), Value::from(1.5), Value::Null, Value::Bool(true), Value::from("s")]);
    let long_str = "x".repeat(1000);
    let v = serde_json::json!({
        "big": big, "wide": wide, "f32": f32s, "f64": f64s, "mixed": mixed, "long": long_str,
        "nested": [[1, 2], {"a": [false, true, null]}], "unicode": "h\u{e9}llo \u{1f600}", "neg": -123456789012_i64,
    });
    let bytes = value_to_bjson(&v).unwrap();
    assert_eq!(bjson_to_value(&bytes).unwrap(), v);
    // depth cap
    let mut deep = Value::Null;
    for _ in 0..70 {
        deep = Value::Array(vec![deep]);
    }
    assert!(value_to_bjson(&deep).is_err());
}

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

/// Houdini's own `.bgeo` files: decode, re-encode with this writer, decode again: identical tree and Detail.
/// Skipped (vacuous) when Houdini is not installed.
#[test]
fn houdini_shipped_bgeo_corpus_survives_decode_encode_decode() {
    let mut files = Vec::new();
    walk(
        Path::new(
            "/Applications/Houdini/Houdini20.5.487/Frameworks/Houdini.framework/Versions/20.5/Resources/houdini/geo",
        ),
        &mut files,
    );
    files.sort();
    let mut checked = 0;
    for p in files.iter().take(40) {
        let orig = std::fs::read(p).unwrap();
        if !orig.starts_with(&[0x7f]) {
            continue;
        }
        let Ok(tree) = bjson_to_value(&orig) else { continue };
        let mine = value_to_bjson(&tree).unwrap();
        assert_eq!(bjson_to_value(&mine).unwrap(), tree, "{}", p.display());
        if let Ok(d) = read_geo_bjson(&orig) {
            assert_eq!(read_geo_bjson(&write_geo_bjson(&d).unwrap()).unwrap(), d, "{}", p.display());
        }
        checked += 1;
    }
    eprintln!("houdini corpus files round-tripped: {checked}");
}
