use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub text: String,
    pub timestamp: u64,
    pub words: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
    pub total_dictations: u64,
    pub total_words: u64,
}

impl Default for History {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            total_dictations: 0,
            total_words: 0,
        }
    }
}

impl History {
    // Mirrors config.rs `config_dir()` (lines 52-63): %APPDATA%/SmoothFlow
    // with a cwd fallback. Not reusing the config.rs fn because it's private
    // and the scope forbids touching config.rs.
    fn config_dir() -> PathBuf {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let mut p = PathBuf::from(appdata);
            p.push("SmoothFlow");
            let _ = std::fs::create_dir_all(&p);
            p
        } else {
            // ponytail: fallback to cwd if APPDATA not set (unlikely on Windows)
            std::env::current_dir().unwrap_or_default()
        }
    }

    pub fn path() -> PathBuf {
        let mut p = Self::config_dir();
        p.push("history.json");
        p
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), s);
        }
    }

    pub fn push(&mut self, text: &str) {
        let words = text.split_whitespace().count() as u64;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.entries.push(HistoryEntry {
            text: text.to_string(),
            timestamp,
            words,
        });
        self.total_dictations += 1;
        self.total_words += words;
        // ponytail: FIFO cap at 100 entries — oldest dropped; bump the constant if needed
        if self.entries.len() > 100 {
            self.entries.drain(..self.entries.len() - 100);
        }
    }
}
