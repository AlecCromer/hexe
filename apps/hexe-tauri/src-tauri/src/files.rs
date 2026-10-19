//! File browser backend (File > Open dialog, `ui/src/filewin.ts`): directory listing, standard places, new folder.
//! std only; paths travel as `/`-separated strings (Windows accepts them), directories end in `/`.
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Listing cap: a hostile / huge directory cannot stall the UI with millions of rows.
pub const MAX_ENTRIES: usize = 20_000;

/// One directory entry as the browser shows it.
#[derive(Serialize, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    /// last modified, ms since the Unix epoch (None = unreadable)
    pub modified: Option<f64>,
    /// bytes (None for directories / unreadable)
    pub size: Option<u64>,
}

/// Locations column: home, desktop, temp, filesystem roots (`C:/`, ... on Windows; `/` elsewhere).
#[derive(Serialize, Debug)]
pub struct Places {
    pub home: String,
    pub desktop: String,
    pub temp: String,
    pub roots: Vec<String>,
}

/// `C:\Users\x` -> `C:/Users/x/` (directory form the UI works in).
pub fn dir_string(p: &Path) -> String {
    let mut s = p.to_string_lossy().replace('\\', "/");
    if !s.ends_with('/') {
        s.push('/');
    }
    s
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Dot names everywhere (Houdini hides `.android`, `.vscode`, ... on Windows too), plus hidden / system attributes.
#[cfg(windows)]
fn hidden(name: &str, meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    name.starts_with('.') || meta.file_attributes() & (HIDDEN | SYSTEM) != 0
}
#[cfg(not(windows))]
fn hidden(name: &str, _meta: &std::fs::Metadata) -> bool {
    name.starts_with('.')
}

/// Entries of `dir` (hidden / system ones skipped, like Houdini's browser), unsorted, at most `MAX_ENTRIES`.
pub fn list(dir: &str) -> Result<Vec<Entry>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{dir}: {e}"))?;
    let mut out = Vec::new();
    for ent in rd.flatten() {
        if out.len() >= MAX_ENTRIES {
            break;
        }
        let name = ent.file_name().to_string_lossy().into_owned();
        // follow links (a linked folder browses as a folder); a dangling link falls back to the link itself
        let Ok(meta) = std::fs::metadata(ent.path()).or_else(|_| ent.metadata()) else { continue };
        if hidden(&name, &meta) {
            continue;
        }
        let modified =
            meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as f64);
        let dir = meta.is_dir();
        out.push(Entry { name, dir, modified, size: if dir { None } else { Some(meta.len()) } });
    }
    Ok(out)
}

pub fn places() -> Places {
    let h = home();
    #[cfg(windows)]
    let roots = (b'A'..=b'Z').map(|c| format!("{}:/", c as char)).filter(|r| Path::new(r).is_dir()).collect();
    #[cfg(not(windows))]
    let roots = vec!["/".to_string()];
    Places {
        home: dir_string(&h),
        desktop: dir_string(&h.join("Desktop")),
        temp: dir_string(&std::env::temp_dir()),
        roots,
    }
}

/// New Folder: `dir/New Folder`, or `New Folder 2`, ... when taken. Returns the created name.
pub fn mkdir(dir: &str) -> Result<String, String> {
    for i in 1..1000 {
        let name = if i == 1 { "New Folder".to_string() } else { format!("New Folder {i}") };
        let p = Path::new(dir).join(&name);
        if p.exists() {
            continue;
        }
        return std::fs::create_dir(&p).map(|()| name).map_err(|e| format!("{}: {e}", p.display()));
    }
    Err(format!("{dir}: no free New Folder name"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_mkdir_round_trip_and_hostile_dirs() -> Result<(), String> {
        let base = std::env::temp_dir().join(format!("hexe-files-{}", std::process::id()));
        std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        std::fs::write(base.join("a.hip"), b"12345").map_err(|e| e.to_string())?;
        std::fs::create_dir_all(base.join(".hidden")).map_err(|e| e.to_string())?;
        let b = dir_string(&base);
        assert_eq!(mkdir(&b)?, "New Folder");
        assert_eq!(mkdir(&b)?, "New Folder 2");
        let mut l = list(&b)?;
        l.sort_by(|x, y| x.name.cmp(&y.name));
        assert_eq!(
            l.iter().map(|e| (e.name.as_str(), e.dir, e.size)).collect::<Vec<_>>(),
            [("New Folder", true, None), ("New Folder 2", true, None), ("a.hip", false, Some(5))]
        );
        assert!(l.iter().all(|e| e.modified.is_some()));
        assert!(list(&format!("{b}does-not-exist/")).is_err());
        assert!(list("").is_err());
        std::fs::remove_dir_all(&base).map_err(|e| e.to_string())?;
        Ok(())
    }

    #[test]
    fn dir_string_forward_slashes_and_trailing_slash() {
        assert_eq!(dir_string(Path::new("C:\\Users\\x")), "C:/Users/x/");
        assert_eq!(dir_string(Path::new("/tmp/")), "/tmp/");
        let p = places();
        assert!(p.home.ends_with('/') && p.temp.ends_with('/') && !p.roots.is_empty());
    }
}
