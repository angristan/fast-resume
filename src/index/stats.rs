use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Local, Timelike};

use crate::model::Session;

#[derive(Debug, Clone, Default)]
pub struct IndexStats {
    pub total_sessions: usize,
    pub total_messages: usize,
    pub sessions_by_agent: BTreeMap<String, usize>,
    pub messages_by_agent: BTreeMap<String, usize>,
    pub top_directories: Vec<(String, usize, usize)>,
    pub oldest: Option<DateTime<Local>>,
    pub newest: Option<DateTime<Local>>,
    pub sessions_by_weekday: BTreeMap<String, usize>,
    pub sessions_by_hour: BTreeMap<u32, usize>,
    pub index_size_bytes: u64,
    pub index_path: PathBuf,
}

pub(super) fn build(sessions: Vec<Session>, path: &Path) -> IndexStats {
    let mut stats = IndexStats {
        total_sessions: sessions.len(),
        index_size_bytes: index_size(path),
        index_path: path.to_path_buf(),
        ..IndexStats::default()
    };

    let mut dir_counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for session in sessions {
        stats.total_messages += session.message_count;
        *stats
            .sessions_by_agent
            .entry(session.agent.clone())
            .or_default() += 1;
        *stats
            .messages_by_agent
            .entry(session.agent.clone())
            .or_default() += session.message_count;

        let dir = if session.directory.is_empty() {
            "n/a".to_string()
        } else {
            session.display_directory()
        };
        let dir_entry = dir_counts.entry(dir).or_default();
        dir_entry.0 += 1;
        dir_entry.1 += session.message_count;

        stats.oldest = Some(match stats.oldest {
            Some(oldest) => oldest.min(session.timestamp),
            None => session.timestamp,
        });
        stats.newest = Some(match stats.newest {
            Some(newest) => newest.max(session.timestamp),
            None => session.timestamp,
        });

        let weekday = session.timestamp.weekday().to_string();
        *stats.sessions_by_weekday.entry(weekday).or_default() += 1;
        *stats
            .sessions_by_hour
            .entry(session.timestamp.hour())
            .or_default() += 1;
    }

    let mut dirs: Vec<_> = dir_counts
        .into_iter()
        .map(|(dir, (sessions, messages))| (dir, sessions, messages))
        .collect();
    dirs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.2.cmp(&a.2)));
    dirs.truncate(10);
    stats.top_directories = dirs;
    stats
}

fn index_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.path().metadata().ok())
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
        .sum()
}
