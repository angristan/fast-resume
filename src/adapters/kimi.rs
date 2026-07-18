use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;
use walkdir::WalkDir;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    IncrementalParse, content_texts, failed_incremental_scan, incremental_from_files,
    incremental_from_files_streaming, incremental_parse_from_option,
    incremental_parse_jsonl_with_partial_check, json_file_has_parse_errors, raw_stats_for_tree,
    string_at, timestamp_from_ms, value_i64_at,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

type SessionFiles = HashMap<String, (PathBuf, f64)>;

#[derive(Debug, Clone)]
pub struct KimiAdapter {
    sessions_dir: PathBuf,
}

impl Default for KimiAdapter {
    fn default() -> Self {
        Self {
            sessions_dir: config::kimi_sessions_dir(),
        }
    }
}

impl KimiAdapter {
    #[allow(dead_code)]
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir }
    }

    fn session_index_file(&self) -> PathBuf {
        self.sessions_dir
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("session_index.jsonl")
    }

    fn read_session_index(&self) -> Option<HashMap<String, String>> {
        let index_file = self.session_index_file();
        if !index_file.exists() {
            return Some(HashMap::new());
        }
        let file = fs::File::open(index_file).ok()?;
        let mut work_dirs = HashMap::new();
        for line in BufReader::new(file).lines() {
            let line = line.ok()?;
            if line.trim().is_empty() {
                continue;
            }
            let Ok(entry) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let session_id = string_at(&entry, &["sessionId"]);
            if session_id.is_empty() {
                continue;
            }
            if entry.get("deleted").and_then(Value::as_bool) == Some(true) {
                work_dirs.remove(&session_id);
                continue;
            }
            let work_dir = string_at(&entry, &["workDir"]);
            if !work_dir.trim().is_empty() {
                work_dirs.insert(session_id, work_dir);
            }
        }
        Some(work_dirs)
    }

    fn scan_session_files(&self, index_mtime: f64) -> Option<(SessionFiles, bool)> {
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
            let state_file = entry.path();
            if state_file.file_name().and_then(|name| name.to_str()) != Some("state.json") {
                continue;
            }
            if json_file_has_parse_errors(state_file) {
                complete = false;
                continue;
            }
            let Some(session_id) = kimi_session_id_from_state_file(state_file) else {
                complete = false;
                continue;
            };
            if current_files.get(&session_id).is_some_and(|(existing, _)| {
                !kimi_state_file_is_legacy(existing) && kimi_state_file_is_legacy(state_file)
            }) {
                continue;
            }
            current_files.insert(
                session_id,
                (
                    state_file.to_path_buf(),
                    kimi_session_mtime(state_file, index_mtime),
                ),
            );
        }

        Some((current_files, complete))
    }

    fn parse_session(
        &self,
        state_file: &Path,
        work_dirs: &HashMap<String, String>,
        index_mtime: f64,
    ) -> Option<Session> {
        let state: Value = serde_json::from_slice(&fs::read(state_file).ok()?).ok()?;
        let session_id = kimi_session_id_from_state_file(state_file)
            .unwrap_or_else(|| string_at(&state, &["id"]));
        if session_id.is_empty() {
            return None;
        }

        let directory = non_empty_string(&state, "cwd")
            .or_else(|| non_empty_string(&state, "workDir"))
            .or_else(|| {
                let custom = state.get("custom").unwrap_or(&Value::Null);
                non_empty_string(custom, "cwd")
            })
            .or_else(|| {
                work_dirs
                    .get(&session_id)
                    .filter(|value| !value.trim().is_empty())
                    .cloned()
            })
            .unwrap_or_default();
        let mut messages = Vec::new();
        let mut first_user_message = String::new();
        let mut message_count = 0usize;
        let wire_file = kimi_wire_file(state_file);
        if wire_file.exists() {
            parse_wire_messages(
                &wire_file,
                &mut messages,
                &mut first_user_message,
                &mut message_count,
            );
        }

        let last_prompt = string_at(&state, &["lastPrompt"]);
        if message_count == 0 && !last_prompt.trim().is_empty() {
            add_message(
                &mut messages,
                &mut first_user_message,
                &mut message_count,
                "user",
                &Value::String(last_prompt.clone()),
                true,
            );
        }
        if message_count == 0 {
            return None;
        }

        let title = kimi_state_title(&state)
            .or_else(|| (!last_prompt.trim().is_empty()).then_some(last_prompt))
            .or_else(|| (!first_user_message.is_empty()).then_some(first_user_message))
            .unwrap_or_else(|| "Kimi session".to_string());
        let timestamp = state_timestamp(&state, "updatedAt")
            .or_else(|| state_timestamp(&state, "createdAt"))
            .unwrap_or_else(|| file_timestamp(state_file));

        let mut session = Session::new(
            session_id,
            self.name(),
            truncate_title(&title, 100, true),
            directory,
            timestamp,
            messages.join("\n\n"),
            message_count,
        );
        session.mtime = kimi_session_mtime(state_file, index_mtime);
        Some(session)
    }

    fn parse_session_incremental(
        &self,
        state_file: &Path,
        work_dirs: &HashMap<String, String>,
        index_mtime: f64,
    ) -> IncrementalParse {
        if json_file_has_parse_errors(state_file) {
            return IncrementalParse::Retain;
        }
        let wire_file = kimi_wire_file(state_file);
        if wire_file.exists() {
            incremental_parse_jsonl_with_partial_check(
                &wire_file,
                || self.parse_session(state_file, work_dirs, index_mtime),
                |session| session.message_count > 0,
            )
        } else {
            incremental_parse_from_option(self.parse_session(state_file, work_dirs, index_mtime))
        }
    }
}

impl Adapter for KimiAdapter {
    fn name(&self) -> &'static str {
        "kimi"
    }

    fn supports_yolo(&self) -> bool {
        true
    }

    fn find_sessions(&self) -> Vec<Session> {
        if !self.sessions_dir.exists() {
            return Vec::new();
        }
        let index_mtime = file_mtime_seconds(&self.session_index_file());
        let work_dirs = self.read_session_index().unwrap_or_default();
        let Some((current_files, _)) = self.scan_session_files(index_mtime) else {
            return Vec::new();
        };
        current_files
            .into_values()
            .filter_map(|(state_file, _)| self.parse_session(&state_file, &work_dirs, index_mtime))
            .collect()
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        let index_mtime = file_mtime_seconds(&self.session_index_file());
        let Some(work_dirs) = self.read_session_index() else {
            return failed_incremental_scan(self.name());
        };
        let Some((current_files, complete)) = self.scan_session_files(index_mtime) else {
            return failed_incremental_scan(self.name());
        };
        let mut scan = incremental_from_files(self.name(), known, current_files, |path| {
            self.parse_session_incremental(path, &work_dirs, index_mtime)
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
        let index_mtime = file_mtime_seconds(&self.session_index_file());
        let Some(work_dirs) = self.read_session_index() else {
            return failed_incremental_scan(self.name());
        };
        let Some((current_files, complete)) = self.scan_session_files(index_mtime) else {
            return failed_incremental_scan(self.name());
        };
        let mut scan = incremental_from_files_streaming(
            self.name(),
            known,
            current_files,
            |path| self.parse_session_incremental(path, &work_dirs, index_mtime),
            on_session,
        );
        if !complete {
            scan.deleted_ids.clear();
        }
        scan
    }

    fn resume_command(&self, session: &Session, yolo: bool) -> Vec<String> {
        let mut command = vec!["kimi".to_string()];
        if yolo {
            command.push("--yolo".to_string());
        }
        command.extend(["--session".to_string(), session.id.clone()]);
        command
    }

    fn raw_stats(&self) -> RawAdapterStats {
        raw_stats_for_tree(self.name(), &self.sessions_dir, "jsonl")
    }
}

fn kimi_state_file_is_legacy(state_file: &Path) -> bool {
    state_file
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        == Some("session-meta")
}

fn kimi_session_dir_from_state_file(state_file: &Path) -> Option<&Path> {
    let metadata_dir = state_file.parent()?;
    if kimi_state_file_is_legacy(state_file) {
        metadata_dir.parent()
    } else {
        Some(metadata_dir)
    }
}

fn kimi_session_id_from_state_file(state_file: &Path) -> Option<String> {
    kimi_session_dir_from_state_file(state_file)?
        .file_name()?
        .to_str()
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn kimi_wire_file(state_file: &Path) -> PathBuf {
    kimi_session_dir_from_state_file(state_file)
        .unwrap_or_else(|| Path::new(""))
        .join("agents")
        .join("main")
        .join("wire.jsonl")
}

fn kimi_session_mtime(state_file: &Path, index_mtime: f64) -> f64 {
    file_mtime_seconds(state_file)
        .max(file_mtime_seconds(&kimi_wire_file(state_file)))
        .max(index_mtime)
}

fn non_empty_string(value: &Value, key: &str) -> Option<String> {
    let value = string_at(value, &[key]);
    (!value.trim().is_empty()).then_some(value)
}

fn kimi_state_title(state: &Value) -> Option<String> {
    // Modern states use `isCustomTitle` as the title-schema discriminator.
    if state.get("isCustomTitle").is_some_and(Value::is_boolean) {
        return non_empty_string(state, "title");
    }
    non_empty_string(state, "customTitle").or_else(|| non_empty_string(state, "title"))
}

fn state_timestamp(state: &Value, key: &str) -> Option<chrono::DateTime<chrono::Local>> {
    timestamp_from_ms(value_i64_at(state, &[key])).or_else(|| {
        non_empty_string(state, key).and_then(|value| super::shared::parse_datetime(&value))
    })
}

fn parse_wire_messages(
    wire_file: &Path,
    messages: &mut Vec<String>,
    first_user_message: &mut String,
    message_count: &mut usize,
) {
    let Ok(file) = fs::File::open(wire_file) else {
        return;
    };
    let mut open_assistant: Option<Vec<String>> = None;

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match string_at(&record, &["type"]).as_str() {
            "context.append_message" => {
                let message = record.get("message").unwrap_or(&Value::Null);
                if !kimi_message_is_searchable(message) {
                    continue;
                }
                add_message(
                    messages,
                    first_user_message,
                    message_count,
                    &string_at(message, &["role"]),
                    message.get("content").unwrap_or(&Value::Null),
                    kimi_message_is_user_turn(message),
                );
            }
            "context.append_loop_event" => {
                let event = record.get("event").unwrap_or(&Value::Null);
                match string_at(event, &["type"]).as_str() {
                    "step.begin" => {
                        // Kimi settles a failed partial attempt before opening its retry.
                        flush_open_assistant(messages, &mut open_assistant);
                        open_assistant = Some(Vec::new());
                    }
                    "content.part" => {
                        if let Some(text) =
                            kimi_text_part(event.get("part").unwrap_or(&Value::Null))
                            && let Some(parts) = open_assistant.as_mut()
                        {
                            parts.push(text);
                        }
                    }
                    "step.end" => flush_open_assistant(messages, &mut open_assistant),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // A session can be indexed while the last streamed step is still open.
    flush_open_assistant(messages, &mut open_assistant);
}

fn flush_open_assistant(messages: &mut Vec<String>, open_assistant: &mut Option<Vec<String>>) {
    if let Some(parts) = open_assistant.take() {
        add_assistant_parts(messages, parts);
    }
}

fn kimi_message_is_searchable(message: &Value) -> bool {
    let kind = string_at(message, &["origin", "kind"]);
    kind != "injection"
        && !(kind == "shell_command" && string_at(message, &["origin", "phase"]) == "output")
}

fn kimi_message_is_user_turn(message: &Value) -> bool {
    if string_at(message, &["role"]) != "user" {
        return false;
    }
    match string_at(message, &["origin", "kind"]).as_str() {
        "" | "user" => true,
        "skill_activation" | "plugin_command" => {
            string_at(message, &["origin", "trigger"]) == "user-slash"
        }
        "shell_command" => string_at(message, &["origin", "phase"]) == "input",
        _ => false,
    }
}

fn kimi_text_part(part: &Value) -> Option<String> {
    (string_at(part, &["type"]) == "text")
        .then(|| string_at(part, &["text"]))
        .filter(|text| !text.is_empty())
}

fn add_message(
    messages: &mut Vec<String>,
    first_user_message: &mut String,
    message_count: &mut usize,
    role: &str,
    content: &Value,
    count_as_user_turn: bool,
) {
    if !matches!(role, "user" | "assistant") {
        return;
    }
    let texts = content_texts(content);
    if role == "user" && count_as_user_turn && !texts.is_empty() {
        *message_count += 1;
        if first_user_message.is_empty() {
            *first_user_message = texts[0].clone();
        }
    }
    let prefix = if role == "user" { "» " } else { "  " };
    for text in texts {
        messages.push(format!("{prefix}{text}"));
    }
}

fn add_assistant_parts(messages: &mut Vec<String>, parts: Vec<String>) {
    let text = parts.join("");
    if !text.is_empty() {
        messages.push(format!("  {text}"));
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::thread;
    use std::time::Duration;

    use serde_json::{Value, json};
    use tempfile::tempdir;

    use crate::adapters::{Adapter, KnownSessions};

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

    fn write_session_index(
        sessions_dir: &Path,
        session_id: &str,
        session_dir: &Path,
        work_dir: &str,
    ) {
        write_jsonl(
            &sessions_dir.parent().unwrap().join("session_index.jsonl"),
            &[json!({
                "sessionId": session_id,
                "sessionDir": session_dir.to_string_lossy(),
                "workDir": work_dir
            })],
        );
    }

    fn write_kimi_session(sessions_dir: &Path, id: &str) -> PathBuf {
        let session_dir = sessions_dir.join("--repo-kimi--").join(id);
        let wire_dir = session_dir.join("agents/main");
        fs::create_dir_all(&wire_dir).unwrap();
        fs::write(
            session_dir.join("state.json"),
            json!({
                "id": "outdated-state-id",
                "title": "Named Kimi session",
                "createdAt": 1784110800000i64,
                "updatedAt": 1784110807000i64,
                "archived": false
            })
            .to_string(),
        )
        .unwrap();
        write_jsonl(
            &wire_dir.join("wire.jsonl"),
            &[
                json!({"type": "metadata", "protocol_version": "1.4", "created_at": 1784110800000i64}),
                json!({"type": "context.append_message", "time": 1784110800500i64, "message": {"role": "user", "content": [{"type": "text", "text": "hidden system reminder"}], "origin": {"kind": "injection"}}}),
                json!({"type": "context.append_message", "time": 1784110801000i64, "message": {"role": "user", "content": [{"type": "text", "text": "Implement the Kimi adapter"}], "origin": {"kind": "user"}}}),
                json!({"type": "context.append_loop_event", "time": 1784110802000i64, "event": {"type": "step.begin", "uuid": "step-1", "turnId": "turn-1", "step": 0}}),
                json!({"type": "context.append_loop_event", "time": 1784110803000i64, "event": {"type": "content.part", "stepUuid": "step-1", "part": {"type": "text", "text": "Added "}}}),
                json!({"type": "context.append_loop_event", "time": 1784110804000i64, "event": {"type": "content.part", "stepUuid": "step-1", "part": {"type": "text", "text": "support"}}}),
                json!({"type": "context.append_loop_event", "time": 1784110805000i64, "event": {"type": "tool.call", "stepUuid": "step-1", "toolCallId": "call-1", "name": "Read", "args": {"path": "secret"}}}),
                json!({"type": "context.append_loop_event", "time": 1784110806000i64, "event": {"type": "step.end", "uuid": "step-1", "turnId": "turn-1", "step": 0}}),
            ],
        );
        write_session_index(sessions_dir, id, &session_dir, "/repo/kimi");
        session_dir
    }

    #[test]
    fn parses_kimi_session_state_and_wire_messages() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_id = "kimi-123";
        write_kimi_session(&sessions_dir, session_id);

        let adapter = KimiAdapter::new(sessions_dir);
        let sessions = adapter.find_sessions();

        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.id, session_id);
        assert_eq!(session.agent, "kimi");
        assert_eq!(session.title, "Named Kimi session");
        assert_eq!(session.directory, "/repo/kimi");
        assert_eq!(session.message_count, 1);
        assert_eq!(session.timestamp.timestamp_millis(), 1784110807000i64);
        assert!(session.content.contains("» Implement the Kimi adapter"));
        assert!(session.content.contains("  Added support"));
        assert!(!session.content.contains("hidden system reminder"));
        assert!(!session.content.contains("secret"));
        assert!(adapter.supports_yolo());
        assert_eq!(
            adapter.resume_command(session, false),
            vec!["kimi", "--session", session_id]
        );
        assert_eq!(
            adapter.resume_command(session, true),
            vec!["kimi", "--yolo", "--session", session_id]
        );
    }

    #[test]
    fn preserves_failed_attempt_before_successful_retry() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_dir = write_kimi_session(&sessions_dir, "kimi-123");
        write_jsonl(
            &session_dir.join("agents/main/wire.jsonl"),
            &[
                json!({"type": "context.append_message", "message": {"role": "user", "content": "Retry this request", "origin": {"kind": "user"}}}),
                json!({"type": "context.append_loop_event", "event": {"type": "step.begin", "uuid": "failed-step"}}),
                json!({"type": "context.append_loop_event", "event": {"type": "content.part", "stepUuid": "failed-step", "part": {"type": "text", "text": "Partial failed response"}}}),
                json!({"type": "context.append_loop_event", "event": {"type": "step.begin", "uuid": "retry-step"}}),
                json!({"type": "context.append_loop_event", "event": {"type": "content.part", "stepUuid": "retry-step", "part": {"type": "text", "text": "Successful retry response"}}}),
                json!({"type": "context.append_loop_event", "event": {"type": "step.end", "uuid": "retry-step"}}),
            ],
        );

        let sessions = KimiAdapter::new(sessions_dir).find_sessions();
        let content = &sessions[0].content;

        let partial = content.find("Partial failed response").unwrap();
        let successful = content.find("Successful retry response").unwrap();
        assert!(
            partial < successful,
            "failed attempt must precede its retry"
        );
    }

    #[test]
    fn parses_legacy_nested_state_path() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_id = "kimi-legacy-v2";
        let session_dir = sessions_dir.join("--repo-kimi--").join(session_id);
        let wire_dir = session_dir.join("agents/main");
        fs::create_dir_all(&wire_dir).unwrap();
        fs::create_dir_all(session_dir.join("session-meta")).unwrap();
        fs::write(
            session_dir.join("session-meta/state.json"),
            json!({
                "id": session_id,
                "title": "Legacy nested metadata",
                "cwd": "/repo/legacy-v2",
                "createdAt": 1784110800000i64,
                "updatedAt": 1784110801000i64,
                "archived": false
            })
            .to_string(),
        )
        .unwrap();
        write_jsonl(
            &wire_dir.join("wire.jsonl"),
            &[
                json!({"type": "metadata", "protocol_version": "1.4", "created_at": 1784110800000i64}),
                json!({"type": "context.append_message", "time": 1784110801000i64, "message": {"role": "user", "content": "Nested state prompt", "origin": {"kind": "user"}}}),
            ],
        );

        let sessions = KimiAdapter::new(sessions_dir).find_sessions();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, session_id);
        assert_eq!(sessions[0].directory, "/repo/legacy-v2");
        assert!(sessions[0].content.contains("Nested state prompt"));
    }

    #[test]
    fn state_directory_takes_precedence_over_stale_index() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_dir = write_kimi_session(&sessions_dir, "kimi-123");
        fs::write(
            session_dir.join("state.json"),
            json!({
                "title": "State directory wins",
                "cwd": "/repo/from-state",
                "createdAt": 1784110800000i64,
                "updatedAt": 1784110807000i64,
                "archived": false
            })
            .to_string(),
        )
        .unwrap();
        write_session_index(&sessions_dir, "kimi-123", &session_dir, "/repo/stale-index");

        let sessions = KimiAdapter::new(sessions_dir).find_sessions();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].directory, "/repo/from-state");
    }

    #[test]
    fn falls_back_to_legacy_state_fields_and_last_prompt() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_dir = sessions_dir.join("--repo-kimi--").join("kimi-legacy");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("state.json"),
            json!({
                "workDir": "/repo/legacy",
                "title": "Generated legacy title",
                "customTitle": "Pinned legacy title",
                "lastPrompt": "Resume a legacy Kimi session",
                "createdAt": "2026-07-15T10:00:00Z"
            })
            .to_string(),
        )
        .unwrap();

        let sessions = KimiAdapter::new(sessions_dir).find_sessions();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "kimi-legacy");
        assert_eq!(sessions[0].title, "Pinned legacy title");
        assert_eq!(sessions[0].directory, "/repo/legacy");
        assert_eq!(sessions[0].message_count, 1);
        assert!(sessions[0].content.contains("Resume a legacy Kimi session"));
    }

    #[test]
    fn modern_title_ignores_stale_legacy_custom_title() {
        let state = json!({
            "title": "Modern title",
            "isCustomTitle": false,
            "customTitle": "Stale legacy title"
        });

        assert_eq!(kimi_state_title(&state).as_deref(), Some("Modern title"));
    }

    #[test]
    fn malformed_index_rows_do_not_hide_valid_sessions() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        write_kimi_session(&sessions_dir, "kimi-123");
        let index_file = temp.path().join("session_index.jsonl");
        let valid_index = fs::read_to_string(&index_file).unwrap();
        fs::write(index_file, format!("{{\n{valid_index}")).unwrap();

        let sessions = KimiAdapter::new(sessions_dir).find_sessions();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].directory, "/repo/kimi");
    }

    #[test]
    fn ignores_injected_messages_for_fallback_title() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_dir = write_kimi_session(&sessions_dir, "kimi-123");
        fs::write(
            session_dir.join("state.json"),
            json!({
                "createdAt": 1784110800000i64,
                "updatedAt": 1784110807000i64,
                "archived": false
            })
            .to_string(),
        )
        .unwrap();

        let sessions = KimiAdapter::new(sessions_dir).find_sessions();

        assert_eq!(sessions[0].title, "Implement the Kimi adapter");
        assert_eq!(sessions[0].message_count, 1);
        assert!(!sessions[0].content.contains("hidden system reminder"));
    }

    #[test]
    fn classifies_user_turn_origins_and_omits_shell_output() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_dir = write_kimi_session(&sessions_dir, "kimi-123");
        fs::write(
            session_dir.join("state.json"),
            json!({
                "createdAt": 1784110800000i64,
                "updatedAt": 1784110807000i64,
                "archived": false
            })
            .to_string(),
        )
        .unwrap();
        write_jsonl(
            &session_dir.join("agents/main/wire.jsonl"),
            &[
                json!({"type": "context.append_message", "message": {"role": "user", "content": "Background task completed", "origin": {"kind": "task"}}}),
                json!({"type": "context.append_message", "message": {"role": "user", "content": "Sensitive shell output", "origin": {"kind": "shell_command", "phase": "output"}}}),
                json!({"type": "context.append_message", "message": {"role": "user", "content": "Hook context", "origin": {"kind": "hook_result"}}}),
                json!({"type": "context.append_message", "message": {"role": "user", "content": "!git status", "origin": {"kind": "shell_command", "phase": "input"}}}),
            ],
        );

        let sessions = KimiAdapter::new(sessions_dir).find_sessions();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "!git status");
        assert_eq!(sessions[0].message_count, 1);
        assert!(sessions[0].content.contains("Background task completed"));
        assert!(sessions[0].content.contains("Hook context"));
        assert!(sessions[0].content.contains("!git status"));
        assert!(!sessions[0].content.contains("Sensitive shell output"));
    }

    #[test]
    fn excludes_empty_sessions_and_recovers_last_prompt() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");

        let state_only = sessions_dir.join("--repo-kimi--").join("state-only");
        fs::create_dir_all(&state_only).unwrap();
        fs::write(
            state_only.join("state.json"),
            json!({"title": "Unused empty session", "createdAt": 1784110800000i64}).to_string(),
        )
        .unwrap();

        let internal_only = sessions_dir.join("--repo-kimi--").join("internal-only");
        fs::create_dir_all(internal_only.join("agents/main")).unwrap();
        fs::write(
            internal_only.join("state.json"),
            json!({"title": "Internal events only", "createdAt": 1784110800000i64}).to_string(),
        )
        .unwrap();
        write_jsonl(
            &internal_only.join("agents/main/wire.jsonl"),
            &[
                json!({"type": "context.append_message", "message": {"role": "user", "content": "Task notification", "origin": {"kind": "task"}}}),
            ],
        );

        let recovered = sessions_dir.join("--repo-kimi--").join("recovered");
        fs::create_dir_all(recovered.join("agents/main")).unwrap();
        fs::write(
            recovered.join("state.json"),
            json!({
                "lastPrompt": "Recovered user prompt",
                "createdAt": 1784110800000i64
            })
            .to_string(),
        )
        .unwrap();
        write_jsonl(
            &recovered.join("agents/main/wire.jsonl"),
            &[
                json!({"type": "context.append_message", "message": {"role": "user", "content": "Task notification", "origin": {"kind": "task"}}}),
            ],
        );

        let sessions = KimiAdapter::new(sessions_dir).find_sessions();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "recovered");
        assert_eq!(sessions[0].title, "Recovered user prompt");
        assert_eq!(sessions[0].message_count, 1);
        assert!(sessions[0].content.contains("Recovered user prompt"));
    }

    #[test]
    fn incremental_refresh_uses_session_index_mtime() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_dir = write_kimi_session(&sessions_dir, "kimi-123");
        let adapter = KimiAdapter::new(sessions_dir.clone());
        let initial = adapter.find_sessions().remove(0);
        thread::sleep(Duration::from_millis(20));
        write_session_index(&sessions_dir, "kimi-123", &session_dir, "/repo/moved");
        let mut known = KnownSessions::new();
        known.insert(("kimi".to_string(), "kimi-123".to_string()), initial.mtime);

        let scan = adapter.find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].directory, "/repo/moved");
        assert!(scan.new_or_modified[0].mtime > initial.mtime);
    }

    #[test]
    fn incremental_refresh_uses_wire_mtime_and_detects_deletions() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_dir = write_kimi_session(&sessions_dir, "kimi-123");
        let adapter = KimiAdapter::new(sessions_dir);
        let initial_mtime = adapter.find_sessions()[0].mtime;
        thread::sleep(Duration::from_millis(20));
        let wire_file = session_dir.join("agents/main/wire.jsonl");
        fs::write(
            &wire_file,
            json!({"type": "context.append_message", "message": {"role": "user", "content": "New Kimi prompt"}}).to_string(),
        )
        .unwrap();

        let mut known = KnownSessions::new();
        known.insert(("kimi".to_string(), "kimi-123".to_string()), initial_mtime);
        let scan = adapter.find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert!(scan.new_or_modified[0].mtime > initial_mtime);
        assert!(scan.new_or_modified[0].content.contains("New Kimi prompt"));

        fs::remove_dir_all(session_dir).unwrap();
        let scan = adapter.find_sessions_incremental(&known);
        assert_eq!(scan.deleted_ids, vec!["kimi-123"]);
    }
}
