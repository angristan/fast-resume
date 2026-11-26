//! Cursor CLI stores each chat in a small SQLite key-value database. Values
//! vary across releases, so parsing accepts structured JSON, hex-encoded
//! metadata, and the legacy plain-user-message encoding while leaving unknown
//! binary records untouched.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use walkdir::WalkDir;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    IncrementalParse, build_resume_command, incremental_scan, percent_decode, sqlite_file_stats,
    sqlite_mtime,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

type SessionFiles = HashMap<String, (PathBuf, f64)>;
/// Workspace hash -> (workspace path, newest mtime of a worker.log naming it).
type WorkspaceIndex = HashMap<String, (String, f64)>;

#[derive(Debug, Clone)]
pub struct CursorAdapter {
    chats_dir: PathBuf,
    projects_dir: PathBuf,
}

impl Default for CursorAdapter {
    fn default() -> Self {
        Self {
            chats_dir: config::cursor_chats_dir(),
            projects_dir: config::cursor_projects_dir(),
        }
    }
}

impl CursorAdapter {
    fn incremental(
        &self,
        known: &KnownSessions,
        on_session: Option<&mut SessionCallback<'_>>,
    ) -> IncrementalScan {
        let workspace_by_hash = self.workspace_paths_by_hash();
        incremental_scan(
            self.name(),
            known,
            self.scan_session_files(&workspace_by_hash),
            |path| self.parse_incremental(path, &workspace_by_hash),
            on_session,
        )
    }

    #[allow(dead_code)]
    pub fn new(chats_dir: PathBuf) -> Self {
        let projects_dir = chats_dir
            .parent()
            .map(|parent| parent.join("projects"))
            .unwrap_or_else(config::cursor_projects_dir);
        Self {
            chats_dir,
            projects_dir,
        }
    }

    #[allow(dead_code)]
    pub fn new_with_projects_dir(chats_dir: PathBuf, projects_dir: PathBuf) -> Self {
        Self {
            chats_dir,
            projects_dir,
        }
    }

    fn scan_session_files(
        &self,
        workspace_by_hash: &WorkspaceIndex,
    ) -> Option<(SessionFiles, bool)> {
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
            files.insert(
                id.to_string(),
                (
                    path.to_path_buf(),
                    cursor_session_mtime(path, workspace_by_hash),
                ),
            );
        }
        Some((files, complete))
    }

    fn workspace_paths_by_hash(&self) -> WorkspaceIndex {
        let mut workspaces: WorkspaceIndex = HashMap::new();
        let Ok(projects) = fs::read_dir(&self.projects_dir) else {
            return workspaces;
        };
        for project in projects.filter_map(Result::ok) {
            let worker_log = project.path().join("worker.log");
            let Ok(contents) = fs::read_to_string(&worker_log) else {
                continue;
            };
            let log_mtime = file_mtime_seconds(&worker_log);
            for line in contents.lines() {
                let Some(workspace) = cursor_workspace_path_from_log_line(line) else {
                    continue;
                };
                let entry = workspaces
                    .entry(cursor_workspace_hash(&workspace))
                    .or_insert_with(|| (workspace, log_mtime));
                entry.1 = entry.1.max(log_mtime);
            }
        }
        workspaces
    }

    fn parse_session(
        &self,
        path: &Path,
        workspace_by_hash: &WorkspaceIndex,
    ) -> rusqlite::Result<Option<Session>> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let mut title = String::new();
        let mut directory = String::new();
        let mut created_at = None;
        let mut yolo = false;
        let mut messages = Vec::new();

        // Newer Cursor CLI releases write a plain meta.json sidecar with the
        // generated title and the workspace cwd; prefer it when present.
        if let Some(value) = path
            .parent()
            .and_then(|dir| fs::read(dir.join("meta.json")).ok())
            .and_then(|bytes| parse_json_bytes(&bytes))
        {
            apply_cursor_metadata(
                &value,
                &mut title,
                &mut directory,
                &mut created_at,
                &mut yolo,
            );
        }

        for (_, bytes) in read_key_value_table(&connection, "meta")? {
            let Some(value) = decode_json(&bytes) else {
                continue;
            };
            apply_cursor_metadata(
                &value,
                &mut title,
                &mut directory,
                &mut created_at,
                &mut yolo,
            );
        }

        for (_, bytes) in read_key_value_table(&connection, "blobs")? {
            if let Some(value) = decode_json(&bytes) {
                collect_cursor_records(
                    &value,
                    &mut messages,
                    &mut title,
                    &mut created_at,
                    &mut yolo,
                );
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
            directory = cursor_directory_from_path(path, workspace_by_hash);
        }
        if directory.trim().is_empty() {
            directory = cursor_workspace_from_user_info(path, &first_user).unwrap_or_default();
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
        session.mtime = cursor_session_mtime(path, workspace_by_hash);
        session.yolo = yolo;
        Ok(Some(session))
    }

    fn parse_incremental(
        &self,
        path: &Path,
        workspace_by_hash: &WorkspaceIndex,
    ) -> IncrementalParse {
        match self.parse_session(path, workspace_by_hash) {
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
        let workspace_by_hash = self.workspace_paths_by_hash();
        self.scan_session_files(&workspace_by_hash)
            .map(|(files, _)| {
                files
                    .into_values()
                    .filter_map(|(path, _)| {
                        self.parse_session(&path, &workspace_by_hash).ok().flatten()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        self.incremental(known, None)
    }

    fn find_sessions_incremental_streaming(
        &self,
        known: &KnownSessions,
        on_session: &mut SessionCallback<'_>,
    ) -> IncrementalScan {
        self.incremental(known, Some(on_session))
    }

    fn resume_command(&self, session: &Session, yolo: bool) -> Vec<String> {
        build_resume_command("agent", &["--yolo"], yolo, &["--resume"], &session.id)
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
    parse_json_bytes(bytes)
        .or_else(|| parse_json_fragment(bytes))
        .or_else(|| {
            let decoded = decode_hex_bytes(bytes)?;
            parse_json_bytes(&decoded).or_else(|| parse_json_fragment(&decoded))
        })
}

fn parse_json_bytes(bytes: &[u8]) -> Option<Value> {
    serde_json::from_slice(bytes).ok()
}

fn parse_json_fragment(bytes: &[u8]) -> Option<Value> {
    let start = bytes.iter().position(|byte| *byte == b'{')?;
    let end = bytes.iter().rposition(|byte| *byte == b'}')?;
    (start < end)
        .then(|| serde_json::from_slice(&bytes[start..=end]).ok())
        .flatten()
}

fn decode_hex_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text.len() < 2 || text.len() % 2 != 0 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut decoded = Vec::with_capacity(text.len() / 2);
    for index in (0..text.len()).step_by(2) {
        decoded.push(u8::from_str_radix(&text[index..index + 2], 16).ok()?);
    }
    Some(decoded)
}

fn apply_cursor_metadata(
    value: &Value,
    title: &mut String,
    directory: &mut String,
    created_at: &mut Option<DateTime<Local>>,
    yolo: &mut bool,
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
    *yolo |= cursor_metadata_uses_yolo(value);
}

fn cursor_metadata_uses_yolo(value: &Value) -> bool {
    value
        .get("isRunEverything")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value
            .get("approvalMode")
            .and_then(Value::as_str)
            .is_some_and(|mode| matches!(mode, "unrestricted" | "runEverything" | "yolo"))
}

fn collect_cursor_records(
    value: &Value,
    messages: &mut Vec<CursorMessage>,
    title: &mut String,
    created_at: &mut Option<DateTime<Local>>,
    yolo: &mut bool,
) {
    apply_cursor_metadata(value, title, &mut String::new(), created_at, yolo);
    if let Some(items) = value.get("messages").and_then(Value::as_array) {
        for item in items {
            collect_cursor_records(item, messages, title, created_at, yolo);
        }
    }
    if let Some(items) = value.get("conversation").and_then(Value::as_array) {
        for item in items {
            collect_cursor_records(item, messages, title, created_at, yolo);
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

fn cursor_directory_from_path(path: &Path, workspace_by_hash: &WorkspaceIndex) -> String {
    let encoded = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if let Some((workspace, _)) = workspace_by_hash.get(encoded) {
        return workspace.clone();
    }
    let decoded = percent_decode(encoded);
    if Path::new(&decoded).is_absolute() {
        decoded
    } else {
        Default::default()
    }
}

fn cursor_workspace_path_from_log_line(line: &str) -> Option<String> {
    let (_, rest) = line.split_once("workspacePath=")?;
    let workspace = rest.trim();
    (!workspace.is_empty()).then(|| workspace.to_string())
}

/// Cursor injects a `<user_info>` block with `Workspace Path: ...` into the
/// first user message. Accept it only when it hashes to the chat's parent
/// directory name, so stray text in a prompt cannot set the directory.
fn cursor_workspace_from_user_info(path: &Path, first_user: &str) -> Option<String> {
    let encoded = path.parent()?.parent()?.file_name()?.to_str()?;
    first_user
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Workspace Path:"))
        .map(str::trim)
        .find(|workspace| !workspace.is_empty() && cursor_workspace_hash(workspace) == encoded)
        .map(str::to_string)
}

/// Update marker for incremental indexing. Title and directory can come from
/// the `meta.json` sidecar and, for older chats without a `cwd` there, from a
/// project `worker.log`, so changes to either must trigger a reparse even when
/// the database itself is untouched. Chats with a recorded `cwd` ignore the
/// logs, which Cursor appends to constantly while a workspace is open.
fn cursor_session_mtime(path: &Path, workspace_by_hash: &WorkspaceIndex) -> f64 {
    let mut mtime = sqlite_mtime(path);
    let Some(session_dir) = path.parent() else {
        return mtime;
    };
    let meta_path = session_dir.join("meta.json");
    mtime = mtime.max(file_mtime_seconds(&meta_path));
    let has_cwd = fs::read(&meta_path)
        .ok()
        .and_then(|bytes| parse_json_bytes(&bytes))
        .and_then(|value| {
            value
                .get("cwd")
                .and_then(Value::as_str)
                .map(|cwd| !cwd.is_empty())
        })
        .unwrap_or(false);
    if !has_cwd
        && let Some((_, log_mtime)) = session_dir
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .and_then(|hash| workspace_by_hash.get(hash))
    {
        mtime = mtime.max(*log_mtime);
    }
    mtime
}

fn cursor_workspace_hash(workspace: &str) -> String {
    format!("{:x}", md5::compute(workspace.as_bytes()))
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
    fn parses_hex_metadata_and_hashed_workspace_dirs() {
        let temp = tempdir().unwrap();
        let chats_dir = temp.path().join("chats");
        let projects_dir = temp.path().join("projects");
        let workspace = "/work/cursor-cli";
        let workspace_hash = cursor_workspace_hash(workspace);
        let id = "cursor-hashed-session";
        let session_dir = chats_dir.join(&workspace_hash).join(id);
        fs::create_dir_all(&session_dir).unwrap();
        fs::create_dir_all(projects_dir.join("work-cursor-cli")).unwrap();
        fs::write(
            projects_dir.join("work-cursor-cli").join("worker.log"),
            format!("[info] Getting tree structure for workspacePath={workspace}\n"),
        )
        .unwrap();

        let db_path = session_dir.join("store.db");
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT); \
                 CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB);",
            )
            .unwrap();
        let metadata = json!({
            "agentId": id,
            "createdAt": 1784282400000_i64,
            "isRunEverything": true,
            "name": "Hex Cursor metadata"
        })
        .to_string();
        let encoded_metadata = metadata
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        connection
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)",
                ("0", encoded_metadata),
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO blobs (id, data) VALUES (?1, ?2)",
                (
                    "u1",
                    json!({"role":"user","content":"Parse hashed Cursor workspace"}).to_string(),
                ),
            )
            .unwrap();
        drop(connection);

        let adapter = CursorAdapter::new_with_projects_dir(chats_dir, projects_dir);
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, id);
        assert_eq!(sessions[0].title, "Hex Cursor metadata");
        assert_eq!(sessions[0].directory, workspace);
        assert!(sessions[0].yolo);
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

    fn write_minimal_store(session_dir: &Path, user_text: &str) {
        fs::create_dir_all(session_dir).unwrap();
        let connection = Connection::open(session_dir.join("store.db")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE meta (key TEXT, value BLOB); CREATE TABLE blobs (key TEXT, value BLOB);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO blobs (key, value) VALUES (?1, ?2)",
                ("u1", json!({"role":"user","content":user_text}).to_string()),
            )
            .unwrap();
    }

    #[test]
    fn prefers_meta_json_sidecar_title_and_cwd() {
        let temp = tempdir().unwrap();
        let workspace = "/work/sidecar";
        let session_dir = temp
            .path()
            .join("chats")
            .join(cursor_workspace_hash(workspace))
            .join("sidecar-session");
        write_minimal_store(
            &session_dir,
            "<user_info>\nOS Version: darwin\n</user_info>\nreal prompt",
        );
        fs::write(
            session_dir.join("meta.json"),
            json!({"schemaVersion":1,"title":"Sidecar title","cwd":workspace}).to_string(),
        )
        .unwrap();

        let adapter = CursorAdapter::new_with_projects_dir(
            temp.path().join("chats"),
            temp.path().join("projects"),
        );
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Sidecar title");
        assert_eq!(sessions[0].directory, workspace);
    }

    #[test]
    fn falls_back_to_user_info_workspace_path_matching_hash() {
        let temp = tempdir().unwrap();
        let workspace = "/work/user-info";
        let chats = temp.path().join("chats");
        let user_info = |path: &str| {
            format!("<user_info>\nShell: zsh\n\nWorkspace Path: {path}\n</user_info>\nhello")
        };
        write_minimal_store(
            &chats
                .join(cursor_workspace_hash(workspace))
                .join("matching"),
            &user_info(workspace),
        );
        // A path that does not hash to the parent directory must be ignored.
        write_minimal_store(
            &chats
                .join(cursor_workspace_hash("/elsewhere"))
                .join("mismatched"),
            &user_info(workspace),
        );

        let adapter = CursorAdapter::new_with_projects_dir(chats, temp.path().join("projects"));
        let sessions: HashMap<_, _> = adapter
            .find_sessions()
            .into_iter()
            .map(|session| (session.id.clone(), session.directory))
            .collect();
        assert_eq!(sessions["matching"], workspace);
        assert_eq!(sessions["mismatched"], "");
    }

    #[test]
    fn incremental_scan_reparses_when_only_worker_log_changes() {
        let temp = tempdir().unwrap();
        let chats = temp.path().join("chats");
        let projects = temp.path().join("projects");
        let workspace = "/work/late-log";
        let id = "late-log-session";
        write_minimal_store(
            &chats.join(cursor_workspace_hash(workspace)).join(id),
            "prompt",
        );

        let adapter = CursorAdapter::new_with_projects_dir(chats, projects.clone());
        let initial = adapter.find_sessions().pop().unwrap();
        assert_eq!(initial.directory, "");
        let mut known = KnownSessions::new();
        known.insert(("cursor".to_string(), id.to_string()), initial.mtime);

        thread::sleep(Duration::from_millis(10));
        fs::create_dir_all(projects.join("work-late-log")).unwrap();
        fs::write(
            projects.join("work-late-log").join("worker.log"),
            format!("[info] workspacePath={workspace}\n"),
        )
        .unwrap();

        let scan = adapter.find_sessions_incremental(&known);
        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].directory, workspace);
        assert!(scan.new_or_modified[0].mtime > initial.mtime);
    }

    #[test]
    fn incremental_scan_reparses_when_only_meta_json_changes() {
        let temp = tempdir().unwrap();
        let chats = temp.path().join("chats");
        let session_dir = chats.join("somehash").join("meta-session");
        write_minimal_store(&session_dir, "prompt");

        let adapter = CursorAdapter::new_with_projects_dir(chats, temp.path().join("projects"));
        let initial = adapter.find_sessions().pop().unwrap();
        let mut known = KnownSessions::new();
        known.insert(("cursor".to_string(), initial.id.clone()), initial.mtime);

        thread::sleep(Duration::from_millis(10));
        fs::write(
            session_dir.join("meta.json"),
            json!({"title":"Generated later"}).to_string(),
        )
        .unwrap();

        let scan = adapter.find_sessions_incremental(&known);
        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].title, "Generated later");
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
