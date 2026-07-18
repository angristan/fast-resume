//! Grok Build persists session metadata beside an ACP update stream. Agent
//! message chunks are grouped by prompt ID to reconstruct complete responses.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone};
use serde_json::Value;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    IncrementalParse, failed_incremental_scan, incremental_from_files,
    incremental_from_files_streaming, incremental_parse_jsonl, json_file_has_parse_errors,
    parse_datetime, raw_stats_for_tree, string_at,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

type SessionFiles = HashMap<String, (PathBuf, f64)>;

#[derive(Debug, Clone)]
pub struct GrokAdapter {
    sessions_dir: PathBuf,
}

impl Default for GrokAdapter {
    fn default() -> Self {
        Self {
            sessions_dir: config::grok_sessions_dir(),
        }
    }
}

impl GrokAdapter {
    #[allow(dead_code)]
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir }
    }

    fn scan_session_files(&self) -> Option<(SessionFiles, bool)> {
        let mut files = HashMap::new();
        let mut complete = true;
        if !self.sessions_dir.exists() {
            return Some((files, complete));
        }
        if !self.sessions_dir.is_dir() {
            return None;
        }
        let workspaces = fs::read_dir(&self.sessions_dir).ok()?;
        for workspace in workspaces {
            let Ok(workspace) = workspace else {
                complete = false;
                continue;
            };
            if !workspace.path().is_dir() {
                continue;
            }
            let Ok(sessions) = fs::read_dir(workspace.path()) else {
                complete = false;
                continue;
            };
            for session in sessions {
                let Ok(session) = session else {
                    complete = false;
                    continue;
                };
                let session_dir = session.path();
                if !session_dir.is_dir() {
                    continue;
                }
                let Some(id) = session_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|id| !id.is_empty())
                else {
                    continue;
                };
                let updates = session_dir.join("updates.jsonl");
                if !updates.is_file() {
                    continue;
                }
                let mtime = file_mtime_seconds(&updates)
                    .max(file_mtime_seconds(&session_dir.join("summary.json")));
                files.insert(id.to_string(), (updates, mtime));
            }
        }
        Some((files, complete))
    }

    fn parse_session(&self, updates_path: &Path) -> Option<Session> {
        let session_dir = updates_path.parent()?;
        let fallback_id = session_dir.file_name()?.to_string_lossy().to_string();
        let summary: Value =
            serde_json::from_slice(&fs::read(session_dir.join("summary.json")).ok()?).ok()?;
        let id = summary
            .pointer("/info/id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .unwrap_or(&fallback_id)
            .to_string();
        let directory = summary
            .pointer("/info/cwd")
            .and_then(Value::as_str)
            .filter(|directory| !directory.trim().is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| grok_directory_from_path(updates_path));

        let file = fs::File::open(updates_path).ok()?;
        let mut messages: Vec<(bool, String)> = Vec::new();
        let mut user_message_indices = Vec::new();
        let mut pending_user: Option<(Option<usize>, usize)> = None;
        let mut pending_agent: Option<(String, usize)> = None;
        let mut seen_prompt_index = false;
        let mut last_activity = None;

        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(record) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let Some(timestamp) = grok_timestamp(record.get("timestamp"))
                && last_activity.is_none_or(|current| timestamp > current)
            {
                last_activity = Some(timestamp);
            }
            let params = record.get("params").unwrap_or(&Value::Null);
            let update = params.get("update").unwrap_or(&Value::Null);
            match string_at(update, &["sessionUpdate"]).as_str() {
                "user_message_chunk" => {
                    pending_agent = None;
                    let content = update.get("content").unwrap_or(&Value::Null);
                    if content.pointer("/_meta/bashCommand").is_some() {
                        pending_user = None;
                        continue;
                    }
                    let text = grok_content_text(content);
                    if text.is_empty() {
                        continue;
                    }
                    let prompt_index = update
                        .pointer("/_meta/promptIndex")
                        .and_then(Value::as_u64)
                        .map(|index| index as usize);
                    if prompt_index.is_some() {
                        seen_prompt_index = true;
                    }
                    let counts_as_user = !seen_prompt_index || prompt_index.is_some();
                    if !counts_as_user {
                        pending_user = None;
                        continue;
                    }
                    if let Some((pending_index, message_index)) = &pending_user
                        && pending_index == &prompt_index
                        && let Some((true, current)) = messages.get_mut(*message_index)
                    {
                        current.push_str(&text);
                        continue;
                    }
                    let message_index = messages.len();
                    user_message_indices.push(message_index);
                    messages.push((true, text));
                    pending_user = Some((prompt_index, message_index));
                }
                "agent_message_chunk" => {
                    pending_user = None;
                    let text = grok_content_text(update.get("content").unwrap_or(&Value::Null));
                    if text.trim().is_empty() {
                        continue;
                    }
                    let prompt_id = params
                        .pointer("/_meta/promptId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if let Some((pending_id, index)) = &pending_agent
                        && pending_id == &prompt_id
                        && let Some((false, current)) = messages.get_mut(*index)
                    {
                        current.push_str(&text);
                        continue;
                    }
                    let index = messages.len();
                    messages.push((false, text));
                    pending_agent = Some((prompt_id, index));
                }
                "rewind_marker" => {
                    if let Some(target) = update
                        .get("target_prompt_index")
                        .or_else(|| update.get("targetPromptIndex"))
                        .and_then(Value::as_u64)
                        .map(|index| index as usize)
                        && let Some(message_index) = user_message_indices.get(target).copied()
                    {
                        messages.truncate(message_index);
                        user_message_indices.truncate(target);
                    }
                    pending_user = None;
                    pending_agent = None;
                }
                _ => {
                    pending_user = None;
                }
            }
        }
        let first_user = messages
            .iter()
            .find(|(user, text)| *user && !text.trim().is_empty())
            .map(|(_, text)| text.trim().to_string())?;
        let user_turns = messages.iter().filter(|(user, _)| *user).count();

        let title = ["generated_title", "session_summary"]
            .into_iter()
            .find_map(|field| {
                summary
                    .get(field)
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or(&first_user);
        let timestamp = last_activity
            .or_else(|| {
                ["updated_at", "last_active_at", "created_at"]
                    .into_iter()
                    .find_map(|field| {
                        summary
                            .get(field)
                            .and_then(Value::as_str)
                            .and_then(parse_datetime)
                    })
            })
            .unwrap_or_else(|| file_timestamp(updates_path));
        let content = messages
            .into_iter()
            .map(|(user, text)| format!("{}{}", if user { "» " } else { "  " }, text))
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut session = Session::new(
            id,
            self.name(),
            truncate_title(title, 100, true),
            directory,
            timestamp,
            content,
            user_turns,
        );
        session.mtime = file_mtime_seconds(updates_path)
            .max(file_mtime_seconds(&session_dir.join("summary.json")));
        Some(session)
    }

    fn parse_incremental(&self, path: &Path) -> IncrementalParse {
        if json_file_has_parse_errors(&path.parent().unwrap_or(Path::new("")).join("summary.json"))
        {
            return IncrementalParse::Retain;
        }
        incremental_parse_jsonl(path, || self.parse_session(path))
    }
}

impl Adapter for GrokAdapter {
    fn name(&self) -> &'static str {
        "grok"
    }

    fn supports_yolo(&self) -> bool {
        true
    }

    fn find_sessions(&self) -> Vec<Session> {
        self.scan_session_files()
            .map(|(files, _)| {
                files
                    .into_values()
                    .filter_map(|(path, _)| self.parse_session(&path))
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
        let mut command = vec!["grok".to_string()];
        if yolo {
            command.push("--always-approve".to_string());
        }
        command.extend(["--resume".to_string(), session.id.clone()]);
        command
    }

    fn raw_stats(&self) -> RawAdapterStats {
        raw_stats_for_tree(self.name(), &self.sessions_dir, "jsonl")
    }
}

fn grok_content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    if let Some(text) = content.get("text").and_then(Value::as_str) {
        return text.to_string();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

fn grok_timestamp(value: Option<&Value>) -> Option<DateTime<Local>> {
    let value = value?;
    if let Some(text) = value.as_str() {
        return parse_datetime(text);
    }
    let number = value
        .as_i64()
        .or_else(|| value.as_f64().map(|value| value as i64))?;
    if number > 100_000_000_000 {
        Local.timestamp_millis_opt(number).single()
    } else {
        Local.timestamp_opt(number, 0).single()
    }
}

fn grok_directory_from_path(path: &Path) -> String {
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
    use std::fs;

    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn parses_streamed_messages_and_summary() {
        let temp = tempdir().unwrap();
        let id = "019edf9c-0000-7000-8000-000000000001";
        let session_dir = temp.path().join("%2Fwork%2Fgrok").join(id);
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("summary.json"),
            json!({
                "info":{"id":id,"cwd":"/work/grok"},
                "created_at":"2026-07-17T10:00:00Z",
                "generated_title":"Grok adapter work"
            })
            .to_string(),
        )
        .unwrap();
        let rows = [
            json!({"timestamp":"2026-07-17T10:00:01Z","params":{"update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"Add Grok "},"_meta":{"promptIndex":0}}}}),
            json!({"timestamp":"2026-07-17T10:00:01Z","params":{"update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"support"},"_meta":{"promptIndex":0}}}}),
            json!({"timestamp":"2026-07-17T10:00:02Z","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Added "}},"_meta":{"promptId":"p1"}}}),
            json!({"timestamp":"2026-07-17T10:00:03Z","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"the adapter"}},"_meta":{"promptId":"p1"}}}),
        ];
        fs::write(
            session_dir.join("updates.jsonl"),
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();

        let adapter = GrokAdapter::new(temp.path().to_path_buf());
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, id);
        assert_eq!(sessions[0].title, "Grok adapter work");
        assert_eq!(sessions[0].directory, "/work/grok");
        assert_eq!(sessions[0].message_count, 1);
        assert!(sessions[0].content.contains("Add Grok support"));
        assert!(sessions[0].content.contains("Added the adapter"));
        assert_eq!(
            adapter.resume_command(&sessions[0], true),
            vec!["grok", "--always-approve", "--resume", id]
        );
    }

    #[test]
    fn rewinds_discard_abandoned_messages() {
        let temp = tempdir().unwrap();
        let id = "019edf9c-0000-7000-8000-000000000002";
        let session_dir = temp.path().join("%2Fwork%2Fgrok").join(id);
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("summary.json"),
            json!({"info":{"id":id,"cwd":"/work/grok"},"created_at":"2026-07-17T10:00:00Z"})
                .to_string(),
        )
        .unwrap();
        let rows = [
            json!({"params":{"update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"Keep prompt"},"_meta":{"promptIndex":0}}}}),
            json!({"params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Keep response"}},"_meta":{"promptId":"p0"}}}),
            json!({"params":{"update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"Discard prompt"},"_meta":{"promptIndex":1}}}}),
            json!({"params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Discard response"}},"_meta":{"promptId":"p1"}}}),
            json!({"method":"_x.ai/session/update","params":{"update":{"sessionUpdate":"rewind_marker","target_prompt_index":1}}}),
            json!({"params":{"update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"Replacement prompt"},"_meta":{"promptIndex":1}}}}),
            json!({"params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Replacement response"}},"_meta":{"promptId":"p2"}}}),
        ];
        fs::write(
            session_dir.join("updates.jsonl"),
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();

        let adapter = GrokAdapter::new(temp.path().to_path_buf());
        let session = adapter.find_sessions().pop().unwrap();

        assert_eq!(session.message_count, 2);
        assert!(session.content.contains("Keep prompt"));
        assert!(session.content.contains("Keep response"));
        assert!(session.content.contains("Replacement prompt"));
        assert!(session.content.contains("Replacement response"));
        assert!(!session.content.contains("Discard prompt"));
        assert!(!session.content.contains("Discard response"));
    }

    #[test]
    fn decodes_workspace_directory_fallback() {
        assert_eq!(percent_decode("%2Fwork%2Fproject"), "/work/project");
    }
}
