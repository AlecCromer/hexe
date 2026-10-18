//! File SOP (`sop-file`): read geometry from disk via `hexe_geo::read_any` (`.geo .bgeo .bgeo.sc .obj .ply`).
//! Params: `file` S (relative paths: the scene file's directory when loaded via `load_scene_in` and the file exists there,
//! else the cwd); `missing` Menu 0 = error, 1 = no_geometry (empty output when the file does not exist; a file that exists
//! but fails to parse is ALWAYS an error). The node keeps its last read keyed by (path, mtime, size): a recook after the
//! `file` param changes re-reads; a recook with an untouched file reuses the parse. The graph itself only recooks a dirty
//! node, so a file edited on disk is picked up on the next recook, not live (UNVERIFIED vs Houdini's file watcher).
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("file", ParamValue::S(String::new())).label("Geometry File"),
        ParamSpec::new("endo_time", ParamValue::F(-1.0)).label("Endorphin Time (s, <0 = $T)").range(-1.0, 600.0),
        ParamSpec::new("missing", ParamValue::Menu(0))
            .label("Missing File")
            .menu(&[("error", "Report Error"), ("no_geometry", "No Geometry")]),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(FileSop { specs: spec(), cache: Mutex::new(None), endo: Mutex::new(None), keyed: AtomicBool::new(false) })
}

type Key = (String, Option<SystemTime>, u64);
/// Parsed Endorphin document + the last sampled detail `(time bits, detail)`, so a timeline scrub only re-poses.
type EndoSlot = (Key, Arc<hexe_geo::endorphin::Document>, Option<(u32, Arc<Detail>)>);
struct FileSop {
    specs: Vec<ParamSpec>,
    cache: Mutex<Option<(Key, Arc<Detail>)>>,
    endo: Mutex<Option<EndoSlot>>,
    /// Set once an Endorphin file was cooked: only then does the output depend on `ctx.time`.
    keyed: AtomicBool,
}

impl Node<Detail> for FileSop {
    fn type_name(&self) -> &'static str {
        "file"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn time_dependent(&self) -> bool {
        self.keyed.load(Ordering::Relaxed)
    }
    fn cook(&self, ctx: &CookCtx, _: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let path =
            ctx.params.string("file").ok_or_else(|| CookError::BadParam("`file` missing or not a string".into()))?;
        let missing = ctx.params.menu("missing").unwrap_or(0);
        if missing > 1 {
            return Err(CookError::BadParam(format!("`missing` menu index {missing} (0 error, 1 no_geometry)")));
        }
        let meta = match std::fs::metadata(path) {
            Ok(m) if !path.is_empty() => m,
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(CookError::Failed(format!("{path}: {e}")));
            }
            _ if missing == 1 => return Ok(Arc::new(Detail::default())),
            _ => return Err(CookError::Failed(format!("file not found: `{path}`"))),
        };
        let key: Key = (path.to_string(), meta.modified().ok(), meta.len());
        if hexe_geo::is_endorphin_path(path) {
            self.keyed.store(true, Ordering::Relaxed);
            let endo_time = ctx.params.float("endo_time").unwrap_or(-1.0);
            let t = if endo_time < 0.0 { ctx.time } else { endo_time };
            let t = if t.is_finite() { t.max(0.0) } else { 0.0 };
            let mut slot = self.endo.lock().map_err(|_| CookError::Failed("endorphin cache poisoned".into()))?;
            if slot.as_ref().is_none_or(|(k, ..)| *k != key) {
                let doc = hexe_geo::read_endorphin_file(path).map_err(|e| CookError::Failed(e.to_string()))?;
                *slot = Some((key, Arc::new(doc), None));
            }
            let Some((_, doc, last)) = slot.as_mut() else {
                return Err(CookError::Failed("endorphin cache empty".into()));
            };
            if let Some((bits, d)) = last
                && *bits == t.to_bits()
            {
                return Ok(Arc::clone(d));
            }
            let mut d = hexe_geo::endorphin_geo::document_detail_at(doc, t);
            // downstream `endosim` finds the scene again through this path
            d.add_attr(Attribute {
                name: "endo_file".into(),
                class: AttrClass::Detail,
                data: AttrData::Str { idx: vec![0], table: vec![path.to_string()] },
                type_info: TypeInfo::None,
            });
            let d = Arc::new(d);
            *last = Some((t.to_bits(), Arc::clone(&d)));
            return Ok(d);
        }
        if let Ok(slot) = self.cache.lock()
            && let Some((k, d)) = slot.as_ref()
            && *k == key
        {
            return Ok(Arc::clone(d));
        }
        let d = Arc::new(hexe_geo::read_any(path).map_err(|e| CookError::Failed(e.to_string()))?);
        if let Ok(mut slot) = self.cache.lock() {
            *slot = Some((key, Arc::clone(&d)));
        }
        Ok(d)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use crate::{load_scene, load_scene_in};
    use std::path::PathBuf;

    pub fn scratch(name: &str) -> PathBuf {
        let dir = std::env::var("HEXE_SCRATCH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/hexe-scratch"));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir.join(name)
    }
    fn scene(file: &str, missing: u32) -> String {
        format!(
            r#"{{"nodes":[{{"name":"f","type":"file","params":{{"file":{file:?},"missing":{missing}}}}}],"display":"f"}}"#
        )
    }
    fn counts(json: &str) -> Result<(u32, u32, u32), String> {
        let (mut g, id) = load_scene(json).map_err(|e| e.to_string())?;
        let d = g.cook(id, 0.0).map_err(|e| e.to_string())?;
        Ok((d.point_count, d.vertex_count(), d.prim_count()))
    }

    /// Oracle: Houdini-shipped `.bgeo` (binary legacy/bjson) -> 6 points / 24 vertices / 8 prims.
    #[test]
    fn reads_houdini_ship_bgeo() {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/houdini-ship/joint_link_n/joint_link_n.bgeo");
        assert_eq!(counts(&scene(p.to_str().expect("utf8"), 0)), Ok((6, 24, 8)));
    }
    #[test]
    fn missing_menu_error_or_no_geometry() {
        let p = scratch("file_sop_missing.obj");
        let _ = std::fs::remove_file(&p);
        let p = p.to_str().expect("utf8");
        assert!(counts(&scene(p, 0)).expect_err("error").contains("not found"));
        assert_eq!(counts(&scene(p, 1)), Ok((0, 0, 0)));
        assert_eq!(counts(&scene("", 1)), Ok((0, 0, 0)));
        assert!(counts(&scene("", 0)).is_err());
        // exists but garbage: error even with no_geometry
        let bad = scratch("file_sop_garbage.ply");
        std::fs::write(&bad, b"not a ply").expect("write");
        assert!(counts(&scene(bad.to_str().expect("utf8"), 1)).is_err());
        assert!(counts(&scene(p, 2)).is_err());
    }
    #[test]
    fn reread_when_path_changes_and_when_file_changes() {
        let (a, b) = (scratch("file_sop_a.obj"), scratch("file_sop_b.obj"));
        std::fs::write(&a, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").expect("write");
        std::fs::write(&b, "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 0\nf 1 2 3 4\n").expect("write");
        let (mut g, id) = load_scene(&scene(a.to_str().expect("utf8"), 0)).expect("loads");
        assert_eq!(g.cook(id, 0.0).expect("a").point_count, 3);
        let named = g.node_ids()[0];
        g.set_param(named, "file", hexe_graph::ParamValue::S(b.to_str().expect("utf8").into())).expect("set");
        assert_eq!(g.cook(id, 0.0).expect("b").point_count, 4);
        // same path, edited content (different size) -> dirty recook re-reads
        std::fs::write(&b, "v 0 0 0\n").expect("write");
        g.set_param(named, "missing", hexe_graph::ParamValue::Menu(1)).expect("dirty");
        assert_eq!(g.cook(id, 0.0).expect("b2").point_count, 1);
    }
    #[test]
    fn relative_path_resolves_against_scene_dir_then_cwd() {
        let dir = scratch("file_sop_dir");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("rel.obj"), "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").expect("write");
        let json = scene("rel.obj", 0);
        let (mut g, id) = load_scene_in(&json, Some(&dir)).expect("loads");
        assert_eq!(g.cook(id, 0.0).expect("scene-dir").prim_count(), 1);
        // no base dir and the cwd (crate dir) has no rel.obj -> error
        assert!(counts(&json).is_err());
    }
}
