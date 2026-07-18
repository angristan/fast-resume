//! Antigravity CLI sessions combine generated JSONL transcripts with a
//! separate history file that maps conversation IDs back to workspaces.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use url::Url;
use walkdir::WalkDir;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    IncrementalParse, failed_incremental_scan, incremental_from_files,
    incremental_from_files_streaming, incremental_parse_jsonl, parse_datetime, raw_stats_for_tree,
    sqlite_file_stats, sqlite_mtime, string_at,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

type SessionFiles = HashMap<String, (PathBuf, f64)>;

const AGY_SQLITE_USER_INPUT: i64 = 14;
const AGY_SQLITE_PLANNER_RESPONSE: i64 = 15;
const AGY_SQLITE_DONE: i64 = 3;

#[derive(Debug, Clone)]
pub struct AntigravityAdapter {
    data_dir: PathBuf,
}

impl Default for AntigravityAdapter {
    fn default() -> Self {
        Self {
            data_dir: config::antigravity_dir(),
        }
    }
}

impl AntigravityAdapter {
    #[allow(dead_code)]
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    fn workspace_map(&self) -> HashMap<String, String> {
        let Ok(file) = fs::File::open(self.data_dir.join("history.jsonl")) else {
            return self.last_conversations_map();
        };
        let mut map = self.last_conversations_map();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let id = string_at(&value, &["conversationId"]);
            let workspace = string_at(&value, &["workspace"]);
            if !id.is_empty() && !workspace.is_empty() {
                map.insert(id, workspace);
            }
        }
        map
    }

    fn last_conversations_map(&self) -> HashMap<String, String> {
        let path = self.data_dir.join("cache").join("last_conversations.json");
        let Ok(value) = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .ok_or(())
        else {
            return HashMap::new();
        };
        value
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(workspace, value)| {
                let id = value.as_str().or_else(|| {
                    value
                        .get("conversationId")
                        .or_else(|| value.get("conversation_id"))
                        .and_then(Value::as_str)
                })?;
                Some((id.to_string(), workspace.to_string()))
            })
            .collect()
    }

    fn scan_session_files(&self) -> Option<(SessionFiles, bool)> {
        let mut files = HashMap::new();
        let mut complete = true;
        let history_mtime = self.workspace_mtime();

        let brain_dir = self.data_dir.join("brain");
        if brain_dir.exists() {
            if !brain_dir.is_dir() {
                return None;
            }
            let entries = fs::read_dir(brain_dir).ok()?;
            for entry in entries {
                let Ok(entry) = entry else {
                    complete = false;
                    continue;
                };
                let conversation_dir = entry.path();
                if !conversation_dir.is_dir() {
                    continue;
                }
                let Some(id) = conversation_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|id| !id.is_empty())
                else {
                    continue;
                };
                let logs = conversation_dir.join(".system_generated").join("logs");
                let Some(transcript) = preferred_transcript(&logs) else {
                    continue;
                };
                let mtime = file_mtime_seconds(&transcript).max(history_mtime);
                files.insert(id.to_string(), (transcript, mtime));
            }
        }

        let conversations_dir = self.data_dir.join("conversations");
        if conversations_dir.exists() {
            if !conversations_dir.is_dir() {
                return None;
            }
            let entries = fs::read_dir(conversations_dir).ok()?;
            for entry in entries {
                let Ok(entry) = entry else {
                    complete = false;
                    continue;
                };
                let path = entry.path();
                if path.extension().and_then(|extension| extension.to_str()) != Some("db") {
                    continue;
                }
                let Some(id) = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .filter(|id| !id.is_empty())
                else {
                    continue;
                };
                let transcript_mtime = files.get(id).map(|(_, mtime)| *mtime).unwrap_or_default();
                files.insert(
                    id.to_string(),
                    (
                        path.clone(),
                        sqlite_mtime(&path).max(history_mtime).max(transcript_mtime),
                    ),
                );
            }
        }

        Some((files, complete))
    }

    fn transcript_for_id(&self, id: &str) -> Option<PathBuf> {
        preferred_transcript(
            &self
                .data_dir
                .join("brain")
                .join(id)
                .join(".system_generated")
                .join("logs"),
        )
    }

    fn workspace_mtime(&self) -> f64 {
        file_mtime_seconds(&self.data_dir.join("history.jsonl")).max(file_mtime_seconds(
            &self.data_dir.join("cache").join("last_conversations.json"),
        ))
    }

    fn source_mtime(&self, path: &Path) -> f64 {
        let source_mtime = if is_sqlite_database(path) {
            sqlite_mtime(path)
        } else {
            file_mtime_seconds(path)
        };
        source_mtime.max(self.workspace_mtime())
    }

    fn parse_jsonl_session(
        &self,
        path: &Path,
        workspaces: &HashMap<String, String>,
    ) -> Option<Session> {
        let id = antigravity_session_id(path)?;
        let file = fs::File::open(path).ok()?;
        let mut rendered = Vec::new();
        let mut first_user = String::new();
        let mut user_turns = 0usize;
        let mut last_activity = None;

        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(step) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let source = string_at(&step, &["source"]);
            let step_type = string_at(&step, &["type"]);
            if matches!(
                step_type.as_str(),
                "CONVERSATION_HISTORY" | "EPHEMERAL_MESSAGE"
            ) {
                continue;
            }
            let is_user = matches!(source.as_str(), "USER_EXPLICIT" | "USER");
            let is_assistant = source == "MODEL" && step_type == "PLANNER_RESPONSE";
            if !is_user && !is_assistant {
                continue;
            }
            let mut text = string_at(&step, &["content"]);
            if is_user {
                text = unwrap_user_request(&text);
            } else {
                text = text.trim().to_string();
                if text.is_empty() {
                    text = string_at(&step, &["thinking"]).trim().to_string();
                }
            }
            if text.is_empty() {
                continue;
            }
            if is_user {
                user_turns += 1;
                if first_user.is_empty() {
                    first_user = text.clone();
                }
            }
            if let Some(timestamp) = parse_datetime(&string_at(&step, &["created_at"]))
                && last_activity.is_none_or(|current| timestamp > current)
            {
                last_activity = Some(timestamp);
            }
            let prefix = if is_user { "» " } else { "  " };
            rendered.push(format!("{prefix}{text}"));
        }
        if first_user.is_empty() {
            return None;
        }
        let mut session = Session::new(
            &id,
            self.name(),
            truncate_title(&first_user, 100, true),
            workspaces.get(&id).cloned().unwrap_or_default(),
            last_activity.unwrap_or_else(|| file_timestamp(path)),
            rendered.join("\n\n"),
            user_turns,
        );
        session.mtime = self.source_mtime(path);
        Some(session)
    }

    fn parse_sqlite_session(
        &self,
        path: &Path,
        workspaces: &HashMap<String, String>,
    ) -> rusqlite::Result<Option<Session>> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let fallback_id = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let id = connection
            .query_row(
                "SELECT cascade_id FROM trajectory_meta LIMIT 1",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
            .filter(|id| !id.trim().is_empty())
            .unwrap_or(fallback_id);
        if id.is_empty() {
            return Ok(None);
        }

        let metadata = connection
            .query_row(
                "SELECT data FROM trajectory_metadata_blob WHERE id = 'main' LIMIT 1",
                [],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        let directory = metadata
            .as_deref()
            .and_then(antigravity_sqlite_directory)
            .or_else(|| workspaces.get(&id).cloned())
            .unwrap_or_default();

        let mut statement = connection
            .prepare("SELECT step_type, status, metadata, step_payload FROM steps ORDER BY idx")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, Option<Vec<u8>>>(3)?,
            ))
        })?;
        let mut rendered = Vec::new();
        let mut first_user = String::new();
        let mut user_turns = 0usize;
        let mut last_activity: Option<DateTime<Local>> = None;

        for row in rows {
            let (column_step_type, column_status, metadata, payload) = row?;
            let Some(payload) = payload else {
                continue;
            };
            let fields = ProtoMessage::new(&payload);
            let step_type = fields
                .first_i64(1)
                .filter(|step_type| *step_type != 0)
                .unwrap_or(column_step_type);
            let status = fields
                .first_i64(4)
                .filter(|status| *status != 0)
                .unwrap_or(column_status);
            if status != 0 && status != AGY_SQLITE_DONE {
                continue;
            }
            let Some((user, mut text)) = antigravity_sqlite_step_text(fields, step_type) else {
                continue;
            };
            if user {
                text = unwrap_user_request(&text);
            } else {
                text = text.trim().to_string();
            }
            if text.is_empty() {
                continue;
            }
            if user {
                user_turns += 1;
                if first_user.is_empty() {
                    first_user = text.clone();
                }
            }
            let timestamp = antigravity_sqlite_payload_timestamp(fields).or_else(|| {
                metadata
                    .as_deref()
                    .and_then(antigravity_sqlite_metadata_timestamp)
            });
            if let Some(timestamp) = timestamp
                && last_activity.is_none_or(|current| timestamp > current)
            {
                last_activity = Some(timestamp);
            }
            rendered.push(format!("{}{}", if user { "» " } else { "  " }, text));
        }
        if first_user.is_empty() {
            return Ok(None);
        }

        let mut session = Session::new(
            &id,
            self.name(),
            truncate_title(&first_user, 100, true),
            directory,
            last_activity.unwrap_or_else(|| file_timestamp(path)),
            rendered.join("\n\n"),
            user_turns,
        );
        session.mtime = self.source_mtime(path);
        Ok(Some(session))
    }

    fn parse_session(&self, path: &Path, workspaces: &HashMap<String, String>) -> Option<Session> {
        if !is_sqlite_database(path) {
            return self.parse_jsonl_session(path, workspaces);
        }
        self.parse_sqlite_session(path, workspaces)
            .ok()
            .flatten()
            .or_else(|| {
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .and_then(|id| self.transcript_for_id(id))
                    .and_then(|transcript| self.parse_jsonl_session(&transcript, workspaces))
            })
    }

    fn parse_incremental(
        &self,
        path: &Path,
        workspaces: &HashMap<String, String>,
    ) -> IncrementalParse {
        if is_sqlite_database(path) {
            return match self.parse_sqlite_session(path, workspaces) {
                Ok(Some(session)) => IncrementalParse::Session(session),
                Ok(None) => path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .and_then(|id| self.transcript_for_id(id))
                    .map_or(IncrementalParse::Delete, |transcript| {
                        incremental_parse_jsonl(&transcript, || {
                            self.parse_jsonl_session(&transcript, workspaces)
                        })
                    }),
                Err(_) => IncrementalParse::Retain,
            };
        }
        incremental_parse_jsonl(path, || self.parse_jsonl_session(path, workspaces))
    }
}

impl Adapter for AntigravityAdapter {
    fn name(&self) -> &'static str {
        "antigravity"
    }

    fn supports_yolo(&self) -> bool {
        true
    }

    fn find_sessions(&self) -> Vec<Session> {
        let workspaces = self.workspace_map();
        self.scan_session_files()
            .map(|(files, _)| {
                files
                    .into_values()
                    .filter_map(|(path, _)| self.parse_session(&path, &workspaces))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        let Some((files, complete)) = self.scan_session_files() else {
            return failed_incremental_scan(self.name());
        };
        let workspaces = self.workspace_map();
        let mut scan = incremental_from_files(self.name(), known, files, |path| {
            self.parse_incremental(path, &workspaces)
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
        let workspaces = self.workspace_map();
        let mut scan = incremental_from_files_streaming(
            self.name(),
            known,
            files,
            |path| self.parse_incremental(path, &workspaces),
            on_session,
        );
        if !complete {
            scan.deleted_ids.clear();
        }
        scan
    }

    fn resume_command(&self, session: &Session, yolo: bool) -> Vec<String> {
        let mut command = vec!["agy".to_string()];
        if yolo {
            command.push("--dangerously-skip-permissions".to_string());
        }
        command.extend(["--conversation".to_string(), session.id.clone()]);
        command
    }

    fn raw_stats(&self) -> RawAdapterStats {
        let brain = raw_stats_for_tree(self.name(), &self.data_dir.join("brain"), "jsonl");
        let mut file_count = brain.file_count;
        let mut total_bytes = brain.total_bytes;
        let conversations_dir = self.data_dir.join("conversations");
        for entry in WalkDir::new(&conversations_dir)
            .max_depth(1)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) == Some("db") {
                let (files, bytes) = sqlite_file_stats(path);
                file_count += files;
                total_bytes += bytes;
            }
        }
        RawAdapterStats {
            agent: self.name(),
            data_dir: self.data_dir.display().to_string(),
            available: brain.available || conversations_dir.is_dir(),
            file_count,
            total_bytes,
        }
    }
}

fn is_sqlite_database(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some("db")
}

fn antigravity_sqlite_step_text(
    fields: ProtoMessage<'_>,
    step_type: i64,
) -> Option<(bool, String)> {
    match step_type {
        AGY_SQLITE_USER_INPUT => fields
            .first_message(19)
            .and_then(|message| message.first_string(2).or_else(|| message.first_string(8)))
            .map(|text| (true, text)),
        AGY_SQLITE_PLANNER_RESPONSE => fields
            .first_message(20)
            .and_then(|message| message.first_string(1).or_else(|| message.first_string(8)))
            .map(|text| (false, text)),
        _ => None,
    }
}

fn antigravity_sqlite_payload_timestamp(fields: ProtoMessage<'_>) -> Option<DateTime<Local>> {
    fields
        .first_message(5)?
        .first_message(1)
        .and_then(antigravity_proto_timestamp)
}

fn antigravity_sqlite_metadata_timestamp(data: &[u8]) -> Option<DateTime<Local>> {
    ProtoMessage::new(data)
        .first_message(1)
        .and_then(antigravity_proto_timestamp)
}

fn antigravity_proto_timestamp(message: ProtoMessage<'_>) -> Option<DateTime<Local>> {
    let seconds = message.first_i64(1)?;
    let nanos = message.first_i64(2).unwrap_or(0);
    let nanos = u32::try_from(nanos)
        .ok()
        .filter(|nanos| *nanos < 1_000_000_000)?;
    Local.timestamp_opt(seconds, nanos).single()
}

fn antigravity_sqlite_directory(data: &[u8]) -> Option<String> {
    let fields = ProtoMessage::new(data);
    let value = fields
        .first_message(1)
        .and_then(|message| message.first_string(1).or_else(|| message.first_string(2)))
        .or_else(|| fields.first_string(7))?;
    if value.starts_with("file:") {
        return Url::parse(&value)
            .ok()?
            .to_file_path()
            .ok()
            .map(|path| path.to_string_lossy().into_owned());
    }
    Some(value)
}

// Agy stores each step as a protobuf blob but does not publish descriptors. These
// bounded wire readers extract only the stable fields required for session replay:
// step kind/status, user or planner text, timestamps, and the workspace URI.
#[derive(Clone, Copy)]
struct ProtoMessage<'a> {
    data: &'a [u8],
}

impl<'a> ProtoMessage<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    fn fields(self) -> ProtoFields<'a> {
        ProtoFields {
            data: self.data,
            position: 0,
        }
    }

    fn first_i64(self, number: u64) -> Option<i64> {
        self.fields().find_map(|field| match field {
            ProtoField::Varint(field_number, value) if field_number == number => {
                i64::try_from(value).ok()
            }
            _ => None,
        })
    }

    fn first_string(self, number: u64) -> Option<String> {
        self.fields().find_map(|field| match field {
            ProtoField::LengthDelimited(field_number, value) if field_number == number => {
                std::str::from_utf8(value)
                    .ok()
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
            }
            _ => None,
        })
    }

    fn first_message(self, number: u64) -> Option<ProtoMessage<'a>> {
        self.fields().find_map(|field| match field {
            ProtoField::LengthDelimited(field_number, value) if field_number == number => {
                Some(ProtoMessage::new(value))
            }
            _ => None,
        })
    }
}

struct ProtoFields<'a> {
    data: &'a [u8],
    position: usize,
}

enum ProtoField<'a> {
    Varint(u64, u64),
    LengthDelimited(u64, &'a [u8]),
    Fixed32,
    Fixed64,
}

impl<'a> Iterator for ProtoFields<'a> {
    type Item = ProtoField<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let key = read_proto_varint(self.data, &mut self.position)?;
        let field_number = key >> 3;
        if field_number == 0 {
            return None;
        }
        match key & 7 {
            0 => read_proto_varint(self.data, &mut self.position)
                .map(|value| ProtoField::Varint(field_number, value)),
            1 => {
                self.position = self.position.checked_add(8)?;
                (self.position <= self.data.len()).then_some(ProtoField::Fixed64)
            }
            2 => {
                let length =
                    usize::try_from(read_proto_varint(self.data, &mut self.position)?).ok()?;
                let end = self.position.checked_add(length)?;
                if end > self.data.len() {
                    return None;
                }
                let value = &self.data[self.position..end];
                self.position = end;
                Some(ProtoField::LengthDelimited(field_number, value))
            }
            5 => {
                self.position = self.position.checked_add(4)?;
                (self.position <= self.data.len()).then_some(ProtoField::Fixed32)
            }
            _ => None,
        }
    }
}

fn read_proto_varint(data: &[u8], position: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    let mut shift = 0u32;
    while *position < data.len() && shift < 64 {
        let byte = data[*position];
        *position += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(value);
        }
        shift += 7;
    }
    None
}

fn preferred_transcript(logs: &Path) -> Option<PathBuf> {
    let regular = logs.join("transcript.jsonl");
    let full = logs.join("transcript_full.jsonl");
    match (
        regular.metadata().ok().map(|metadata| metadata.len()),
        full.metadata().ok().map(|metadata| metadata.len()),
    ) {
        (Some(regular_size), Some(full_size)) if full_size >= regular_size && full_size > 0 => {
            Some(full)
        }
        (Some(_), _) => Some(regular),
        (_, Some(full_size)) if full_size > 0 => Some(full),
        _ => None,
    }
}

fn antigravity_session_id(path: &Path) -> Option<String> {
    path.parent()?
        .parent()?
        .parent()?
        .file_name()?
        .to_str()
        .map(ToString::to_string)
}

fn unwrap_user_request(content: &str) -> String {
    if let Some(after) = content.split_once("<USER_REQUEST>").map(|(_, after)| after)
        && let Some((request, _)) = after.split_once("</USER_REQUEST>")
    {
        return request.trim().to_string();
    }
    let end = ["<ADDITIONAL_METADATA>", "<USER_SETTINGS_CHANGE>"]
        .into_iter()
        .filter_map(|marker| content.find(marker))
        .min()
        .unwrap_or(content.len());
    content[..end].trim().to_string()
}

#[cfg(test)]
mod tests {
    use std::{fs, thread, time::Duration};

    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::*;

    fn proto_varint(mut value: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            bytes.push(byte);
            if value == 0 {
                return bytes;
            }
        }
    }

    fn proto_int(number: u64, value: u64) -> Vec<u8> {
        let mut bytes = proto_varint(number << 3);
        bytes.extend(proto_varint(value));
        bytes
    }

    fn proto_bytes(number: u64, value: &[u8]) -> Vec<u8> {
        let mut bytes = proto_varint((number << 3) | 2);
        bytes.extend(proto_varint(value.len() as u64));
        bytes.extend(value);
        bytes
    }

    fn proto_string(number: u64, value: &str) -> Vec<u8> {
        proto_bytes(number, value.as_bytes())
    }

    fn sqlite_step_payload(step_type: u64, text: &str, timestamp: u64) -> Vec<u8> {
        let timestamp = [proto_int(1, timestamp), proto_int(2, 0)].concat();
        let metadata = proto_bytes(1, &timestamp);
        let text = if step_type == AGY_SQLITE_USER_INPUT as u64 {
            proto_bytes(19, &proto_string(2, text))
        } else {
            proto_bytes(20, &proto_string(1, text))
        };
        [
            proto_int(1, step_type),
            proto_int(4, 3),
            proto_bytes(5, &metadata),
            text,
        ]
        .concat()
    }

    #[test]
    fn parses_transcript_and_workspace_history() {
        let temp = tempdir().unwrap();
        let id = "52d82992-7695-4d38-8d02-9747eecba839";
        let logs = temp
            .path()
            .join("brain")
            .join(id)
            .join(".system_generated/logs");
        fs::create_dir_all(&logs).unwrap();
        let rows = [
            json!({"source":"USER_EXPLICIT","type":"USER_INPUT","created_at":"2026-07-17T10:00:00Z","content":"<USER_REQUEST>\nAdd Antigravity support\n</USER_REQUEST>\n<ADDITIONAL_METADATA>ignored</ADDITIONAL_METADATA>"}),
            json!({"source":"SYSTEM","type":"CONVERSATION_HISTORY","content":"ignore"}),
            json!({"source":"MODEL","type":"VIEW_FILE","content":"large tool result that should not be indexed"}),
            json!({"source":"MODEL","type":"PLANNER_RESPONSE","created_at":"2026-07-17T10:00:01Z","content":"Implemented the adapter"}),
        ];
        fs::write(
            logs.join("transcript.jsonl"),
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        fs::write(
            temp.path().join("history.jsonl"),
            json!({"conversationId":id,"workspace":"/work/antigravity"}).to_string(),
        )
        .unwrap();
        fs::create_dir_all(temp.path().join("conversations")).unwrap();
        fs::write(
            temp.path().join("conversations").join(format!("{id}.db")),
            b"unsupported database",
        )
        .unwrap();

        let adapter = AntigravityAdapter::new(temp.path().to_path_buf());
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, id);
        assert_eq!(sessions[0].directory, "/work/antigravity");
        assert_eq!(sessions[0].title, "Add Antigravity support");
        assert_eq!(sessions[0].message_count, 1);
        assert!(sessions[0].content.contains("Implemented the adapter"));
        assert!(!sessions[0].content.contains("ADDITIONAL_METADATA"));
        assert!(!sessions[0].content.contains("large tool result"));
        assert!(adapter.supports_yolo());
        assert_eq!(
            adapter.resume_command(&sessions[0], false),
            vec!["agy", "--conversation", id]
        );
        assert_eq!(
            adapter.resume_command(&sessions[0], true),
            vec![
                "agy",
                "--dangerously-skip-permissions",
                "--conversation",
                id
            ]
        );
    }

    #[test]
    fn parses_sqlite_and_detects_wal_updates() {
        let temp = tempdir().unwrap();
        let id = "52d82992-7695-4d38-8d02-9747eecba840";
        let conversations = temp.path().join("conversations");
        fs::create_dir_all(&conversations).unwrap();
        let db_path = conversations.join(format!("{id}.db"));
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0; \
                 CREATE TABLE trajectory_meta (cascade_id TEXT); \
                 CREATE TABLE trajectory_metadata_blob (id TEXT PRIMARY KEY, data BLOB); \
                 CREATE TABLE steps (idx INTEGER PRIMARY KEY, step_type INTEGER NOT NULL, \
                     status INTEGER NOT NULL, metadata BLOB, step_payload BLOB);",
            )
            .unwrap();
        connection
            .execute("INSERT INTO trajectory_meta (cascade_id) VALUES (?1)", [id])
            .unwrap();
        let workspace = proto_bytes(1, &proto_string(1, "file:///work/sqlite"));
        connection
            .execute(
                "INSERT INTO trajectory_metadata_blob (id, data) VALUES ('main', ?1)",
                [&workspace],
            )
            .unwrap();
        for (idx, step_type, text, timestamp) in [
            (
                0,
                AGY_SQLITE_USER_INPUT as u64,
                "<USER_REQUEST>SQLite prompt</USER_REQUEST>",
                1_784_282_400,
            ),
            (
                1,
                AGY_SQLITE_PLANNER_RESPONSE as u64,
                "SQLite response",
                1_784_282_401,
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO steps (idx, step_type, status, step_payload) \
                     VALUES (?1, ?2, 3, ?3)",
                    rusqlite::params![
                        idx,
                        step_type as i64,
                        sqlite_step_payload(step_type, text, timestamp)
                    ],
                )
                .unwrap();
        }

        let adapter = AntigravityAdapter::new(temp.path().to_path_buf());
        let session = adapter.find_sessions().pop().unwrap();
        assert_eq!(session.id, id);
        assert_eq!(session.directory, "/work/sqlite");
        assert_eq!(session.title, "SQLite prompt");
        assert_eq!(session.message_count, 1);
        assert!(session.content.contains("SQLite response"));
        let mut known = KnownSessions::new();
        known.insert(("antigravity".to_string(), id.to_string()), session.mtime);

        thread::sleep(Duration::from_millis(10));
        connection
            .execute(
                "INSERT INTO steps (idx, step_type, status, step_payload) \
                 VALUES (2, 15, 3, ?1)",
                [sqlite_step_payload(
                    AGY_SQLITE_PLANNER_RESPONSE as u64,
                    "WAL response",
                    1_784_282_402,
                )],
            )
            .unwrap();

        let scan = adapter.find_sessions_incremental(&known);
        assert_eq!(scan.new_or_modified.len(), 1);
        assert!(scan.new_or_modified[0].content.contains("WAL response"));
        assert!(scan.new_or_modified[0].mtime > session.mtime);
        let stats = adapter.raw_stats();
        assert!(stats.file_count >= 2);
        assert!(stats.total_bytes >= db_path.metadata().unwrap().len());
    }

    #[test]
    fn prefers_larger_full_transcript() {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join("transcript.jsonl"), "x").unwrap();
        fs::write(temp.path().join("transcript_full.jsonl"), "longer").unwrap();
        assert_eq!(
            preferred_transcript(temp.path()),
            Some(temp.path().join("transcript_full.jsonl"))
        );
    }
}
