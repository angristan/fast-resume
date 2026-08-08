use std::env;
use std::io;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use fast_resume::adapters::all_adapters;
use fast_resume::config::{VERSION, index_dir, is_agent};
use fast_resume::index::SessionIndex;
use fast_resume::output::{DEFAULT_LIST_LIMIT, print_sessions_json, print_sessions_table};
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

    /// Output a stable JSON session list instead of opening the TUI.
    #[arg(long, conflicts_with = "stats")]
    json: bool,

    /// Maximum sessions to return in list or JSON output.
    #[arg(long, value_parser = parse_positive_usize, conflicts_with = "all")]
    limit: Option<usize>,

    /// Skip this many matching sessions in list or JSON output.
    #[arg(long)]
    offset: Option<usize>,

    /// Return all matching sessions from the requested offset.
    #[arg(long)]
    all: bool,

    /// Serve the existing index without scanning for session changes.
    #[arg(long)]
    no_refresh: bool,

    /// Force a fresh session scan and rebuild the Tantivy index.
    #[arg(long)]
    rebuild: bool,

    /// Show index/session statistics.
    #[arg(long)]
    stats: bool,

    /// Print concise instructions for coding agents.
    #[arg(long)]
    agent_context: bool,

    /// Resume sessions with auto-approve/skip-permissions flags where supported.
    #[arg(long)]
    yolo: bool,

    /// Retained as a hidden no-op for compatibility with the Python CLI.
    #[arg(long = "no-version-check", hide = true)]
    _no_version_check: bool,

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
    validate_pagination_args(&args)?;
    let query = args.query.clone().unwrap_or_default();

    if args.agent_context {
        print!("{}", include_str!("../skills/fast-resume/SKILL.md"));
        return Ok(());
    }

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
        if !args.no_tui && !args.list_only && !args.json && query.is_empty() && !args.stats {
            return Ok(());
        }
    }

    if args.stats {
        let index = refreshed_index(args.no_refresh)?;
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

    if args.no_tui || args.list_only || args.json {
        let index = refreshed_index(args.no_refresh)?;
        let engine = SearchEngine::from_index(index);
        let total = engine
            .count_result(&query, args.agent.as_deref(), args.directory.as_deref())
            .context("search failed")?;
        let offset = args.offset.unwrap_or(0);
        let limit = if args.all {
            total.saturating_sub(offset)
        } else {
            args.limit.unwrap_or(DEFAULT_LIST_LIMIT)
        };
        let results = engine
            .search_result_with_offset(
                &query,
                args.agent.as_deref(),
                args.directory.as_deref(),
                offset,
                limit,
            )
            .context("search failed")?;
        if args.json {
            print_sessions_json(&results, total, offset, limit, args.yolo)?;
        } else {
            print_sessions_table(&results, total, offset);
        }
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

fn parse_positive_usize(value: &str) -> std::result::Result<usize, String> {
    let value = value
        .parse::<usize>()
        .map_err(|_| "must be a positive integer".to_string())?;
    if value == 0 {
        return Err("must be greater than zero".to_string());
    }
    Ok(value)
}

fn validate_pagination_args(args: &Args) -> Result<()> {
    if (args.limit.is_some() || args.offset.is_some() || args.all)
        && !(args.json || args.list_only || args.no_tui)
    {
        bail!("--limit, --offset, and --all require --json, --list, or --no-tui");
    }
    if args.no_refresh && !(args.json || args.list_only || args.no_tui || args.stats) {
        bail!("--no-refresh requires --json, --list, --no-tui, or --stats");
    }
    Ok(())
}

fn refreshed_index(no_refresh: bool) -> Result<SessionIndex> {
    let index = SessionIndex::open_default()?;
    if no_refresh {
        return Ok(index);
    }
    index.refresh_incremental_notify(|| {
        eprintln!(
            "Waiting for another fr process to finish refreshing; pass --no-refresh to search the current index."
        );
    })?;
    Ok(index)
}

fn exec_resume(command: Vec<String>, directory: String) -> Result<()> {
    let mut backend = ProcessExecBackend;
    exec_resume_with(&mut backend, command, directory)
}

trait ExecBackend {
    fn set_current_dir(&mut self, directory: &str) -> io::Result<()>;
    fn exec(&mut self, command: &[String]) -> io::Error;
}

struct ProcessExecBackend;

impl ExecBackend for ProcessExecBackend {
    fn set_current_dir(&mut self, directory: &str) -> io::Result<()> {
        env::set_current_dir(directory)
    }

    fn exec(&mut self, command: &[String]) -> io::Error {
        #[cfg(unix)]
        {
            Command::new(&command[0]).args(&command[1..]).exec()
        }
        #[cfg(not(unix))]
        {
            match Command::new(&command[0]).args(&command[1..]).status() {
                Ok(status) => std::process::exit(status.code().unwrap_or(1)),
                Err(error) => error,
            }
        }
    }
}

fn exec_resume_with(
    backend: &mut impl ExecBackend,
    command: Vec<String>,
    directory: String,
) -> Result<()> {
    if command.is_empty() {
        bail!("selected session has no resume command");
    }
    if !directory.is_empty() {
        backend
            .set_current_dir(&directory)
            .with_context(|| format!("failed to change directory to {directory}"))?;
    }

    let err = backend.exec(&command);
    Err(err).with_context(|| format!("failed to exec {}", command[0]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingExec {
        directories: Vec<String>,
        commands: Vec<Vec<String>>,
    }

    impl ExecBackend for RecordingExec {
        fn set_current_dir(&mut self, directory: &str) -> io::Result<()> {
            self.directories.push(directory.to_string());
            Ok(())
        }

        fn exec(&mut self, command: &[String]) -> io::Error {
            self.commands.push(command.to_vec());
            io::Error::new(io::ErrorKind::NotFound, "missing command")
        }
    }

    #[test]
    fn exec_resume_hands_off_directory_and_command() {
        let mut backend = RecordingExec::default();

        let error = exec_resume_with(
            &mut backend,
            vec![
                "codex".to_string(),
                "resume".to_string(),
                "session-1".to_string(),
            ],
            "/repo/backend".to_string(),
        )
        .unwrap_err();

        assert_eq!(backend.directories, vec!["/repo/backend"]);
        assert_eq!(
            backend.commands,
            vec![vec![
                "codex".to_string(),
                "resume".to_string(),
                "session-1".to_string()
            ]]
        );
        assert!(error.to_string().contains("failed to exec codex"));
    }

    #[test]
    fn exec_resume_skips_empty_directory_and_rejects_empty_command() {
        let mut backend = RecordingExec::default();

        let error =
            exec_resume_with(&mut backend, vec!["code".to_string()], String::new()).unwrap_err();

        assert!(backend.directories.is_empty());
        assert_eq!(backend.commands, vec![vec!["code".to_string()]]);
        assert!(error.to_string().contains("failed to exec code"));

        let mut backend = RecordingExec::default();
        let error = exec_resume_with(&mut backend, Vec::new(), "/repo".to_string()).unwrap_err();
        assert!(backend.directories.is_empty());
        assert!(backend.commands.is_empty());
        assert!(error.to_string().contains("no resume command"));
    }

    #[test]
    fn accepts_legacy_no_version_check_flag() {
        let args = Args::try_parse_from(["fr", "--no-version-check", "--list"]).unwrap();

        assert!(args._no_version_check);
        assert!(args.list_only);
    }
}
