use std::collections::{BTreeMap, BTreeSet};
use std::env;

use crate::index::IndexStats;
use crate::model::RawAdapterStats;

pub fn print_stats(stats: &IndexStats, raw_stats: &[RawAdapterStats]) {
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
        "  Index size              {}",
        human_bytes(stats.index_size_bytes)
    );
    println!(
        "  Index location          {}",
        display_path(&stats.index_path.display().to_string())
    );
    if let (Some(oldest), Some(newest)) = (stats.oldest, stats.newest) {
        println!(
            "  Date range              {} to {}",
            oldest.format("%Y-%m-%d"),
            newest.format("%Y-%m-%d")
        );
    }

    print_agent_stats(stats, raw_stats);
    print_day_activity(stats);
    print_hour_activity(stats);

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

fn print_agent_stats(stats: &IndexStats, raw_stats: &[RawAdapterStats]) {
    let raw_by_agent: BTreeMap<_, _> = raw_stats.iter().map(|raw| (raw.agent, raw)).collect();
    let mut agents: BTreeSet<String> = stats.sessions_by_agent.keys().cloned().collect();
    agents.extend(raw_stats.iter().map(|raw| raw.agent.to_string()));
    let mut agents: Vec<_> = agents.into_iter().collect();
    agents.sort_by(|a, b| {
        stats
            .sessions_by_agent
            .get(b)
            .copied()
            .unwrap_or_default()
            .cmp(&stats.sessions_by_agent.get(a).copied().unwrap_or_default())
            .then_with(|| a.cmp(b))
    });

    println!("\nData by Agent\n");
    println!(
        "{:<16} {:>7} {:>10} {:>10} {:>10} {:>10}  {}",
        "Agent", "Files", "Disk", "Sessions", "Messages", "Content", "Data Dir"
    );
    println!("{}", "-".repeat(93));
    for agent in agents {
        let raw = raw_by_agent.get(agent.as_str()).copied();
        let sessions = stats.sessions_by_agent.get(&agent).copied().unwrap_or(0);
        let messages = stats.messages_by_agent.get(&agent).copied().unwrap_or(0);
        let content_bytes = stats
            .content_bytes_by_agent
            .get(&agent)
            .copied()
            .unwrap_or(0);
        let files = raw.map(|raw| raw.file_count).unwrap_or(0);
        let disk = raw
            .filter(|raw| raw.available)
            .map(|raw| human_bytes(raw.total_bytes))
            .unwrap_or_else(|| "n/a".to_string());
        let data_dir = raw
            .map(|raw| truncate(&display_path(&raw.data_dir), 28))
            .unwrap_or_else(|| "n/a".to_string());

        println!(
            "{:<16} {:>7} {:>10} {:>10} {:>10} {:>10}  {}",
            agent,
            files,
            disk,
            sessions,
            messages,
            human_bytes(content_bytes),
            data_dir
        );
    }
}

fn print_day_activity(stats: &IndexStats) {
    let max = stats
        .sessions_by_weekday
        .values()
        .copied()
        .max()
        .unwrap_or(0);
    if max == 0 {
        return;
    }

    println!("\nActivity by Day\n");
    for day in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] {
        let count = stats.sessions_by_weekday.get(day).copied().unwrap_or(0);
        println!(" {:<3}   {:<24} {:>5}", day, bar(count, max, 24), count);
    }
}

fn print_hour_activity(stats: &IndexStats) {
    let max = stats.sessions_by_hour.values().copied().max().unwrap_or(0);
    if max == 0 {
        return;
    }

    let sparkline: String = (0..24)
        .map(|hour| spark(stats.sessions_by_hour.get(&hour).copied().unwrap_or(0), max))
        .collect();
    let mut peaks: Vec<_> = stats
        .sessions_by_hour
        .iter()
        .filter(|(_, count)| **count > 0)
        .collect();
    peaks.sort_by(|(hour_a, count_a), (hour_b, count_b)| {
        count_b.cmp(count_a).then_with(|| hour_a.cmp(hour_b))
    });
    let peaks = peaks
        .into_iter()
        .take(3)
        .map(|(hour, count)| format!("{hour:02}:00 ({count})"))
        .collect::<Vec<_>>()
        .join(", ");

    println!("\nActivity by Hour\n");
    println!("  0h {sparkline} 23h");
    if !peaks.is_empty() {
        println!("  Peak hours: {peaks}");
    }
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

fn bar(count: usize, max: usize, width: usize) -> String {
    if count == 0 || max == 0 {
        return String::new();
    }
    let filled = ((count as f64 / max as f64) * width as f64)
        .round()
        .max(1.0) as usize;
    "█".repeat(filled)
}

fn spark(count: usize, max: usize) -> char {
    const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if count == 0 || max == 0 {
        return ' ';
    }
    let idx = ((count as f64 / max as f64) * (LEVELS.len() - 1) as f64).ceil() as usize;
    LEVELS[idx.min(LEVELS.len() - 1)]
}

fn display_path(path: &str) -> String {
    let home = env::var("HOME").unwrap_or_default();
    if !home.is_empty() && path.starts_with(&home) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
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
