use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Local;
use rusqlite::{Connection, OptionalExtension, ToSql, params_from_iter};
use serde_json::Value;
use walkdir::WalkDir;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp};

use super::shared::{
    datetime_to_seconds, deleted_ids_for_agent, failed_incremental_scan, raw_stats_for_tree,
    session_needs_update, string_at, timestamp_from_ms, value_i64_at,
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
        match self.db_path.try_exists() {
            Ok(true) => load_opencode_db(self.name(), &self.db_path),
            Ok(false) => {
                let (scanned, _) = scan_opencode_legacy_sessions(&self.legacy_dir);
                let mut sessions = load_opencode_legacy(self.name(), &self.legacy_dir);
                for session in &mut sessions {
                    if let Some((_, mtime)) = scanned.get(&session.id) {
                        session.mtime = *mtime;
                    }
                }
                sessions
            }
            Err(_) => Vec::new(),
        }
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        match self.db_path.try_exists() {
            Ok(true) => load_opencode_db_incremental(self.name(), &self.db_path, known),
            Ok(false) => load_opencode_legacy_incremental(self.name(), &self.legacy_dir, known),
            Err(_) => failed_incremental_scan(self.name()),
        }
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

/// Table set that holds one OpenCode session and its conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DbSchema {
    /// OpenCode 1.x: `session`, `message`, and `part` tables.
    V1,
    /// OpenCode 2.x: `session_v2` plus one `session_message` row per turn.
    V2,
}

impl DbSchema {
    fn activity_tables(self) -> &'static [&'static str] {
        match self {
            Self::V1 => &["message", "part"],
            Self::V2 => &["session_message"],
        }
    }
}

/// OpenCode 2 tracks its 1.x import under this `kv` key.
const V1_MIGRATION_STATE_KEY: &str = "migration.v1-v2";

const SESSION_COLUMNS: &str = "id, title, directory, time_created, time_updated";

/// Where an OpenCode database currently keeps its sessions.
///
/// Fresh OpenCode 2 databases only have `session_v2`. When OpenCode 2 opens a
/// 1.x database, it adds `session_v2` and copies 1.x sessions in the
/// background, one transaction per session, newest id first. It saves its
/// progress in `kv['migration.v1-v2']` and leaves the 1.x tables behind.
#[derive(Debug, PartialEq, Eq)]
enum DbLayout {
    V1,
    V2,
    /// OpenCode 2 has not finished copying 1.x sessions. Sessions with an id
    /// at or above `cursor` are already in `session_v2`; older ones still
    /// live only in the 1.x tables. A missing cursor means nothing was copied.
    Migrating {
        cursor: Option<String>,
    },
}

impl DbLayout {
    /// Returns `None` when there is no session table or SQLite cannot tell.
    fn detect(conn: &Connection) -> Option<Self> {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name IN ('session', 'session_v2', 'kv')",
            )
            .ok()?;
        let tables = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .ok()?
            .collect::<rusqlite::Result<HashSet<_>>>()
            .ok()?;
        match (tables.contains("session_v2"), tables.contains("session")) {
            (false, false) => None,
            (false, true) => Some(Self::V1),
            (true, false) => Some(Self::V2),
            (true, true) if !tables.contains("kv") => Some(Self::Migrating { cursor: None }),
            (true, true) => Self::detect_migration(conn),
        }
    }

    /// Mirrors how OpenCode reads its migration state: a missing or unknown
    /// value means the copy has not started yet.
    fn detect_migration(conn: &Connection) -> Option<Self> {
        let state = conn
            .query_row(
                "SELECT value FROM kv WHERE key = ?1",
                [V1_MIGRATION_STATE_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()?
            .and_then(|value| serde_json::from_str::<Value>(&value).ok());
        let field = |key: &str| {
            state
                .as_ref()
                .and_then(|state| state.get(key))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        };
        Some(match field("phase").as_deref() {
            // After the copy, OpenCode deletes sessions only from
            // `session_v2`. Reading the 1.x leftovers would resurrect them.
            Some("completed") => Self::V2,
            Some("sessions") => Self::Migrating {
                cursor: field("cursor"),
            },
            _ => Self::Migrating { cursor: None },
        })
    }

    fn sources(&self) -> &'static [DbSchema] {
        match self {
            Self::V1 => &[DbSchema::V1],
            Self::V2 => &[DbSchema::V2],
            Self::Migrating { .. } => &[DbSchema::V2, DbSchema::V1],
        }
    }

    /// Reads the session rows of every source. A full rebuild skips
    /// unreadable rows; an incremental scan (`strict`) fails on them instead
    /// of reporting the missing sessions as deleted.
    fn session_rows(&self, conn: &Connection, strict: bool) -> Option<Vec<DbSessionRow>> {
        let mut sessions = Vec::new();
        for &source in self.sources() {
            let (query, params): (String, Vec<&dyn ToSql>) = match (self, source) {
                (_, DbSchema::V2) => (format!("SELECT {SESSION_COLUMNS} FROM session_v2"), vec![]),
                (Self::Migrating { cursor }, DbSchema::V1) => (
                    // Only the 1.x sessions OpenCode has not copied yet. The
                    // `NOT IN` guard covers ids that were already present in
                    // `session_v2`, which the copy skips with `INSERT OR IGNORE`.
                    format!(
                        "SELECT {SESSION_COLUMNS} FROM session
                         WHERE (?1 IS NULL OR id < ?1) AND id NOT IN (SELECT id FROM session_v2)"
                    ),
                    vec![cursor as &dyn ToSql],
                ),
                (_, DbSchema::V1) => (format!("SELECT {SESSION_COLUMNS} FROM session"), vec![]),
            };
            let mut stmt = conn.prepare(&query).ok()?;
            let rows = stmt
                .query_map(params.as_slice(), |row| DbSessionRow::from_row(row, source))
                .ok()?;
            for row in rows {
                match row {
                    Ok(row) => sessions.push(row),
                    Err(_) if strict => return None,
                    Err(_) => {}
                }
            }
        }
        Some(sessions)
    }
}

struct DbSessionRow {
    source: DbSchema,
    id: String,
    title: String,
    directory: String,
    time_created: i64,
    time_updated: i64,
}

impl DbSessionRow {
    fn from_row(row: &rusqlite::Row<'_>, source: DbSchema) -> rusqlite::Result<Self> {
        Ok(Self {
            source,
            id: row.get(0)?,
            title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            directory: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            time_created: row.get::<_, Option<i64>>(3)?.unwrap_or_default(),
            time_updated: row.get::<_, Option<i64>>(4)?.unwrap_or_default(),
        })
    }

    // Full and incremental scans must agree on this value, or the next launch
    // re-parses every session that a rebuild just indexed.
    fn mtime(&self, activity_mtimes: &ActivityMtimes, db_path: &Path) -> f64 {
        let activity_ms = activity_mtimes
            .get(&self.source)
            .and_then(|mtimes| mtimes.get(&self.id))
            .copied()
            .unwrap_or_default();
        let timestamp_ms = self.time_created.max(self.time_updated).max(activity_ms);
        timestamp_from_ms(Some(timestamp_ms))
            .map(datetime_to_seconds)
            .unwrap_or_else(|| file_mtime_seconds(db_path))
    }
}

/// Latest message activity per session, kept per source because a migrated
/// session also has stale 1.x rows under the same id.
type ActivityMtimes = HashMap<DbSchema, HashMap<String, i64>>;

fn activity_mtimes(conn: &Connection, layout: &DbLayout) -> ActivityMtimes {
    layout
        .sources()
        .iter()
        .map(|&source| (source, opencode_activity_mtimes_by_session(conn, source)))
        .collect()
}

/// Conversation rows grouped for rendering, independent of the schema.
#[derive(Default)]
struct DbContent {
    /// Ordered `(message id, role)` pairs per session.
    messages_by_session: HashMap<String, Vec<(String, String)>>,
    texts_by_message: HashMap<String, Vec<String>>,
}

impl DbContent {
    /// Loads conversation rows for every session in `source`, or only `ids`.
    /// Loading every row skips unreadable ones; loading `ids` fails on them.
    fn load(conn: &Connection, source: DbSchema, ids: Option<&[String]>) -> Option<Self> {
        match (source, ids) {
            (DbSchema::V1, None) => Some(load_opencode_v1_content(conn)),
            (DbSchema::V1, Some(ids)) => load_opencode_v1_content_for(conn, ids),
            (DbSchema::V2, ids) => load_opencode_v2_content(conn, ids),
        }
    }

    fn session(&mut self, agent: &'static str, row: DbSessionRow, mtime: f64) -> Session {
        let session_messages = self.messages_by_session.remove(&row.id).unwrap_or_default();
        let mut rendered = Vec::new();
        for (message_id, role) in &session_messages {
            let prefix = if role == "user" { "» " } else { "  " };
            for text in self.texts_by_message.get(message_id).into_iter().flatten() {
                rendered.push(format!("{prefix}{text}"));
            }
        }
        let timestamp = timestamp_from_ms(Some(row.time_created.max(row.time_updated)))
            .unwrap_or_else(Local::now);
        let mut session = Session::new(
            row.id,
            agent,
            if row.title.is_empty() {
                "Untitled session".to_string()
            } else {
                row.title
            },
            row.directory,
            timestamp,
            rendered.join("\n\n"),
            session_messages.len(),
        );
        session.mtime = mtime;
        session
    }
}

/// Renders rows with the content of their own source. Migrated OpenCode 2
/// messages keep their 1.x ids, so the sources cannot share one map.
fn render_sessions(
    agent: &'static str,
    rows: impl IntoIterator<Item = (DbSessionRow, f64)>,
    contents: &mut HashMap<DbSchema, DbContent>,
) -> Vec<Session> {
    rows.into_iter()
        .filter_map(|(row, mtime)| {
            let content = contents.get_mut(&row.source)?;
            Some(content.session(agent, row, mtime))
        })
        .collect()
}

fn load_opencode_db(agent: &'static str, db_path: &Path) -> Vec<Session> {
    let Ok(conn) = Connection::open(db_path) else {
        return Vec::new();
    };
    let Some(layout) = DbLayout::detect(&conn) else {
        return Vec::new();
    };
    let Some(mut rows) = layout.session_rows(&conn, false) else {
        return Vec::new();
    };
    rows.sort_by_key(|row| std::cmp::Reverse(row.time_updated));

    let mut contents = HashMap::new();
    for &source in layout.sources() {
        let Some(content) = DbContent::load(&conn, source, None) else {
            return Vec::new();
        };
        contents.insert(source, content);
    }
    let activity_mtimes = activity_mtimes(&conn, &layout);
    let rows = rows.into_iter().map(|row| {
        let mtime = row.mtime(&activity_mtimes, db_path);
        (row, mtime)
    });
    render_sessions(agent, rows, &mut contents)
}

fn load_opencode_db_incremental(
    agent: &'static str,
    db_path: &Path,
    known: &KnownSessions,
) -> IncrementalScan {
    let Ok(conn) = Connection::open(db_path) else {
        return failed_incremental_scan(agent);
    };
    let Some(layout) = DbLayout::detect(&conn) else {
        return failed_incremental_scan(agent);
    };
    let Some(rows) = layout.session_rows(&conn, true) else {
        return failed_incremental_scan(agent);
    };

    let mut current_ids = HashSet::new();
    let mut sessions_to_fetch = Vec::new();
    let activity_mtimes = activity_mtimes(&conn, &layout);
    for row in rows {
        current_ids.insert(row.id.clone());
        let mtime = row.mtime(&activity_mtimes, db_path);
        if session_needs_update(known, agent, &row.id, mtime) {
            sessions_to_fetch.push((row, mtime));
        }
    }

    let deleted_ids = deleted_ids_for_agent(known, agent, &current_ids);
    let mut contents = HashMap::new();
    for &source in layout.sources() {
        let fetch_ids: Vec<_> = sessions_to_fetch
            .iter()
            .filter(|(row, _)| row.source == source)
            .map(|(row, _)| row.id.clone())
            .collect();
        if fetch_ids.is_empty() {
            continue;
        }
        let Some(content) = DbContent::load(&conn, source, Some(&fetch_ids)) else {
            return failed_incremental_scan(agent);
        };
        contents.insert(source, content);
    }

    IncrementalScan {
        agent,
        new_or_modified: render_sessions(agent, sessions_to_fetch, &mut contents),
        deleted_ids,
    }
}

/// Loads every OpenCode 1.x message for a full rebuild. Unreadable rows are
/// skipped so one bad row does not hide the rest of the history.
fn load_opencode_v1_content(conn: &Connection) -> DbContent {
    let mut content = DbContent::default();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, session_id, COALESCE(json_extract(data, '$.role'), '') FROM message ORDER BY time_created ASC",
    )
        && let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }) {
            for (msg_id, session_id, role) in rows.filter_map(Result::ok) {
                content
                    .messages_by_session
                    .entry(session_id)
                    .or_default()
                    .push((msg_id, role));
            }
        }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT message_id, json_extract(data, '$.text') FROM part WHERE json_extract(data, '$.type') = 'text' ORDER BY time_created ASC",
    )
        && let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            ))
        }) {
            for (message_id, text) in rows.filter_map(Result::ok) {
                if !text.is_empty() {
                    content.texts_by_message.entry(message_id).or_default().push(text);
                }
            }
        }
    content
}

/// Loads OpenCode 1.x messages for the given sessions. Any error fails the
/// whole fetch so an incremental refresh never replaces good indexed data.
fn load_opencode_v1_content_for(conn: &Connection, ids: &[String]) -> Option<DbContent> {
    let mut content = DbContent::default();
    for chunk in ids.chunks(900) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let query = format!(
            "SELECT id, session_id, COALESCE(json_extract(data, '$.role'), '') FROM message WHERE session_id IN ({placeholders}) ORDER BY time_created ASC"
        );
        let mut stmt = conn.prepare(&query).ok()?;
        let rows = stmt
            .query_map(params_from_iter(chunk.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .ok()?;
        for row in rows {
            let (msg_id, session_id, role) = row.ok()?;
            content
                .messages_by_session
                .entry(session_id)
                .or_default()
                .push((msg_id, role));
        }
    }

    for chunk in ids.chunks(900) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let query = format!(
            "SELECT message_id, json_extract(data, '$.text') FROM part WHERE session_id IN ({placeholders}) AND json_extract(data, '$.type') = 'text' ORDER BY time_created ASC"
        );
        let mut stmt = conn.prepare(&query).ok()?;
        let rows = stmt
            .query_map(params_from_iter(chunk.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            })
            .ok()?;
        for row in rows {
            let (message_id, text) = row.ok()?;
            if !text.is_empty() {
                content
                    .texts_by_message
                    .entry(message_id)
                    .or_default()
                    .push(text);
            }
        }
    }
    Some(content)
}

/// Loads OpenCode 2.x conversation rows for every session, or only `ids`.
///
/// Only user prompts and assistant text are indexed; reasoning, tool calls,
/// synthetic and system context, shell output, and lifecycle rows are
/// skipped. Like the 1.x loaders, a full rebuild skips unreadable rows while
/// an incremental fetch fails on them, so it never replaces an indexed
/// session with partial content.
fn load_opencode_v2_content(conn: &Connection, ids: Option<&[String]>) -> Option<DbContent> {
    const QUERY: &str = "SELECT id, session_id, type, data FROM session_message WHERE type IN ('user', 'assistant')";
    let mut content = DbContent::default();
    match ids {
        None => collect_opencode_v2_rows(
            conn,
            &format!("{QUERY} ORDER BY session_id, seq"),
            [],
            false,
            &mut content,
        )?,
        Some(ids) => {
            for chunk in ids.chunks(900) {
                let placeholders = vec!["?"; chunk.len()].join(",");
                collect_opencode_v2_rows(
                    conn,
                    &format!("{QUERY} AND session_id IN ({placeholders}) ORDER BY session_id, seq"),
                    params_from_iter(chunk.iter()),
                    true,
                    &mut content,
                )?;
            }
        }
    }
    Some(content)
}

fn collect_opencode_v2_rows(
    conn: &Connection,
    query: &str,
    params: impl rusqlite::Params,
    strict: bool,
    content: &mut DbContent,
) -> Option<()> {
    let mut stmt = conn.prepare(query).ok()?;
    let rows = stmt
        .query_map(params, |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .ok()?;
    for row in rows {
        let parsed = row.ok().and_then(|(message_id, session_id, role, data)| {
            let data = serde_json::from_str::<Value>(&data).ok()?;
            Some((message_id, session_id, role, data))
        });
        let Some((message_id, session_id, role, data)) = parsed else {
            if strict {
                return None;
            }
            continue;
        };
        let texts = opencode_v2_message_texts(&role, &data);
        if !texts.is_empty() {
            content.texts_by_message.insert(message_id.clone(), texts);
        }
        content
            .messages_by_session
            .entry(session_id)
            .or_default()
            .push((message_id, role));
    }
    Some(())
}

/// User rows keep the prompt in `text`; assistant rows keep an ordered
/// `content` array whose `text` items are the visible reply.
fn opencode_v2_message_texts(role: &str, data: &Value) -> Vec<String> {
    let texts: Vec<&str> = match role {
        "user" => data
            .get("text")
            .and_then(Value::as_str)
            .into_iter()
            .collect(),
        "assistant" => data
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect(),
        _ => Vec::new(),
    };
    texts
        .into_iter()
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn opencode_activity_mtimes_by_session(
    conn: &Connection,
    schema: DbSchema,
) -> HashMap<String, i64> {
    let mut mtimes = HashMap::new();
    for table in schema.activity_tables() {
        collect_opencode_activity_mtimes(conn, table, &mut mtimes);
    }
    mtimes
}

fn collect_opencode_activity_mtimes(
    conn: &Connection,
    table: &str,
    mtimes: &mut HashMap<String, i64>,
) {
    let columns = table_columns(conn, table);
    if !columns.contains("session_id") {
        return;
    }
    let Some(time_expr) = row_time_expr(&columns) else {
        return;
    };

    let query = format!("SELECT session_id, MAX({time_expr}) FROM {table} GROUP BY session_id");
    let Ok(mut stmt) = conn.prepare(&query) else {
        return;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<i64>>(1)?.unwrap_or_default(),
        ))
    }) else {
        return;
    };

    for (session_id, mtime) in rows.filter_map(Result::ok) {
        mtimes
            .entry(session_id)
            .and_modify(|known| *known = (*known).max(mtime))
            .or_insert(mtime);
    }
}

fn table_columns(conn: &Connection, table: &str) -> HashSet<String> {
    let query = format!("PRAGMA table_info({table})");
    let Ok(mut stmt) = conn.prepare(&query) else {
        return HashSet::new();
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(1)) else {
        return HashSet::new();
    };
    rows.filter_map(Result::ok).collect()
}

fn row_time_expr(columns: &HashSet<String>) -> Option<String> {
    let mut parts = Vec::new();
    if columns.contains("time_created") {
        parts.push("COALESCE(time_created, 0)");
    }
    if columns.contains("time_updated") {
        parts.push("COALESCE(time_updated, 0)");
    }
    match parts.as_slice() {
        [] => None,
        [only] => Some((*only).to_string()),
        _ => Some(format!("MAX({})", parts.join(", "))),
    }
}

struct LegacyLoad {
    sessions: Vec<Session>,
    incomplete_session_ids: HashSet<String>,
    complete: bool,
}

impl Default for LegacyLoad {
    fn default() -> Self {
        Self {
            sessions: Vec::new(),
            incomplete_session_ids: HashSet::new(),
            complete: true,
        }
    }
}

fn load_opencode_legacy(agent: &'static str, legacy_dir: &Path) -> Vec<Session> {
    let mut load = load_opencode_legacy_with_health(agent, legacy_dir, None);
    if !load.complete {
        return Vec::new();
    }
    load.sessions
        .retain(|session| !load.incomplete_session_ids.contains(&session.id));
    load.sessions
}

/// `only` limits content parsing to the given session ids so an incremental
/// refresh does not re-read every message and part in the legacy store.
fn load_opencode_legacy_with_health(
    agent: &'static str,
    legacy_dir: &Path,
    only: Option<&HashSet<String>>,
) -> LegacyLoad {
    let session_dir = legacy_dir.join("session");
    let message_dir = legacy_dir.join("message");
    let part_dir = legacy_dir.join("part");
    match session_dir.try_exists() {
        Ok(false) => return LegacyLoad::default(),
        Err(_) => {
            return LegacyLoad {
                complete: false,
                ..LegacyLoad::default()
            };
        }
        Ok(true) => {}
    }
    let (activity_mtimes, _) = opencode_legacy_activity_mtimes(legacy_dir);

    let mut messages_by_session: HashMap<String, Vec<(PathBuf, String, String)>> = HashMap::new();
    let mut message_sessions = HashMap::new();
    let mut incomplete_session_ids = HashSet::new();
    let mut complete = true;
    let message_dir_exists = match message_dir.try_exists() {
        Ok(exists) => exists,
        Err(_) => {
            complete = false;
            false
        }
    };
    if message_dir_exists {
        for entry in WalkDir::new(&message_dir) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    if let Some(session_id) = error
                        .path()
                        .and_then(|path| legacy_child_id(&message_dir, path))
                    {
                        incomplete_session_ids.insert(session_id);
                    } else {
                        complete = false;
                    }
                    continue;
                }
            };
            let path = entry.path();
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("msg_") && name.ends_with(".json"))
            {
                continue;
            }
            let Some(session_id) = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(ToString::to_string)
            else {
                continue;
            };
            if only.is_some_and(|ids| !ids.contains(&session_id)) {
                continue;
            }
            let Ok(data_bytes) = fs::read(path) else {
                incomplete_session_ids.insert(session_id);
                continue;
            };
            let Ok(data) = serde_json::from_slice::<Value>(&data_bytes) else {
                incomplete_session_ids.insert(session_id);
                continue;
            };
            let msg_id = string_at(&data, &["id"]);
            if msg_id.is_empty() {
                incomplete_session_ids.insert(session_id);
                continue;
            }
            let role = string_at(&data, &["role"]);
            message_sessions.insert(msg_id.clone(), session_id.clone());
            messages_by_session.entry(session_id).or_default().push((
                path.to_path_buf(),
                msg_id,
                role,
            ));
        }
    }

    let mut parts_by_message: HashMap<String, Vec<String>> = HashMap::new();
    let part_dir_exists = match part_dir.try_exists() {
        Ok(exists) => exists,
        Err(_) => {
            complete = false;
            false
        }
    };
    if part_dir_exists {
        for entry in WalkDir::new(&part_dir) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    let Some(message_id) = error
                        .path()
                        .and_then(|path| legacy_child_id(&part_dir, path))
                    else {
                        complete = false;
                        continue;
                    };
                    if let Some(session_id) = message_sessions.get(&message_id) {
                        incomplete_session_ids.insert(session_id.clone());
                    }
                    continue;
                }
            };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(message_id) = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
            else {
                continue;
            };
            let Some(session_id) = message_sessions.get(message_id) else {
                continue;
            };
            let Ok(data_bytes) = fs::read(path) else {
                incomplete_session_ids.insert(session_id.clone());
                continue;
            };
            let Ok(data) = serde_json::from_slice::<Value>(&data_bytes) else {
                incomplete_session_ids.insert(session_id.clone());
                continue;
            };
            let Some(part_type) = data.get("type").and_then(Value::as_str) else {
                incomplete_session_ids.insert(session_id.clone());
                continue;
            };
            if part_type != "text" {
                continue;
            }
            let Some(text) = data.get("text").and_then(Value::as_str) else {
                incomplete_session_ids.insert(session_id.clone());
                continue;
            };
            if !text.is_empty() {
                parts_by_message
                    .entry(message_id.to_string())
                    .or_default()
                    .push(text.to_string());
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
        if only.is_some_and(|ids| !ids.contains(&id)) {
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
        session.mtime = opencode_legacy_mtime(&data, path)
            .max(activity_mtimes.get(&session.id).copied().unwrap_or(0.0));
        sessions.push(session);
    }
    LegacyLoad {
        sessions,
        incomplete_session_ids,
        complete,
    }
}

fn legacy_child_id(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()?
        .components()
        .next()?
        .as_os_str()
        .to_str()
        .filter(|id| !id.is_empty())
        .map(ToString::to_string)
}

fn load_opencode_legacy_incremental(
    agent: &'static str,
    legacy_dir: &Path,
    known: &KnownSessions,
) -> IncrementalScan {
    let (current_files, complete) = scan_opencode_legacy_sessions(legacy_dir);
    let current_ids: HashSet<_> = current_files.keys().cloned().collect();
    let deleted_ids = if complete {
        deleted_ids_for_agent(known, agent, &current_ids)
    } else {
        Vec::new()
    };
    let changed_ids: HashSet<_> = current_files
        .iter()
        .filter(|&(id, (_, mtime))| session_needs_update(known, agent, id, *mtime))
        .map(|(id, (_, _mtime))| id.clone())
        .collect();

    if changed_ids.is_empty() {
        return IncrementalScan {
            agent,
            new_or_modified: Vec::new(),
            deleted_ids,
        };
    }

    let LegacyLoad {
        sessions,
        incomplete_session_ids,
        complete: content_complete,
    } = load_opencode_legacy_with_health(agent, legacy_dir, Some(&changed_ids));
    if !content_complete {
        return IncrementalScan {
            agent,
            new_or_modified: Vec::new(),
            deleted_ids: Vec::new(),
        };
    }
    let mut new_or_modified = Vec::new();
    for mut session in sessions {
        if !changed_ids.contains(&session.id) {
            continue;
        }
        if incomplete_session_ids.contains(&session.id) {
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

fn scan_opencode_legacy_sessions(legacy_dir: &Path) -> (HashMap<String, (PathBuf, f64)>, bool) {
    let mut current_files = HashMap::new();
    let mut complete = true;
    let session_dir = legacy_dir.join("session");
    match session_dir.try_exists() {
        Ok(false) => return (current_files, complete),
        Err(_) => return (current_files, false),
        Ok(true) => {}
    }
    let (activity_mtimes, activity_complete) = opencode_legacy_activity_mtimes(legacy_dir);
    complete &= activity_complete;

    for entry in WalkDir::new(&session_dir) {
        let Ok(entry) = entry else {
            complete = false;
            continue;
        };
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("ses_") && name.ends_with(".json"))
        {
            continue;
        }
        let Ok(data_bytes) = fs::read(path) else {
            complete = false;
            continue;
        };
        let Ok(data) = serde_json::from_slice::<Value>(&data_bytes) else {
            complete = false;
            continue;
        };
        let id = string_at(&data, &["id"]);
        if id.is_empty() {
            complete = false;
            continue;
        }
        let mtime = opencode_legacy_mtime(&data, path)
            .max(activity_mtimes.get(&id).copied().unwrap_or(0.0));
        current_files.insert(id, (path.to_path_buf(), mtime));
    }

    (current_files, complete)
}

fn opencode_legacy_activity_mtimes(legacy_dir: &Path) -> (HashMap<String, f64>, bool) {
    let message_dir = legacy_dir.join("message");
    let part_dir = legacy_dir.join("part");
    let mut session_mtimes: HashMap<String, f64> = HashMap::new();
    let mut message_sessions: HashMap<String, String> = HashMap::new();
    let mut complete = true;

    let message_dir_exists = match message_dir.try_exists() {
        Ok(exists) => exists,
        Err(_) => {
            complete = false;
            false
        }
    };
    if message_dir_exists {
        for entry in WalkDir::new(&message_dir) {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let path = entry.path();
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("msg_") && name.ends_with(".json"))
            {
                continue;
            }
            let Some(session_id) = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .filter(|id| !id.is_empty())
                .map(ToString::to_string)
            else {
                continue;
            };
            let mtime = file_mtime_seconds(path);
            session_mtimes
                .entry(session_id.clone())
                .and_modify(|known| *known = known.max(mtime))
                .or_insert(mtime);
            let Ok(data_bytes) = fs::read(path) else {
                complete = false;
                continue;
            };
            let Ok(data) = serde_json::from_slice::<Value>(&data_bytes) else {
                complete = false;
                continue;
            };
            let msg_id = string_at(&data, &["id"]);
            if !msg_id.is_empty() {
                message_sessions.insert(msg_id, session_id);
            } else {
                complete = false;
            }
        }
    }

    let part_dir_exists = match part_dir.try_exists() {
        Ok(exists) => exists,
        Err(_) => {
            complete = false;
            false
        }
    };
    if part_dir_exists {
        for entry in WalkDir::new(&part_dir) {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(message_id) = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
            else {
                continue;
            };
            let Some(session_id) = message_sessions.get(message_id) else {
                continue;
            };
            let mtime = file_mtime_seconds(path);
            session_mtimes
                .entry(session_id.clone())
                .and_modify(|known| *known = known.max(mtime))
                .or_insert(mtime);
        }
    }

    (session_mtimes, complete)
}

fn opencode_legacy_mtime(data: &Value, path: &Path) -> f64 {
    let time_ms = value_i64_at(data, &["time", "updated"])
        .or_else(|| value_i64_at(data, &["time", "created"]));
    timestamp_from_ms(time_ms)
        .map(datetime_to_seconds)
        .unwrap_or(0.0)
        .max(file_mtime_seconds(path))
}

#[cfg(test)]
mod tests {
    use std::{fs, thread, time::Duration};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

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

    #[test]
    fn legacy_incremental_parses_only_changed_sessions_and_keeps_the_rest() {
        let temp = tempdir().unwrap();
        let legacy_dir = temp.path().join("legacy");
        for (session, msg, text) in [
            ("opencode-a", "msg-a", "Alpha content"),
            ("opencode-b", "msg-b", "Beta content"),
        ] {
            let session_dir = legacy_dir.join("session");
            let message_dir = legacy_dir.join("message").join(session);
            let part_dir = legacy_dir.join("part").join(msg);
            fs::create_dir_all(&session_dir).unwrap();
            fs::create_dir_all(&message_dir).unwrap();
            fs::create_dir_all(&part_dir).unwrap();
            fs::write(
                session_dir.join(format!("ses_{session}.json")),
                json!({
                    "id": session,
                    "title": "Thread",
                    "directory": "/work/opencode",
                    "time": {"updated": 1_720_000_000_000_i64}
                })
                .to_string(),
            )
            .unwrap();
            fs::write(
                message_dir.join("msg_1.json"),
                json!({"id": msg, "role": "user"}).to_string(),
            )
            .unwrap();
            fs::write(
                part_dir.join("part.json"),
                json!({"type": "text", "text": text}).to_string(),
            )
            .unwrap();
        }
        let adapter = OpenCodeAdapter {
            data_dir: temp.path().join("data"),
            db_path: temp.path().join("data/opencode.db"),
            legacy_dir: legacy_dir.clone(),
        };
        let known: KnownSessions = adapter
            .find_sessions()
            .into_iter()
            .map(|session| (("opencode".to_string(), session.id), session.mtime))
            .collect();
        assert_eq!(known.len(), 2);

        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(
            legacy_dir.join("part/msg-b/part.json"),
            json!({"type": "text", "text": "Beta content updated"}).to_string(),
        )
        .unwrap();

        let scan = adapter.find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].id, "opencode-b");
        assert!(
            scan.new_or_modified[0]
                .content
                .contains("Beta content updated")
        );
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn legacy_incremental_uses_file_mtime_when_json_time_is_unchanged() {
        let temp = tempdir().unwrap();
        let legacy_dir = temp.path().join("legacy");
        let session_dir = legacy_dir.join("session");
        fs::create_dir_all(&session_dir).unwrap();
        let session_file = session_dir.join("ses_opencode-1.json");
        fs::write(
            &session_file,
            json!({
                "id": "opencode-1",
                "title": "Original title",
                "directory": "/work/opencode",
                "time": {"updated": 1_720_000_000_000_i64}
            })
            .to_string(),
        )
        .unwrap();

        let adapter = OpenCodeAdapter {
            data_dir: temp.path().join("data"),
            db_path: temp.path().join("data/opencode.db"),
            legacy_dir,
        };
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        let mut known = KnownSessions::new();
        known.insert(
            ("opencode".to_string(), "opencode-1".to_string()),
            sessions[0].mtime,
        );

        thread::sleep(Duration::from_millis(20));
        fs::write(
            &session_file,
            json!({
                "id": "opencode-1",
                "title": "Updated title",
                "directory": "/work/opencode",
                "time": {"updated": 1_720_000_000_000_i64}
            })
            .to_string(),
        )
        .unwrap();

        let scan = adapter.find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].title, "Updated title");
        assert!(scan.new_or_modified[0].mtime > sessions[0].mtime);
    }

    #[test]
    fn legacy_incremental_uses_part_mtime() {
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
        let part_file = part_dir.join("part.json");
        fs::write(
            &part_file,
            json!({"type": "text", "text": "Original OpenCode text"}).to_string(),
        )
        .unwrap();

        let adapter = OpenCodeAdapter {
            data_dir: temp.path().join("data"),
            db_path: temp.path().join("data/opencode.db"),
            legacy_dir,
        };
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        let mut known = KnownSessions::new();
        known.insert(
            ("opencode".to_string(), "opencode-1".to_string()),
            sessions[0].mtime,
        );

        thread::sleep(Duration::from_millis(20));
        fs::write(
            &part_file,
            json!({"type": "text", "text": "Updated OpenCode text"}).to_string(),
        )
        .unwrap();

        let scan = adapter.find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert!(
            scan.new_or_modified[0]
                .content
                .contains("Updated OpenCode text")
        );
        assert!(scan.new_or_modified[0].mtime > sessions[0].mtime);
    }

    #[test]
    fn legacy_malformed_part_retains_known_session_and_recovers() {
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
        let part_file = part_dir.join("part.json");
        fs::write(
            &part_file,
            json!({"type": "text", "text": "Original OpenCode text"}).to_string(),
        )
        .unwrap();
        let adapter = OpenCodeAdapter {
            data_dir: temp.path().join("data"),
            db_path: temp.path().join("data/opencode.db"),
            legacy_dir,
        };
        let session = adapter.find_sessions().pop().unwrap();
        let mut known = KnownSessions::new();
        known.insert(
            ("opencode".to_string(), "opencode-1".to_string()),
            session.mtime,
        );

        thread::sleep(Duration::from_millis(20));
        fs::write(&part_file, "{").unwrap();
        let malformed_scan = adapter.find_sessions_incremental(&known);

        assert!(malformed_scan.new_or_modified.is_empty());
        assert!(malformed_scan.deleted_ids.is_empty());
        assert!(adapter.find_sessions().is_empty());

        thread::sleep(Duration::from_millis(20));
        fs::write(&part_file, "{}").unwrap();
        let structurally_invalid_scan = adapter.find_sessions_incremental(&known);

        assert!(structurally_invalid_scan.new_or_modified.is_empty());
        assert!(structurally_invalid_scan.deleted_ids.is_empty());
        assert!(adapter.find_sessions().is_empty());

        thread::sleep(Duration::from_millis(20));
        fs::write(
            &part_file,
            json!({"type": "text", "text": "Repaired OpenCode text"}).to_string(),
        )
        .unwrap();
        let repaired_scan = adapter.find_sessions_incremental(&known);

        assert_eq!(repaired_scan.new_or_modified.len(), 1);
        assert!(
            repaired_scan.new_or_modified[0]
                .content
                .contains("Repaired OpenCode text")
        );
        assert!(repaired_scan.deleted_ids.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn legacy_part_walk_error_retains_known_session_and_recovers() {
        let temp = tempdir().unwrap();
        let legacy_dir = temp.path().join("legacy");
        let session_dir = legacy_dir.join("session");
        let message_dir = legacy_dir.join("message/opencode-1");
        let part_dir = legacy_dir.join("part/msg-1");
        fs::create_dir_all(&session_dir).unwrap();
        fs::create_dir_all(&message_dir).unwrap();
        fs::create_dir_all(&part_dir).unwrap();
        let session_file = session_dir.join("ses_opencode-1.json");
        fs::write(
            &session_file,
            json!({
                "id": "opencode-1",
                "title": "Original title",
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
            json!({"type": "text", "text": "Original OpenCode text"}).to_string(),
        )
        .unwrap();
        let adapter = OpenCodeAdapter {
            data_dir: temp.path().join("data"),
            db_path: temp.path().join("data/opencode.db"),
            legacy_dir,
        };
        let session = adapter.find_sessions().pop().unwrap();
        let mut known = KnownSessions::new();
        known.insert(
            ("opencode".to_string(), "opencode-1".to_string()),
            session.mtime,
        );

        thread::sleep(Duration::from_millis(20));
        fs::write(
            &session_file,
            json!({
                "id": "opencode-1",
                "title": "Updated title",
                "directory": "/work/opencode",
                "time": {"updated": 1_720_000_000_000_i64}
            })
            .to_string(),
        )
        .unwrap();
        let mut permissions = fs::metadata(&part_dir).unwrap().permissions();
        permissions.set_mode(0o000);
        fs::set_permissions(&part_dir, permissions).unwrap();

        let inaccessible_scan = adapter.find_sessions_incremental(&known);

        let mut permissions = fs::metadata(&part_dir).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&part_dir, permissions).unwrap();
        assert!(inaccessible_scan.new_or_modified.is_empty());
        assert!(inaccessible_scan.deleted_ids.is_empty());

        let recovered_scan = adapter.find_sessions_incremental(&known);

        assert_eq!(recovered_scan.new_or_modified.len(), 1);
        assert_eq!(recovered_scan.new_or_modified[0].title, "Updated title");
        assert!(
            recovered_scan.new_or_modified[0]
                .content
                .contains("Original OpenCode text")
        );
        assert!(recovered_scan.deleted_ids.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn legacy_session_metadata_error_retains_known_sessions() {
        let temp = tempdir().unwrap();
        let legacy_dir = temp.path().join("legacy");
        fs::create_dir_all(legacy_dir.join("session")).unwrap();
        let adapter = OpenCodeAdapter {
            data_dir: temp.path().join("data"),
            db_path: temp.path().join("data/opencode.db"),
            legacy_dir: legacy_dir.clone(),
        };
        let mut known = KnownSessions::new();
        known.insert(("opencode".to_string(), "known".to_string()), 1.0);
        let mut permissions = fs::metadata(&legacy_dir).unwrap().permissions();
        permissions.set_mode(0o000);
        fs::set_permissions(&legacy_dir, permissions).unwrap();

        let scan = adapter.find_sessions_incremental(&known);

        let mut permissions = fs::metadata(&legacy_dir).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&legacy_dir, permissions).unwrap();
        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn sqlite_metadata_error_does_not_fall_back_and_delete_sessions() {
        let temp = tempdir().unwrap();
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let db_path = data_dir.join("opencode.db");
        fs::write(&db_path, "placeholder").unwrap();
        let adapter = OpenCodeAdapter {
            data_dir: data_dir.clone(),
            db_path,
            legacy_dir: temp.path().join("legacy"),
        };
        let mut known = KnownSessions::new();
        known.insert(("opencode".to_string(), "known".to_string()), 1.0);
        let mut permissions = fs::metadata(&data_dir).unwrap().permissions();
        permissions.set_mode(0o000);
        fs::set_permissions(&data_dir, permissions).unwrap();

        let scan = adapter.find_sessions_incremental(&known);

        let mut permissions = fs::metadata(&data_dir).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&data_dir, permissions).unwrap();
        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn legacy_scan_errors_retain_known_sessions_and_update_valid_ones() {
        let temp = tempdir().unwrap();
        let legacy_dir = temp.path().join("legacy");
        let session_dir = legacy_dir.join("session");
        let message_dir = legacy_dir.join("message/good");
        let part_dir = legacy_dir.join("part/msg-good");
        fs::create_dir_all(&session_dir).unwrap();
        fs::create_dir_all(&message_dir).unwrap();
        fs::create_dir_all(&part_dir).unwrap();
        fs::write(session_dir.join("ses_malformed.json"), "{").unwrap();
        fs::write(
            session_dir.join("ses_good.json"),
            json!({
                "id": "good",
                "title": "Good OpenCode session",
                "directory": "/work/good",
                "time": {"updated": 1_720_000_000_000_i64}
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            message_dir.join("msg_good.json"),
            json!({"id": "msg-good", "role": "user"}).to_string(),
        )
        .unwrap();
        fs::write(
            part_dir.join("part.json"),
            json!({"type": "text", "text": "Updated content"}).to_string(),
        )
        .unwrap();

        let adapter = OpenCodeAdapter {
            data_dir: temp.path().join("data"),
            db_path: temp.path().join("data/opencode.db"),
            legacy_dir,
        };
        let mut known = KnownSessions::new();
        known.insert(("opencode".to_string(), "malformed".to_string()), 0.0);
        known.insert(("opencode".to_string(), "good".to_string()), 0.0);

        let scan = adapter.find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].id, "good");
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn parses_sqlite_session_incrementally() {
        let temp = tempdir().unwrap();
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let db_path = data_dir.join("opencode.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT,
                directory TEXT,
                time_created INTEGER,
                time_updated INTEGER
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                time_created INTEGER,
                data TEXT
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT,
                session_id TEXT,
                time_created INTEGER,
                data TEXT
            );
            INSERT INTO session
                (id, title, directory, time_created, time_updated)
                VALUES ('opencode-1', 'OpenCode thread', '/work/opencode', 1720000000000, 1720000000000);
            INSERT INTO message
                (id, session_id, time_created, data)
                VALUES ('msg-1', 'opencode-1', 1720000000001, '{"role":"user"}');
            INSERT INTO part
                (id, message_id, session_id, time_created, data)
                VALUES ('part-1', 'msg-1', 'opencode-1', 1720000000002, '{"type":"text","text":"Hello OpenCode"}');
            "#,
        )
        .unwrap();

        let adapter = OpenCodeAdapter {
            data_dir,
            db_path,
            legacy_dir: temp.path().join("legacy"),
        };
        let scan = adapter.find_sessions_incremental(&KnownSessions::new());
        assert_eq!(scan.new_or_modified.len(), 1);
        let session = &scan.new_or_modified[0];
        assert_eq!(session.id, "opencode-1");
        assert!(session.content.contains("» Hello OpenCode"));

        let mut known = KnownSessions::new();
        known.insert(
            ("opencode".to_string(), "opencode-1".to_string()),
            session.mtime,
        );
        let scan = adapter.find_sessions_incremental(&known);
        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn full_scan_mtimes_match_the_incremental_scan() {
        let temp = tempdir().unwrap();
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let db_path = data_dir.join("opencode.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT,
                directory TEXT,
                time_created INTEGER,
                time_updated INTEGER
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                time_created INTEGER,
                time_updated INTEGER,
                data TEXT
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT,
                session_id TEXT,
                time_created INTEGER,
                time_updated INTEGER,
                data TEXT
            );
            INSERT INTO session
                (id, title, directory, time_created, time_updated)
                VALUES ('parity-1', 'Parity', '/work/opencode', 1720000000000, 1720000000000);
            INSERT INTO message
                (id, session_id, time_created, time_updated, data)
                VALUES ('msg-1', 'parity-1', 1720000000001, 1720000000500, '{"role":"user"}');
            INSERT INTO part
                (id, message_id, session_id, time_created, time_updated, data)
                VALUES ('part-1', 'msg-1', 'parity-1', 1720000000002, 1720000000600, '{"type":"text","text":"Content"}');
            "#,
        )
        .unwrap();
        let adapter = OpenCodeAdapter {
            data_dir,
            db_path,
            legacy_dir: temp.path().join("legacy"),
        };

        let full = adapter.find_sessions();
        assert_eq!(full.len(), 1);
        let known: KnownSessions = full
            .iter()
            .map(|session| (("opencode".to_string(), session.id.clone()), session.mtime))
            .collect();

        let scan = adapter.find_sessions_incremental(&known);

        assert!(
            scan.new_or_modified.is_empty(),
            "rebuild mtimes must satisfy the incremental scan"
        );
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn sqlite_incremental_uses_message_and_part_mtimes() {
        let temp = tempdir().unwrap();
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let db_path = data_dir.join("opencode.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT,
                directory TEXT,
                time_created INTEGER,
                time_updated INTEGER
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                time_created INTEGER,
                time_updated INTEGER,
                data TEXT
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT,
                session_id TEXT,
                time_created INTEGER,
                time_updated INTEGER,
                data TEXT
            );
            INSERT INTO session
                (id, title, directory, time_created, time_updated)
                VALUES ('opencode-1', 'OpenCode thread', '/work/opencode', 1720000000000, 1720000000000);
            INSERT INTO message
                (id, session_id, time_created, time_updated, data)
                VALUES ('msg-1', 'opencode-1', 1720000000001, 1720000000500, '{"role":"user"}');
            INSERT INTO part
                (id, message_id, session_id, time_created, time_updated, data)
                VALUES ('part-1', 'msg-1', 'opencode-1', 1720000000002, 1720000000600, '{"type":"text","text":"Updated OpenCode content"}');
            "#,
        )
        .unwrap();

        let adapter = OpenCodeAdapter {
            data_dir,
            db_path,
            legacy_dir: temp.path().join("legacy"),
        };
        let mut known = KnownSessions::new();
        let session_row_mtime = timestamp_from_ms(Some(1_720_000_000_000))
            .map(datetime_to_seconds)
            .unwrap();
        known.insert(
            ("opencode".to_string(), "opencode-1".to_string()),
            session_row_mtime,
        );

        let scan = adapter.find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert!(scan.new_or_modified[0].content.contains("Updated OpenCode"));
        assert!(scan.new_or_modified[0].mtime > session_row_mtime);

        let mut refreshed_known = KnownSessions::new();
        refreshed_known.insert(
            ("opencode".to_string(), "opencode-1".to_string()),
            scan.new_or_modified[0].mtime,
        );
        let unchanged = adapter.find_sessions_incremental(&refreshed_known);
        assert!(unchanged.new_or_modified.is_empty());
        assert!(unchanged.deleted_ids.is_empty());
    }

    #[test]
    fn sqlite_incremental_errors_do_not_delete_known_sessions() {
        let temp = tempdir().unwrap();
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let db_path = data_dir.join("opencode.db");
        fs::create_dir(&db_path).unwrap();

        let adapter = OpenCodeAdapter {
            data_dir: data_dir.clone(),
            db_path: db_path.clone(),
            legacy_dir: temp.path().join("legacy"),
        };
        let mut known = KnownSessions::new();
        known.insert(("opencode".to_string(), "opencode-1".to_string()), 1.0);

        let scan = adapter.find_sessions_incremental(&known);

        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());

        fs::remove_dir(&db_path).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE not_session (id TEXT PRIMARY KEY);")
            .unwrap();
        drop(conn);

        let scan = adapter.find_sessions_incremental(&known);

        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn sqlite_content_fetch_errors_do_not_replace_known_sessions() {
        let temp = tempdir().unwrap();
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let db_path = data_dir.join("opencode.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT,
                directory TEXT,
                time_created INTEGER,
                time_updated INTEGER
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                time_created INTEGER,
                time_updated INTEGER,
                data TEXT
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT,
                session_id TEXT,
                time_created INTEGER,
                time_updated INTEGER
            );
            INSERT INTO session
                (id, title, directory, time_created, time_updated)
                VALUES ('opencode-1', 'OpenCode thread', '/work/opencode', 1720000000000, 1720000000000);
            INSERT INTO message
                (id, session_id, time_created, time_updated, data)
                VALUES ('msg-1', 'opencode-1', 1720000000001, 1720000000500, '{"role":"user"}');
            INSERT INTO part
                (id, message_id, session_id, time_created, time_updated)
                VALUES ('part-1', 'msg-1', 'opencode-1', 1720000000002, 1720000000600);
            "#,
        )
        .unwrap();

        let adapter = OpenCodeAdapter {
            data_dir,
            db_path,
            legacy_dir: temp.path().join("legacy"),
        };
        let mut known = KnownSessions::new();
        known.insert(("opencode".to_string(), "opencode-1".to_string()), 1.0);

        let scan = adapter.find_sessions_incremental(&known);

        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }

    const V2_SCHEMA: &str = r#"
        CREATE TABLE kv (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL
        );
        CREATE TABLE session_v2 (
            id TEXT PRIMARY KEY,
            directory TEXT NOT NULL,
            title TEXT,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL
        );
        CREATE TABLE session_message (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            type TEXT NOT NULL,
            seq INTEGER NOT NULL,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            data TEXT NOT NULL
        );
    "#;

    fn v2_adapter(temp: &tempfile::TempDir) -> (OpenCodeAdapter, Connection) {
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let db_path = data_dir.join("opencode.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(V2_SCHEMA).unwrap();
        let adapter = OpenCodeAdapter {
            data_dir,
            db_path,
            legacy_dir: temp.path().join("legacy"),
        };
        (adapter, conn)
    }

    fn insert_v2_session(conn: &Connection, id: &str, title: &str, time_ms: i64) {
        conn.execute(
            "INSERT INTO session_v2 (id, directory, title, time_created, time_updated)
             VALUES (?1, '/work/opencode', ?2, ?3, ?3)",
            (id, title, time_ms),
        )
        .unwrap();
    }

    fn insert_v2_message(
        conn: &Connection,
        session_id: &str,
        seq: i64,
        kind: &str,
        time_ms: i64,
        data: &str,
    ) {
        conn.execute(
            "INSERT INTO session_message (id, session_id, type, seq, time_created, time_updated, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
            (
                format!("{session_id}-msg-{seq}"),
                session_id,
                kind,
                seq,
                time_ms,
                data,
            ),
        )
        .unwrap();
    }

    #[test]
    fn parses_v2_sqlite_conversation_text_only() {
        let temp = tempdir().unwrap();
        let (adapter, conn) = v2_adapter(&temp);
        insert_v2_session(&conn, "ses_v2", "OpenCode 2 thread", 1_720_000_000_000);
        let rows = [
            (
                "synthetic",
                json!({"text": "AGENTS.md instructions", "time": {"created": 1}}),
            ),
            (
                "user",
                json!({"text": "Find the flaky test", "files": [], "time": {"created": 2}}),
            ),
            (
                "assistant",
                json!({"content": [
                    {"type": "reasoning", "text": "private reasoning"},
                    {"type": "tool", "name": "bash", "state": {"output": "tool output"}},
                    {"type": "text", "text": "The flaky test is in cli.rs"},
                ]}),
            ),
            (
                "shell",
                json!({"command": "ls", "output": {"output": "shell output"}}),
            ),
            ("idle", json!({"outcome": "completed"})),
        ];
        for (seq, (kind, data)) in rows.iter().enumerate() {
            insert_v2_message(
                &conn,
                "ses_v2",
                seq as i64,
                kind,
                1_720_000_000_001 + seq as i64,
                &data.to_string(),
            );
        }

        let full = adapter.find_sessions();
        let scan = adapter.find_sessions_incremental(&KnownSessions::new());

        assert_eq!(full.len(), 1);
        assert_eq!(scan.new_or_modified.len(), 1);
        for session in [&full[0], &scan.new_or_modified[0]] {
            assert_eq!(session.id, "ses_v2");
            assert_eq!(session.title, "OpenCode 2 thread");
            assert_eq!(session.directory, "/work/opencode");
            assert_eq!(
                session.content,
                "» Find the flaky test\n\n  The flaky test is in cli.rs"
            );
            assert_eq!(session.message_count, 2);
        }
        assert_eq!(full[0].mtime, scan.new_or_modified[0].mtime);
        assert_eq!(
            adapter.resume_command(&full[0], false),
            vec!["opencode", "/work/opencode", "--session", "ses_v2"]
        );
    }

    #[test]
    fn v2_incremental_tracks_message_updates_and_deletions() {
        let temp = tempdir().unwrap();
        let (adapter, conn) = v2_adapter(&temp);
        for id in ["ses_a", "ses_b"] {
            insert_v2_session(&conn, id, "Thread", 1_720_000_000_000);
            insert_v2_message(
                &conn,
                id,
                0,
                "user",
                1_720_000_000_001,
                &json!({"text": format!("{id} prompt")}).to_string(),
            );
        }
        let known: KnownSessions = adapter
            .find_sessions()
            .into_iter()
            .map(|session| (("opencode".to_string(), session.id), session.mtime))
            .collect();
        assert_eq!(known.len(), 2);
        assert!(
            adapter
                .find_sessions_incremental(&known)
                .new_or_modified
                .is_empty()
        );

        insert_v2_message(
            &conn,
            "ses_b",
            1,
            "assistant",
            1_720_000_005_000,
            &json!({"content": [{"type": "text", "text": "New reply"}]}).to_string(),
        );
        conn.execute("DELETE FROM session_v2 WHERE id = 'ses_a'", [])
            .unwrap();

        let scan = adapter.find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].id, "ses_b");
        assert!(scan.new_or_modified[0].content.contains("  New reply"));
        assert_eq!(scan.deleted_ids, vec!["ses_a".to_string()]);
    }

    #[test]
    fn v2_malformed_message_keeps_indexed_session_until_repaired() {
        let temp = tempdir().unwrap();
        let (adapter, conn) = v2_adapter(&temp);
        insert_v2_session(&conn, "ses_v2", "Thread", 1_720_000_000_000);
        insert_v2_message(
            &conn,
            "ses_v2",
            0,
            "user",
            1_720_000_000_001,
            &json!({"text": "Original prompt"}).to_string(),
        );
        let mut known = KnownSessions::new();
        known.insert(
            ("opencode".to_string(), "ses_v2".to_string()),
            adapter.find_sessions()[0].mtime,
        );

        insert_v2_message(&conn, "ses_v2", 1, "assistant", 1_720_000_005_000, "{");
        let malformed = adapter.find_sessions_incremental(&known);

        assert!(malformed.new_or_modified.is_empty());
        assert!(malformed.deleted_ids.is_empty());
        let rebuilt = adapter.find_sessions();
        assert_eq!(rebuilt.len(), 1);
        assert_eq!(rebuilt[0].content, "» Original prompt");

        conn.execute(
            "UPDATE session_message SET data = ?1 WHERE seq = 1",
            [json!({"content": [{"type": "text", "text": "Repaired reply"}]}).to_string()],
        )
        .unwrap();
        let repaired = adapter.find_sessions_incremental(&known);

        assert_eq!(repaired.new_or_modified.len(), 1);
        assert!(
            repaired.new_or_modified[0]
                .content
                .contains("Repaired reply")
        );
        assert!(repaired.deleted_ids.is_empty());
    }

    /// Adds the 1.x tables that OpenCode 2 leaves behind after an upgrade.
    fn create_v1_tables(conn: &Connection) {
        conn.execute_batch(
            r#"
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT,
                directory TEXT,
                time_created INTEGER,
                time_updated INTEGER
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                time_created INTEGER,
                data TEXT
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT,
                session_id TEXT,
                time_created INTEGER,
                data TEXT
            );
            "#,
        )
        .unwrap();
    }

    fn insert_v1_session(conn: &Connection, id: &str, title: &str, prompt: &str) {
        conn.execute(
            "INSERT INTO session VALUES (?1, ?2, '/work/opencode', 1720000000000, 1720000000000)",
            (id, title),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message VALUES (?1 || '-msg', ?1, 1720000000001, '{\"role\":\"user\"}')",
            [id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES (?1 || '-part', ?1 || '-msg', ?1, 1720000000002, ?2)",
            (id, json!({"type": "text", "text": prompt}).to_string()),
        )
        .unwrap();
    }

    fn set_v1_migration_state(conn: &Connection, state: Value) {
        conn.execute(
            "INSERT INTO kv (key, value, time_created, time_updated) VALUES ('migration.v1-v2', ?1, 1, 1)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [state.to_string()],
        )
        .unwrap();
    }

    /// Mimics OpenCode 2 copying one 1.x session into `session_v2`.
    fn copy_v1_session_to_v2(conn: &Connection, id: &str, prompt: &str) {
        insert_v2_session(conn, id, &format!("{id} 2.x"), 1_720_000_000_000);
        insert_v2_message(
            conn,
            id,
            0,
            "user",
            1_720_000_000_001,
            &json!({"text": prompt}).to_string(),
        );
    }

    fn session_ids(sessions: &[Session]) -> Vec<&str> {
        let mut ids: Vec<_> = sessions.iter().map(|session| session.id.as_str()).collect();
        ids.sort_unstable();
        ids
    }

    fn known_from(sessions: &[Session]) -> KnownSessions {
        sessions
            .iter()
            .map(|session| (("opencode".to_string(), session.id.clone()), session.mtime))
            .collect()
    }

    #[test]
    fn v2_tables_take_precedence_over_leftover_v1_tables() {
        let temp = tempdir().unwrap();
        let (adapter, conn) = v2_adapter(&temp);
        // OpenCode 2 finished copying both sessions, then the user deleted
        // `ses_deleted` in 2.x. Both 1.x rows stay behind.
        create_v1_tables(&conn);
        insert_v1_session(&conn, "ses_copied", "Stale 1.x title", "Stale 1.x copy");
        insert_v1_session(&conn, "ses_deleted", "Deleted in 2.x", "Deleted prompt");
        copy_v1_session_to_v2(&conn, "ses_copied", "Copied prompt");
        set_v1_migration_state(&conn, json!({"phase": "completed"}));
        let mut known = KnownSessions::new();
        known.insert(("opencode".to_string(), "ses_deleted".to_string()), 1.0);

        let full = adapter.find_sessions();
        let scan = adapter.find_sessions_incremental(&known);

        for sessions in [&full, &scan.new_or_modified] {
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].id, "ses_copied");
            assert_eq!(sessions[0].title, "ses_copied 2.x");
            assert_eq!(sessions[0].content, "» Copied prompt");
        }
        assert_eq!(scan.deleted_ids, vec!["ses_deleted".to_string()]);
    }

    #[test]
    fn v2_migration_in_progress_keeps_uncopied_v1_sessions() {
        let temp = tempdir().unwrap();
        let (adapter, conn) = v2_adapter(&temp);
        create_v1_tables(&conn);
        for id in ["ses_a", "ses_b", "ses_c"] {
            insert_v1_session(&conn, id, id, &format!("{id} 1.x prompt"));
        }
        insert_v2_session(&conn, "ses_new", "New in 2.x", 1_720_000_000_000);

        // OpenCode 2 has created `session_v2` but not copied anything yet.
        let started = adapter.find_sessions();
        assert_eq!(
            session_ids(&started),
            ["ses_a", "ses_b", "ses_c", "ses_new"]
        );
        let known = known_from(&started);
        let unchanged = adapter.find_sessions_incremental(&known);
        assert!(unchanged.new_or_modified.is_empty());
        assert!(unchanged.deleted_ids.is_empty());

        // The copy runs newest id first and stops after `ses_b`; the user
        // then deletes the copied `ses_c` in 2.x.
        copy_v1_session_to_v2(&conn, "ses_c", "ses_c 2.x prompt");
        copy_v1_session_to_v2(&conn, "ses_b", "ses_b 2.x prompt");
        set_v1_migration_state(&conn, json!({"phase": "sessions", "cursor": "ses_b"}));
        conn.execute("DELETE FROM session_v2 WHERE id = 'ses_c'", [])
            .unwrap();

        let partial = adapter.find_sessions_incremental(&known);
        assert_eq!(partial.deleted_ids, vec!["ses_c".to_string()]);
        let rebuilt = adapter.find_sessions();
        assert_eq!(session_ids(&rebuilt), ["ses_a", "ses_b", "ses_new"]);
        let content = |id: &str| {
            rebuilt
                .iter()
                .find(|session| session.id == id)
                .map(|session| session.content.as_str())
        };
        assert_eq!(content("ses_a"), Some("» ses_a 1.x prompt"));
        assert_eq!(content("ses_b"), Some("» ses_b 2.x prompt"));

        // Once the copy completes, every session comes from `session_v2`.
        copy_v1_session_to_v2(&conn, "ses_a", "ses_a 2.x prompt");
        set_v1_migration_state(&conn, json!({"phase": "completed"}));
        let completed = adapter.find_sessions_incremental(&known_from(&rebuilt));
        assert!(completed.deleted_ids.is_empty());
        assert_eq!(
            session_ids(&adapter.find_sessions()),
            ["ses_a", "ses_b", "ses_new"]
        );
    }
}
