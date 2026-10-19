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
/// One `.chn` channel: `channel tx { start = s  segment { length = L value = a b expr = ... } ... }`.
#[derive(Debug, Clone, PartialEq)]
pub struct HipChannel {
    pub name: String,
    /// Time (seconds) the first segment starts at (`start =`, default 0).
    pub start: f64,
    pub segments: Vec<HipSegment>,
}
/// One channel segment (`length` seconds; `length = 0` = the last, open-ended one).
#[derive(Debug, Clone, PartialEq)]
pub struct HipSegment {
    pub length: f64,
    /// `value = in out`: the key values at the segment's start / end.
    pub value: Option<[f64; 2]>,
    /// Expression text, escapes removed (`ch("../ctrl/width")`).
    pub expr: Option<String>,
    /// `language = python`.
    pub python: bool,
}
/// One node input: source node name (same network) + its output index. A network dot source (`(__dot1)` in the `.def`)
/// is stored by its bare name (`__dot1`, see [`HipNetwork::dots`]).
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
    /// The `.chn` channels (expressions / keys), by channel name (`tx`, `width`).
    pub channels: Vec<HipChannel>,
    /// Editor position (`position x y` of the `.def`).
    pub pos: Option<[f32; 2]>,
    /// Direct children exist (subnet etc.): NOT imported.
    pub has_children: bool,
    /// Hard lock flag (`flags = lock on`).
    pub lock: bool,
    /// User colour (`color UT_Color RGB r g b` with `colordefault off`); `None` = the type's default colour.
    pub color: Option<[f32; 3]>,
    /// Network box the node sits in (`<path>.netbox` names it, e.g. `__netbox2`).
    pub netbox: Option<String>,
}
/// A network box (`__netboxN.netboxinit`): titled coloured backdrop in the editor. Houdini units, y up.
#[derive(Debug, Clone, PartialEq)]
pub struct HipNetBox {
    pub name: String,
    /// Title shown in the box header.
    pub comment: String,
    pub color: [f32; 3],
    /// Lower-left corner.
    pub pos: [f32; 2],
    /// Width, height.
    pub size: [f32; 2],
    pub minimized: bool,
}
/// A network dot (`__dotN.networkdotinit`, flat JSON): a wire waypoint. Houdini units, y up; `pos` is the dot's centre.
#[derive(Debug, Clone, PartialEq)]
pub struct HipDot {
    pub name: String,
    pub pos: [f32; 2],
    pub pinned: bool,
    /// What feeds it (a node, or another dot by bare name); `None` = unwired.
    pub input: Option<HipInput>,
}
/// A sticky note (`__stickynoteN.postitdef`). Houdini units, y up.
#[derive(Debug, Clone, PartialEq)]
pub struct HipNote {
    pub name: String,
    pub text: String,
    pub color: [f32; 3],
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub minimized: bool,
}
/// One container under `/obj` (`geo`, `subnet`, `topnet`...) with its direct child nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct HipNetwork {
    pub name: String,
    /// Object-level path of the container holding this one (`""` = /obj itself, `"A/B"` = /obj/A/B).
    pub parent: String,
    pub ty: String,
    pub nodes: Vec<HipNode>,
    /// Object-level editor position (`/obj` network), Houdini units.
    pub pos: Option<[f32; 2]>,
    /// Object display flag (viewport visibility) and selectable flag.
    pub display: bool,
    pub selectable: bool,
    /// User colour of the object node (`None` = default).
    pub color: Option<[f32; 3]>,
    /// Network boxes and sticky notes inside the object's network.
    pub boxes: Vec<HipNetBox>,
    pub notes: Vec<HipNote>,
    /// Network dots inside the object's network (not in `nodes`: Houdini keeps them out of the node list too).
    pub dots: Vec<HipDot>,
}
/// A whole scene file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HipScene {
    pub networks: Vec<HipNetwork>,
    /// Saved playbar time (seconds) and fps (`.start`); frame = `tcur * fps + 1`.
    pub tcur: f64,
    pub fps: f64,
    /// Playbar range (`.start` `frange a b`): `$FSTART` / `$FEND`.
    pub frange: [f64; 2],
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
/// Section marker of the `HouNC` container (`.hipnc` / `.hiplc` saved by Apprentice / Indie builds).
const HOUNC: &[u8] = b"HouNC\x1a";
/// Opaque per-section header after the marker: 28 ASCII hex digits (not interpreted).
const HOUNC_HEAD: usize = 28;
/// Longest section name accepted.
const MAX_NAME: usize = 4096;
/// Read the `HouNC` container: a sequence of `HouNC\x1a` + 28 hex digits + `name` + NUL + plain-text body (up to the next
/// marker). The bodies are the same text sections a CPIO `.hip` holds. Nothing is decoded: the reader only splits on the
/// marker. A marker inside a body is recognised only when followed by 28 hex digits; `NON_COMMERCIAL_FILE!!!` sections
/// (embedded operator dialog scripts) are skipped.
pub fn read_hounc(b: &[u8]) -> Result<Vec<CpioEntry>, HipError> {
    if b.len() > MAX_HIP_BYTES {
        return err("archive too large");
    }
    if !b.starts_with(HOUNC) {
        return err("hounc: missing HouNC marker");
    }
    let is_head = |at: usize| {
        b.get(at..at + HOUNC.len()) == Some(HOUNC)
            && b.get(at + HOUNC.len()..at + HOUNC.len() + HOUNC_HEAD)
                .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit))
    };
    // section starts: every marker followed by a well-formed header
    let mut starts = Vec::new();
    let mut i = 0;
    while let Some(off) = b.get(i..).and_then(|r| r.windows(HOUNC.len()).position(|w| w == HOUNC)) {
        let at = i + off;
        if at == 0 || is_head(at) {
            starts.push(at);
            if starts.len() > MAX_ENTRIES {
                return err("hounc: too many sections");
            }
        }
        i = at + HOUNC.len();
    }
    let mut out = Vec::new();
    for (k, &at) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(b.len());
        let sec = b.get(at + HOUNC.len()..end).unwrap_or_default();
        if sec.is_empty() {
            continue; // bare trailing marker
        }
        let head = sec.get(..HOUNC_HEAD).ok_or_else(|| HipError("hounc: truncated section header".into()))?;
        if !head.iter().all(u8::is_ascii_hexdigit) {
            return err("hounc: bad section header");
        }
        let rest = &sec[HOUNC_HEAD..];
        let nul = rest
            .iter()
            .take(MAX_NAME + 1)
            .position(|&c| c == 0)
            .ok_or_else(|| HipError("hounc: unterminated or overlong section name".into()))?;
        let name = String::from_utf8_lossy(&rest[..nul]).into_owned();
        if name.ends_with("NON_COMMERCIAL_FILE!!!") {
            continue;
        }
        out.push(CpioEntry { name, data: rest[nul + 1..].to_vec() });
    }
    Ok(out)
}
/// Read a `.hip` / `.hipnc` / `.hiplc` / `.hip.gz` from memory (CPIO, gzip-wrapped CPIO, or the `HouNC` container).
pub fn read_hip(bytes: &[u8]) -> Result<HipScene, HipError> {
    if bytes.starts_with(HOUNC) {
        let entries = read_hounc(bytes)?;
        let mut scene = parse_entries(&entries)?;
        scene.cpio = "hounc".into();
        return Ok(scene);
    }
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
    let mut scene = HipScene { members: entries.len(), fps: 24.0, frange: [1.0, 240.0], ..HipScene::default() };
    if let Some(s) = text(".start") {
        for line in s.lines() {
            let mut it = line.split_whitespace();
            match (it.next(), it.next()) {
                (Some("tcur"), Some(v)) => scene.tcur = v.parse().unwrap_or(0.0),
                (Some("fps"), Some(v)) => scene.fps = v.parse().unwrap_or(24.0),
                (Some("frange"), Some(a)) => {
                    let b = it.next().and_then(|b| b.parse().ok());
                    if let (Ok(a), Some(b)) = (a.parse::<f64>(), b)
                        && a.is_finite()
                        && f64::is_finite(b)
                    {
                        scene.frange = [a, b];
                    }
                }
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
    // containers = everything directly under /obj, plus `geo` objects nested in object-level subnets / digital assets
    // (`obj/asset1/inner_geo1`; a `geo` only ever lives at object level, never inside a SOP network) and every
    // object-level container on the way down to one (so the editor can rebuild the /obj tree)
    let geo_paths: Vec<&str> = inits.iter().copied().filter(|p| p.starts_with("obj/") && optype(p) == "geo").collect();
    let holds_geo = |p: &str| geo_paths.iter().any(|g| g.starts_with(&format!("{p}/")));
    for net_path in inits
        .iter()
        .filter(|p| p.starts_with("obj/") && (p.matches('/').count() == 1 || optype(p) == "geo" || holds_geo(p)))
    {
        let name = net_path.rsplit('/').next().unwrap_or_default().to_string();
        let pre = format!("{net_path}/");
        let mut nodes = Vec::new();
        for p in inits.iter().filter(|p| p.starts_with(&pre) && !p[pre.len()..].contains('/')) {
            nodes.push(read_node(p, &pre, &optype(p), &map, has_children(p)));
        }
        // the object node's own `.def`: position, display / selectable flags, colour
        let def = text(&format!("{net_path}.def")).unwrap_or_default();
        let fl = def_flags(&def);
        let flag = |k: &str, dflt: bool| fl.get(k).copied().unwrap_or(dflt);
        let mut net = HipNetwork {
            // path between /obj and this container ("" = directly under /obj; "asset1" for the nested geo)
            parent: net_path
                .trim_start_matches("obj/")
                .rsplit_once('/')
                .map(|(p, _)| p.to_string())
                .unwrap_or_default(),
            name,
            ty: optype(net_path),
            nodes,
            pos: def_position(&def),
            display: flag("display", true),
            selectable: flag("selectable", true),
            color: if flag("colordefault", true) { None } else { def_color(&def) },
            boxes: Vec::new(),
            notes: Vec::new(),
            dots: Vec::new(),
        };
        // network boxes + sticky notes: direct children of the network with their own section kinds
        for (k, body) in map.iter().filter(|(k, _)| k.starts_with(&pre) && !k[pre.len()..].contains('/')) {
            let body = String::from_utf8_lossy(body);
            if let Some(n) = k[pre.len()..].strip_suffix(".netboxinit") {
                net.boxes.push(read_netbox(n, &body));
            } else if let Some(n) = k[pre.len()..].strip_suffix(".postitdef") {
                net.notes.push(read_note(n, &body));
            } else if let Some(n) = k[pre.len()..].strip_suffix(".networkdotinit") {
                net.dots.push(read_dot(n, &body));
            }
        }
        scene.networks.push(net);
    }
    Ok(scene)
}
/// `flags =  lock off display on ...` of a `.def` -> key -> on.
fn def_flags(def: &str) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    for line in def.lines() {
        if let Some(rest) = line.trim().strip_prefix("flags =") {
            let toks: Vec<&str> = rest.split_whitespace().collect();
            for w in toks.chunks(2) {
                if let [k, v] = w {
                    out.insert((*k).to_string(), *v == "on");
                }
            }
        }
    }
    out
}
/// `position x y` (finite only).
fn def_position(def: &str) -> Option<[f32; 2]> {
    def.lines().find_map(|l| l.trim().strip_prefix("position ")).and_then(|rest| {
        let v: Vec<f32> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        match v[..] {
            [x, y] if x.is_finite() && y.is_finite() => Some([x, y]),
            _ => None,
        }
    })
}
/// `UT_Color RGB r g b` anywhere in `s` -> rgb clamped to [0, 1] (finite only).
fn parse_rgb(s: &str) -> Option<[f32; 3]> {
    let rest = s.split("UT_Color RGB").nth(1)?;
    let v: Vec<f32> = rest
        .split(|c: char| c.is_whitespace() || c == '"' || c == ';')
        .filter_map(|x| x.parse().ok())
        .take(3)
        .collect();
    match v[..] {
        [r, g, b] if [r, g, b].iter().all(|x| x.is_finite()) => {
            Some([r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)])
        }
        _ => None,
    }
}
/// The `color UT_Color RGB ...` line of a `.def` / `.postitdef`.
fn def_color(def: &str) -> Option<[f32; 3]> {
    def.lines().find(|l| l.trim().starts_with("color ")).and_then(parse_rgb)
}
/// `key := value;` fields of a `.netboxinit` body.
fn kv_fields(body: &str) -> BTreeMap<String, String> {
    body.split(';')
        .filter_map(|f| f.split_once(":="))
        .map(|(k, v)| {
            // the key is the last word before `:=` (the first field also carries the `2\n2\n{` preamble)
            let k = k
                .split(|c: char| c.is_whitespace() || c == '{')
                .rfind(|w| !w.is_empty())
                .unwrap_or_default()
                .to_string();
            (k, v.trim().trim_matches('"').to_string())
        })
        .collect()
}
fn read_netbox(name: &str, body: &str) -> HipNetBox {
    let f = kv_fields(body);
    let num = |k: &str, d: f32| f.get(k).and_then(|v| v.parse::<f32>().ok()).filter(|x| x.is_finite()).unwrap_or(d);
    HipNetBox {
        name: name.to_string(),
        comment: f.get("comment").cloned().unwrap_or_default(),
        color: f.get("color").and_then(|c| parse_rgb(c)).unwrap_or([0.52, 0.52, 0.52]),
        pos: [num("x_position", 0.0), num("y_position", 0.0)],
        size: [num("width", 1.0).max(0.0), num("height", 1.0).max(0.0)],
        minimized: f.get("minimized").is_some_and(|v| v.trim() == "1"),
    }
}
fn read_note(name: &str, body: &str) -> HipNote {
    let mut note = HipNote {
        name: name.to_string(),
        text: String::new(),
        color: [1.0, 0.969, 0.522],
        pos: [0.0, 0.0],
        size: [2.5, 2.5],
        minimized: false,
    };
    let pair = |rest: &str| -> Option<[f32; 2]> {
        let v: Vec<f32> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        match v[..] {
            [a, b] if a.is_finite() && b.is_finite() => Some([a, b]),
            _ => None,
        }
    };
    for line in body.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("text ") {
            note.text = tokens(rest).into_iter().next().unwrap_or_default();
        } else if let Some(p) = t.strip_prefix("position ").and_then(pair) {
            note.pos = p;
        } else if let Some(s) = t.strip_prefix("size ").and_then(pair) {
            note.size = [s[0].max(0.0), s[1].max(0.0)];
        } else if t.starts_with("color ") {
            if let Some(c) = parse_rgb(t) {
                note.color = c;
            }
        } else if let Some(rest) = t.strip_prefix("flags =") {
            note.minimized = rest.split_whitespace().collect::<Vec<_>>().chunks(2).any(|w| w == ["minimize", "on"]);
        }
    }
    note
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
        channels: get("chn").map(|s| parse_channels(&s)).unwrap_or_default(),
        pos: None,
        has_children: children,
        lock: false,
        color: None,
        netbox: get("netbox").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
    };
    let fl = def_flags(&def);
    node.lock = fl.get("lock").copied().unwrap_or(false);
    if !fl.get("colordefault").copied().unwrap_or(true) {
        node.color = def_color(&def);
    }
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
                        let src = dot_ref(src);
                        if !src.is_empty() {
                            node.inputs[idx] = Some(HipInput { src: src.to_string(), output: out });
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
/// `(__dot1)` (how a `.def` / dot input names a network dot) -> `__dot1`; node names pass through.
fn dot_ref(src: &str) -> &str {
    src.strip_prefix('(').and_then(|s| s.strip_suffix(')')).unwrap_or(src)
}
/// `"key": value` pairs of a FLAT JSON object (the `.networkdotinit` body): string values unquoted / unescaped, other
/// values as their raw token. Nested values and anything malformed are skipped, never an error.
fn flat_json(body: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut it = body.chars().peekable();
    while let Some(c) = it.next() {
        if c != '"' {
            continue;
        }
        let key = read_quoted(&mut it);
        while it.peek().is_some_and(|c| c.is_whitespace()) {
            it.next();
        }
        if it.peek() != Some(&':') {
            continue;
        }
        it.next();
        while it.peek().is_some_and(|c| c.is_whitespace()) {
            it.next();
        }
        let value = if it.peek() == Some(&'"') {
            it.next();
            read_quoted(&mut it)
        } else {
            let mut v = String::new();
            while let Some(&c) = it.peek() {
                if c == ',' || c == '}' || c == '\n' {
                    break;
                }
                v.push(c);
                it.next();
            }
            v.trim().to_string()
        };
        out.insert(key, value);
    }
    out
}
fn read_dot(name: &str, body: &str) -> HipDot {
    let f = flat_json(body);
    let num = |k: &str| f.get(k).and_then(|v| v.parse::<f32>().ok()).filter(|x| x.is_finite()).unwrap_or(0.0);
    // `"input": "box1 0 1"` = source, its output index, a flag; `""` = unwired
    let toks = f.get("input").map(|s| tokens(s)).unwrap_or_default();
    let input = toks
        .first()
        .map(|s| dot_ref(s))
        .filter(|s| !s.is_empty())
        .map(|src| HipInput { src: src.to_string(), output: toks.get(1).and_then(|x| x.parse().ok()).unwrap_or(0) });
    HipDot {
        name: name.to_string(),
        pos: [num("posx"), num("posy")],
        pinned: f.get("pinned").is_some_and(|v| v == "true"),
        input,
    }
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
/// Most channels / segments read from one `.chn` section (hostile-input cap).
const MAX_CHANNELS: usize = 4096;
/// Byte index just past the `}` closing the `{` at `open` (braces inside `"..."` ignored); `None` = unbalanced.
fn close_brace(b: &[u8], open: usize) -> Option<usize> {
    let (mut depth, mut i, mut quoted) = (0usize, open, false);
    while let Some(&c) = b.get(i) {
        match c {
            // `\"` escapes a quote inside AND outside strings (bare `.chn` exprs: `ch(\"../a/b\")`)
            b'\\' => i += 1,
            b'"' => quoted = !quoted,
            b'{' if !quoted => depth += 1,
            b'}' if !quoted => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}
/// `key = number` inside a segment / channel body.
fn field_num(body: &str, key: &str) -> Option<f64> {
    let at = body.find(&format!("{key} = "))? + key.len() + 3;
    body.get(at..)?.split_whitespace().next()?.parse().ok().filter(|x: &f64| x.is_finite())
}
/// The `expr = ...` of a segment: a quoted string (escapes removed) or bare text up to the segment's closing brace.
fn segment_expr(body: &str) -> Option<String> {
    let at = body.find("expr = ")? + 7;
    let rest = body.get(at..)?;
    if let Some(q) = rest.strip_prefix('"') {
        let mut s = String::new();
        let mut it = q.chars();
        while let Some(c) = it.next() {
            match c {
                '\\' => match it.next() {
                    Some('n') => s.push('\n'),
                    Some(o) => s.push(o),
                    None => break,
                },
                '"' => return Some(s),
                o => s.push(o),
            }
        }
        return Some(s);
    }
    // bare: up to `language =` or the end of the body (the closing brace is not part of `body`)
    let end = rest.find(" language = ").unwrap_or(rest.len());
    Some(rest[..end].trim().replace("\\\"", "\"").replace("\\\\", "\\"))
}
/// Parse a `.chn` section into channels (unbalanced / oversized input stops early, never panics).
pub fn parse_channels(text: &str) -> Vec<HipChannel> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(k) = text.get(from..).and_then(|t| t.find("channel ")).map(|k| k + from) {
        let Some(open) = text[k..].find('{').map(|o| o + k) else { break };
        let name = text[k + 8..open].trim().to_string();
        let Some(close) = close_brace(b, open) else { break };
        let body = &text[open + 1..close - 1];
        let mut ch = HipChannel {
            name,
            start: field_num(body.split("segment").next().unwrap_or(""), "start").unwrap_or(0.0),
            segments: Vec::new(),
        };
        let bb = body.as_bytes();
        let mut s = 0usize;
        while let Some(j) = body.get(s..).and_then(|t| t.find("segment {")).map(|j| j + s) {
            let Some(end) = close_brace(bb, j + 8) else { break };
            let seg = &body[j + 9..end - 1];
            // drop a nested `options = { ... }` block before reading fields
            let seg_clean =
                match seg.find("options = {").and_then(|o| close_brace(seg.as_bytes(), o + 10).map(|e| (o, e))) {
                    Some((o, e)) => format!("{} {}", &seg[..o], &seg[e..]),
                    None => seg.to_string(),
                };
            let value = seg_clean.find("value = ").and_then(|v| {
                let mut it =
                    seg_clean[v + 8..].split_whitespace().map(|x| x.parse::<f64>().ok().filter(|x| x.is_finite()));
                Some([it.next()??, it.next()??])
            });
            ch.segments.push(HipSegment {
                length: field_num(&seg_clean, "length").unwrap_or(0.0).max(0.0),
                value,
                expr: segment_expr(&seg_clean),
                python: seg_clean.contains("language = python"),
            });
            s = end;
            if ch.segments.len() >= MAX_CHANNELS {
                break;
            }
        }
        out.push(ch);
        from = close;
        if out.len() >= MAX_CHANNELS {
            break;
        }
    }
    out
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
            1,
            1,
            0o644,
            0,
            0,
            1,
            0,
            0,
            name.len() + 1,
            data.len()
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
            1,
            0o644,
            0,
            0,
            1,
            0,
            data.len(),
            0,
            0,
            0,
            0,
            name.len() + 1,
            0
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
    fn chn_section_reads_expressions_keys_and_python() {
        // shapes Houdini 20.5 writes: keyed segments with `options = { }`, bare / quoted / python exprs
        let text = "{\n    channel sizex {\n      lefttype = extend\n      default = 1.5\n      segment { options = {\t autoslope  ao }\n\n\t length = 5.5 value = 1.5 0 accel = 0 1 expr = ch(\\\"../ctrl/width\\\") }\n      segment { options = {\t autoslope  ai }\n\n\t length = 0 value = 0 1.5 expr = ch(\\\"../ctrl/width\\\") }\n    }\n    channel tx {\n      segment { length = 0 value = 0.8 0.8 expr = ch(\\\"../ctrl/width\\\")/2+ch(\\\"../ctrl/depth\\\")/2 }\n    }\n    channel ty {\n      segment { length = 0 expr = \"1 / __import__('loputils').getMetersPerUnit(pwd())\" language = python }\n    }\n    channel tz {\n      segment { length = 0 expr = \"ifs(!strcmp(chs(\\\"aperture\\\"), \\\"setratio\\\"), \\\"set\\\", chs(\\\"aperture\\\"))\" }\n    }\n  }";
        let c = parse_channels(text);
        assert_eq!(c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["sizex", "tx", "ty", "tz"]);
        assert_eq!(c[0].segments.len(), 2);
        assert!((c[0].segments[0].length - 5.5).abs() < 1e-12);
        assert_eq!(c[0].segments[0].value, Some([1.5, 0.0]));
        assert_eq!(c[0].segments[1].expr.as_deref(), Some("ch(\"../ctrl/width\")"));
        assert_eq!(c[1].segments[0].expr.as_deref(), Some("ch(\"../ctrl/width\")/2+ch(\"../ctrl/depth\")/2"));
        assert!(c[2].segments[0].python);
        assert_eq!(
            c[3].segments[0].expr.as_deref(),
            Some("ifs(!strcmp(chs(\"aperture\"), \"setratio\"), \"set\", chs(\"aperture\"))")
        );
        // hostile: unbalanced / truncated text stops without panicking
        for t in [
            "channel x {",
            "channel x { segment { expr = \"abc",
            "channel",
            "{ channel {} }",
            "channel x { segment { options = { }",
        ] {
            let _ = parse_channels(t);
        }
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
        a.extend(odc(
            "obj/geo1/xf.def",
            b"position 1 2\nflags =  lock off display on render on bypass on\ninputs\n{\n0 \t\"box1\" 0 1\n}\n",
        ));
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
    #[test]
    fn geo_nested_in_an_object_subnet_is_found() {
        // an object-level asset /obj/asset1 holding the geo with the SOPs
        let mut a = odc("obj/asset1.init", b"type = myasset\n");
        a.extend(odc("obj/asset1/inner_geo1.init", b"type = geo\n"));
        a.extend(odc("obj/asset1/inner_geo1/sphere1.init", b"type = sphere\n"));
        a.extend(odc("obj/asset1/inner_geo1/group1.init", b"type = groupcreate\n"));
        a.extend(odc("obj/asset1/inner_geo1/group1.def", b"inputs\n{\n0 \t\"sphere1\" 0 1\n}\n"));
        a.extend(odc("TRAILER!!!", b""));
        let s = read_hip(&a).unwrap();
        let geo = s.networks.iter().find(|n| n.ty == "geo").expect("nested geo found");
        assert_eq!((geo.name.as_str(), geo.parent.as_str()), ("inner_geo1", "asset1"));
        let mut names: Vec<_> = geo.nodes.iter().map(|n| n.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["group1", "sphere1"], "its own SOPs only");
        // the asset itself is still listed (as a non-geo container) and does not swallow the geo's SOPs
        let outer = s.networks.iter().find(|n| n.name == "asset1").expect("outer listed");
        assert_eq!(outer.parent, "", "directly under /obj");
        assert_eq!(outer.nodes.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["inner_geo1"]);
    }
    /// A `HouNC` section: marker, 28 hex chars (opaque header, not interpreted), `name`, NUL, plain-text body.
    fn nc(name: &str, body: &[u8]) -> Vec<u8> {
        let mut v = b"HouNC\x1a1033600bb006ad476a80368c8c22".to_vec();
        v.extend_from_slice(name.as_bytes());
        v.push(0);
        v.extend_from_slice(body);
        v
    }
    #[test]
    fn hounc_container_reads_plain_sections() {
        let mut a = nc(".start", b"tcur 0\nfps 24\n");
        a.extend(nc("obj/geo1.init", b"type = geo\n"));
        a.extend(nc("obj/geo1.def", b"position -9.5 2.5\nflags =  lock off display on selectable off colordefault on\ncolor UT_Color RGB 0.8 0.8 0.8 \n"));
        a.extend(nc("obj/geo1/box1.init", b"type = box\n"));
        // a marker-like string NOT followed by 28 hex chars is body text, not a new section
        a.extend(nc("obj/geo1/box1.def", b"comment \"HouNC\x1a not a section\"\nposition 1 2\nflags =  lock on display on bypass off colordefault off\ncolor UT_Color RGB 0.9 0.5 0 \n"));
        a.extend(nc("obj/geo1/box1.netbox", b"__netbox2\n"));
        a.extend(nc("obj/geo1/__netbox2.netboxinit", b"2\n2\n{\n\tcolor := \"UT_Color RGB 0.584 0.776 1 \";\n\tcomment := \"Container Wall\";\n\theight := 11.5;\n\tminimized := 0;\n\twidth := 7.5;\n\tx_position := -8.75;\n\ty_position := 1.25;\n}\n"));
        a.extend(nc("obj/geo1/__stickynote1.postitinit", b"type = postitnote\n"));
        a.extend(nc("obj/geo1/__stickynote1.postitdef", b"text \"hello\"\nposition 6 -39\nsize 2.5 2.5\nflags = minimize off\ncolor UT_Color RGB 1 0.969 0.522 \nend\n"));
        a.extend(nc("NON_COMMERCIAL_FILE!!!", b"# Dialog script for foo\n"));
        let s = read_hip(&a).unwrap();
        assert_eq!(s.cpio, "hounc");
        let g = &s.networks[0];
        assert_eq!(
            (g.name.as_str(), g.pos, g.display, g.selectable, g.color),
            ("geo1", Some([-9.5, 2.5]), true, false, None)
        );
        let b = &g.nodes[0];
        assert_eq!(
            (b.lock, b.color, b.netbox.as_deref(), b.pos),
            (true, Some([0.9, 0.5, 0.0]), Some("__netbox2"), Some([1.0, 2.0]))
        );
        assert_eq!(g.boxes.len(), 1);
        let x = &g.boxes[0];
        assert_eq!(
            (x.name.as_str(), x.comment.as_str(), x.pos, x.size, x.minimized),
            ("__netbox2", "Container Wall", [-8.75, 1.25], [7.5, 11.5], false)
        );
        assert_eq!(x.color, [0.584, 0.776, 1.0]);
        assert_eq!(g.notes.len(), 1);
        assert_eq!((g.notes[0].text.as_str(), g.notes[0].pos, g.notes[0].size), ("hello", [6.0, -39.0], [2.5, 2.5]));
    }
    #[test]
    fn hounc_hostile_input_is_err_or_empty() {
        assert!(read_hip(b"HouNC\x1a").unwrap().networks.is_empty());
        assert!(read_hip(b"HouNC\x1a1033600bb0").is_err()); // header cut short
        assert!(read_hip(b"HouNC\x1azz33600bb006ad476a80368c8c22x.init\0type = box\n").is_err()); // non-hex header
        let mut no_nul = b"HouNC\x1a1033600bb006ad476a80368c8c22".to_vec();
        no_nul.extend(std::iter::repeat_n(b'a', 5000));
        assert!(read_hip(&no_nul).is_err()); // name never terminated (capped)
        // garbage flags / colours / box fields never panic
        let mut a = nc("obj/g.init", b"type = geo\n");
        a.extend(nc("obj/g/n.init", b"type = null\n"));
        a.extend(nc("obj/g/n.def", b"position x y\nflags = display\ncolor UT_Color RGB nan 1\n"));
        a.extend(nc("obj/g/__netbox1.netboxinit", b"{ width := ; x_position := inf; color := \"junk\"; }"));
        let s = read_hip(&a).unwrap();
        let n = &s.networks[0].nodes[0];
        assert_eq!((n.pos, n.color), (None, None));
        assert!(s.networks[0].boxes[0].size.iter().chain(s.networks[0].boxes[0].pos.iter()).all(|v| v.is_finite()));
    }

    #[test]
    fn network_dots_parse_and_dot_sources_lose_their_parens() {
        let e = |name: &str, data: &str| CpioEntry { name: name.into(), data: data.as_bytes().to_vec() };
        let entries = vec![
            e("obj/geo1.init", "type = geo\n"),
            e("obj/geo1/box1.init", "type = box\n"),
            e("obj/geo1/xform1.init", "type = xform\n"),
            e("obj/geo1/xform1.def", "inputs\n{\n0 \t(__dot1) 0 1\n}\n"),
            e(
                "obj/geo1/__dot1.networkdotinit",
                "{\n\t\"version\":1,\n\t\"posx\":0.5,\n\t\"posy\":-2,\n\t\"pinned\":false,\n\t\"color\":\"FROMOUTPUT\",\n\t\"input\":\"box1 0 1\"\n}\n",
            ),
            e("obj/geo1/__dot2.networkdotinit", "{\"posx\":2,\"posy\":-3,\"pinned\":true,\"input\":\"(__dot1) 0 1\"}"),
            // hostile: garbage, non-finite position, unwired
            e("obj/geo1/__dot3.networkdotinit", "{\"posx\":\"nan\",\"posy\":inf,\"input\":\"\"} trailing ] \" junk"),
            e("obj/geo1/__dot4.networkdotinit", "not json at all"),
        ];
        let s = parse_entries(&entries).expect("parses");
        let net = &s.networks[0];
        assert_eq!(net.nodes.len(), 2, "dots are not nodes");
        let xf = net.nodes.iter().find(|n| n.name == "xform1").expect("xform1");
        assert_eq!(xf.inputs, vec![Some(HipInput { src: "__dot1".into(), output: 0 })]);
        let mut dots = net.dots.clone();
        dots.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<&str> = dots.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["__dot1", "__dot2", "__dot3", "__dot4"]);
        assert_eq!(
            dots[0],
            HipDot {
                name: "__dot1".into(),
                pos: [0.5, -2.0],
                pinned: false,
                input: Some(HipInput { src: "box1".into(), output: 0 })
            }
        );
        assert_eq!(dots[1].input, Some(HipInput { src: "__dot1".into(), output: 0 }));
        assert!(dots[1].pinned);
        assert_eq!((dots[2].pos, dots[2].input.clone()), ([0.0, 0.0], None));
        assert_eq!((dots[3].pos, dots[3].pinned, dots[3].input.clone()), ([0.0, 0.0], false, None));
    }
}
