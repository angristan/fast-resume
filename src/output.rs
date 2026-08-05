use std::io::{self, Write};

use anyhow::Result;
use serde::Serialize;

use crate::adapters::adapter_for;
use crate::model::Session;

pub const DEFAULT_LIST_LIMIT: usize = 50;
pub const LIST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
struct SessionOutput<'a> {
    id: &'a str,
    agent: &'a str,
    title: &'a str,
    directory: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Local>,
    message_count: usize,
    resume_command: Vec<String>,
}

impl<'a> SessionOutput<'a> {
    fn new(session: &'a Session, force_yolo: bool) -> Self {
        let resume_command = adapter_for(&session.agent)
            .map(|adapter| adapter.resume_command(session, force_yolo || session.yolo))
            .unwrap_or_default();
        Self {
            id: &session.id,
            agent: &session.agent,
            title: &session.title,
            directory: &session.directory,
            timestamp: &session.timestamp,
            message_count: session.message_count,
            resume_command,
        }
    }
}

#[derive(Debug, Serialize)]
struct PaginationMeta {
    state: PaginationState,
    total: usize,
    offset: usize,
    limit: usize,
    returned: usize,
    next_offset: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum PaginationState {
    More,
    Complete,
    PastEnd,
}

#[derive(Debug, Serialize)]
struct SessionListOutput<'a> {
    schema_version: u32,
    sessions: Vec<SessionOutput<'a>>,
    meta: PaginationMeta,
}

pub fn print_sessions_json(
    sessions: &[Session],
    total: usize,
    offset: usize,
    limit: usize,
    force_yolo: bool,
) -> Result<()> {
    let returned = sessions.len();
    let has_more = offset.saturating_add(returned) < total;
    let state = if total > 0 && offset >= total {
        PaginationState::PastEnd
    } else if has_more {
        PaginationState::More
    } else {
        PaginationState::Complete
    };
    let output = SessionListOutput {
        schema_version: LIST_SCHEMA_VERSION,
        sessions: sessions
            .iter()
            .map(|session| SessionOutput::new(session, force_yolo))
            .collect(),
        meta: PaginationMeta {
            state,
            total,
            offset,
            limit,
            returned,
            next_offset: has_more.then_some(offset.saturating_add(returned)),
        },
    };

    let stdout = io::stdout();
    let mut writer = stdout.lock();
    serde_json::to_writer(&mut writer, &output)?;
    writeln!(writer)?;
    Ok(())
}

pub fn print_sessions_table(sessions: &[Session], total: usize, offset: usize) {
    if sessions.is_empty() {
        println!("No sessions found.");
        return;
    }

    println!("{:<15}  {:<52}  {:<38}  ID", "Agent", "Title", "Directory");
    println!("{}", "-".repeat(124));
    for session in sessions {
        println!(
            "{:<15}  {:<52}  {:<38}  {}",
            session.agent,
            truncate_for_terminal(&session.title, 52),
            truncate_for_terminal(&session.display_directory(), 38),
            session.id
        );
    }

    if offset == 0 {
        println!("\nShowing {} of {} sessions", sessions.len(), total);
    } else {
        println!(
            "\nShowing {}-{} of {} sessions",
            offset + 1,
            offset + sessions.len(),
            total
        );
    }
    let next_offset = offset + sessions.len();
    if next_offset < total {
        eprintln!("More sessions available; continue with --offset {next_offset}");
    }
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

#[cfg(test)]
mod tests {
    use chrono::Local;

    use super::*;

    #[test]
    fn pagination_state_distinguishes_boundaries() {
        let session = Session::new("id", "codex", "Title", "/repo", Local::now(), "content", 1);

        let json = serde_json::to_value(SessionListOutput {
            schema_version: LIST_SCHEMA_VERSION,
            sessions: vec![SessionOutput::new(&session, false)],
            meta: PaginationMeta {
                state: PaginationState::More,
                total: 2,
                offset: 0,
                limit: 1,
                returned: 1,
                next_offset: Some(1),
            },
        })
        .unwrap();

        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["meta"]["state"], "more");
        assert_eq!(json["meta"]["next_offset"], 1);
        assert!(json["sessions"][0].get("content").is_none());
        assert!(json["sessions"][0].get("mtime").is_none());
        assert!(json["sessions"][0].get("yolo").is_none());
    }
}
