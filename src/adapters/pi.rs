use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone};
use rayon::prelude::*;
use serde::de::{self, Deserialize, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use walkdir::WalkDir;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    IncrementalParse, JsonlHealth, JsonlHealthTracker, incremental_parse_with_health,
    incremental_scan, is_uuid_like, parse_datetime, raw_stats_for_tree,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

type SessionFiles = HashMap<String, (PathBuf, f64)>;

/// Read buffer for transcripts. Pi rows can be megabytes long (tool output,
/// compaction summaries), so a larger buffer means fewer `read` syscalls.
const READ_BUFFER_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct PiAdapter {
    sessions_dir: PathBuf,
}

impl Default for PiAdapter {
    fn default() -> Self {
        Self {
            sessions_dir: config::pi_sessions_dir(),
        }
    }
}

impl PiAdapter {
    fn incremental(
        &self,
        known: &KnownSessions,
        on_session: Option<&mut SessionCallback<'_>>,
    ) -> IncrementalScan {
        incremental_scan(
            self.name(),
            known,
            self.scan_session_files(),
            |path| self.parse_session_incremental(path),
            on_session,
        )
    }

    #[allow(dead_code)]
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir }
    }

    fn scan_session_files(&self) -> Option<(SessionFiles, bool)> {
        let mut current_files = HashMap::new();
        let mut complete = true;
        if !self.sessions_dir.exists() {
            return Some((current_files, complete));
        }
        if !self.sessions_dir.is_dir() {
            return None;
        }

        for entry in WalkDir::new(&self.sessions_dir) {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let session_id = self.session_id_from_file(path);
            current_files.insert(session_id, (path.to_path_buf(), file_mtime_seconds(path)));
        }

        Some((current_files, complete))
    }

    /// Resolve the session id used as the incremental-scan key.
    ///
    /// Pi names transcripts `<timestamp>_<sessionId>.jsonl`, so a UUID suffix
    /// is trusted without opening the file. That keeps unchanged sessions at
    /// one `stat` each per refresh instead of an open, a read, and a JSON
    /// parse. Other names fall back to the `session` header row.
    fn session_id_from_file(&self, path: &Path) -> String {
        let from_name = pi_session_id_from_path(path);
        if is_uuid_like(&from_name) {
            return from_name;
        }
        if let Ok(file) = fs::File::open(path) {
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(row) = serde_json::from_str::<PiRow>(&line) else {
                    continue;
                };
                if row.kind.as_deref() == Some("session")
                    && let Some(id) = row.id.filter(|id| !id.is_empty())
                {
                    return id;
                }
                break;
            }
        }
        from_name
    }

    fn parse_session(&self, path: &Path) -> Option<Session> {
        self.parse_file(path).0
    }

    /// Parse a transcript and classify its JSONL health in a single pass.
    ///
    /// Each row is decoded into [`PiRow`], which keeps only what the index
    /// needs. Tool results, tool-call arguments, thinking blocks, and
    /// extension state make up most of a Pi transcript's bytes; they are
    /// syntax-checked but never materialized.
    fn parse_file(&self, path: &Path) -> (Option<Session>, JsonlHealth) {
        let Ok(file) = fs::File::open(path) else {
            return (None, JsonlHealth::Invalid);
        };
        let mut reader = BufReader::with_capacity(READ_BUFFER_BYTES, file);
        let mut line = String::new();
        let mut health = JsonlHealthTracker::default();
        let mut transcript = PiTranscript::default();
        let mut read_failed = false;

        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                // Unreadable bytes or invalid UTF-8: keep what was parsed for
                // full scans, but report the file as invalid so incremental
                // refresh retains the indexed copy.
                Err(_) => {
                    read_failed = true;
                    break;
                }
            }
            if line.trim().is_empty() {
                continue;
            }
            // `PiRow` accepts any JSON shape, so an error here means the row
            // is not valid JSON.
            match serde_json::from_str::<PiRow>(&line) {
                Ok(row) => {
                    health.valid_row();
                    transcript.add_row(row);
                }
                Err(_) => health.malformed_row(),
            }
        }

        let health = if read_failed {
            JsonlHealth::Invalid
        } else {
            health.finish()
        };
        (transcript.into_session(self.name(), path), health)
    }

    fn parse_session_incremental(&self, path: &Path) -> IncrementalParse {
        let (session, health) = self.parse_file(path);
        incremental_parse_with_health(health, session, |_| true)
    }
}

impl Adapter for PiAdapter {
    fn name(&self) -> &'static str {
        "pi"
    }

    fn find_sessions(&self) -> Vec<Session> {
        if !self.sessions_dir.exists() {
            return Vec::new();
        }
        let paths: Vec<PathBuf> = WalkDir::new(&self.sessions_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
            .map(walkdir::DirEntry::into_path)
            .collect();
        paths
            .par_iter()
            .filter_map(|path| self.parse_session(path))
            .collect()
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

    fn resume_command(&self, session: &Session, _yolo: bool) -> Vec<String> {
        // Pi's `--session <path|id>` accepts the session ID directly (even a
        // partial UUID), and `Session.id` always holds the full UUID from the
        // `session` row. `fr` only indexes sessions under the global
        // `pi_sessions_dir()` — the same store `pi --session <id>` resolves
        // against — so no path resolution is needed here.
        vec![
            "pi".to_string(),
            "--session".to_string(),
            session.id.clone(),
        ]
    }

    fn raw_stats(&self) -> RawAdapterStats {
        raw_stats_for_tree(self.name(), &self.sessions_dir, "jsonl")
    }
}

/// Session state accumulated row by row while reading a transcript.
#[derive(Default)]
struct PiTranscript {
    session_id: String,
    directory: String,
    session_name: Option<String>,
    first_user_message: String,
    messages: Vec<String>,
    message_count: usize,
    header_timestamp: Option<DateTime<Local>>,
    last_activity: Option<DateTime<Local>>,
}

impl PiTranscript {
    fn add_row(&mut self, row: PiRow) {
        match row.kind.as_deref().unwrap_or_default() {
            "session" => {
                if self.session_id.is_empty() {
                    self.session_id = row.id.unwrap_or_default();
                }
                if self.directory.is_empty() {
                    self.directory = row.cwd.unwrap_or_default();
                }
                if self.header_timestamp.is_none() {
                    self.header_timestamp = parse_datetime(row.timestamp.as_deref().unwrap_or(""));
                }
            }
            "session_info" => {
                let name = row.name.unwrap_or_default();
                self.session_name = (!name.trim().is_empty()).then(|| name.trim().to_string());
            }
            "message" => {
                let Some(message) = row.message else {
                    return;
                };
                let is_user = message.role.as_deref() == Some("user");
                let is_assistant = message.role.as_deref() == Some("assistant");
                let is_visible_custom = message.role.as_deref().is_some_and(is_custom_role)
                    && message.display == Some(true);
                if !is_user && !is_assistant && !is_visible_custom {
                    return;
                }
                if is_user {
                    self.message_count += 1;
                }
                if is_user || is_assistant {
                    let timestamp = message
                        .timestamp_ms
                        .and_then(|ms| Local.timestamp_millis_opt(ms).single())
                        .or_else(|| parse_datetime(row.timestamp.as_deref().unwrap_or("")));
                    if let Some(timestamp) = timestamp
                        && self.last_activity.is_none_or(|current| timestamp > current)
                    {
                        self.last_activity = Some(timestamp);
                    }
                }

                let role_prefix = if is_user { "» " } else { "  " };
                for text in message.content.0 {
                    if is_user && self.first_user_message.is_empty() {
                        self.first_user_message = text.clone();
                    }
                    self.messages.push(format!("{role_prefix}{text}"));
                }
            }
            "custom_message" if row.display == Some(true) => {
                for text in row.content.0 {
                    self.messages.push(format!("  {text}"));
                }
            }
            "compaction" | "branch_summary" => {
                let summary = row.summary.unwrap_or_default();
                if !summary.trim().is_empty() {
                    self.messages.push(format!("  {summary}"));
                }
            }
            _ => {}
        }
    }

    fn into_session(self, agent: &'static str, path: &Path) -> Option<Session> {
        if self.first_user_message.is_empty() && self.messages.is_empty() {
            return None;
        }
        let session_id = if self.session_id.is_empty() {
            pi_session_id_from_path(path)
        } else {
            self.session_id
        };
        let title_source = self.session_name.unwrap_or_else(|| {
            if self.first_user_message.is_empty() {
                "(no messages)".to_string()
            } else {
                self.first_user_message
            }
        });
        let mut session = Session::new(
            session_id,
            agent,
            truncate_title(&title_source, 100, true),
            self.directory,
            self.last_activity
                .or(self.header_timestamp)
                .unwrap_or_else(|| file_timestamp(path)),
            self.messages.join("\n\n"),
            self.message_count,
        );
        session.mtime = file_mtime_seconds(path);
        Some(session)
    }
}

/// Message roles that extensions use for messages they may show in the
/// transcript. Only rows with `display: true` are indexed.
fn is_custom_role(role: &str) -> bool {
    matches!(role, "custom" | "hookMessage")
}

/// Whether a message role's content is ever indexed. Content for any other
/// role (tool results, bash output, ...) is skipped without being decoded.
fn role_content_is_indexed(role: &str) -> bool {
    matches!(role, "user" | "assistant") || is_custom_role(role)
}

fn pi_session_id_from_path(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    stem.rsplit_once('_')
        .map(|(_, id)| id.to_string())
        .unwrap_or_else(|| stem.to_string())
}

// ---------------------------------------------------------------------------
// Row decoding
//
// Decoding every row into `serde_json::Value` spent most of a refresh
// allocating tool output that the index then threw away. The types below
// decode only the fields the adapter reads and let serde skip everything else.
//
// Every decoder is lenient: a field with an unexpected JSON type decodes as
// absent rather than failing the row, and a row that is not an object decodes
// as an empty row. This matches the previous `Value` lookups, and it means a
// row fails to decode only when it is not valid JSON, which is exactly what
// the JSONL health check counts as malformed.
// ---------------------------------------------------------------------------

/// Implements `Visitor` methods for JSON types a decoder does not use. The
/// value is consumed (containers are drained with `IgnoredAny`) and the
/// decoder returns its default.
macro_rules! ignore_json_types {
    ($de:lifetime; $($kind:ident),+ $(,)?) => {
        $(ignore_json_types!(@one $de $kind);)+
    };
    (@one $de:lifetime bool) => {
        fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
            Ok(Default::default())
        }
    };
    (@one $de:lifetime i64) => {
        fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
            Ok(Default::default())
        }
    };
    (@one $de:lifetime u64) => {
        fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
            Ok(Default::default())
        }
    };
    (@one $de:lifetime f64) => {
        fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
            Ok(Default::default())
        }
    };
    (@one $de:lifetime str) => {
        fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
            Ok(Default::default())
        }
    };
    (@one $de:lifetime unit) => {
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(Default::default())
        }
    };
    (@one $de:lifetime seq) => {
        fn visit_seq<A: SeqAccess<$de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok(Default::default())
        }
    };
    (@one $de:lifetime map) => {
        fn visit_map<A: MapAccess<$de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
            Ok(Default::default())
        }
    };
}

/// Implements `Deserialize` for a type through a unit visitor struct, always
/// using `deserialize_any` so the visitor sees the actual JSON type.
macro_rules! deserialize_any_with {
    ($target:ty, $visitor:ident) => {
        impl<'de> Deserialize<'de> for $target {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                deserializer.deserialize_any($visitor)
            }
        }
    };
}

/// Object key, borrowed from the input when it has no escapes.
struct Key<'de>(Cow<'de, str>);

struct KeyVisitor;

impl<'de> Visitor<'de> for KeyVisitor {
    type Value = Key<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an object key")
    }

    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
        Ok(Key(Cow::Borrowed(value)))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Key(Cow::Owned(value.to_owned())))
    }
}

impl<'de> Deserialize<'de> for Key<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(KeyVisitor)
    }
}

/// A JSON string, or `None` for any other JSON type.
#[derive(Default)]
struct LenientString(Option<String>);

struct LenientStringVisitor;

impl<'de> Visitor<'de> for LenientStringVisitor {
    type Value = LenientString;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(LenientString(Some(value.to_owned())))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(LenientString(Some(value)))
    }

    ignore_json_types!('de; bool, i64, u64, f64, unit, seq, map);
}

deserialize_any_with!(LenientString, LenientStringVisitor);

/// A JSON boolean, or `None` for any other JSON type.
#[derive(Default)]
struct LenientBool(Option<bool>);

struct LenientBoolVisitor;

impl<'de> Visitor<'de> for LenientBoolVisitor {
    type Value = LenientBool;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(LenientBool(Some(value)))
    }

    ignore_json_types!('de; i64, u64, f64, str, unit, seq, map);
}

deserialize_any_with!(LenientBool, LenientBoolVisitor);

/// A millisecond Unix timestamp from a JSON number, or `None` for any other
/// JSON type. Fractional values are truncated.
#[derive(Default)]
struct LenientMillis(Option<i64>);

struct LenientMillisVisitor;

impl<'de> Visitor<'de> for LenientMillisVisitor {
    type Value = LenientMillis;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(LenientMillis(Some(value)))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(LenientMillis(Some(
            i64::try_from(value).unwrap_or(i64::MAX),
        )))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        Ok(LenientMillis(Some(value as i64)))
    }

    ignore_json_types!('de; bool, str, unit, seq, map);
}

deserialize_any_with!(LenientMillis, LenientMillisVisitor);

/// Non-empty text from Pi message content: either a plain string or the
/// `text` of every `{"type": "text"}` part. Other parts (tool calls, thinking,
/// images) are skipped.
#[derive(Default)]
struct PiContent(Vec<String>);

struct PiContentVisitor;

impl<'de> Visitor<'de> for PiContentVisitor {
    type Value = PiContent;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(PiContent(
            (!value.is_empty())
                .then(|| value.to_owned())
                .into_iter()
                .collect(),
        ))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(PiContent(
            (!value.is_empty()).then_some(value).into_iter().collect(),
        ))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut texts = Vec::new();
        while let Some(part) = seq.next_element::<PiContentPart>()? {
            if part.kind.as_deref() == Some("text")
                && let Some(text) = part.text.filter(|text| !text.is_empty())
            {
                texts.push(text);
            }
        }
        Ok(PiContent(texts))
    }

    ignore_json_types!('de; bool, i64, u64, f64, unit, map);
}

deserialize_any_with!(PiContent, PiContentVisitor);

/// One element of an array-shaped content field.
#[derive(Default)]
struct PiContentPart {
    kind: Option<String>,
    text: Option<String>,
}

struct PiContentPartVisitor;

impl<'de> Visitor<'de> for PiContentPartVisitor {
    type Value = PiContentPart;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut part = PiContentPart::default();
        while let Some(key) = map.next_key::<Key>()? {
            match key.0.as_ref() {
                "type" => part.kind = map.next_value::<LenientString>()?.0,
                "text" => part.text = map.next_value::<LenientString>()?.0,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(part)
    }

    ignore_json_types!('de; bool, i64, u64, f64, str, unit, seq);
}

deserialize_any_with!(PiContentPart, PiContentPartVisitor);

/// The nested `message` object of a `{"type": "message"}` row.
#[derive(Default)]
struct PiMessage {
    role: Option<String>,
    content: PiContent,
    timestamp_ms: Option<i64>,
    display: Option<bool>,
}

struct PiMessageVisitor;

impl<'de> Visitor<'de> for PiMessageVisitor {
    type Value = PiMessage;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut message = PiMessage::default();
        while let Some(key) = map.next_key::<Key>()? {
            match key.0.as_ref() {
                "role" => message.role = map.next_value::<LenientString>()?.0,
                // Pi writes `role` before `content`, so tool results and other
                // unindexed roles skip their (often huge) content here. If
                // `content` ever comes first it is decoded and ignored later.
                "content" if message.role.as_deref().is_none_or(role_content_is_indexed) => {
                    message.content = map.next_value()?;
                }
                "timestamp" => message.timestamp_ms = map.next_value::<LenientMillis>()?.0,
                "display" => message.display = map.next_value::<LenientBool>()?.0,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(message)
    }

    ignore_json_types!('de; bool, i64, u64, f64, str, unit, seq);
}

deserialize_any_with!(PiMessage, PiMessageVisitor);

/// One transcript row, reduced to the fields the adapter reads.
#[derive(Default)]
struct PiRow {
    kind: Option<String>,
    id: Option<String>,
    cwd: Option<String>,
    timestamp: Option<String>,
    name: Option<String>,
    summary: Option<String>,
    display: Option<bool>,
    content: PiContent,
    message: Option<PiMessage>,
}

struct PiRowVisitor;

impl<'de> Visitor<'de> for PiRowVisitor {
    type Value = PiRow;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut row = PiRow::default();
        while let Some(key) = map.next_key::<Key>()? {
            // `type` is the first key Pi writes, so row kinds that never use
            // `content` or `message` skip those fields without decoding them.
            let kind = row.kind.as_deref();
            match key.0.as_ref() {
                "type" => row.kind = map.next_value::<LenientString>()?.0,
                "id" => row.id = map.next_value::<LenientString>()?.0,
                "cwd" => row.cwd = map.next_value::<LenientString>()?.0,
                "timestamp" => row.timestamp = map.next_value::<LenientString>()?.0,
                "name" => row.name = map.next_value::<LenientString>()?.0,
                "summary" => row.summary = map.next_value::<LenientString>()?.0,
                "display" => row.display = map.next_value::<LenientBool>()?.0,
                "content" if kind.is_none_or(|kind| kind == "custom_message") => {
                    row.content = map.next_value()?;
                }
                "message" if kind.is_none_or(|kind| kind == "message") => {
                    row.message = Some(map.next_value()?);
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(row)
    }

    ignore_json_types!('de; bool, i64, u64, f64, str, unit, seq);
}

deserialize_any_with!(PiRow, PiRowVisitor);

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use serde_json::{Value, json};
    use tempfile::tempdir;

    use crate::adapters::{Adapter, KnownSessions};
    use crate::model::file_mtime_seconds;

    use super::*;

    fn write_jsonl(path: &Path, rows: &[Value]) {
        fs::write(
            path,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
    }

    #[test]
    fn parses_pi_session_messages_and_metadata() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(sessions_dir.join("--repo-app--")).unwrap();
        let session_id = "11111111-1111-4111-8111-111111111111";
        let session_file = sessions_dir
            .join("--repo-app--")
            .join(format!("2026-07-15T10-00-00-000Z_{session_id}.jsonl"));
        write_jsonl(
            &session_file,
            &[
                json!({"type":"session","version":3,"id":session_id,"timestamp":"2026-07-15T10:00:00.000Z","cwd":"/repo/app"}),
                json!({"type":"message","id":"a1","parentId":null,"timestamp":"2026-07-15T10:00:01.000Z","message":{"role":"user","content":"Implement Pi adapter","timestamp":1784110801000i64}}),
                json!({"type":"message","id":"a2","parentId":"a1","timestamp":"2026-07-15T10:00:02.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Added parser"},{"type":"toolCall","id":"call_1","name":"bash","arguments":{"command":"secret args"}}],"timestamp":1784110802000i64}}),
                json!({"type":"message","id":"a3","parentId":"a2","timestamp":"2026-07-15T10:00:03.000Z","message":{"role":"toolResult","toolName":"bash","content":[{"type":"text","text":"tool output should stay out"}],"timestamp":1784110803000i64}}),
                json!({"type":"message","id":"a3custom","parentId":"a3","timestamp":"2026-07-15T10:00:03.100Z","message":{"role":"custom","customType":"message-note","content":[{"type":"text","text":"nested custom searchable note"}],"display":true,"timestamp":1784110803100i64}}),
                json!({"type":"message","id":"a3legacy","parentId":"a3custom","timestamp":"2026-07-15T10:00:03.200Z","message":{"role":"hookMessage","hookName":"legacy-note","content":"legacy hook searchable note","display":true,"timestamp":1784110803200i64}}),
                json!({"type":"message","id":"a3customhidden","parentId":"a3legacy","timestamp":"2026-07-15T10:00:03.300Z","message":{"role":"custom","customType":"hidden-message-note","content":"hidden nested extension context","display":false,"timestamp":1784110803300i64}}),
                json!({"type":"custom_message","id":"a4","parentId":"a3customhidden","timestamp":"2026-07-15T10:00:04.000Z","customType":"note","content":[{"type":"text","text":"top-level custom searchable note"}],"display":true}),
                json!({"type":"custom_message","id":"a4hidden","parentId":"a4","timestamp":"2026-07-15T10:00:04.500Z","customType":"hidden-note","content":[{"type":"text","text":"hidden extension context"}],"display":false}),
                json!({"type":"compaction","id":"a5","parentId":"a4hidden","timestamp":"2026-07-15T10:00:05.000Z","summary":"compacted context summary","firstKeptEntryId":"a2","tokensBefore":1000}),
                json!({"type":"message","id":"a6","parentId":"a5","timestamp":"2026-07-15T10:00:06.000Z","message":{"role":"bashExecution","command":"ls","output":"bash output should stay out","exitCode":0,"cancelled":false,"truncated":false,"timestamp":1784110806000i64}}),
                json!({"type":"session_info","id":"a7","parentId":"a6","timestamp":"2026-07-15T10:00:07.000Z","name":"Named Pi session"}),
            ],
        );

        let adapter = PiAdapter::new(sessions_dir);
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.id, session_id);
        assert_eq!(session.agent, "pi");
        assert_eq!(session.title, "Named Pi session");
        assert_eq!(session.directory, "/repo/app");
        assert_eq!(session.message_count, 1);
        assert!(session.content.contains("» Implement Pi adapter"));
        assert!(session.content.contains("Added parser"));
        assert!(session.content.contains("nested custom searchable note"));
        assert!(session.content.contains("legacy hook searchable note"));
        assert!(session.content.contains("top-level custom searchable note"));
        assert!(session.content.contains("compacted context summary"));
        assert!(!session.content.contains("tool output should stay out"));
        assert!(!session.content.contains("bash output should stay out"));
        assert!(!session.content.contains("hidden nested extension context"));
        assert!(!session.content.contains("hidden extension context"));
        assert!(!session.content.contains("secret args"));
        assert_eq!(
            adapter.resume_command(session, false),
            vec![
                "pi".to_string(),
                "--session".to_string(),
                session_id.to_string(),
            ]
        );
    }

    fn known_at_old_mtime(id: &str) -> KnownSessions {
        KnownSessions::from([(("pi".to_string(), id.to_string()), 1.0)])
    }

    #[test]
    fn rows_with_unexpected_shapes_do_not_mark_the_file_malformed() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let session_file = sessions_dir.join("session_shapes.jsonl");
        // Valid JSON rows whose fields have unexpected types. They must be
        // ignored field by field, not counted as malformed: a malformed last
        // row would make refresh keep the stale indexed copy.
        write_jsonl(
            &session_file,
            &[
                json!({"type":"session","id":"shapes","cwd":7,"timestamp":"2026-07-15T10:00:00.000Z"}),
                json!({"type":"message","timestamp":"2026-07-15T10:00:05.000Z","message":{"role":"user","content":[{"type":"text","text":"kept prompt"},{"type":"text","text":42},"bare"],"timestamp":"not a number"}}),
                json!({"type":"message","message":"not an object"}),
                json!({"type":"custom_message","display":"true","content":"string display flag is not true"}),
                json!(["message", {"role":"user","content":"array row"}]),
                json!(null),
            ],
        );

        let scan =
            PiAdapter::new(sessions_dir).find_sessions_incremental(&known_at_old_mtime("shapes"));

        assert_eq!(scan.new_or_modified.len(), 1);
        let session = &scan.new_or_modified[0];
        assert_eq!(session.content, "» kept prompt");
        assert_eq!(session.directory, "");
        // A non-numeric message timestamp falls back to the row timestamp.
        assert_eq!(
            session.timestamp,
            parse_datetime("2026-07-15T10:00:05.000Z").unwrap()
        );
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn content_is_filtered_by_role_regardless_of_key_order() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let session_file = sessions_dir.join("session_order.jsonl");
        // `json!` sorts keys, so write raw rows to control key order.
        fs::write(
            &session_file,
            [
                r#"{"type":"session","id":"order","cwd":"/repo"}"#,
                r#"{"type":"message","message":{"role":"user","content":"role first"}}"#,
                r#"{"type":"message","message":{"content":[{"text":"content first","type":"text"}],"role":"assistant"}}"#,
                r#"{"type":"message","message":{"role":"toolResult","content":"tool output role first"}}"#,
                r#"{"type":"message","message":{"content":"tool output content first","role":"toolResult"}}"#,
                r#"{"message":{"role":"user","content":"type after message"},"type":"message"}"#,
                r#"{"content":"type after content","display":true,"type":"custom_message"}"#,
            ]
            .join("\n"),
        )
        .unwrap();

        let sessions = PiAdapter::new(sessions_dir).find_sessions();

        assert_eq!(sessions.len(), 1);
        let content = &sessions[0].content;
        assert!(content.contains("» role first"));
        assert!(content.contains("  content first"));
        assert!(content.contains("» type after message"));
        assert!(content.contains("  type after content"));
        assert!(!content.contains("tool output"));
        assert_eq!(sessions[0].message_count, 2);
    }

    #[test]
    fn malformed_rows_follow_the_jsonl_health_rules() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let header = r#"{"type":"session","id":"torn","cwd":"/repo"}"#;
        let prompt = r#"{"type":"message","message":{"role":"user","content":"prompt"}}"#;
        let session_file = sessions_dir.join("session_torn.jsonl");
        let adapter = PiAdapter::new(sessions_dir);
        let known = known_at_old_mtime("torn");

        // A trailing torn row usually means Pi is still writing: keep the
        // indexed copy instead of replacing or deleting it.
        fs::write(&session_file, format!("{header}\n{prompt}\n{{\"type\":")).unwrap();
        let scan = adapter.find_sessions_incremental(&known);
        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());

        // A torn row followed by valid rows is a usable partial file.
        fs::write(&session_file, format!("{header}\n{{\"type\":\n{prompt}\n")).unwrap();
        let scan = adapter.find_sessions_incremental(&known);
        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].content, "» prompt");

        // Invalid UTF-8 makes the whole file unreadable for refresh.
        let mut bytes = format!("{header}\n{prompt}\n").into_bytes();
        bytes.extend_from_slice(b"{\"type\":\"\xff\"}\n");
        fs::write(&session_file, bytes).unwrap();
        let scan = adapter.find_sessions_incremental(&known);
        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn uuid_file_names_key_the_scan_without_hiding_the_header_id() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let file_id = "22222222-2222-4222-8222-222222222222";
        let header_id = "33333333-3333-4333-8333-333333333333";
        // A renamed or copied file whose name no longer matches its header.
        let session_file = sessions_dir.join(format!("2026-07-15T10-00-00-000Z_{file_id}.jsonl"));
        write_jsonl(
            &session_file,
            &[
                json!({"type":"session","id":header_id,"cwd":"/repo"}),
                json!({"type":"message","message":{"role":"user","content":"prompt"}}),
            ],
        );

        let scan =
            PiAdapter::new(sessions_dir).find_sessions_incremental(&known_at_old_mtime(header_id));

        // The session keeps its header id (used by `pi --session`), and the
        // refresh that re-indexes it must not also delete it.
        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].id, header_id);
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn falls_back_to_file_stem_session_id_suffix() {
        let path = Path::new("2026-07-15T10-00-00-000Z_abc123.jsonl");
        assert_eq!(pi_session_id_from_path(path), "abc123");
    }

    #[test]
    fn incremental_detects_deleted_pi_sessions() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let session_file = sessions_dir.join("session_test123.jsonl");
        write_jsonl(
            &session_file,
            &[
                json!({"type":"session","version":3,"id":"test123","timestamp":"2026-07-15T10:00:00.000Z","cwd":"/repo/app"}),
                json!({"type":"message","id":"a1","parentId":null,"timestamp":"2026-07-15T10:00:01.000Z","message":{"role":"user","content":"First prompt"}}),
            ],
        );

        let adapter = PiAdapter::new(sessions_dir);
        let mut known = KnownSessions::new();
        known.insert(
            ("pi".to_string(), "test123".to_string()),
            file_mtime_seconds(&session_file),
        );
        fs::remove_file(session_file).unwrap();

        let scan = adapter.find_sessions_incremental(&known);
        assert!(scan.new_or_modified.is_empty());
        assert_eq!(scan.deleted_ids, vec!["test123"]);
    }
}
