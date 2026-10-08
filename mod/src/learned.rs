//! enemy_dirs.learned_table: zones learned per (enemy behavior name, attack animation id), so an attack
//! seen once shows its zone from the first frame of its wind-up afterwards. A flat JSON object of strings,
//! e.g. {"c3100:3005": "upper_left", "c3100:3010": "thrust"}.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::logf;

pub struct Learned {
    map: BTreeMap<String, String>,
    dirty: bool,
}

pub static LEARNED: Mutex<Learned> = Mutex::new(Learned { map: BTreeMap::new(), dirty: false });

fn path() -> PathBuf {
    crate::log::dir().join("learned_zones.json")
}

pub fn key(behavior: &str, anim: i32) -> String {
    format!("{behavior}:{}", anim.rem_euclid(1_000_000))
}

impl Learned {
    pub fn load() {
        let Ok(t) = std::fs::read_to_string(path()) else { return };
        let mut l = LEARNED.lock().unwrap();
        // Minimal parser for our own flat {"k": "v", ...} file.
        let parts: Vec<&str> = t.split('"').collect();
        let mut i = 1;
        while i + 2 < parts.len() {
            l.map.insert(parts[i].to_string(), parts[i + 2].to_string());
            i += 4;
        }
        logf!("[learned] {} attack zones loaded from {}", l.map.len(), path().display());
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.map.get(k).map(String::as_str)
    }

    pub fn set(&mut self, k: String, v: &str) {
        if self.map.get(&k).map(String::as_str) != Some(v) {
            logf!("[learned] {k} -> {v}");
            self.map.insert(k, v.to_string());
            self.dirty = true;
        }
    }

    pub fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let body: Vec<String> = self.map.iter().map(|(k, v)| format!("  \"{k}\": \"{v}\"")).collect();
        let _ = std::fs::write(path(), format!("{{\n{}\n}}\n", body.join(",\n")));
    }
}
