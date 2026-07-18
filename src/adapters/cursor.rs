//! Cursor CLI stores each chat in a small SQLite key-value database. Values
//! vary across releases, so parsing accepts structured JSON and the legacy
//! plain-user-message encoding while leaving unknown binary records untouched.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use walkdir::WalkDir;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_timestamp, truncate_title};

use super::shared::{
    IncrementalParse, failed_incremental_scan, incremental_from_files,
    incremental_from_files_streaming, sqlite_file_stats, sqlite_mtime,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

type SessionFiles = HashMap<String, (PathBuf, f64)>;

#[derive(Debug, Clone)]
pub struct CursorAdapter {
    chats_dir: PathBuf,
}

impl Default for CursorAdapter {
    fn default() -> Self {
        Self {
            chats_dir: config::cursor_chats_dir(),
        }
    }
}

impl CursorAdapter {
    #[allow(dead_code)]
    pub fn new(chats_dir: PathBuf) -> Self {
        Self { chats_dir }
    }

    fn scan_session_files(&self) -> Option<(SessionFiles, bool)> {
        let mut files = HashMap::new();
        let mut complete = true;
        if !self.chats_dir.exists() {
            return Some((files, complete));
        }
        if !self.chats_dir.is_dir() {
            return None;
        }
        for entry in WalkDir::new(&self.chats_dir) {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let path = entry.path();
            if path.file_name().and_then(|name| name.to_str()) != Some("store.db") {
                continue;
            }
            let Some(id) = path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            files.insert(id.to_string(), (path.to_path_buf(), sqlite_mtime(path)));
        }
        Some((files, complete))
    }

    fn parse_session(&self, path: &Path) -> rusqlite::Result<Option<Session>> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let mut title = String::new();
        let mut directory = String::new();
        let mut created_at = None;
        let mut messages = Vec::new();

        for (_, bytes) in read_key_value_table(&connection, "meta")? {
            let Some(value) = decode_json(&bytes) else {
                continue;
            };
            apply_cursor_metadata(&value, &mut title, &mut directory, &mut created_at);
        }

        for (_, bytes) in read_key_value_table(&connection, "blobs")? {
            if let Some(value) = decode_json(&bytes) {
                collect_cursor_records(&value, &mut messages, &mut title, &mut created_at);
            } else if let Some(text) = cursor_plain_user_message(&bytes) {
                messages.push(CursorMessage {
                    user: true,
                    text,
                    timestamp: None,
                });
            }
        }

        let first_user = messages
            .iter()
            .find(|message| message.user && !message.text.trim().is_empty())
            .map(|message| message.text.trim().to_string());
        let Some(first_user) = first_user else {
            return Ok(None);
        };
        if title.trim().is_empty() {
            title = first_user.clone();
        }
        if directory.trim().is_empty() {
            directory = cursor_directory_from_path(path);
        }
        let timestamp = messages
            .iter()
            .filter_map(|message| message.timestamp)
            .max()
            .or(created_at)
            .unwrap_or_else(|| file_timestamp(path));
        let user_turns = messages.iter().filter(|message| message.user).count();
        let content = messages
            .into_iter()
            .filter(|message| !message.text.trim().is_empty())
            .map(|message| {
                format!(
                    "{}{}",
                    if message.user { "» " } else { "  " },
                    message.text.trim()
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let id = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let mut session = Session::new(
            id,
            self.name(),
            truncate_title(&title, 100, true),
            directory,
            timestamp,
            content,
            user_turns,
        );
        session.mtime = sqlite_mtime(path);
        Ok(Some(session))
    }

    fn parse_incremental(&self, path: &Path) -> IncrementalParse {
        match self.parse_session(path) {
            Ok(Some(session)) => IncrementalParse::Session(session),
            Ok(None) => IncrementalParse::Delete,
            Err(_) => IncrementalParse::Retain,
        }
    }
}

impl Adapter for CursorAdapter {
    fn name(&self) -> &'static str {
        "cursor"
    }

    fn supports_yolo(&self) -> bool {
        true
    }

    fn find_sessions(&self) -> Vec<Session> {
        self.scan_session_files()
            .map(|(files, _)| {
                files
                    .into_values()
                    .filter_map(|(path, _)| self.parse_session(&path).ok().flatten())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        let Some((files, complete)) = self.scan_session_files() else {
            return failed_incremental_scan(self.name());
        };
        let mut scan = incremental_from_files(self.name(), known, files, |path| {
            self.parse_incremental(path)
        });
        if !complete {
            scan.deleted_ids.clear();
        }
        scan
    }

    fn find_sessions_incremental_streaming(
        &self,
        known: &KnownSessions,
        on_session: &mut SessionCallback<'_>,
    ) -> IncrementalScan {
        let Some((files, complete)) = self.scan_session_files() else {
            return failed_incremental_scan(self.name());
        };
        let mut scan = incremental_from_files_streaming(
            self.name(),
            known,
            files,
            |path| self.parse_incremental(path),
            on_session,
        );
        if !complete {
            scan.deleted_ids.clear();
        }
        scan
    }

    fn resume_command(&self, session: &Session, yolo: bool) -> Vec<String> {
        let mut command = vec!["agent".to_string()];
        if yolo {
            command.push("--yolo".to_string());
        }
        command.extend(["--resume".to_string(), session.id.clone()]);
        command
    }

    fn raw_stats(&self) -> RawAdapterStats {
        if !self.chats_dir.exists() {
            return RawAdapterStats {
                agent: self.name(),
                data_dir: self.chats_dir.display().to_string(),
                available: false,
                file_count: 0,
                total_bytes: 0,
            };
        }
        let mut file_count = 0;
        let mut total_bytes = 0;
        for entry in WalkDir::new(&self.chats_dir)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.path().file_name().and_then(|name| name.to_str()) == Some("store.db") {
                let (files, bytes) = sqlite_file_stats(entry.path());
                file_count += files;
                total_bytes += bytes;
            }
        }
        RawAdapterStats {
            agent: self.name(),
            data_dir: self.chats_dir.display().to_string(),
            available: true,
            file_count,
            total_bytes,
        }
    }
}

#[derive(Debug)]
struct CursorMessage {
    user: bool,
    text: String,
    timestamp: Option<DateTime<Local>>,
}

fn read_key_value_table(
    connection: &Connection,
    table: &str,
) -> rusqlite::Result<Vec<(String, Vec<u8>)>> {
    let mut columns = Vec::new();
    let mut info = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = info.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        columns.push(row?);
    }
    let (key_column, value_column) = if columns.iter().any(|column| column == "key")
        && columns.iter().any(|column| column == "value")
    {
        ("key", "value")
    } else if columns.iter().any(|column| column == "id")
        && columns.iter().any(|column| column == "data")
    {
        ("id", "data")
    } else {
        return Ok(Vec::new());
    };
    let mut statement = connection.prepare(&format!(
        "SELECT {key_column}, {value_column} FROM {table} WHERE {value_column} IS NOT NULL ORDER BY rowid"
    ))?;
    let rows = statement.query_map([], |row| {
        let key = match row.get_ref(0)? {
            ValueRef::Text(value) | ValueRef::Blob(value) => {
                String::from_utf8_lossy(value).into_owned()
            }
            ValueRef::Integer(value) => value.to_string(),
            ValueRef::Real(value) => value.to_string(),
            ValueRef::Null => String::new(),
        };
        let value = match row.get_ref(1)? {
            ValueRef::Text(value) | ValueRef::Blob(value) => value.to_vec(),
            ValueRef::Integer(value) => value.to_string().into_bytes(),
            ValueRef::Real(value) => value.to_string().into_bytes(),
            ValueRef::Null => Vec::new(),
        };
        Ok((key, value))
    })?;
    rows.collect()
}

fn decode_json(bytes: &[u8]) -> Option<Value> {
    serde_json::from_slice(bytes).ok().or_else(|| {
        let start = bytes.iter().position(|byte| *byte == b'{')?;
        let end = bytes.iter().rposition(|byte| *byte == b'}')?;
        (start < end)
            .then(|| serde_json::from_slice(&bytes[start..=end]).ok())
            .flatten()
    })
}

fn apply_cursor_metadata(
    value: &Value,
    title: &mut String,
    directory: &mut String,
    created_at: &mut Option<DateTime<Local>>,
) {
    if title.is_empty() {
        *title = value
            .get("name")
            .or_else(|| value.get("title"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    if directory.is_empty() {
        *directory = value
            .get("cwd")
            .or_else(|| value.get("workspace"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    if created_at.is_none() {
        *created_at = cursor_timestamp(value.get("createdAt"));
    }
}

fn collect_cursor_records(
    value: &Value,
    messages: &mut Vec<CursorMessage>,
    title: &mut String,
    created_at: &mut Option<DateTime<Local>>,
) {
    apply_cursor_metadata(value, title, &mut String::new(), created_at);
    if let Some(items) = value.get("messages").and_then(Value::as_array) {
        for item in items {
            collect_cursor_records(item, messages, title, created_at);
        }
    }
    if let Some(items) = value.get("conversation").and_then(Value::as_array) {
        for item in items {
            collect_cursor_records(item, messages, title, created_at);
        }
    }

    let role = value.get("role").and_then(Value::as_str);
    let bubble_type = value.get("type");
    let user = match (role, bubble_type) {
        (Some("user"), _) => Some(true),
        (Some("assistant"), _) => Some(false),
        (_, Some(Value::Number(number))) if number.as_i64() == Some(1) => Some(true),
        (_, Some(Value::Number(number))) if number.as_i64() == Some(2) => Some(false),
        (_, Some(Value::String(kind))) if kind == "user" => Some(true),
        (_, Some(Value::String(kind))) if kind == "ai" || kind == "assistant" => Some(false),
        _ => None,
    };
    let Some(user) = user else {
        return;
    };
    let text = cursor_content_text(value);
    if text.trim().is_empty() {
        return;
    }
    messages.push(CursorMessage {
        user,
        text,
        timestamp: cursor_timestamp(value.get("timestamp")),
    });
}

fn cursor_content_text(value: &Value) -> String {
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return text.to_string();
    }
    match value.get("content") {
        Some(Value::String(text)) => text.to_string(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .or_else(|| part.get("data"))
                    .and_then(Value::as_str)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn cursor_plain_user_message(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let (content, id) = text.trim_matches(char::from(0)).trim().rsplit_once('$')?;
    let id = id.trim_matches(|character: char| character.is_control() || character.is_whitespace());
    if content.trim().is_empty() || id.len() < 8 || !id.contains('-') {
        return None;
    }
    Some(
        content
            .trim_matches(|character: char| character.is_control())
            .trim()
            .to_string(),
    )
}

fn cursor_timestamp(value: Option<&Value>) -> Option<DateTime<Local>> {
    let value = value?;
    if let Some(text) = value.as_str() {
        return super::shared::parse_datetime(text);
    }
    let number = value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))?;
    if number > 100_000_000_000 {
        Local.timestamp_millis_opt(number).single()
    } else {
        Local.timestamp_opt(number, 0).single()
    }
}

fn cursor_directory_from_path(path: &Path) -> String {
    let encoded = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let decoded = percent_decode(encoded);
    Path::new(&decoded)
        .is_absolute()
        .then_some(decoded)
        .unwrap_or_default()
}

fn percent_decode(value: &str) -> String {
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

#[cfg(test)]
mod tests {
    use std::{fs, thread, time::Duration};

    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn parses_cursor_store_database() {
        let temp = tempdir().unwrap();
        let id = "cursor-session-id";
        let session_dir = temp.path().join("%2Fwork%2Fcursor").join(id);
        fs::create_dir_all(&session_dir).unwrap();
        let db_path = session_dir.join("store.db");
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE meta (key TEXT, value BLOB); CREATE TABLE blobs (key TEXT, value BLOB);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)",
                (
                    "0",
                    json!({"createdAt":1784282400000_i64,"name":"Cursor adapter work"}).to_string(),
                ),
            )
            .unwrap();
        for (key, value) in [
            (
                "u1",
                json!({"id":"u1","role":"user","content":[{"type":"text","text":"Add Cursor support"}]}),
            ),
            (
                "a1",
                json!({"id":"a1","role":"assistant","content":[{"type":"text","text":"Added the adapter"}]}),
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO blobs (key, value) VALUES (?1, ?2)",
                    (key, value.to_string()),
                )
                .unwrap();
        }
        drop(connection);

        let adapter = CursorAdapter::new(temp.path().to_path_buf());
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, id);
        assert_eq!(sessions[0].title, "Cursor adapter work");
        assert_eq!(sessions[0].directory, "/work/cursor");
        assert_eq!(sessions[0].message_count, 1);
        assert!(sessions[0].content.contains("Added the adapter"));
        assert_eq!(
            adapter.resume_command(&sessions[0], true),
            vec!["agent", "--yolo", "--resume", id]
        );
        fs::write(db_path.with_file_name("store.db-wal"), b"wal").unwrap();
        fs::write(db_path.with_file_name("store.db-shm"), b"shm").unwrap();
        let stats = adapter.raw_stats();
        assert_eq!(stats.file_count, 3);
        assert_eq!(
            stats.total_bytes,
            db_path.metadata().unwrap().len() + b"wal".len() as u64 + b"shm".len() as u64
        );
    }

    #[test]
    fn incremental_scan_detects_wal_writes() {
        let temp = tempdir().unwrap();
        let id = "cursor-wal-session";
        let session_dir = temp.path().join("%2Fwork%2Fcursor").join(id);
        fs::create_dir_all(&session_dir).unwrap();
        let db_path = session_dir.join("store.db");
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0; \
                 CREATE TABLE meta (key TEXT, value BLOB); \
                 CREATE TABLE blobs (key TEXT, value BLOB);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO blobs (key, value) VALUES (?1, ?2)",
                (
                    "u1",
                    json!({"role":"user","content":"Initial prompt"}).to_string(),
                ),
            )
            .unwrap();

        let adapter = CursorAdapter::new(temp.path().to_path_buf());
        let initial = adapter.find_sessions().pop().unwrap();
        let mut known = KnownSessions::new();
        known.insert(("cursor".to_string(), id.to_string()), initial.mtime);

        thread::sleep(Duration::from_millis(10));
        connection
            .execute(
                "INSERT INTO blobs (key, value) VALUES (?1, ?2)",
                (
                    "a1",
                    json!({"role":"assistant","content":"Written to the WAL"}).to_string(),
                ),
            )
            .unwrap();

        let scan = adapter.find_sessions_incremental(&known);
        assert_eq!(scan.new_or_modified.len(), 1);
        assert!(
            scan.new_or_modified[0]
                .content
                .contains("Written to the WAL")
        );
        assert!(scan.new_or_modified[0].mtime > initial.mtime);
    }

    #[test]
    fn parses_plain_user_blob_format() {
        let id = "027f8b2f-d09c-4a69-98b0-b53f0118605d";
        let message = cursor_plain_user_message(format!("hello Cursor${id}").as_bytes());
        assert_eq!(message.as_deref(), Some("hello Cursor"));

        let message = cursor_plain_user_message(format!("price is $5${id}").as_bytes());
        assert_eq!(message.as_deref(), Some("price is $5"));
    }
}
