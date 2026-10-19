//! `hexe-cli open <file.hip|.hipnc|.hiplc|.hip.gz> [--geo NAME] [--scene-out F] [--cook-out F.geo] [--all-out DIR]` (H40).
//! Reads a Houdini scene (`hexe_graph::hip`), maps each `/obj/<geo>` SOP network onto a hexe scene (`hexe_sop::import_hip`),
//! prints an import report. `--scene-out` writes the chosen geo's scene JSON; `--cook-out` cooks its display node at the
//! scene's saved frame; `--all-out DIR` writes `<geo>.scene.json` for every geo + `import.json` (machine-readable report);
//! `--exprs` prints every parm channel's value at the saved frame (`hexe_sop::hip_channel_report`).
use std::fmt::Write as _;

const MAX_HIP: u64 = 1 << 30;

struct Args {
    file: String,
    geo: Option<String>,
    scene_out: Option<String>,
    cook_out: Option<String>,
    all_out: Option<String>,
    exprs: bool,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let usage = "usage: hexe-cli open <file.hip> [--geo NAME] [--scene-out F.json] [--cook-out F.geo] [--all-out DIR] [--exprs]";
    let mut a = Args { file: String::new(), geo: None, scene_out: None, cook_out: None, all_out: None, exprs: false };
    let mut it = args.iter();
    while let Some(x) = it.next() {
        let mut val = |what: &str| it.next().cloned().ok_or_else(|| format!("open: {what} needs a value"));
        match x.as_str() {
            "--geo" => a.geo = Some(val("--geo")?),
            "--scene-out" => a.scene_out = Some(val("--scene-out")?),
            "--cook-out" => a.cook_out = Some(val("--cook-out")?),
            "--all-out" => a.all_out = Some(val("--all-out")?),
            "--exprs" => a.exprs = true,
            f if !f.starts_with("--") && a.file.is_empty() => a.file = f.to_string(),
            other => return Err(format!("open: unexpected `{other}`\n{usage}")),
        }
    }
    if a.file.is_empty() {
        return Err(usage.into());
    }
    Ok(a)
}

pub fn open_cmd(args: &[String]) -> Result<String, String> {
    let a = parse(args)?;
    let meta = std::fs::metadata(&a.file).map_err(|e| format!("{}: {e}", a.file))?;
    if meta.len() > MAX_HIP {
        return Err(format!("{}: larger than {MAX_HIP} bytes", a.file));
    }
    let bytes = std::fs::read(&a.file).map_err(|e| format!("{}: {e}", a.file))?;
    let imp = hexe_sop::import_hip(&bytes).map_err(|e| e.to_string())?;
    let mut out = String::new();
    if a.exprs {
        // every channel's value at the saved frame (tab-separated: path, channel, value | ERR reason)
        let hip = hexe_graph::hip::read_hip(&bytes).map_err(|e| e.to_string())?;
        for (path, chan, v) in hexe_sop::hip_channel_report(&hip) {
            let v = match v {
                Ok(hexe_graph::hscript::Value::Num(x)) => format!("{x}"),
                Ok(hexe_graph::hscript::Value::Str(s)) => format!("{s:?}"),
                Err(e) => format!("ERR {e}"),
            };
            let _ = writeln!(out, "{path}\t{chan}\t{v}");
        }
        return Ok(out);
    }
    let _ = writeln!(
        out,
        "{}: frame {} @ {} fps, {} geo network(s), {} other container(s) skipped",
        a.file,
        imp.frame,
        imp.fps,
        imp.geos.len(),
        imp.skipped.len()
    );
    for g in &imp.geos {
        let _ = writeln!(
            out,
            "  /obj/{}: {} nodes, {} wires, display `{}`, {} unsupported{}, {} animated, {} dropped wires, {} notes",
            g.name,
            g.node_count,
            g.wire_count,
            g.display,
            g.unsupported.len(),
            if g.unsupported.is_empty() { String::new() } else { format!(" [{}]", g.unsupported.join(" ")) },
            g.animated.len(),
            g.dropped_wires,
            g.notes.len()
        );
    }
    let pick = || -> Result<&hexe_sop::GeoImport, String> {
        match &a.geo {
            Some(n) => imp.geos.iter().find(|g| &g.name == n).ok_or_else(|| format!("open: no geo network `{n}`")),
            None => imp.geos.first().ok_or_else(|| "open: file has no /obj geo network with SOP nodes".to_string()),
        }
    };
    if let Some(dir) = &a.all_out {
        std::fs::create_dir_all(dir).map_err(|e| format!("{dir}: {e}"))?;
        let mut report = Vec::new();
        for g in &imp.geos {
            let p = format!("{dir}/{}.scene.json", g.name);
            let text = serde_json::to_string_pretty(&g.scene).map_err(|e| e.to_string())?;
            std::fs::write(&p, text).map_err(|e| format!("{p}: {e}"))?;
            report.push(serde_json::json!({
                "geo": g.name, "container": g.container, "nodes": g.node_count, "wires": g.wire_count, "display": g.display,
                "unsupported": g.unsupported, "animated": g.animated, "dropped_wires": g.dropped_wires, "notes": g.notes
            }));
        }
        let doc = serde_json::json!({ "file": a.file, "frame": imp.frame, "fps": imp.fps, "skipped": imp.skipped, "geos": report });
        let p = format!("{dir}/import.json");
        std::fs::write(&p, serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?)
            .map_err(|e| format!("{p}: {e}"))?;
        let _ = writeln!(out, "wrote {} scene(s) + import.json -> {dir}", imp.geos.len());
    }
    if a.scene_out.is_some() || a.cook_out.is_some() {
        let g = pick()?;
        let json = serde_json::to_string_pretty(&g.scene).map_err(|e| e.to_string())?;
        if let Some(p) = &a.scene_out {
            std::fs::write(p, &json).map_err(|e| format!("{p}: {e}"))?;
            let _ = writeln!(out, "scene `{}` -> {p}", g.name);
        }
        if let Some(p) = &a.cook_out {
            let time = ((imp.frame - 1.0) / imp.fps.max(1.0)) as f32;
            let (bytes, points, prims) = crate::cook_scene_range(&json, time, None, None)?;
            std::fs::write(p, bytes).map_err(|e| format!("{p}: {e}"))?;
            let _ = writeln!(
                out,
                "cooked `{}` display `{}` at t={time}: {points} points, {prims} prims -> {p}",
                g.name, g.display
            );
        }
    }
    if a.scene_out.is_none() && a.cook_out.is_none() && a.all_out.is_none() {
        for g in &imp.geos {
            for n in &g.notes {
                let _ = writeln!(out, "  note [{}] {n}", g.name);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    #[test]
    fn open_args_parse_and_reject() {
        assert!(parse(&s(&[])).is_err());
        assert!(parse(&s(&["a.hip", "--bogus"])).is_err());
        assert!(parse(&s(&["a.hip", "--geo"])).is_err());
        let a = parse(&s(&["a.hip", "--geo", "g", "--scene-out", "x.json"])).unwrap();
        assert_eq!((a.file.as_str(), a.geo.as_deref(), a.scene_out.as_deref()), ("a.hip", Some("g"), Some("x.json")));
    }
    #[test]
    fn open_missing_and_garbage_files_are_errors() {
        assert!(open_cmd(&s(&["/nonexistent/none.hip"])).is_err());
    }
}
