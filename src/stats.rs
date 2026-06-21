use crate::index::IndexStats;

pub fn print_stats(stats: &IndexStats) {
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
        "  Tantivy index size      {}",
        human_bytes(stats.index_size_bytes)
    );
    println!("  Tantivy index location  {}", stats.index_path.display());
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
