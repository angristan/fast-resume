use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Local;
use rusqlite::{Connection, params_from_iter};
use serde_json::Value;
use walkdir::WalkDir;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp};

use super::shared::{
    datetime_to_seconds, deleted_ids_for_agent, raw_stats_for_tree, session_needs_update,
    string_at, timestamp_from_ms, value_i64_at,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

#[derive(Debug, Clone)]
pub struct OpenCodeAdapter {
    data_dir: PathBuf,
    db_path: PathBuf,
    legacy_dir: PathBuf,
}

impl Default for OpenCodeAdapter {
    fn default() -> Self {
        Self {
            data_dir: config::opencode_dir(),
            db_path: config::opencode_db(),
            legacy_dir: config::opencode_legacy_dir(),
        }
    }
}

impl Adapter for OpenCodeAdapter {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn find_sessions(&self) -> Vec<Session> {
        if self.db_path.exists() {
            return load_opencode_db(self.name(), &self.db_path);
        }
        load_opencode_legacy(self.name(), &self.legacy_dir)
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        if self.db_path.exists() {
            return load_opencode_db_incremental(self.name(), &self.db_path, known);
        }
        load_opencode_legacy_incremental(self.name(), &self.legacy_dir, known)
    }

    fn find_sessions_incremental_streaming(
        &self,
        known: &KnownSessions,
        on_session: &mut SessionCallback<'_>,
    ) -> IncrementalScan {
        let scan = self.find_sessions_incremental(known);
        for session in &scan.new_or_modified {
            on_session(session.clone());
        }
        scan
    }

    fn resume_command(&self, session: &Session, _yolo: bool) -> Vec<String> {
        vec![
            "opencode".to_string(),
            session.directory.clone(),
            "--session".to_string(),
            session.id.clone(),
        ]
    }

    fn raw_stats(&self) -> RawAdapterStats {
        if self.db_path.exists() {
            let mut total_bytes = self.db_path.metadata().map(|m| m.len()).unwrap_or(0);
            let mut files = 1usize;
            for suffix in ["-wal", "-shm"] {
                let path = self.db_path.with_file_name(format!(
                    "{}{}",
                    self.db_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    suffix
                ));
                if let Ok(meta) = path.metadata() {
                    total_bytes += meta.len();
                    files += 1;
                }
            }
            return RawAdapterStats {
                agent: self.name(),
                data_dir: format!("{} (sqlite)", self.data_dir.display()),
                available: true,
                file_count: files,
                total_bytes,
            };
        }
        raw_stats_for_tree(self.name(), &self.legacy_dir, "json")
    }
}

fn load_opencode_db(agent: &'static str, db_path: &Path) -> Vec<Session> {
    let Ok(conn) = Connection::open(db_path) else {
        return Vec::new();
    };

    let mut sessions_meta = Vec::new();
    let mut stmt = match conn.prepare(
        "SELECT id, title, directory, time_created, time_updated FROM session ORDER BY time_updated DESC",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(3)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(4)?.unwrap_or_default(),
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };
    for row in rows.filter_map(Result::ok) {
        sessions_meta.push(row);
    }
    drop(stmt);

    let mut messages_by_session: HashMap<String, Vec<(String, String)>> = HashMap::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT id, session_id, data FROM message ORDER BY time_created ASC")
    {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }) {
            for (msg_id, session_id, data) in rows.filter_map(Result::ok) {
                let role = serde_json::from_str::<Value>(&data)
                    .ok()
                    .map(|value| string_at(&value, &["role"]))
                    .unwrap_or_default();
                messages_by_session
                    .entry(session_id)
                    .or_default()
                    .push((msg_id, role));
            }
        }
    }

    let mut parts_by_message: HashMap<String, Vec<String>> = HashMap::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT message_id, data FROM part ORDER BY time_created ASC")
    {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
            for (message_id, data) in rows.filter_map(Result::ok) {
                let Ok(value) = serde_json::from_str::<Value>(&data) else {
                    continue;
                };
                if string_at(&value, &["type"]) == "text" {
                    let text = string_at(&value, &["text"]);
                    if !text.is_empty() {
                        parts_by_message.entry(message_id).or_default().push(text);
                    }
                }
            }
        }
    }

    let mut sessions = Vec::new();
    for (id, title, directory, time_created, time_updated) in sessions_meta {
        let mut rendered = Vec::new();
        let session_messages = messages_by_session.remove(&id).unwrap_or_default();
        for (message_id, role) in &session_messages {
            let prefix = if role == "user" { "» " } else { "  " };
            for text in parts_by_message
                .get(message_id)
                .cloned()
                .unwrap_or_default()
            {
                rendered.push(format!("{prefix}{text}"));
            }
        }
        let timestamp =
            timestamp_from_ms(Some(time_created.max(time_updated))).unwrap_or_else(Local::now);
        let mut session = Session::new(
            id,
            agent,
            if title.is_empty() {
                "Untitled session".to_string()
            } else {
                title
            },
            directory,
            timestamp,
            rendered.join("\n\n"),
            session_messages.len(),
        );
        session.mtime = session.timestamp.timestamp() as f64;
        sessions.push(session);
    }
    sessions
}

fn load_opencode_db_incremental(
    agent: &'static str,
    db_path: &Path,
    known: &KnownSessions,
) -> IncrementalScan {
    let Ok(conn) = Connection::open(db_path) else {
        return IncrementalScan {
            agent,
            new_or_modified: Vec::new(),
            deleted_ids: deleted_ids_for_agent(known, agent, &HashSet::new()),
        };
    };

    let mut stmt = match conn
        .prepare("SELECT id, title, directory, time_created, time_updated FROM session")
    {
        Ok(stmt) => stmt,
        Err(_) => {
            return IncrementalScan {
                agent,
                new_or_modified: Vec::new(),
                deleted_ids: deleted_ids_for_agent(known, agent, &HashSet::new()),
            };
        }
    };

    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(3)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(4)?.unwrap_or_default(),
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => {
            return IncrementalScan {
                agent,
                new_or_modified: Vec::new(),
                deleted_ids: deleted_ids_for_agent(known, agent, &HashSet::new()),
            };
        }
    };

    let mut current_ids = HashSet::new();
    let mut sessions_to_fetch = Vec::new();
    for row in rows.filter_map(Result::ok) {
        let (id, title, directory, time_created, time_updated) = row;
        current_ids.insert(id.clone());
        let timestamp_ms = time_created.max(time_updated);
        let mtime = timestamp_from_ms(Some(timestamp_ms))
            .map(datetime_to_seconds)
            .unwrap_or_else(|| file_mtime_seconds(db_path));
        if session_needs_update(known, agent, &id, mtime) {
            sessions_to_fetch.push((id, title, directory, time_created, time_updated, mtime));
        }
    }
    drop(stmt);

    let deleted_ids = deleted_ids_for_agent(known, agent, &current_ids);
    if sessions_to_fetch.is_empty() {
        return IncrementalScan {
            agent,
            new_or_modified: Vec::new(),
            deleted_ids,
        };
    }

    let fetch_ids: Vec<_> = sessions_to_fetch
        .iter()
        .map(|(id, _, _, _, _, _)| id.clone())
        .collect();
    let mut messages_by_session: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for chunk in fetch_ids.chunks(900) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let query = format!(
            "SELECT id, session_id, data FROM message WHERE session_id IN ({placeholders}) ORDER BY time_created ASC"
        );
        let Ok(mut stmt) = conn.prepare(&query) else {
            continue;
        };
        let Ok(rows) = stmt.query_map(params_from_iter(chunk.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }) else {
            continue;
        };
        for (msg_id, session_id, data) in rows.filter_map(Result::ok) {
            let role = serde_json::from_str::<Value>(&data)
                .ok()
                .map(|value| string_at(&value, &["role"]))
                .unwrap_or_default();
            messages_by_session
                .entry(session_id)
                .or_default()
                .push((msg_id, role));
        }
    }

    let mut parts_by_message: HashMap<String, Vec<String>> = HashMap::new();
    for chunk in fetch_ids.chunks(900) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let query = format!(
            "SELECT p.message_id, p.data FROM part p JOIN message m ON m.id = p.message_id WHERE m.session_id IN ({placeholders}) ORDER BY p.time_created ASC"
        );
        let Ok(mut stmt) = conn.prepare(&query) else {
            continue;
        };
        let Ok(rows) = stmt.query_map(params_from_iter(chunk.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) else {
            continue;
        };
        for (message_id, data) in rows.filter_map(Result::ok) {
            let Ok(value) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            if string_at(&value, &["type"]) == "text" {
                let text = string_at(&value, &["text"]);
                if !text.is_empty() {
                    parts_by_message.entry(message_id).or_default().push(text);
                }
            }
        }
    }

    let mut new_or_modified = Vec::new();
    for (id, title, directory, time_created, time_updated, mtime) in sessions_to_fetch {
        let mut rendered = Vec::new();
        let session_messages = messages_by_session.remove(&id).unwrap_or_default();
        for (message_id, role) in &session_messages {
            let prefix = if role == "user" { "» " } else { "  " };
            for text in parts_by_message
                .get(message_id)
                .cloned()
                .unwrap_or_default()
            {
                rendered.push(format!("{prefix}{text}"));
            }
        }
        let timestamp =
            timestamp_from_ms(Some(time_created.max(time_updated))).unwrap_or_else(Local::now);
        let mut session = Session::new(
            id,
            agent,
            if title.is_empty() {
                "Untitled session".to_string()
            } else {
                title
            },
            directory,
            timestamp,
            rendered.join("\n\n"),
            session_messages.len(),
        );
        session.mtime = mtime;
        new_or_modified.push(session);
    }

    IncrementalScan {
        agent,
        new_or_modified,
        deleted_ids,
    }
}

fn load_opencode_legacy(agent: &'static str, legacy_dir: &Path) -> Vec<Session> {
    let session_dir = legacy_dir.join("session");
    let message_dir = legacy_dir.join("message");
    let part_dir = legacy_dir.join("part");
    if !session_dir.exists() {
        return Vec::new();
    }

    let mut messages_by_session: HashMap<String, Vec<(PathBuf, String, String)>> = HashMap::new();
    if message_dir.exists() {
        for entry in WalkDir::new(&message_dir)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("msg_") && name.ends_with(".json"))
            {
                continue;
            }
            let Ok(data) = serde_json::from_slice::<Value>(&fs::read(path).unwrap_or_default())
            else {
                continue;
            };
            let msg_id = string_at(&data, &["id"]);
            let role = string_at(&data, &["role"]);
            let Some(session_id) = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
            else {
                continue;
            };
            if !msg_id.is_empty() {
                messages_by_session
                    .entry(session_id.to_string())
                    .or_default()
                    .push((path.to_path_buf(), msg_id, role));
            }
        }
    }

    let mut parts_by_message: HashMap<String, Vec<String>> = HashMap::new();
    if part_dir.exists() {
        for entry in WalkDir::new(&part_dir).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(data) = serde_json::from_slice::<Value>(&fs::read(path).unwrap_or_default())
            else {
                continue;
            };
            if string_at(&data, &["type"]) != "text" {
                continue;
            }
            let text = string_at(&data, &["text"]);
            let Some(message_id) = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
            else {
                continue;
            };
            if !text.is_empty() {
                parts_by_message
                    .entry(message_id.to_string())
                    .or_default()
                    .push(text);
            }
        }
    }

    let mut sessions = Vec::new();
    for entry in WalkDir::new(&session_dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("ses_") && name.ends_with(".json"))
        {
            continue;
        }
        let Ok(data) = serde_json::from_slice::<Value>(&fs::read(path).unwrap_or_default()) else {
            continue;
        };
        let id = string_at(&data, &["id"]);
        if id.is_empty() {
            continue;
        }
        let title = {
            let value = string_at(&data, &["title"]);
            if value.is_empty() {
                "Untitled session".to_string()
            } else {
                value
            }
        };
        let directory = string_at(&data, &["directory"]);
        let time_ms = value_i64_at(&data, &["time", "updated"])
            .or_else(|| value_i64_at(&data, &["time", "created"]));
        let timestamp = timestamp_from_ms(time_ms).unwrap_or_else(|| file_timestamp(path));

        let mut rendered = Vec::new();
        let mut session_messages = messages_by_session.remove(&id).unwrap_or_default();
        session_messages.sort_by(|a, b| a.0.cmp(&b.0));
        for (_path, msg_id, role) in &session_messages {
            let prefix = if role == "user" { "» " } else { "  " };
            for text in parts_by_message.get(msg_id).cloned().unwrap_or_default() {
                rendered.push(format!("{prefix}{text}"));
            }
        }

        let mut session = Session::new(
            id,
            agent,
            title,
            directory,
            timestamp,
            rendered.join("\n\n"),
            session_messages.len(),
        );
        session.mtime = file_mtime_seconds(path);
        sessions.push(session);
    }
    sessions
}

fn load_opencode_legacy_incremental(
    agent: &'static str,
    legacy_dir: &Path,
    known: &KnownSessions,
) -> IncrementalScan {
    let current_files = scan_opencode_legacy_sessions(legacy_dir);
    let current_ids: HashSet<_> = current_files.keys().cloned().collect();
    let deleted_ids = deleted_ids_for_agent(known, agent, &current_ids);
    let changed_ids: HashSet<_> = current_files
        .iter()
        .filter_map(|(id, (_, mtime))| {
            session_needs_update(known, agent, id, *mtime).then(|| id.clone())
        })
        .collect();

    if changed_ids.is_empty() {
        return IncrementalScan {
            agent,
            new_or_modified: Vec::new(),
            deleted_ids,
        };
    }

    let mut new_or_modified = Vec::new();
    for mut session in load_opencode_legacy(agent, legacy_dir) {
        if !changed_ids.contains(&session.id) {
            continue;
        }
        if let Some((_, mtime)) = current_files.get(&session.id) {
            session.mtime = *mtime;
        }
        new_or_modified.push(session);
    }

    IncrementalScan {
        agent,
        new_or_modified,
        deleted_ids,
    }
}

fn scan_opencode_legacy_sessions(legacy_dir: &Path) -> HashMap<String, (PathBuf, f64)> {
    let mut current_files = HashMap::new();
    let session_dir = legacy_dir.join("session");
    if !session_dir.exists() {
        return current_files;
    }

    for entry in WalkDir::new(&session_dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("ses_") && name.ends_with(".json"))
        {
            continue;
        }
        let Ok(data) = serde_json::from_slice::<Value>(&fs::read(path).unwrap_or_default()) else {
            continue;
        };
        let id = string_at(&data, &["id"]);
        if id.is_empty() {
            continue;
        }
        let time_ms = value_i64_at(&data, &["time", "updated"])
            .or_else(|| value_i64_at(&data, &["time", "created"]));
        let mtime = timestamp_from_ms(time_ms)
            .map(datetime_to_seconds)
            .unwrap_or_else(|| file_mtime_seconds(path));
        current_files.insert(id, (path.to_path_buf(), mtime));
    }

    current_files
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::adapters::Adapter;

    use super::*;

    #[test]
    fn parses_legacy_session_and_resume_command() {
        let temp = tempdir().unwrap();
        let legacy_dir = temp.path().join("legacy");
        let session_dir = legacy_dir.join("session");
        let message_dir = legacy_dir.join("message/opencode-1");
        let part_dir = legacy_dir.join("part/msg-1");
        fs::create_dir_all(&session_dir).unwrap();
        fs::create_dir_all(&message_dir).unwrap();
        fs::create_dir_all(&part_dir).unwrap();

        fs::write(
            session_dir.join("ses_opencode-1.json"),
            json!({
                "id": "opencode-1",
                "title": "OpenCode thread",
                "directory": "/work/opencode",
                "time": {"updated": 1_720_000_000_000_i64}
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            message_dir.join("msg_1.json"),
            json!({"id": "msg-1", "role": "user"}).to_string(),
        )
        .unwrap();
        fs::write(
            part_dir.join("part.json"),
            json!({"type": "text", "text": "Hello OpenCode"}).to_string(),
        )
        .unwrap();

        let adapter = OpenCodeAdapter {
            data_dir: temp.path().join("data"),
            db_path: temp.path().join("data/opencode.db"),
            legacy_dir,
        };
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "opencode-1");
        assert_eq!(sessions[0].title, "OpenCode thread");
        assert_eq!(sessions[0].directory, "/work/opencode");
        assert!(sessions[0].content.contains("» Hello OpenCode"));
        assert_eq!(
            adapter.resume_command(&sessions[0], false),
            vec!["opencode", "/work/opencode", "--session", "opencode-1"]
        );
    }
}
