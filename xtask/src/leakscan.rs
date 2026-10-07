//! `cargo xtask leakscan [dir]` (default spec/): the gate on every spec commit (Rust-only replacement for a script).
//! Rejects decompiler identifiers, calling conventions, source paths, code fences and quoted strings > 12 words.
use crate::{Res, root};
use std::path::Path;

const PREFIXES: &[&str] = &["sub_", "loc_", "unk_", "dword_", "qword_", "word_", "byte_", "off_", "nullsub_"];
const WORDS: &[&str] = &["__thiscall", "__fastcall", "__cdecl", "__stdcall", "this+0x", ".pdb"];
const FENCES: &[&str] = &["```c\n", "```cpp", "```rust", "```asm", "```python", "```js", "```ts"];
const SRC_EXT: &[&str] = &[".cpp", ".cc", ".hpp", ".h", ".c", ".cs"];

/// Findings (message) for one file's text.
pub fn scan(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for f in FENCES.iter().filter(|f| text.contains(*f)) {
        out.push(format!("code fence `{}` (spec is prose + tables, no code)", f.trim()));
    }
    for (n, line) in text.lines().enumerate() {
        let n = n + 1;
        for p in PREFIXES {
            let mut rest = line;
            while let Some(i) = rest.find(p) {
                let after = &rest[i + p.len()..];
                let boundary = i == 0 || !rest.as_bytes()[i - 1].is_ascii_alphanumeric();
                if boundary && after.chars().take_while(char::is_ascii_hexdigit).count() >= 3 {
                    out.push(format!("line {n}: decompiler identifier `{p}…`"));
                }
                rest = after;
            }
        }
        for w in WORDS.iter().filter(|w| line.contains(*w)) {
            out.push(format!("line {n}: `{w}`"));
        }
        if line.split_whitespace().any(|t| {
            let t = t.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '/');
            (t.contains('/') || t.contains('\\')) && SRC_EXT.iter().any(|e| t.ends_with(e))
        }) {
            out.push(format!("line {n}: source path"));
        }
        for (i, seg) in line.split('"').enumerate() {
            if i % 2 == 1 && seg.split_whitespace().count() > 12 {
                out.push(format!("line {n}: quoted string of {} words (> 12)", seg.split_whitespace().count()));
            }
        }
    }
    out
}

fn walk(dir: &Path, errs: &mut Vec<String>, files: &mut usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, errs, files);
        } else if let Ok(t) = std::fs::read_to_string(&p) {
            *files += 1;
            errs.extend(scan(&t).into_iter().map(|m| format!("{}: {m}", p.display())));
        }
    }
}

pub fn run(args: &[String]) -> Res {
    let dir = root().join(args.first().map_or("spec", String::as_str));
    let (mut errs, mut files) = (Vec::new(), 0);
    walk(&dir, &mut errs, &mut files);
    for e in &errs {
        println!("  - {e}");
    }
    if errs.is_empty() {
        println!("leakscan: 0 hits in {files} files");
        Ok(())
    } else {
        Err(format!("{} leak hit(s)", errs.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_prose_and_tables_pass() {
        assert!(scan("# Header\n| magic | 0x25504446 |\nThe file starts with the marker \"%PDF-\".").is_empty());
    }

    #[test]
    fn decompiler_residue_is_caught() {
        assert_eq!(scan("calls sub_4012F0 here").len(), 1);
        assert_eq!(scan("a __thiscall method").len(), 1);
        assert_eq!(scan("see src/render/pipeline.cpp").len(), 1);
        assert_eq!(scan("```cpp\nint x;\n```").len(), 1);
        assert_eq!(scan("says \"a b c d e f g h i j k l m\" literally").len(), 1);
        assert!(scan("subtle sub_menu word_count").is_empty());
    }
}
