use std::env;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use fast_resume::adapters::all_adapters;
use fast_resume::config::{VERSION, index_dir, is_agent};
use fast_resume::index::SessionIndex;
use fast_resume::search::SearchEngine;
use fast_resume::stats::print_stats;
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

    /// Force a fresh session scan and rebuild the Tantivy index.
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
}

fn main() -> Result<()> {
    let args = Args::parse();
    let query = args.query.unwrap_or_default();

    if args.rebuild {
        let start = Instant::now();
        let index = SessionIndex::open_default()?;
        let sessions = SessionIndex::scan_all_sessions();
        let summary = index
            .rebuild(sessions)
            .context("failed to rebuild Tantivy index")?;
        eprintln!(
            "Indexed {} sessions in {:.1}ms ({})",
            summary.sessions,
            start.elapsed().as_secs_f64() * 1000.0,
            index_dir().display()
        );
        if !args.no_tui && !args.list_only && query.is_empty() && !args.stats {
            return Ok(());
        }
    }

    if args.stats {
        let index = refreshed_index()?;
        let stats = index.stats()?;
        if stats.total_sessions == 0 {
            println!("No sessions indexed.");
            return Ok(());
        }
        let raw_stats: Vec<_> = all_adapters()
            .into_iter()
            .map(|adapter| adapter.raw_stats())
            .collect();
        print_stats(&stats, &raw_stats);
        return Ok(());
    }

    if args.no_tui || args.list_only {
        let index = refreshed_index()?;
        let engine = SearchEngine::from_index(index);
        let results = engine.search(&query, args.agent.as_deref(), args.directory.as_deref(), 50);
        let total = engine.count_matches(&query, args.agent.as_deref(), args.directory.as_deref());
        print_sessions(&results, total);
        return Ok(());
    }

    let image_protocol = if args.no_images && !args.images {
        None
    } else {
        Some(args.image_protocol.into())
    };

    match run_tui(query, args.agent, args.directory, args.yolo, image_protocol)? {
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

fn refreshed_index() -> Result<SessionIndex> {
    let index = SessionIndex::open_default()?;
    if index.total_len()? == 0 {
        let sessions = SessionIndex::scan_all_sessions();
        index.rebuild(sessions)?;
    } else {
        index.refresh_incremental()?;
    }
    Ok(index)
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
