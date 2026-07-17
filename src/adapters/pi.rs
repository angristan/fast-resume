use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone};
use serde_json::Value;
use walkdir::WalkDir;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    IncrementalParse, failed_incremental_scan, incremental_from_files,
    incremental_from_files_streaming, incremental_parse_jsonl, parse_datetime, raw_stats_for_tree,
    string_at,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

type SessionFiles = HashMap<String, (PathBuf, f64)>;

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

    fn session_id_from_file(&self, path: &Path) -> String {
        if let Ok(file) = fs::File::open(path) {
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(data) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if string_at(&data, &["type"]) == "session" {
                    let id = string_at(&data, &["id"]);
                    if !id.is_empty() {
                        return id;
                    }
                }
                break;
            }
        }
        pi_session_id_from_path(path)
    }

    fn parse_session(&self, path: &Path) -> Option<Session> {
        let file = fs::File::open(path).ok()?;
        let mut session_id = String::new();
        let mut directory = String::new();
        let mut session_name: Option<String> = None;
        let mut first_user_message = String::new();
        let mut messages = Vec::new();
        let mut message_count = 0usize;
        let mut header_timestamp: Option<DateTime<Local>> = None;
        let mut last_activity: Option<DateTime<Local>> = None;

        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(data) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            match string_at(&data, &["type"]).as_str() {
                "session" => {
                    if session_id.is_empty() {
                        session_id = string_at(&data, &["id"]);
                    }
                    if directory.is_empty() {
                        directory = string_at(&data, &["cwd"]);
                    }
                    if header_timestamp.is_none() {
                        header_timestamp = parse_datetime(&string_at(&data, &["timestamp"]));
                    }
                }
                "session_info" => {
                    let name = string_at(&data, &["name"]);
                    session_name = (!name.trim().is_empty()).then(|| name.trim().to_string());
                }
                "message" => {
                    let message = data.get("message").unwrap_or(&Value::Null);
                    let role = string_at(message, &["role"]);
                    let is_user = role == "user";
                    let is_assistant = role == "assistant";
                    let is_visible_custom = matches!(role.as_str(), "custom" | "hookMessage")
                        && message.get("display").and_then(Value::as_bool) == Some(true);
                    if !is_user && !is_assistant && !is_visible_custom {
                        continue;
                    }
                    if is_user {
                        message_count += 1;
                    }
                    if is_user || is_assistant {
                        update_latest_timestamp(
                            &mut last_activity,
                            message_timestamp(message)
                                .or_else(|| parse_datetime(&string_at(&data, &["timestamp"]))),
                        );
                    }

                    let role_prefix = if is_user { "» " } else { "  " };
                    for text in pi_content_texts(message.get("content").unwrap_or(&Value::Null)) {
                        if is_user && first_user_message.is_empty() {
                            first_user_message = text.clone();
                        }
                        messages.push(format!("{role_prefix}{text}"));
                    }
                }
                "custom_message" if data.get("display").and_then(Value::as_bool) == Some(true) => {
                    for text in pi_content_texts(data.get("content").unwrap_or(&Value::Null)) {
                        messages.push(format!("  {text}"));
                    }
                }
                "compaction" | "branch_summary" => {
                    let summary = string_at(&data, &["summary"]);
                    if !summary.trim().is_empty() {
                        messages.push(format!("  {summary}"));
                    }
                }
                _ => {}
            }
        }

        if session_id.is_empty() {
            session_id = pi_session_id_from_path(path);
        }
        if first_user_message.is_empty() && messages.is_empty() {
            return None;
        }

        let named = session_name.is_some();
        let title_source = session_name.unwrap_or_else(|| {
            if first_user_message.is_empty() {
                "(no messages)".to_string()
            } else {
                first_user_message
            }
        });
        let mut session = Session::new(
            session_id,
            self.name(),
            truncate_title(&title_source, 100, true),
            directory,
            last_activity
                .or(header_timestamp)
                .unwrap_or_else(|| file_timestamp(path)),
            messages.join("\n\n"),
            message_count,
        );
        session.mtime = file_mtime_seconds(path);
        session.named = named;
        Some(session)
    }

    fn parse_session_incremental(&self, path: &Path) -> IncrementalParse {
        incremental_parse_jsonl(path, || self.parse_session(path))
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
        WalkDir::new(&self.sessions_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
            .filter_map(|entry| self.parse_session(entry.path()))
            .collect()
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        let Some((current_files, complete)) = self.scan_session_files() else {
            return failed_incremental_scan(self.name());
        };
        let mut scan = incremental_from_files(self.name(), known, current_files, |path| {
            self.parse_session_incremental(path)
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
        let Some((current_files, complete)) = self.scan_session_files() else {
            return failed_incremental_scan(self.name());
        };
        let mut scan = incremental_from_files_streaming(
            self.name(),
            known,
            current_files,
            |path| self.parse_session_incremental(path),
            on_session,
        );
        if !complete {
            scan.deleted_ids.clear();
        }
        scan
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

fn pi_content_texts(content: &Value) -> Vec<String> {
    match content {
        Value::String(text) if !text.is_empty() => vec![text.clone()],
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                if string_at(part, &["type"]) == "text" {
                    let text = string_at(part, &["text"]);
                    (!text.is_empty()).then_some(text)
                } else {
                    None
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn message_timestamp(message: &Value) -> Option<DateTime<Local>> {
    let timestamp = message.get("timestamp")?;
    if let Some(ms) = timestamp.as_i64() {
        return Local.timestamp_millis_opt(ms).single();
    }
    timestamp
        .as_f64()
        .and_then(|ms| Local.timestamp_millis_opt(ms as i64).single())
}

fn update_latest_timestamp(
    target: &mut Option<DateTime<Local>>,
    candidate: Option<DateTime<Local>>,
) {
    if let Some(candidate) = candidate
        && target.is_none_or(|current| candidate > current)
    {
        *target = Some(candidate);
    }
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
