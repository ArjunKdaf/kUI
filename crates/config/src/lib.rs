//! THE single config store (SPEC decision: one config, zero flag files).
//!
//! One human-editable `kui.cfg` (flat `key = value` lines) under the shared
//! userdata dir, atomic writes (tmp + rename — FAT-safe enough), typed
//! accessors with defaults. Nothing else in the OS persists settings any
//! other way. No lineage names anywhere in our own artifacts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const FILE: &str = "kui.cfg";

pub struct Config {
    path: PathBuf,
    map: BTreeMap<String, String>,
}

impl Config {
    /// Load (or start empty) from `<shared_dir>/kui.cfg`.
    pub fn load(shared_dir: &Path) -> Self {
        let path = shared_dir.join(FILE);
        let mut map = BTreeMap::new();
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    map.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }
        Self { path, map }
    }

    pub fn exists(shared_dir: &Path) -> bool {
        shared_dir.join(FILE).is_file()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(|s| s.as_str())
    }

    pub fn get_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).unwrap_or(default)
    }

    pub fn get_i32(&self, key: &str, default: i32) -> i32 {
        self.get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    /// Hex RRGGBB color as premultiplied-friendly RGBA floats.
    pub fn get_color(&self, key: &str, default: u32) -> [f32; 4] {
        let v = self
            .get(key)
            .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(default);
        [
            ((v >> 16) & 0xFF) as f32 / 255.0,
            ((v >> 8) & 0xFF) as f32 / 255.0,
            (v & 0xFF) as f32 / 255.0,
            1.0,
        ]
    }

    /// All keys with a given prefix (core options enumeration).
    pub fn keys_with_prefix(&self, prefix: &str) -> Vec<String> {
        self.map.keys().filter(|k| k.starts_with(prefix)).cloned().collect()
    }

    /// Remove every key with the given prefix (page resets).
    pub fn remove_prefix(&mut self, prefix: &str) {
        self.map.retain(|k, _| !k.starts_with(prefix));
    }

    pub fn set(&mut self, key: &str, value: impl ToString) {
        self.map.insert(key.to_string(), value.to_string());
    }

    /// Atomic persist: whole file, tmp + rename.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut out = String::from("# kUI configuration\n");
        for (k, v) in &self.map {
            out.push_str(k);
            out.push_str(" = ");
            out.push_str(v);
            out.push('\n');
        }
        let tmp = self.path.with_extension("cfg.tmp");
        std::fs::write(&tmp, out)?;
        std::fs::rename(&tmp, &self.path)
    }
}
