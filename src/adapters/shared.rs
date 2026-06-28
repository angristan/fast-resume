use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use serde_json::Value;
use walkdir::WalkDir;

use crate::model::{RawAdapterStats, Session};

use super::{IncrementalScan, KnownSessions, MTIME_TOLERANCE, SessionCallback};

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
        .filter_map(|((known_agent, id), _)| {
            (known_agent == agent && !current_ids.contains(id)).then(|| id.clone())
        })
        .collect()
}

pub(super) fn incremental_from_files<F>(
    agent: &'static str,
    known: &KnownSessions,
    current_files: HashMap<String, (PathBuf, f64)>,
    mut parse: F,
) -> IncrementalScan
where
    F: FnMut(&Path) -> Option<Session>,
{
    let current_ids: HashSet<_> = current_files.keys().cloned().collect();
    let mut new_or_modified = Vec::new();

    for (session_id, (path, mtime)) in current_files {
        if !session_needs_update(known, agent, &session_id, mtime) {
            continue;
        }
        if let Some(mut session) = parse(&path) {
            session.mtime = mtime;
            new_or_modified.push(session);
        }
    }

    IncrementalScan {
        agent,
        new_or_modified,
        deleted_ids: deleted_ids_for_agent(known, agent, &current_ids),
    }
}

pub(super) fn incremental_from_files_streaming<F>(
    agent: &'static str,
    known: &KnownSessions,
    current_files: HashMap<String, (PathBuf, f64)>,
    mut parse: F,
    on_session: &mut SessionCallback<'_>,
) -> IncrementalScan
where
    F: FnMut(&Path) -> Option<Session>,
{
    let current_ids: HashSet<_> = current_files.keys().cloned().collect();
    let mut new_or_modified = Vec::new();

    for (session_id, (path, mtime)) in current_files {
        if !session_needs_update(known, agent, &session_id, mtime) {
            continue;
        }
        if let Some(mut session) = parse(&path) {
            session.mtime = mtime;
            on_session(session.clone());
            new_or_modified.push(session);
        }
    }

    IncrementalScan {
        agent,
        new_or_modified,
        deleted_ids: deleted_ids_for_agent(known, agent, &current_ids),
    }
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
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
                .ok()
                .and_then(|dt| Local.from_local_datetime(&dt).single())
                .map(datetime_to_seconds)
        })
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
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
                .ok()
                .and_then(|dt| Local.from_local_datetime(&dt).single())
        })
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

pub(super) fn normalize_seconds(value: i64) -> Option<i64> {
    if value <= 0 {
        None
    } else if value > 100_000_000_000 {
        Some(value / 1000)
    } else {
        Some(value)
    }
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
    use super::*;

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
}
