//! `.hip` reader (H40): Houdini scene file -> payload-free IR (`HipScene`). Clean-room: the container is a plain CPIO archive
//! (ASCII `odc` magic `070707`, or `newc` `070701`/`070702`), optionally gzip-wrapped (`.hip.gz`); `.hipnc` / `.hiplc` share the
//! layout. Every member is a text section: `<path>.init` (`type = <optype>`), `<path>.def` (flags + `inputs { idx src out flag }`),
//! `<path>.parm` (`name [ flags locks=0 ] ( v v v )`), `<path>.chn` (channels), `.start` (`tcur`, `fps`).
//! This module knows NO node semantics: mapping Houdini types / parms onto hexe nodes lives in `hexe-sop` (`hip_import`).
//! Hostile input is the default (AGENTS.md §4): every cap below is checked, every failure is `Err(HipError)`.
use std::collections::BTreeMap;
use std::fmt;
/// Largest archive / member / decompressed size accepted.
pub const MAX_HIP_BYTES: usize = 1 << 30;
/// Most members read from one archive.
pub const MAX_ENTRIES: usize = 1 << 20;
/// Why a `.hip` failed to read. Message only; never a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HipError(pub String);
impl fmt::Display for HipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "hip: {}", self.0)
    }
}
impl std::error::Error for HipError {}
fn err<T>(m: impl Into<String>) -> Result<T, HipError> {
    Err(HipError(m.into()))
}
/// One CPIO member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpioEntry {
    pub name: String,
    pub data: Vec<u8>,
}
fn field(b: &[u8], at: usize, len: usize, radix: u32) -> Result<usize, HipError> {
    let end = at.checked_add(len).ok_or_else(|| HipError("cpio: header overflow".into()))?;
    let s = b.get(at..end).ok_or_else(|| HipError("cpio: truncated header".into()))?;
    let s = std::str::from_utf8(s).map_err(|_| HipError("cpio: non-ascii header".into()))?;
    usize::from_str_radix(s, radix).map_err(|_| HipError(format!("cpio: bad header field `{s}`")))
}
/// Read a CPIO archive (odc or newc, detected by magic). Stops at `TRAILER!!!`.
pub fn read_cpio(b: &[u8]) -> Result<Vec<CpioEntry>, HipError> {
    if b.len() > MAX_HIP_BYTES {
        return err("archive too large");
    }
    let mut at = 0usize;
    let mut out = Vec::new();
    loop {
        // Old (H8-era) writers leave a stray newline / NUL after a member: tolerate up to 8 bytes of it.
        let mut skipped = 0;
        while skipped < 8 && matches!(b.get(at), Some(b'\n' | 0)) {
            at += 1;
            skipped += 1;
        }
        let magic = b.get(at..at.saturating_add(6)).ok_or_else(|| HipError("cpio: truncated (no TRAILER)".into()))?;
        let (hdr, newc) = match magic {
            b"070707" => (76usize, false),
            b"070701" | b"070702" => (110usize, true),
            b"MIME-V" if at == 0 => return err("MIME-encoded .hip (text/MIME wrapper) is not supported"),
            _ => return err(format!("not a cpio archive (magic {:?} at {at})", String::from_utf8_lossy(magic))),
        };
        let (namesize, filesize) = if newc {
            (field(b, at + 94, 8, 16)?, field(b, at + 54, 8, 16)?)
        } else {
            (field(b, at + 59, 6, 8)?, field(b, at + 65, 11, 8)?)
        };
        if namesize == 0 || filesize > MAX_HIP_BYTES {
            return err("cpio: bad name/file size");
        }
        let name_start = at + hdr;
        let name_end = name_start.checked_add(namesize).ok_or_else(|| HipError("cpio: overflow".into()))?;
        let raw = b.get(name_start..name_end).ok_or_else(|| HipError("cpio: truncated name".into()))?;
        let name = String::from_utf8_lossy(raw.split(|&c| c == 0).next().unwrap_or(raw)).into_owned();
        let data_at = if newc { (name_end + 3) & !3 } else { name_end };
        let data_end = data_at.checked_add(filesize).ok_or_else(|| HipError("cpio: overflow".into()))?;
        if name == "TRAILER!!!" {
            return Ok(out);
        }
        let data = b.get(data_at..data_end).ok_or_else(|| HipError(format!("cpio: truncated member `{name}`")))?;
        if out.len() >= MAX_ENTRIES {
            return err("cpio: too many members");
        }
        out.push(CpioEntry { name, data: data.to_vec() });
        at = if newc { (data_end + 3) & !3 } else { data_end };
    }
}
/// gzip (RFC 1952) -> raw bytes, capped at `MAX_HIP_BYTES`.
pub fn gunzip(b: &[u8]) -> Result<Vec<u8>, HipError> {
    if b.len() < 18 || b[0] != 0x1f || b[1] != 0x8b || b[2] != 8 {
        return err("gzip: bad header");
    }
    let flg = b[3];
    let mut at = 10usize;
    if flg & 4 != 0 {
        let xlen = usize::from(*b.get(at).unwrap_or(&0)) | (usize::from(*b.get(at + 1).unwrap_or(&0)) << 8);
        at = at.saturating_add(2).saturating_add(xlen);
    }
    for bit in [8u8, 16u8] {
        if flg & bit != 0 {
            let rest = b.get(at..).ok_or_else(|| HipError("gzip: truncated header".into()))?;
            let n = rest.iter().position(|&c| c == 0).ok_or_else(|| HipError("gzip: unterminated name".into()))?;
            at += n + 1;
        }
    }
    if flg & 2 != 0 {
        at = at.saturating_add(2);
    }
    let body = b.get(at..b.len() - 8).ok_or_else(|| HipError("gzip: truncated".into()))?;
    miniz_oxide::inflate::decompress_to_vec_with_limit(body, MAX_HIP_BYTES)
        .map_err(|e| HipError(format!("gzip: inflate failed ({:?})", e.status)))
}
/// One parameter component.
#[derive(Debug, Clone, PartialEq)]
pub enum HipValue {
    Num(f64),
    Str(String),
    /// `[ channel value ]`: keyframed / expression component; `value` = the value saved with the scene.
    Chan(String, Box<HipValue>),
}
impl HipValue {
    /// Value with the channel wrapper removed.
    pub fn plain(&self) -> &HipValue {
        match self {
            HipValue::Chan(_, v) => v.plain(),
            v => v,
        }
    }
    pub fn is_animated(&self) -> bool {
        matches!(self, HipValue::Chan(..))
    }
}
/// One `.parm` entry (a parm tuple: 1..n components).
#[derive(Debug, Clone, PartialEq)]
pub struct HipParm {
    pub name: String,
    pub flags: i32,
    pub values: Vec<HipValue>,
}
/// One node input: source node name (same network) + its output index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HipInput {
    pub src: String,
    pub output: usize,
}
/// One node of a network (`/obj/<net>/<name>`).
#[derive(Debug, Clone, PartialEq)]
pub struct HipNode {
    pub name: String,
    /// Operator type as saved (`box`, `attribcreate::2.0`, `jlait_foo`).
    pub ty: String,
    pub parms: Vec<HipParm>,
    /// Input slot -> source (`None` = unconnected slot). Length = highest listed slot + 1.
    pub inputs: Vec<Option<HipInput>>,
    pub display: bool,
    pub render: bool,
    pub bypass: bool,
    pub template: bool,
    /// Has a `.chn` section (some parm is keyframed / expression driven).
    pub has_channels: bool,
    /// Editor position (`position x y` of the `.def`).
    pub pos: Option<[f32; 2]>,
    /// Direct children exist (subnet etc.): NOT imported.
    pub has_children: bool,
}
/// One container under `/obj` (`geo`, `subnet`, `topnet`...) with its direct child nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct HipNetwork {
    pub name: String,
    pub ty: String,
    pub nodes: Vec<HipNode>,
}
/// A whole scene file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HipScene {
    pub networks: Vec<HipNetwork>,
    /// Saved playbar time (seconds) and fps (`.start`); frame = `tcur * fps + 1`.
    pub tcur: f64,
    pub fps: f64,
    /// `odc` | `newc`, and whether a gzip wrapper was removed.
    pub cpio: String,
    pub gzipped: bool,
    /// Sections found (diagnostics).
    pub members: usize,
}
impl HipScene {
    /// Playbar frame at save time (Houdini: `frame = time * fps + 1`).
    pub fn frame(&self) -> f64 {
        self.tcur * self.fps + 1.0
    }
}
/// Read a `.hip` / `.hipnc` / `.hiplc` / `.hip.gz` from memory.
pub fn read_hip(bytes: &[u8]) -> Result<HipScene, HipError> {
    let (data, gz) = if bytes.starts_with(&[0x1f, 0x8b]) { (gunzip(bytes)?, true) } else { (bytes.to_vec(), false) };
    let entries = read_cpio(&data)?;
    let mut scene = parse_entries(&entries)?;
    scene.gzipped = gz;
    scene.cpio = if data.starts_with(b"070707") { "odc".into() } else { "newc".into() };
    Ok(scene)
}
/// Parse already-extracted members (`name` -> text section) into the IR.
pub fn parse_entries(entries: &[CpioEntry]) -> Result<HipScene, HipError> {
    let map: BTreeMap<&str, &[u8]> = entries.iter().map(|e| (e.name.as_str(), e.data.as_slice())).collect();
    let text = |n: &str| map.get(n).map(|d| String::from_utf8_lossy(d).into_owned());
    let mut scene = HipScene { members: entries.len(), fps: 24.0, ..HipScene::default() };
    if let Some(s) = text(".start") {
        for line in s.lines() {
            let mut it = line.split_whitespace();
            match (it.next(), it.next()) {
                (Some("tcur"), Some(v)) => scene.tcur = v.parse().unwrap_or(0.0),
                (Some("fps"), Some(v)) => scene.fps = v.parse().unwrap_or(24.0),
                _ => {}
            }
        }
    }
    // Node paths = every `<path>.init`.
    let inits: Vec<&str> = map.keys().filter_map(|k| k.strip_suffix(".init")).collect();
    let has_children = |path: &str| {
        let pre = format!("{path}/");
        inits.iter().any(|p| p.starts_with(&pre))
    };
    let optype = |path: &str| -> String {
        text(&format!("{path}.init"))
            .and_then(|s| s.lines().find_map(|l| l.strip_prefix("type = ").map(|t| t.trim().to_string())))
            .unwrap_or_default()
    };
    for net_path in inits.iter().filter(|p| p.starts_with("obj/") && p.matches('/').count() == 1) {
        let name = net_path.trim_start_matches("obj/").to_string();
        let pre = format!("{net_path}/");
        let mut nodes = Vec::new();
        for p in inits.iter().filter(|p| p.starts_with(&pre) && !p[pre.len()..].contains('/')) {
            nodes.push(read_node(p, &pre, &optype(p), &map, has_children(p)));
        }
        scene.networks.push(HipNetwork { name, ty: optype(net_path), nodes });
    }
    Ok(scene)
}
fn read_node(path: &str, pre: &str, ty: &str, map: &BTreeMap<&str, &[u8]>, children: bool) -> HipNode {
    let get = |ext: &str| map.get(format!("{path}.{ext}").as_str()).map(|d| String::from_utf8_lossy(d).into_owned());
    let def = get("def").unwrap_or_default();
    let mut node = HipNode {
        name: path.get(pre.len()..).unwrap_or(path).to_string(),
        ty: ty.to_string(),
        parms: get("parm").map(|s| parse_parms(&s)).unwrap_or_default(),
        inputs: Vec::new(),
        display: false,
        render: false,
        bypass: false,
        template: false,
        has_channels: map.contains_key(format!("{path}.chn").as_str()),
        pos: None,
        has_children: children,
    };
    let mut in_inputs = false;
    for line in def.lines() {
        let t = line.trim();
        if in_inputs {
            if t == "}" {
                in_inputs = false;
            } else if t != "{" {
                let toks = tokens(t);
                if let (Some(idx), Some(src)) = (toks.first().and_then(|x| x.parse::<usize>().ok()), toks.get(1)) {
                    let out = toks.get(2).and_then(|x| x.parse().ok()).unwrap_or(0);
                    if idx < 4096 {
                        if node.inputs.len() <= idx {
                            node.inputs.resize(idx + 1, None);
                        }
                        if !src.is_empty() {
                            node.inputs[idx] = Some(HipInput { src: src.clone(), output: out });
                        }
                    }
                }
            }
        } else if t == "inputs" {
            in_inputs = true;
        } else if let Some(rest) = t.strip_prefix("flags =") {
            let toks: Vec<&str> = rest.split_whitespace().collect();
            for w in toks.chunks(2) {
                if let [k, v] = w {
                    let on = *v == "on";
                    match *k {
                        "display" => node.display = on,
                        "render" => node.render = on,
                        "bypass" => node.bypass = on,
                        "template" => node.template = on,
                        _ => {}
                    }
                }
            }
        } else if let Some(rest) = t.strip_prefix("position ") {
            let v: Vec<f32> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if let [x, y] = v[..] {
                node.pos = Some([x, y]);
            }
        }
    }
    node
}
/// Whitespace tokens with `"..."` honoured (quotes stripped, `\"` / `\\` unescaped).
fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = s.chars().peekable();
    while let Some(&c) = it.peek() {
        if c.is_whitespace() {
            it.next();
        } else if c == '"' {
            it.next();
            out.push(read_quoted(&mut it));
        } else {
            let mut w = String::new();
            while let Some(&c) = it.peek() {
                if c.is_whitespace() {
                    break;
                }
                w.push(c);
                it.next();
            }
            out.push(w);
        }
    }
    out
}
fn read_quoted(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut s = String::new();
    while let Some(c) = it.next() {
        match c {
            '"' => break,
            '\\' => match it.next() {
                Some('n') => s.push('\n'),
                Some('t') => s.push('\t'),
                Some(o) => s.push(o),
                None => break,
            },
            o => s.push(o),
        }
    }
    s
}
/// Parse a whole `.parm` section: `{ version 0.8 \n name [ flags locks=0 ] ( v ... ) ... }`.
pub fn parse_parms(text: &str) -> Vec<HipParm> {
    let mut out = Vec::new();
    let mut it = text.chars().peekable();
    let skip_ws = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
        while it.peek().is_some_and(|c| c.is_whitespace()) {
            it.next();
        }
    };
    loop {
        skip_ws(&mut it);
        let mut name = String::new();
        match it.peek() {
            None => break,
            Some('{') | Some('}') => {
                it.next();
                continue;
            }
            _ => {}
        }
        while let Some(&c) = it.peek() {
            if c.is_whitespace() {
                break;
            }
            name.push(c);
            it.next();
        }
        skip_ws(&mut it);
        if it.peek() != Some(&'[') {
            // `version 0.8` and other header lines: swallow the rest of the line.
            while it.peek().is_some_and(|&c| c != '\n') {
                it.next();
            }
            continue;
        }
        it.next();
        let mut flag_txt = String::new();
        for c in it.by_ref() {
            if c == ']' {
                break;
            }
            flag_txt.push(c);
        }
        let flags = flag_txt.split_whitespace().next().and_then(|x| x.parse().ok()).unwrap_or(0);
        skip_ws(&mut it);
        if it.peek() != Some(&'(') {
            continue;
        }
        it.next();
        let values = read_values(&mut it);
        out.push(HipParm { name, flags, values });
    }
    out
}
fn read_values(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Vec<HipValue> {
    let mut vals = Vec::new();
    loop {
        while it.peek().is_some_and(|c| c.is_whitespace()) {
            it.next();
        }
        match it.peek().copied() {
            None | Some(')') => {
                it.next();
                return vals;
            }
            Some('"') => {
                it.next();
                vals.push(HipValue::Str(read_quoted(it)));
            }
            Some('[') => {
                it.next();
                let mut inner = String::new();
                for c in it.by_ref() {
                    if c == ']' {
                        break;
                    }
                    inner.push(c);
                }
                let t = tokens(&inner);
                let ch = t.first().cloned().unwrap_or_default();
                let v = t.get(1).map_or(HipValue::Num(0.0), |s| scalar(s));
                vals.push(HipValue::Chan(ch, Box::new(v)));
            }
            Some(_) => {
                let mut w = String::new();
                while let Some(&c) = it.peek() {
                    if c.is_whitespace() || c == ')' {
                        break;
                    }
                    w.push(c);
                    it.next();
                }
                vals.push(scalar(&w));
            }
        }
    }
}
fn scalar(s: &str) -> HipValue {
    match s.parse::<f64>() {
        Ok(x) if x.is_finite() => HipValue::Num(x),
        _ => HipValue::Str(s.to_string()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn odc(name: &str, data: &[u8]) -> Vec<u8> {
        let mut o = Vec::new();
        let h = format!(
            "070707{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:011o}{:06o}{:011o}",
            1, 1, 0o644, 0, 0, 1, 0, 0, name.len() + 1, data.len()
        );
        o.extend_from_slice(h.as_bytes());
        o.extend_from_slice(name.as_bytes());
        o.push(0);
        o.extend_from_slice(data);
        o
    }
    fn newc(name: &str, data: &[u8]) -> Vec<u8> {
        let mut o = Vec::new();
        let h = format!(
            "070701{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
            1, 0o644, 0, 0, 1, 0, data.len(), 0, 0, 0, 0, name.len() + 1, 0
        );
        o.extend_from_slice(h.as_bytes());
        o.extend_from_slice(name.as_bytes());
        o.push(0);
        while o.len() % 4 != 0 {
            o.push(0);
        }
        o.extend_from_slice(data);
        while o.len() % 4 != 0 {
            o.push(0);
        }
        o
    }
    #[test]
    fn cpio_odc_and_newc_roundtrip() {
        let mut a = odc("obj/geo1/box1.init", b"type = box\n");
        a.extend(odc("TRAILER!!!", b""));
        let e = read_cpio(&a).unwrap();
        assert_eq!(e, vec![CpioEntry { name: "obj/geo1/box1.init".into(), data: b"type = box\n".to_vec() }]);
        let mut b = newc("a.txt", b"hello");
        b.extend(newc("b.txt", b"xy"));
        b.extend(newc("TRAILER!!!", b""));
        let e = read_cpio(&b).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!((e[0].name.as_str(), e[0].data.as_slice()), ("a.txt", &b"hello"[..]));
        assert_eq!((e[1].name.as_str(), e[1].data.as_slice()), ("b.txt", &b"xy"[..]));
    }
    #[test]
    fn cpio_hostile_input_is_err() {
        assert!(read_cpio(b"").is_err());
        assert!(read_cpio(b"not a cpio").is_err());
        let mut a = odc("x", b"abcdef");
        a.truncate(a.len() - 3);
        assert!(read_cpio(&a).is_err());
        assert!(read_cpio(&a[..40]).is_err());
        // stray newline between members (seen in Houdini 8 era files) is tolerated
        let mut c = odc("a", b"12");
        c.push(b'\n');
        c.extend(odc("TRAILER!!!", b""));
        assert_eq!(read_cpio(&c).unwrap().len(), 1);
        assert!(gunzip(b"\x1f\x8b\x08").is_err());
        assert!(read_hip(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]).is_err());
    }
    #[test]
    fn parm_section_parses_scalars_tuples_channels_and_strings() {
        let t = "{\nversion 0.8\ntype\t[ 0\tlocks=0 ]\t(\t\"poly\"\t)\nsize\t[ 0\tlocks=0 ]\t(\t1\t2.5\t3\t)\nt\t[ 8\tlocks=0 ]\t(\t0\t0\t[ tz\t0.5 ] \t)\ncopyg\t[ 0\tlocks=0 ]\t(\tcopyGroup`$CY`\t)\nsnip\t[ 0\tlocks=0 ]\t(\t\"a\\\"b\\nc\"\t)\n}\n";
        let p = parse_parms(t);
        assert_eq!(p.len(), 5);
        assert_eq!(p[0].values, vec![HipValue::Str("poly".into())]);
        assert_eq!(p[1].values, vec![HipValue::Num(1.0), HipValue::Num(2.5), HipValue::Num(3.0)]);
        assert_eq!(p[2].flags, 8);
        assert_eq!(p[2].values[2], HipValue::Chan("tz".into(), Box::new(HipValue::Num(0.5))));
        assert_eq!(p[3].values, vec![HipValue::Str("copyGroup`$CY`".into())]);
        assert_eq!(p[4].values, vec![HipValue::Str("a\"b\nc".into())]);
    }
    #[test]
    fn synthetic_scene_wires_and_flags() {
        let mut a = odc(".start", b"tcur 1\nfps 24\n");
        a.extend(odc("obj/geo1.init", b"type = geo\n"));
        a.extend(odc("obj/geo1/box1.init", b"type = box\n"));
        a.extend(odc("obj/geo1/box1.parm", b"{\nversion 0.8\nsize\t[ 0\tlocks=0 ]\t(\t2\t2\t2\t)\n}\n"));
        a.extend(odc("obj/geo1/box1.def", b"flags =  lock off display off render off\ninputs\n{\n}\n"));
        a.extend(odc("obj/geo1/xf.init", b"type = xform\n"));
        a.extend(odc("obj/geo1/xf.def", b"position 1 2\nflags =  lock off display on render on bypass on\ninputs\n{\n0 \t\"box1\" 0 1\n}\n"));
        a.extend(odc("TRAILER!!!", b""));
        let s = read_hip(&a).unwrap();
        assert_eq!(s.frame(), 25.0);
        let n = &s.networks[0].nodes;
        assert_eq!(s.networks[0].ty, "geo");
        let xf = n.iter().find(|x| x.name == "xf").unwrap();
        assert!(xf.display && xf.render && xf.bypass && !xf.template);
        assert_eq!(xf.inputs, vec![Some(HipInput { src: "box1".into(), output: 0 })]);
        assert_eq!(xf.pos, Some([1.0, 2.0]));
        assert_eq!(n.iter().find(|x| x.name == "box1").unwrap().parms[0].values.len(), 3);
    }
}
