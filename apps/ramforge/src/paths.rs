//! Portable data folder and settings.
//!
//! RAMForge keeps everything in `RAMForge-data/` next to the EXE, so the whole
//! tool lives on the USB stick. If that folder is not writable (read-only
//! medium, Program Files) it falls back to the user profile.
//! `RAMFORGE_DATA` overrides the location (used by the boot edition).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub struct Paths {
    pub root: PathBuf,
    pub portable: bool,
    pub logs: PathBuf,
    pub reports: PathBuf,
    pub screenshots: PathBuf,
    pub sensors: PathBuf,
}

fn writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".write-test");
    let ok = std::fs::write(&probe, b"ok").is_ok();
    let _ = std::fs::remove_file(probe);
    ok
}

impl Paths {
    pub fn resolve() -> Self {
        let mut candidates: Vec<(PathBuf, bool)> = Vec::new();
        if let Ok(p) = std::env::var("RAMFORGE_DATA") {
            candidates.push((PathBuf::from(p), true));
        }
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
        {
            candidates.push((dir.join("RAMForge-data"), true));
        }
        if let Ok(p) = std::env::var("LOCALAPPDATA") {
            candidates.push((PathBuf::from(p).join("RAMForge"), false));
        }
        if let Ok(p) = std::env::var("HOME") {
            candidates.push((PathBuf::from(p).join(".local/share/ramforge"), false));
        }
        candidates.push((std::env::temp_dir().join("RAMForge"), false));
        let (root, portable) = candidates
            .into_iter()
            .find(|(p, _)| writable(p))
            .unwrap_or_else(|| (std::env::temp_dir(), false));
        let sub = |n: &str| {
            let p = root.join(n);
            let _ = std::fs::create_dir_all(&p);
            p
        };
        Self {
            logs: sub("logs"),
            reports: sub("reports"),
            screenshots: sub("screenshots"),
            sensors: sub("sensors"),
            root,
            portable,
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }
}

/// Bump when DISCLAIMER.md changes in substance: users must accept it again.
pub const DISCLAIMER_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub disclaimer_accepted: u32,
    pub theme: String,
    pub mem_percent: f32,
    pub sensor_interval_ms: u64,
    /// "Sensoren während des Tests aufzeichnen" (RAM test and benchmark).
    pub record_sensors: bool,
    /// Sample interval while recording: 500, 1000, 2000 or 5000 ms.
    pub record_interval_ms: u64,
    pub pinned_sensors: Vec<String>,
    pub anonymize_reports: bool,
    /// "Beim Start nach Updates und Neuigkeiten suchen" (default on).
    pub check_updates: bool,
    pub skipped_update: String,
    /// "Anonyme Statistik senden": opt-in, default off.
    pub stats_consent: bool,
    /// Random UUIDv4, only exists while consent is given.
    pub install_id: String,
    /// Date (YYYY-MM-DD) of the last statistics ping.
    pub last_ping: String,
    pub last_news_id: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            disclaimer_accepted: 0,
            theme: "dark".into(),
            mem_percent: 85.0,
            sensor_interval_ms: 1000,
            record_sensors: true,
            record_interval_ms: 1000,
            pinned_sensors: Vec::new(),
            anonymize_reports: true,
            check_updates: true,
            skipped_update: String::new(),
            stats_consent: false,
            install_id: String::new(),
            last_ping: String::new(),
            last_news_id: 0,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        // Editors and PowerShell may save with a UTF-8 BOM; serde_json rejects it.
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, path: &Path) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, s);
        }
    }
}

/// Opens a folder in the system file manager.
pub fn open_folder(p: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

pub fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}
