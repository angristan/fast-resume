use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use rayon::prelude::*;
use serde::de::IgnoredAny;
use serde_json::Value;
use walkdir::WalkDir;

use crate::model::{RawAdapterStats, Session, file_mtime_seconds};

use super::{IncrementalScan, KnownSessions, MTIME_TOLERANCE, SessionCallback};

/// Session files keyed by id with `(path, mtime)`, plus a completeness flag
/// that is false when any part of the scan could not be read.
pub(super) type SessionFileScan = (HashMap<String, (PathBuf, f64)>, bool);

pub(super) enum IncrementalParse {
    Session(Session),
    Delete,
    Retain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum JsonlHealth {
    Clean,
    Partial,
    Invalid,
}

/// Classifies a JSONL file row by row, so parsers that already decode every
/// row can report health without a second pass over the file.
///
/// A file is `Partial` when malformed rows are followed by valid rows (for
/// example a torn write in the middle of a session), and `Invalid` when it has
/// no valid rows or ends in a malformed row (typically a writer still
/// appending). Unreadable files should be reported as `Invalid` directly.
#[derive(Debug, Default)]
pub(super) struct JsonlHealthTracker {
    valid_rows: usize,
    malformed_rows: usize,
    valid_after_last_malformed: bool,
}

impl JsonlHealthTracker {
    pub(super) fn valid_row(&mut self) {
        self.valid_rows += 1;
        if self.malformed_rows > 0 {
            self.valid_after_last_malformed = true;
        }
    }

    pub(super) fn malformed_row(&mut self) {
        self.malformed_rows += 1;
        self.valid_after_last_malformed = false;
    }

    pub(super) fn finish(&self) -> JsonlHealth {
        match (self.valid_rows, self.malformed_rows) {
            (_, 0) => JsonlHealth::Clean,
            (0, _) => JsonlHealth::Invalid,
            _ if self.valid_after_last_malformed => JsonlHealth::Partial,
            _ => JsonlHealth::Invalid,
        }
    }
}

pub(super) fn session_needs_update(
    known: &KnownSessions,
    agent: &str,
    id: &str,
    mtime: f64,
) -> bool {
    known
        .get(&(agent.to_string(), id.to_string()))
        .is_none_or(|known_mtime| (mtime - *known_mtime).abs() > MTIME_TOLERANCE)
}

pub(super) fn deleted_ids_for_agent(
    known: &KnownSessions,
    agent: &str,
    current_ids: &HashSet<String>,
) -> Vec<String> {
    known
        .iter()
        .filter(|&((known_agent, id), _)| known_agent == agent && !current_ids.contains(id))
        .map(|((_known_agent, id), _)| id.clone())
        .collect()
}

pub(super) fn failed_incremental_scan(agent: &'static str) -> IncrementalScan {
    IncrementalScan {
        agent,
        new_or_modified: Vec::new(),
        deleted_ids: Vec::new(),
    }
}

pub(super) fn sqlite_mtime(path: &Path) -> f64 {
    file_mtime_seconds(path).max(file_mtime_seconds(&sqlite_sidecar_path(path, "-wal")))
}

pub(super) fn sqlite_file_stats(path: &Path) -> (usize, u64) {
    [
        path.to_path_buf(),
        sqlite_sidecar_path(path, "-wal"),
        sqlite_sidecar_path(path, "-shm"),
    ]
    .into_iter()
    .filter_map(|path| path.metadata().ok())
    .fold((0, 0), |(files, bytes), metadata| {
        (files + 1, bytes + metadata.len())
    })
}

fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    path.with_file_name(format!(
        "{}{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        suffix
    ))
}

/// Run the shared incremental-scan skeleton over keyed session files,
/// streaming each parsed session to `on_session` when a callback is given.
/// `scanned` is None when the file scan itself failed. An incomplete scan
/// (`complete == false`) reports no deletions: files that could not be
/// listed may still exist, and deleting them would drop good indexed data.
///
/// Changed files are parsed in parallel on the rayon pool. Transcripts vary
/// from a few kilobytes to hundreds of megabytes, so a serial loop makes a
/// refresh as slow as the sum of every changed file. Parsed results are sent
/// back to the calling thread, which owns `on_session` (it is `FnMut`, not
/// `Sync`) and streams sessions in completion order.
pub(super) fn incremental_scan<F>(
    agent: &'static str,
    known: &KnownSessions,
    scanned: Option<SessionFileScan>,
    parse: F,
    mut on_session: Option<&mut SessionCallback<'_>>,
) -> IncrementalScan
where
    F: Fn(&Path) -> IncrementalParse + Sync,
{
    let Some((current_files, complete)) = scanned else {
        return failed_incremental_scan(agent);
    };
    let mut current_ids = HashSet::with_capacity(current_files.len());
    let mut changed = Vec::new();
    for (session_id, (path, mtime)) in current_files {
        if session_needs_update(known, agent, &session_id, mtime) {
            changed.push((session_id.clone(), path, mtime));
        }
        current_ids.insert(session_id);
    }

    let mut new_or_modified = Vec::with_capacity(changed.len());
    if !changed.is_empty() {
        let parse = &parse;
        thread::scope(|scope| {
            let (tx, rx) = mpsc::channel();
            // The producer runs on its own scoped thread so this thread can
            // stream results while rayon workers are still parsing. Blocking
            // on the channel from inside a rayon worker could starve the pool.
            scope.spawn(move || {
                changed
                    .into_par_iter()
                    .for_each_with(tx, |tx, (session_id, path, mtime)| {
                        // The receiver only disappears if this thread panicked.
                        let _ = tx.send((session_id, mtime, parse(&path)));
                    });
            });

            for (session_id, mtime, parsed) in rx {
                match parsed {
                    IncrementalParse::Session(mut session) => {
                        session.mtime = mtime;
                        // Keep the parsed id alive as well as the file key.
                        // They match for well-formed files, but if a file's
                        // key and embedded id ever diverge, deleting the
                        // parsed id would drop the session we just indexed.
                        current_ids.insert(session.id.clone());
                        if let Some(on_session) = on_session.as_mut() {
                            on_session(session.clone());
                        }
                        new_or_modified.push(session);
                    }
                    IncrementalParse::Delete => {
                        current_ids.remove(&session_id);
                    }
                    IncrementalParse::Retain => {}
                }
            }
        });
    }

    IncrementalScan {
        agent,
        new_or_modified,
        deleted_ids: if complete {
            deleted_ids_for_agent(known, agent, &current_ids)
        } else {
            Vec::new()
        },
    }
}

/// Build the common resume command shape: binary, yolo flags when yolo mode
/// is on and the agent supports it, resume flags, then the session id.
pub(super) fn build_resume_command(
    binary: &str,
    yolo_flags: &[&str],
    yolo: bool,
    resume_flags: &[&str],
    id: &str,
) -> Vec<String> {
    let mut command = vec![binary.to_string()];
    if yolo {
        command.extend(yolo_flags.iter().map(|flag| (*flag).to_string()));
    }
    command.extend(resume_flags.iter().map(|flag| (*flag).to_string()));
    command.push(id.to_string());
    command
}

pub(super) fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&value[index + 1..index + 3], 16)
        {
            output.push(byte);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).unwrap_or_else(|_| value.to_string())
}

pub(super) fn incremental_parse_from_option(session: Option<Session>) -> IncrementalParse {
    session.map_or(IncrementalParse::Delete, IncrementalParse::Session)
}

pub(super) fn incremental_parse_jsonl<F>(path: &Path, parse: F) -> IncrementalParse
where
    F: FnOnce() -> Option<Session>,
{
    incremental_parse_jsonl_with_partial_check(path, parse, |_| true)
}

pub(super) fn incremental_parse_jsonl_with_partial_check<F, P>(
    path: &Path,
    parse: F,
    partial_session_is_usable: P,
) -> IncrementalParse
where
    F: FnOnce() -> Option<Session>,
    P: FnOnce(&Session) -> bool,
{
    match jsonl_health(path) {
        JsonlHealth::Invalid => IncrementalParse::Retain,
        health => incremental_parse_with_health(health, parse(), partial_session_is_usable),
    }
}

/// Map a parse result and the file's JSONL health to an incremental decision.
/// Invalid files keep their indexed data, partial files only replace it when
/// the parsed session is still usable, and clean files that yield no session
/// are deleted.
pub(super) fn incremental_parse_with_health<P>(
    health: JsonlHealth,
    session: Option<Session>,
    partial_session_is_usable: P,
) -> IncrementalParse
where
    P: FnOnce(&Session) -> bool,
{
    match health {
        JsonlHealth::Invalid => IncrementalParse::Retain,
        JsonlHealth::Partial => session
            .filter(partial_session_is_usable)
            .map_or(IncrementalParse::Retain, IncrementalParse::Session),
        JsonlHealth::Clean => incremental_parse_from_option(session),
    }
}

pub(super) fn json_file_has_parse_errors(path: &Path) -> bool {
    let Ok(data) = fs::read(path) else {
        return true;
    };
    serde_json::from_slice::<Value>(&data).is_err()
}

fn jsonl_health(path: &Path) -> JsonlHealth {
    let Ok(file) = fs::File::open(path) else {
        return JsonlHealth::Invalid;
    };
    let mut health = JsonlHealthTracker::default();
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            return JsonlHealth::Invalid;
        };
        if line.trim().is_empty() {
            continue;
        }
        // `IgnoredAny` validates the syntax without building a `Value` tree,
        // which matters for multi-hundred-megabyte agent transcripts.
        if serde_json::from_str::<IgnoredAny>(&line).is_err() {
            health.malformed_row();
        } else {
            health.valid_row();
        }
    }
    health.finish()
}

pub(super) fn content_texts(content: &Value) -> Vec<String> {
    match content {
        Value::String(text) if !text.is_empty() => vec![text.clone()],
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                text_from_part(part).or_else(|| part.as_str().map(ToString::to_string))
            })
            .filter(|text| !text.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

pub(super) fn text_from_part(part: &Value) -> Option<String> {
    if let Some(text) = part.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = part.get("input_text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    None
}

pub(super) fn string_at(value: &Value, path: &[&str]) -> String {
    let mut current = value;
    for key in path {
        current = current.get(*key).unwrap_or(&Value::Null);
    }
    current.as_str().unwrap_or_default().to_string()
}

pub(super) fn value_i64_at(value: &Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_i64()
        .or_else(|| current.as_f64().map(|v| v as i64))
}

pub(super) fn fallback_session_id(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    stem.split_once('-')
        .map(|(_, rest)| rest.to_string())
        .unwrap_or_else(|| stem.to_string())
}

pub(super) fn codex_session_id_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let candidate = stem.get(stem.len().saturating_sub(36)..)?;
    is_uuid_like(candidate).then(|| candidate.to_string())
}

pub(super) fn is_uuid_like(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    value.chars().enumerate().all(|(idx, ch)| match idx {
        8 | 13 | 18 | 23 => ch == '-',
        _ => ch.is_ascii_hexdigit(),
    })
}

pub(super) fn copilot_fallback_session_id(path: &Path, sessions_dir: &Path) -> String {
    if path.parent() != Some(sessions_dir) {
        path.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string()
    } else {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string()
    }
}

pub(super) fn parse_timestamp_seconds(value: &str) -> Option<f64> {
    if value.trim().is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.timestamp() as f64 + f64::from(dt.timestamp_subsec_nanos()) / 1e9)
        .ok()
        .or_else(|| parse_naive_local_datetime(value).map(datetime_to_seconds))
}

pub(super) fn datetime_to_seconds(timestamp: DateTime<Local>) -> f64 {
    timestamp.timestamp() as f64 + f64::from(timestamp.timestamp_subsec_nanos()) / 1e9
}

pub(super) fn parse_datetime(value: &str) -> Option<DateTime<Local>> {
    if value.trim().is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Local))
        .ok()
        .or_else(|| parse_naive_local_datetime(value))
}

fn parse_naive_local_datetime(value: &str) -> Option<DateTime<Local>> {
    ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S"]
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(value, format).ok())
        .and_then(|dt| Local.from_local_datetime(&dt).single())
}

pub(super) fn timestamp_from_ms(value: Option<i64>) -> Option<DateTime<Local>> {
    let value = value?;
    if value <= 0 {
        return None;
    }
    Local.timestamp_millis_opt(value).single()
}

pub(super) fn timestamp_from_seconds(value: Option<i64>) -> Option<DateTime<Local>> {
    let value = value?;
    if value <= 0 {
        return None;
    }
    Local.timestamp_opt(value, 0).single()
}

pub(super) fn raw_stats_for_tree(
    agent: &'static str,
    dir: &Path,
    extension: &str,
) -> RawAdapterStats {
    if !dir.exists() {
        return RawAdapterStats {
            agent,
            data_dir: dir.display().to_string(),
            available: false,
            file_count: 0,
            total_bytes: 0,
        };
    }
    let mut seen = HashSet::new();
    let mut total_bytes = 0;
    for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(extension) {
            continue;
        }
        if seen.insert(path.to_path_buf()) {
            total_bytes += path.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    RawAdapterStats {
        agent,
        data_dir: dir.display().to_string(),
        available: true,
        file_count: seen.len(),
        total_bytes,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    fn current_file(id: &str, mtime: f64) -> HashMap<String, (PathBuf, f64)> {
        HashMap::from([(id.to_string(), (PathBuf::from(id), mtime))])
    }

    #[test]
    fn classifies_clean_partial_and_invalid_jsonl() {
        let temp = tempdir().unwrap();
        let clean = temp.path().join("clean.jsonl");
        let partial = temp.path().join("partial.jsonl");
        let trailing_partial = temp.path().join("trailing-partial.jsonl");
        let invalid = temp.path().join("invalid.jsonl");
        fs::write(&clean, "{\"valid\":true}\n").unwrap();
        fs::write(&partial, "{\"valid\":true}\n{\n{\"later\":true}\n").unwrap();
        fs::write(&trailing_partial, "{\"valid\":true}\n{\n").unwrap();
        fs::write(&invalid, "{\n").unwrap();

        assert_eq!(jsonl_health(&clean), JsonlHealth::Clean);
        assert_eq!(jsonl_health(&partial), JsonlHealth::Partial);
        assert_eq!(jsonl_health(&trailing_partial), JsonlHealth::Invalid);
        assert_eq!(jsonl_health(&invalid), JsonlHealth::Invalid);
        assert_eq!(
            jsonl_health(&temp.path().join("missing.jsonl")),
            JsonlHealth::Invalid
        );
    }

    #[test]
    fn mtime_decreases_trigger_incremental_updates() {
        let mut known = KnownSessions::new();
        known.insert(("codex".to_string(), "abc123".to_string()), 10.0);

        assert!(!session_needs_update(
            &known,
            "codex",
            "abc123",
            10.0 + MTIME_TOLERANCE / 2.0
        ));
        assert!(session_needs_update(&known, "codex", "abc123", 9.0));
        assert!(session_needs_update(&known, "codex", "missing", 9.0));
    }

    #[test]
    fn changed_files_that_no_longer_parse_are_deleted() {
        let mut known = KnownSessions::new();
        known.insert(("codex".to_string(), "abc123".to_string()), 1.0);

        let scan = incremental_scan(
            "codex",
            &known,
            Some((current_file("abc123", 2.0), true)),
            |_| IncrementalParse::Delete,
            None,
        );

        assert!(scan.new_or_modified.is_empty());
        assert_eq!(scan.deleted_ids, vec!["abc123"]);
    }

    #[test]
    fn streaming_changed_files_that_no_longer_parse_are_deleted() {
        let mut known = KnownSessions::new();
        known.insert(("codex".to_string(), "abc123".to_string()), 1.0);
        let mut streamed = Vec::new();

        let scan = incremental_scan(
            "codex",
            &known,
            Some((current_file("abc123", 2.0), true)),
            |_| IncrementalParse::Delete,
            Some(&mut |session| streamed.push(session)),
        );

        assert!(streamed.is_empty());
        assert!(scan.new_or_modified.is_empty());
        assert_eq!(scan.deleted_ids, vec!["abc123"]);
    }

    #[test]
    fn parallel_parsing_routes_every_outcome_to_the_caller() {
        // Enough files to spread across rayon workers, with every outcome
        // mixed together. Each result must reach the streaming callback and
        // the returned scan exactly once, keyed to the right session.
        let ids: Vec<String> = (0..200).map(|index| format!("s{index}")).collect();
        let known: KnownSessions = ids
            .iter()
            .map(|id| (("codex".to_string(), id.clone()), 1.0))
            .collect();
        let files = ids
            .iter()
            .map(|id| (id.clone(), (PathBuf::from(id), 2.0)))
            .collect();
        let outcome = |id: &str| id[1..].parse::<usize>().unwrap() % 3;
        let mut streamed = Vec::new();

        let scan = incremental_scan(
            "codex",
            &known,
            Some((files, true)),
            |path| {
                let id = path.to_str().unwrap();
                match outcome(id) {
                    0 => IncrementalParse::Session(Session::new(
                        id,
                        "codex",
                        "title",
                        "/repo",
                        Local::now(),
                        "content",
                        1,
                    )),
                    1 => IncrementalParse::Delete,
                    _ => IncrementalParse::Retain,
                }
            },
            Some(&mut |session| streamed.push(session.id)),
        );

        let expected = |wanted| -> HashSet<String> {
            ids.iter()
                .filter(|id| outcome(id) == wanted)
                .cloned()
                .collect()
        };
        let returned: Vec<_> = scan.new_or_modified.iter().map(|s| s.id.clone()).collect();
        assert_eq!(streamed.len(), expected(0).len());
        assert_eq!(streamed.into_iter().collect::<HashSet<_>>(), expected(0));
        assert_eq!(returned.len(), expected(0).len());
        assert_eq!(returned.into_iter().collect::<HashSet<_>>(), expected(0));
        assert!(scan.new_or_modified.iter().all(|s| s.mtime == 2.0));
        assert_eq!(
            scan.deleted_ids.into_iter().collect::<HashSet<_>>(),
            expected(1)
        );
    }

    #[test]
    fn parse_failed_changed_files_are_retained() {
        let mut known = KnownSessions::new();
        known.insert(("codex".to_string(), "abc123".to_string()), 1.0);

        let scan = incremental_scan(
            "codex",
            &known,
            Some((current_file("abc123", 2.0), true)),
            |_| IncrementalParse::Retain,
            None,
        );

        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn streaming_parse_failed_changed_files_are_retained() {
        let mut known = KnownSessions::new();
        known.insert(("codex".to_string(), "abc123".to_string()), 1.0);
        let mut streamed = Vec::new();

        let scan = incremental_scan(
            "codex",
            &known,
            Some((current_file("abc123", 2.0), true)),
            |_| IncrementalParse::Retain,
            Some(&mut |session| streamed.push(session)),
        );

        assert!(streamed.is_empty());
        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn unchanged_files_are_retained_without_parsing() {
        let mut known = KnownSessions::new();
        known.insert(("codex".to_string(), "abc123".to_string()), 1.0);

        let scan = incremental_scan(
            "codex",
            &known,
            Some((current_file("abc123", 1.0), true)),
            |_| panic!("unchanged sessions should not be parsed"),
            None,
        );

        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn incomplete_scans_do_not_delete_known_sessions() {
        let mut known = KnownSessions::new();
        known.insert(("codex".to_string(), "gone".to_string()), 1.0);

        let scan = incremental_scan(
            "codex",
            &known,
            Some((HashMap::new(), false)),
            |_| IncrementalParse::Retain,
            None,
        );

        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn failed_file_scans_report_no_changes_or_deletions() {
        let mut known = KnownSessions::new();
        known.insert(("codex".to_string(), "gone".to_string()), 1.0);

        let scan = incremental_scan("codex", &known, None, |_| IncrementalParse::Retain, None);

        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn builds_resume_commands_with_optional_yolo_flags() {
        assert_eq!(
            build_resume_command("crush", &["--yolo"], false, &["--session"], "abc"),
            ["crush", "--session", "abc"]
        );
        assert_eq!(
            build_resume_command("crush", &["--yolo"], true, &["--session"], "abc"),
            ["crush", "--yolo", "--session", "abc"]
        );
    }

    #[test]
    fn failed_incremental_scans_do_not_delete_known_sessions() {
        let scan = failed_incremental_scan("codex");

        assert_eq!(scan.agent, "codex");
        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }
}
