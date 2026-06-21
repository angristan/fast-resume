use std::collections::HashMap;
use std::env;
use std::path::Path;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub id: String,
    pub agent: String,
    pub title: String,
    pub directory: String,
    pub timestamp: DateTime<Local>,
    pub content: String,
    pub message_count: usize,
    pub mtime: f64,
    pub yolo: bool,
}

impl Session {
    pub fn new(
        id: impl Into<String>,
        agent: impl Into<String>,
        title: impl Into<String>,
        directory: impl Into<String>,
        timestamp: DateTime<Local>,
        content: impl Into<String>,
        message_count: usize,
    ) -> Self {
        Self {
            id: id.into(),
            agent: agent.into(),
            title: title.into(),
            directory: directory.into(),
            timestamp,
            content: content.into(),
            message_count,
            mtime: 0.0,
            yolo: false,
        }
    }

    pub fn display_directory(&self) -> String {
        let home = env::var("HOME").unwrap_or_default();
        if !home.is_empty() && self.directory.starts_with(&home) {
            format!("~{}", &self.directory[home.len()..])
        } else if self.directory.is_empty() {
            "n/a".to_string()
        } else {
            self.directory.clone()
        }
    }
}

#[derive(Debug, Clone)]
pub struct RawAdapterStats {
    pub agent: &'static str,
    pub data_dir: String,
    pub available: bool,
    pub file_count: usize,
    pub total_bytes: u64,
}

pub fn truncate_title(text: &str, max_len: usize, word_break: bool) -> String {
    let text = text.trim();
    if text.chars().count() <= max_len {
        return text.to_string();
    }

    let mut truncated: String = text.chars().take(max_len).collect();
    if word_break {
        if let Some((prefix, _)) = truncated.rsplit_once(' ') {
            truncated = prefix.to_string();
        }
    }
    truncated.push_str("...");
    truncated
}

pub fn file_mtime_seconds(path: &Path) -> f64 {
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn file_timestamp(path: &Path) -> DateTime<Local> {
    path.metadata()
        .and_then(|m| m.modified())
        .map(DateTime::<Local>::from)
        .unwrap_or_else(|_| Local::now())
}

pub fn sort_and_dedupe_sessions(sessions: Vec<Session>) -> Vec<Session> {
    let mut by_key: HashMap<(String, String), Session> = HashMap::new();
    for session in sessions {
        let key = (session.agent.clone(), session.id.clone());
        match by_key.get(&key) {
            Some(existing)
                if existing.mtime >= session.mtime
                    && existing.timestamp >= session.timestamp
                    && existing.content.len() >= session.content.len() => {}
            _ => {
                by_key.insert(key, session);
            }
        }
    }

    let mut sessions: Vec<_> = by_key.into_values().collect();
    sessions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    sessions
}
