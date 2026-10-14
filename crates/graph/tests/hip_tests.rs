//! H40 `.hip` reader tests: cpio vs `hexpand`, gzip, and the 12-file Houdini 20.5 corpus (NO-OP when Houdini is not installed).
use hexe_graph::hip::{read_cpio, read_hip};
use std::path::Path;

const CHAIN: &[u8] = include_bytes!("../../../fixtures/hip/authored-chain.hip");
const CHAIN_GZ: &[u8] = include_bytes!("../../../fixtures/hip/authored-chain.hip.gz");
/// Member list written by the real `hexpand authored-chain.hip` (`.contents`).
const CHAIN_CONTENTS: &str = include_str!("../../../fixtures/hip/authored-chain.contents");
const HOUDINI: &str = "/Applications/Houdini/Houdini20.5.487/Frameworks/Houdini.framework/Versions/20.5/Resources/houdini/";
/// The 12 corpus files (`fixtures/hip/MANIFEST.md`), relative to `HOUDINI`.
const CORPUS: [&str; 12] = [
    "help/files/expression_cookbook/AnimationCycleExample.hip",
    "help/files/obj_xform.hip",
    "help/files/xforms_from_disk.hip",
    "help/files/file_to_load.hip",
    "help/files/pdg_examples/top_rop/input/external.hip",
    "help/files/expression_cookbook/RippleExample.hip",
    "help/files/expression_cookbook/LogSpiral.hip",
    "help/files/expression_cookbook/ModulusExample.hip",
    "help/files/pdg_examples/top_roadbuilder/roadbuilder_tile.hip",
    "help/files/pdg_examples/top_invoke/invoke_lite.hip",
    "help/files/vary_per_copy.hip",
    "help/files/color_falloff.hip",
];

#[test]
fn hip_cpio_members_equal_hexpand_contents() {
    let got: Vec<String> = read_cpio(CHAIN).unwrap().into_iter().map(|e| e.name).collect();
    let want: Vec<String> = CHAIN_CONTENTS.lines().map(String::from).collect();
    assert_eq!(got, want);
}

#[test]
fn hip_gzip_and_plain_parse_identically() {
    let (a, b) = (read_hip(CHAIN).unwrap(), read_hip(CHAIN_GZ).unwrap());
    assert!(b.gzipped && !a.gzipped);
    assert_eq!(a.networks, b.networks);
    let n = &a.networks[0];
    assert_eq!((n.name.as_str(), n.ty.as_str(), n.nodes.len()), ("geo1", "geo", 4));
    assert_eq!(n.nodes.iter().flat_map(|x| x.inputs.iter().flatten()).count(), 3);
    let m = n.nodes.iter().find(|x| x.name == "merge1").unwrap();
    assert!(m.display && m.render);
    assert_eq!(m.inputs.iter().flatten().map(|i| i.src.as_str()).collect::<Vec<_>>(), vec!["xform1", "grid1"]);
    let bx = n.nodes.iter().find(|x| x.name == "box1").unwrap();
    assert_eq!(bx.ty, "box");
    assert!(bx.parms.iter().any(|p| p.name == "size" && p.values.len() == 3));
}

/// Node / wire counts of the IR must equal an independent count over the raw cpio members (`obj/*/*.init`, `inputs { }` lines).
#[test]
fn hip_corpus_parse_12() {
    let mut ok = 0;
    let mut seen = 0;
    for rel in CORPUS {
        let path = format!("{HOUDINI}{rel}");
        if !Path::new(&path).exists() {
            continue;
        }
        seen += 1;
        let bytes = std::fs::read(&path).unwrap();
        let scene = read_hip(&bytes).unwrap_or_else(|e| panic!("{rel}: {e}"));
        let entries = read_cpio(&bytes).unwrap();
        let depth2 = |n: &str, ext: &str| n.strip_suffix(ext).is_some_and(|p| p.starts_with("obj/") && p.matches('/').count() == 2);
        let nodes = entries.iter().filter(|e| depth2(&e.name, ".init")).count();
        let mut wires = 0;
        for e in entries.iter().filter(|e| depth2(&e.name, ".def")) {
            let t = String::from_utf8_lossy(&e.data);
            if let Some(i) = t.find("\ninputs\n{\n") {
                let body: Vec<&str> = t[i + 10..].lines().take_while(|l| l.trim() != "}").collect();
                wires += body.iter().filter(|l| l.split_whitespace().nth(1).is_some_and(|s| s != "\"\"")).count();
            }
        }
        let got_nodes: usize = scene.networks.iter().map(|n| n.nodes.len()).sum();
        let got_wires: usize = scene.networks.iter().flat_map(|n| &n.nodes).map(|n| n.inputs.iter().flatten().count()).sum();
        println!("{rel}: nodes {got_nodes}/{nodes} wires {got_wires}/{wires}");
        assert_eq!((got_nodes, got_wires), (nodes, wires), "{rel}");
        ok += 1;
    }
    assert!(seen == 0 || ok == seen);
}

/// Every `.hip*` of the install parses, except the MIME-wrapped variant (reported as such, never a panic).
#[test]
fn hip_install_wide_parse() {
    let Ok(out) = std::process::Command::new("find").args(["/Applications/Houdini", "-name", "*.hip*"]).output() else { return };
    let (mut ok, mut mime) = (0, 0);
    for p in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(b) = std::fs::read(p) else { continue };
        match read_hip(&b) {
            Ok(_) => ok += 1,
            Err(e) => {
                assert!(e.0.contains("MIME"), "{p}: {e}");
                mime += 1;
            }
        }
    }
    println!("parsed {ok} hip files, {mime} MIME-wrapped (unsupported)");
}
