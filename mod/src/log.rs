//! Plain file log at %LOCALAPPDATA%\KCTarnished\kctarnished.log, truncated at every start.
//! Lines tagged `[hook:<id>]` are the evidence for the hooks sheet.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

static LOG: Mutex<Option<(File, Instant)>> = Mutex::new(None);

pub fn dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("KCTarnished")
}

/// Where the swing lab writes its bigger files (KCT_LAB_DIR when set, else the log folder).
pub fn lab_dir() -> PathBuf {
    let d = std::env::var_os("KCT_LAB_DIR").map(PathBuf::from).unwrap_or_else(dir);
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn init() {
    let d = dir();
    let _ = std::fs::create_dir_all(&d);
    if let Ok(f) = File::create(d.join("kctarnished.log")) {
        *LOG.lock().unwrap() = Some((f, Instant::now()));
    }
}

pub fn write(msg: &str) {
    if let Ok(mut g) = LOG.lock() {
        if let Some((f, t0)) = g.as_mut() {
            let _ = writeln!(f, "{:>9.3} {}", t0.elapsed().as_secs_f32(), msg);
            let _ = f.flush();
        }
    }
}

#[macro_export]
macro_rules! logf {
    ($($a:tt)*) => { $crate::log::write(&format!($($a)*)) };
}

/// Log a message once per distinct key (for per-frame code).
pub fn once(key: &str, msg: &str) {
    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mut s = SEEN.lock().unwrap();
    if !s.iter().any(|k| k == key) {
        s.push(key.to_string());
        drop(s);
        write(msg);
    }
}
