use std::env;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use fast_resume::cache::{load_sessions, save_sessions};
use fast_resume::config::{VERSION, cache_file, is_agent};
use fast_resume::scan::{refresh_sessions_incremental, scan_all_sessions};
use fast_resume::search::SearchEngine;
use fast_resume::stats::{build_stats, print_stats};
use fast_resume::tui::{TuiExit, run_tui};

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum ImageProtocolArg {
    Auto,
    Kitty,
    Sixel,
    Iterm2,
}

#[derive(Debug, Parser)]
#[command(name = "fr", version = VERSION, about = "Search and resume coding agent sessions")]
struct Args {
    /// Search query.
    query: Option<String>,

    /// Filter by agent.
    #[arg(short, long, value_parser = validate_agent)]
    agent: Option<String>,

    /// Filter by directory substring.
    #[arg(short, long)]
    directory: Option<String>,

    /// Output list to stdout instead of opening the TUI.
    #[arg(long)]
    no_tui: bool,

    /// Just list sessions, don't resume.
    #[arg(long = "list")]
    list_only: bool,

    /// Force a fresh session scan and rewrite the Rust cache.
    #[arg(long)]
    rebuild: bool,

    /// Show index/session statistics.
    #[arg(long)]
    stats: bool,

    /// Resume sessions with auto-approve/skip-permissions flags where supported.
    #[arg(long)]
    yolo: bool,

    /// Render agent PNGs in the preview pane (enabled by default when supported).
    #[arg(long)]
    images: bool,

    /// Disable agent PNGs in the TUI.
    #[arg(long)]
    no_images: bool,

    /// Force a terminal image protocol for --images.
    #[arg(long, value_enum, default_value_t = ImageProtocolArg::Auto)]
    image_protocol: ImageProtocolArg,

    /// Accepted for CLI compatibility; update distribution checks are ignored in the Rust rewrite.
    #[arg(long)]
    no_version_check: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let query = args.query.unwrap_or_default();

    if args.rebuild {
        let start = Instant::now();
        let sessions = scan_all_sessions();
        save_sessions(&sessions).context("failed to write Rust session cache")?;
        eprintln!(
            "Indexed {} sessions in {:.1}ms ({})",
            sessions.len(),
            start.elapsed().as_secs_f64() * 1000.0,
            cache_file().display()
        );
        if !args.no_tui && !args.list_only && query.is_empty() && !args.stats {
            return Ok(());
        }
    }

    if args.stats {
        let sessions = cached_or_fresh_sessions();
        print_stats(&build_stats(&sessions));
        return Ok(());
    }

    if args.no_tui || args.list_only {
        let sessions = cached_or_fresh_sessions();
        let engine = SearchEngine::new(sessions);
        let results = engine.search(&query, args.agent.as_deref(), args.directory.as_deref(), 50);
        print_sessions(&results, engine.total_len());
        return Ok(());
    }

    let image_protocol = if args.no_images && !args.images {
        None
    } else {
        Some(args.image_protocol.into())
    };

    match run_tui(query, args.agent, args.yolo, image_protocol)? {
        TuiExit::Quit => Ok(()),
        TuiExit::Resume { command, directory } => exec_resume(command, directory),
    }
}

impl From<ImageProtocolArg> for fast_resume::tui::ImageProtocol {
    fn from(value: ImageProtocolArg) -> Self {
        match value {
            ImageProtocolArg::Auto => Self::Auto,
            ImageProtocolArg::Kitty => Self::Kitty,
            ImageProtocolArg::Sixel => Self::Sixel,
            ImageProtocolArg::Iterm2 => Self::Iterm2,
        }
    }
}

fn validate_agent(value: &str) -> std::result::Result<String, String> {
    if is_agent(value) {
        Ok(value.to_string())
    } else {
        Err(format!("unknown agent: {value}"))
    }
}

fn cached_or_fresh_sessions() -> Vec<fast_resume::model::Session> {
    if let Ok(sessions) = load_sessions() {
        if !sessions.is_empty() {
            let refreshed = refresh_sessions_incremental(sessions);
            if refreshed.new_or_modified > 0 || refreshed.deleted > 0 {
                let _ = save_sessions(&refreshed.sessions);
            }
            return refreshed.sessions;
        }
    }

    let sessions = scan_all_sessions();
    let _ = save_sessions(&sessions);
    sessions
}

fn print_sessions(results: &[fast_resume::model::Session], total: usize) {
    if results.is_empty() {
        println!("No sessions found.");
        return;
    }

    println!(
        "{:<15}  {:<52}  {:<38}  {}",
        "Agent", "Title", "Directory", "ID"
    );
    println!("{}", "-".repeat(124));
    for session in results {
        println!(
            "{:<15}  {:<52}  {:<38}  {}",
            session.agent,
            truncate_for_terminal(&session.title, 52),
            truncate_for_terminal(&session.display_directory(), 38),
            session.id
        );
    }
    println!("\nShowing {} of {} sessions", results.len(), total);
}

fn truncate_for_terminal(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    let keep = width.saturating_sub(3);
    let mut out: String = value.chars().take(keep).collect();
    out.push_str("...");
    out
}

fn exec_resume(command: Vec<String>, directory: String) -> Result<()> {
    if command.is_empty() {
        bail!("selected session has no resume command");
    }
    if !directory.is_empty() {
        env::set_current_dir(&directory)
            .with_context(|| format!("failed to change directory to {directory}"))?;
    }

    let err = Command::new(&command[0]).args(&command[1..]).exec();
    Err(err).with_context(|| format!("failed to exec {}", command[0]))
}
