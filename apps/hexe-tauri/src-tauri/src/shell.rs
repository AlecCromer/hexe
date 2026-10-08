//! `--script` runner: executes the parsed ops one at a time against the live session, tells the window to re-sync
//! (event `hexe://script`, answered by the `script_ack` command), and takes window screenshots for `shot`.
use crate::App;
use crate::script::{self, Op};
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Window -> runner handshake (the UI calls `script_ready` once booted, `script_ack` after each re-sync).
pub struct Gate {
    pub ready: Sender<()>,
    pub ack: Sender<()>,
}

pub fn gate() -> (Gate, Receiver<()>, Receiver<()>) {
    let (ready, ready_rx) = channel();
    let (ack, ack_rx) = channel();
    (Gate { ready, ack }, ready_rx, ack_rx)
}

/// Max wait for the UI to boot / to re-sync after an op.
const WAIT: Duration = Duration::from_secs(30);
/// Paint settle time before a screenshot (three.js renders on the next animation frame).
const SETTLE: Duration = Duration::from_millis(700);

/// CGWindowID of the first normal-layer window owned by `argv[0]` (a pid), via JXA + CoreGraphics (no unsafe, no extra crate).
const WINDOW_ID_JXA: &str = r#"ObjC.import("CoreGraphics");
function run(argv) {
  var pid = Number(argv[0]);
  var l = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo(1, 0)));
  var o = l.filter(function (w) { return w.kCGWindowOwnerPID === pid && w.kCGWindowLayer === 0; });
  return o.length ? String(o[0].kCGWindowNumber) : "";
}"#;

/// Screenshot of this process's own window (`screencapture -l <window id>`).
pub fn shot(path: &str) -> Result<(), String> {
    let pid = std::process::id().to_string();
    let out = Command::new("osascript")
        .args(["-l", "JavaScript", "-e", WINDOW_ID_JXA, &pid])
        .output()
        .map_err(|e| format!("shot: osascript: {e}"))?;
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("shot: no window found for pid {pid} ({})", String::from_utf8_lossy(&out.stderr).trim()));
    }
    if let Some(dir) = std::path::Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("shot: {}: {e}", dir.display()))?;
    }
    let st = Command::new("screencapture")
        .args(["-x", "-o", "-l", &id, path])
        .status()
        .map_err(|e| format!("shot: screencapture: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("shot: screencapture exited {st}")) }
}

/// Run `ops` on a worker thread once the UI reports ready. Any failing op ends the app with exit code 2 (3 = UI timeout).
pub fn spawn(app: AppHandle, ops: Vec<Op>, ready: Receiver<()>, ack: Receiver<()>) {
    std::thread::spawn(move || {
        let die = |code: i32, why: String| {
            eprintln!("hexe: {why}");
            app.exit(code);
        };
        if ready.recv_timeout(WAIT).is_err() {
            return die(3, "script: the window never became ready".into());
        }
        for op in ops {
            let state = app.state::<App>();
            let eff = match script::apply(&mut state.lock(), &op) {
                Ok(e) => e,
                Err(e) => return die(2, format!("{e} (op {op:?})")),
            };
            while ack.try_recv().is_ok() {}
            if let Err(e) = app.emit("hexe://script", serde_json::json!({ "select": eff.select })) {
                return die(3, format!("script: emit: {e}"));
            }
            if ack.recv_timeout(WAIT).is_err() {
                return die(3, format!("script: the window did not re-sync after {op:?}"));
            }
            if let Some(p) = eff.shot {
                std::thread::sleep(SETTLE);
                if let Err(e) = shot(&p) {
                    return die(2, e);
                }
            }
            if eff.quit {
                return app.exit(0);
            }
        }
    });
}
