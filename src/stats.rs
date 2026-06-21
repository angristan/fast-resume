use std::collections::BTreeMap;
use std::fs;

use chrono::{Datelike, Timelike};

use crate::config::cache_file;
use crate::model::Session;

#[derive(Debug, Default)]
pub struct Stats {
    pub total_sessions: usize,
    pub total_messages: usize,
    pub sessions_by_agent: BTreeMap<String, usize>,
    pub messages_by_agent: BTreeMap<String, usize>,
    pub top_directories: Vec<(String, usize, usize)>,
    pub oldest: Option<chrono::DateTime<chrono::Local>>,
    pub newest: Option<chrono::DateTime<chrono::Local>>,
    pub sessions_by_weekday: BTreeMap<String, usize>,
    pub sessions_by_hour: BTreeMap<u32, usize>,
    pub cache_size_bytes: u64,
}

pub fn build_stats(sessions: &[Session]) -> Stats {
    let mut stats = Stats {
        total_sessions: sessions.len(),
        cache_size_bytes: fs::metadata(cache_file()).map(|m| m.len()).unwrap_or(0),
        ..Stats::default()
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

pub fn print_stats(stats: &Stats) {
    println!("\nIndex Statistics\n");
    println!("  Total sessions          {}", stats.total_sessions);
    println!("  Total messages          {}", stats.total_messages);
    if stats.total_sessions > 0 {
        println!(
            "  Avg messages/session    {:.1}",
            stats.total_messages as f64 / stats.total_sessions as f64
        );
    }
    println!(
        "  Rust cache size         {}",
        human_bytes(stats.cache_size_bytes)
    );
    println!("  Rust cache location     {}", cache_file().display());
    if let (Some(oldest), Some(newest)) = (stats.oldest, stats.newest) {
        println!(
            "  Date range              {} to {}",
            oldest.format("%Y-%m-%d"),
            newest.format("%Y-%m-%d")
        );
    }

    println!("\nData by Agent\n");
    println!("{:<16} {:>10} {:>10}", "Agent", "Sessions", "Messages");
    println!("{}", "-".repeat(40));
    for (agent, sessions) in &stats.sessions_by_agent {
        let messages = stats.messages_by_agent.get(agent).copied().unwrap_or(0);
        println!("{:<16} {:>10} {:>10}", agent, sessions, messages);
    }

    if !stats.top_directories.is_empty() {
        println!("\nTop Directories\n");
        println!("{:<56} {:>9} {:>9}", "Directory", "Sessions", "Messages");
        println!("{}", "-".repeat(78));
        for (dir, sessions, messages) in &stats.top_directories {
            println!("{:<56} {:>9} {:>9}", truncate(dir, 56), sessions, messages);
        }
    }
    println!();
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    let mut out: String = value.chars().take(width.saturating_sub(3)).collect();
    out.push_str("...");
    out
}
