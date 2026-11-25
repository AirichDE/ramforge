//! Session log: shown in the "Protokoll" tab and written to logs/*.log.

use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Entry {
    pub time: chrono::DateTime<chrono::Local>,
    pub level: Level,
    pub text: String,
}

pub struct AppLog {
    pub entries: Vec<Entry>,
    file: Option<std::fs::File>,
    pub path: PathBuf,
}

impl AppLog {
    pub fn new(dir: &Path) -> Self {
        let path = dir.join(format!("ramforge-{}.log", crate::paths::stamp()));
        let file = std::fs::File::create(&path).ok();
        Self {
            entries: Vec::new(),
            file,
            path,
        }
    }

    pub fn add(&mut self, level: Level, text: impl Into<String>) {
        let e = Entry {
            time: chrono::Local::now(),
            level,
            text: text.into(),
        };
        if let Some(f) = self.file.as_mut() {
            let tag = match level {
                Level::Info => "INFO ",
                Level::Warn => "WARN ",
                Level::Error => "FEHLER",
            };
            let _ = writeln!(f, "{} {tag} {}", e.time.format("%Y-%m-%d %H:%M:%S"), e.text);
            let _ = f.flush();
        }
        self.entries.push(e);
        if self.entries.len() > 20_000 {
            self.entries.drain(..5_000);
        }
    }
    pub fn info(&mut self, t: impl Into<String>) {
        self.add(Level::Info, t)
    }
    pub fn warn(&mut self, t: impl Into<String>) {
        self.add(Level::Warn, t)
    }
    pub fn error(&mut self, t: impl Into<String>) {
        self.add(Level::Error, t)
    }
}
