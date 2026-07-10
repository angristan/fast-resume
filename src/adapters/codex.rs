use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;
use walkdir::WalkDir;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    codex_session_id_from_path, content_texts, deleted_ids_for_agent, failed_incremental_scan,
    fallback_session_id, jsonl_has_parse_errors, parse_timestamp_seconds, raw_stats_for_tree,
    session_needs_update, string_at,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

#[derive(Debug, Clone)]
pub struct CodexAdapter {
    sessions_dir: PathBuf,
    session_index_file: PathBuf,
}

impl Default for CodexAdapter {
    fn default() -> Self {
        Self {
            sessions_dir: config::codex_dir(),
            session_index_file: config::codex_session_index_file(),
        }
    }
}

impl CodexAdapter {
    #[allow(dead_code)]
    pub fn new(sessions_dir: PathBuf, session_index_file: PathBuf) -> Self {
        Self {
            sessions_dir,
            session_index_file,
        }
    }

    fn parse_session(
        &self,
        path: &Path,
        thread_names: &HashMap<String, String>,
    ) -> Option<Session> {
        let file = fs::File::open(path).ok()?;
        let mut session_id = codex_session_id_from_path(path).unwrap_or_default();
        let mut directory = String::new();
        let mut messages = Vec::new();
        let mut user_prompts = Vec::new();
        let mut turns = 0usize;
        let mut yolo = false;

        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(data) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let msg_type = string_at(&data, &["type"]);
            let payload = data.get("payload").unwrap_or(&Value::Null);

            match msg_type.as_str() {
                "session_meta" => {
                    if session_id.is_empty() {
                        session_id = string_at(payload, &["id"]);
                    }
                    if directory.is_empty() {
                        directory = string_at(payload, &["cwd"]);
                    }
                }
                "turn_context" => {
                    let approval = string_at(payload, &["approval_policy"]);
                    let sandbox_mode = payload
                        .pointer("/sandbox_policy/mode")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if approval == "never" || sandbox_mode == "danger-full-access" {
                        yolo = true;
                    }
                }
                "response_item" => {
                    let role = string_at(payload, &["role"]);
                    if role == "user" || role == "assistant" {
                        let role_prefix = if role == "user" { "» " } else { "  " };
                        if let Some(content) = payload.get("content") {
                            for text in content_texts(content) {
                                if !text.trim_start().starts_with("<environment_context>") {
                                    messages.push(format!("{role_prefix}{text}"));
                                }
                            }
                        }
                    }
                }
                "event_msg" => match string_at(payload, &["type"]).as_str() {
                    "user_message" => {
                        let message = string_at(payload, &["message"]);
                        if !message.is_empty() {
                            messages.push(format!("» {message}"));
                            user_prompts.push(message);
                            turns += 1;
                        }
                    }
                    "agent_reasoning" => {
                        let text = string_at(payload, &["text"]);
                        if !text.is_empty() {
                            messages.push(format!("  {text}"));
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        if session_id.is_empty() {
            session_id = fallback_session_id(path);
        }
        if user_prompts.is_empty() {
            return None;
        }

        let title_source = thread_names
            .get(&session_id)
            .cloned()
            .unwrap_or_else(|| user_prompts[0].clone());
        let mut session = Session::new(
            session_id,
            self.name(),
            truncate_title(&title_source, 80, false),
            directory,
            file_timestamp(path),
            messages.join("\n\n"),
            turns,
        );
        session.mtime = file_mtime_seconds(path);
        session.yolo = yolo;
        Some(session)
    }

    fn load_thread_index(&self) -> HashMap<String, (String, f64)> {
        let index_mtime = file_mtime_seconds(&self.session_index_file);
        let Ok(file) = fs::File::open(&self.session_index_file) else {
            return HashMap::new();
        };
        let mut out = HashMap::new();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(data) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let id = string_at(&data, &["id"]);
            let thread_name = string_at(&data, &["thread_name"]);
            if !id.is_empty() && !thread_name.trim().is_empty() {
                let updated_at = string_at(&data, &["updated_at"]);
                let mtime = parse_timestamp_seconds(&updated_at).unwrap_or(index_mtime);
                out.insert(id, (thread_name.trim().to_string(), mtime));
            }
        }
        out
    }

    fn load_thread_names(&self) -> HashMap<String, String> {
        self.load_thread_index()
            .into_iter()
            .map(|(id, (thread_name, _))| (id, thread_name))
            .collect()
    }

    fn session_id_from_file(&self, path: &Path) -> String {
        if let Some(session_id) = codex_session_id_from_path(path) {
            return session_id;
        }
        if let Ok(file) = fs::File::open(path) {
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(data) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if string_at(&data, &["type"]) == "session_meta" {
                    let id = string_at(data.get("payload").unwrap_or(&Value::Null), &["id"]);
                    if !id.is_empty() {
                        return id;
                    }
                    break;
                }
            }
        }
        fallback_session_id(path)
    }

    fn scan_session_files(&self) -> Option<(HashMap<String, (PathBuf, f64)>, bool)> {
        let mut current_files = HashMap::new();
        let mut complete = true;
        if !self.sessions_dir.exists() {
            return Some((current_files, complete));
        }
        if !self.sessions_dir.is_dir() {
            return None;
        }

        let thread_index = self.load_thread_index();
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
            let mut mtime = file_mtime_seconds(path);
            if let Some((_, index_mtime)) = thread_index.get(&session_id) {
                mtime = mtime.max(*index_mtime);
            }
            current_files.insert(session_id, (path.to_path_buf(), mtime));
        }
        Some((current_files, complete))
    }
}

impl Adapter for CodexAdapter {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn supports_yolo(&self) -> bool {
        true
    }

    fn find_sessions(&self) -> Vec<Session> {
        if !self.sessions_dir.exists() {
            return Vec::new();
        }
        let thread_names = self.load_thread_names();
        WalkDir::new(&self.sessions_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
            .filter_map(|entry| self.parse_session(entry.path(), &thread_names))
            .collect()
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        let thread_names = self.load_thread_names();
        let Some((current_files, complete)) = self.scan_session_files() else {
            return failed_incremental_scan(self.name());
        };
        let mut current_ids = HashSet::new();
        let mut new_or_modified = Vec::new();

        for (session_id, (path, mtime)) in current_files {
            current_ids.insert(session_id.clone());
            if !session_needs_update(known, self.name(), &session_id, mtime) {
                continue;
            }

            if jsonl_has_parse_errors(&path) {
                continue;
            } else if let Some(mut session) = self.parse_session(&path, &thread_names) {
                session.mtime = mtime;
                new_or_modified.push(session);
            } else {
                current_ids.remove(&session_id);
            }
        }

        IncrementalScan {
            agent: self.name(),
            new_or_modified,
            deleted_ids: if complete {
                deleted_ids_for_agent(known, self.name(), &current_ids)
            } else {
                Vec::new()
            },
        }
    }

    fn find_sessions_incremental_streaming(
        &self,
        known: &KnownSessions,
        on_session: &mut SessionCallback<'_>,
    ) -> IncrementalScan {
        let thread_names = self.load_thread_names();
        let Some((current_files, complete)) = self.scan_session_files() else {
            return failed_incremental_scan(self.name());
        };
        let mut current_ids = HashSet::new();
        let mut new_or_modified = Vec::new();

        for (session_id, (path, mtime)) in current_files {
            current_ids.insert(session_id.clone());
            if !session_needs_update(known, self.name(), &session_id, mtime) {
                continue;
            }

            if jsonl_has_parse_errors(&path) {
                continue;
            } else if let Some(mut session) = self.parse_session(&path, &thread_names) {
                session.mtime = mtime;
                on_session(session.clone());
                new_or_modified.push(session);
            } else {
                current_ids.remove(&session_id);
            }
        }

        IncrementalScan {
            agent: self.name(),
            new_or_modified,
            deleted_ids: if complete {
                deleted_ids_for_agent(known, self.name(), &current_ids)
            } else {
                Vec::new()
            },
        }
    }

    fn resume_command(&self, session: &Session, yolo: bool) -> Vec<String> {
        let mut cmd = vec!["codex".to_string()];
        if yolo {
            cmd.push("--dangerously-bypass-approvals-and-sandbox".to_string());
        }
        cmd.extend(["resume".to_string(), session.id.clone()]);
        cmd
    }

    fn raw_stats(&self) -> RawAdapterStats {
        raw_stats_for_tree(self.name(), &self.sessions_dir, "jsonl")
    }
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
    fn uses_thread_name_and_detects_yolo() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(sessions_dir.join("2026/06/21")).unwrap();
        let session_file = sessions_dir.join("2026/06/21/rollout-2026-06-21T10-00-00-test.jsonl");
        write_jsonl(
            &session_file,
            &[
                json!({"type": "session_meta", "payload": {"id": "abc123", "cwd": "/work/zeno"}}),
                json!({
                    "type": "turn_context",
                    "payload": {
                        "approval_policy": "never",
                        "sandbox_policy": {"mode": "danger-full-access"}
                    }
                }),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "Original prompt"}}),
                json!({
                    "type": "response_item",
                    "payload": {
                        "role": "user",
                        "content": [{"text": "<environment_context>skip me</environment_context>"}]
                    }
                }),
                json!({"type": "response_item", "payload": {"role": "assistant", "content": [{"text": "Answer"}]}}),
            ],
        );
        let session_index = temp.path().join("session_index.jsonl");
        fs::write(
            &session_index,
            json!({"id": "abc123", "thread_name": "Renamed Codex thread"}).to_string(),
        )
        .unwrap();

        let adapter = CodexAdapter::new(sessions_dir, session_index);
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Renamed Codex thread");
        assert_eq!(sessions[0].message_count, 1);
        assert!(sessions[0].yolo);
        assert!(!sessions[0].content.contains("<environment_context>"));
        assert_eq!(
            adapter.resume_command(&sessions[0], true),
            vec![
                "codex",
                "--dangerously-bypass-approvals-and-sandbox",
                "resume",
                "abc123"
            ]
        );
    }

    #[test]
    fn turn_count_uses_human_user_messages() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(sessions_dir.join("2026/06/21")).unwrap();
        let session_file = sessions_dir.join("2026/06/21/rollout-test123.jsonl");
        write_jsonl(
            &session_file,
            &[
                json!({"type": "session_meta", "payload": {"id": "test123", "cwd": "/work/app"}}),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "First prompt"}}),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "Second prompt"}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Replay of first prompt"}]}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "First answer"}]}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Replay after compaction"}]}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Second answer"}]}}),
            ],
        );

        let adapter = CodexAdapter::new(sessions_dir, temp.path().join("session_index.jsonl"));
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].message_count, 2);
    }

    #[test]
    fn keeps_initial_session_meta_identity() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(sessions_dir.join("2026/06/21")).unwrap();
        let session_file = sessions_dir.join("2026/06/21/rollout-first-id.jsonl");
        write_jsonl(
            &session_file,
            &[
                json!({"type": "session_meta", "payload": {"id": "first-id", "cwd": "/work/first"}}),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "Original prompt"}}),
                json!({"type": "session_meta", "payload": {"id": "replayed-id", "cwd": "/work/replayed"}}),
                json!({"type": "response_item", "payload": {"role": "assistant", "content": [{"text": "Answer"}]}}),
            ],
        );

        let adapter = CodexAdapter::new(sessions_dir, temp.path().join("session_index.jsonl"));
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "first-id");
        assert_eq!(sessions[0].directory, "/work/first");

        let mut known = KnownSessions::new();
        known.insert(
            ("codex".to_string(), "first-id".to_string()),
            file_mtime_seconds(&session_file),
        );
        let scan = adapter.find_sessions_incremental(&known);
        assert_eq!(scan.new_or_modified.len(), 0);
        assert_eq!(scan.deleted_ids.len(), 0);
    }

    #[test]
    fn incremental_uses_session_index_mtime() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(sessions_dir.join("2026/06/21")).unwrap();
        let session_file = sessions_dir.join("2026/06/21/rollout-test123.jsonl");
        write_jsonl(
            &session_file,
            &[
                json!({"type": "session_meta", "payload": {"id": "test123", "cwd": "/work/app"}}),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "Original prompt"}}),
                json!({"type": "response_item", "payload": {"role": "assistant", "content": [{"text": "Answer"}]}}),
            ],
        );
        let session_index = temp.path().join("session_index.jsonl");
        fs::write(
            &session_index,
            json!({
                "id": "test123",
                "thread_name": "Renamed from index",
                "updated_at": "2030-01-01T00:00:00Z"
            })
            .to_string(),
        )
        .unwrap();

        let adapter = CodexAdapter::new(sessions_dir, session_index);
        let file_mtime = file_mtime_seconds(&session_file);
        let mut known = KnownSessions::new();
        known.insert(("codex".to_string(), "test123".to_string()), file_mtime);

        let scan = adapter.find_sessions_incremental(&known);
        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].title, "Renamed from index");
        assert!(scan.new_or_modified[0].mtime > file_mtime);

        let mut refreshed_known = KnownSessions::new();
        refreshed_known.insert(
            ("codex".to_string(), "test123".to_string()),
            scan.new_or_modified[0].mtime,
        );
        let unchanged = adapter.find_sessions_incremental(&refreshed_known);
        assert!(unchanged.new_or_modified.is_empty());
        assert!(unchanged.deleted_ids.is_empty());
    }

    #[test]
    fn incremental_retains_malformed_changed_file() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(sessions_dir.join("2026/06/21")).unwrap();
        let session_file = sessions_dir.join("2026/06/21/rollout-malformed.jsonl");
        fs::write(&session_file, "{").unwrap();

        let adapter = CodexAdapter::new(sessions_dir, temp.path().join("session_index.jsonl"));
        let mut known = KnownSessions::new();
        known.insert(("codex".to_string(), "malformed".to_string()), 0.0);

        let scan = adapter.find_sessions_incremental(&known);

        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());

        let mut emitted = Vec::new();
        let mut on_session = |session| emitted.push(session);
        let streaming_scan = adapter.find_sessions_incremental_streaming(&known, &mut on_session);
        assert!(emitted.is_empty());
        assert!(streaming_scan.new_or_modified.is_empty());
        assert!(streaming_scan.deleted_ids.is_empty());
    }

    #[test]
    fn malformed_file_does_not_block_other_incremental_updates() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let day_dir = sessions_dir.join("2026/06/21");
        fs::create_dir_all(&day_dir).unwrap();
        fs::write(day_dir.join("rollout-malformed.jsonl"), "{").unwrap();
        write_jsonl(
            &day_dir.join("rollout-good.jsonl"),
            &[
                json!({"type": "session_meta", "payload": {"id": "good", "cwd": "/work/good"}}),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "Updated prompt"}}),
            ],
        );

        let adapter = CodexAdapter::new(sessions_dir, temp.path().join("session_index.jsonl"));
        let mut known = KnownSessions::new();
        known.insert(("codex".to_string(), "malformed".to_string()), 0.0);
        known.insert(("codex".to_string(), "good".to_string()), 0.0);

        let scan = adapter.find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].id, "good");
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn incremental_deletes_valid_changed_file_that_no_longer_qualifies() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(sessions_dir.join("2026/06/21")).unwrap();
        let session_file = sessions_dir.join("2026/06/21/rollout-parse-gone.jsonl");
        fs::write(
            &session_file,
            json!({"type": "session_meta", "payload": {"id": "parse-gone"}}).to_string(),
        )
        .unwrap();

        let adapter = CodexAdapter::new(sessions_dir, temp.path().join("session_index.jsonl"));
        let mut known = KnownSessions::new();
        known.insert(("codex".to_string(), "parse-gone".to_string()), 0.0);

        let scan = adapter.find_sessions_incremental(&known);

        assert!(scan.new_or_modified.is_empty());
        assert_eq!(scan.deleted_ids, vec!["parse-gone"]);

        let mut emitted = Vec::new();
        let mut on_session = |session| emitted.push(session);
        let streaming_scan = adapter.find_sessions_incremental_streaming(&known, &mut on_session);
        assert!(emitted.is_empty());
        assert!(streaming_scan.new_or_modified.is_empty());
        assert_eq!(streaming_scan.deleted_ids, vec!["parse-gone"]);
    }

    #[test]
    fn incremental_scan_errors_do_not_delete_known_sessions() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::write(&sessions_dir, "not a directory").unwrap();

        let adapter = CodexAdapter::new(sessions_dir, temp.path().join("session_index.jsonl"));
        let mut known = KnownSessions::new();
        known.insert(("codex".to_string(), "abc123".to_string()), 1.0);

        let scan = adapter.find_sessions_incremental(&known);
        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());

        let mut emitted = Vec::new();
        let mut on_session = |session| emitted.push(session);
        let streaming_scan = adapter.find_sessions_incremental_streaming(&known, &mut on_session);
        assert!(emitted.is_empty());
        assert!(streaming_scan.new_or_modified.is_empty());
        assert!(streaming_scan.deleted_ids.is_empty());
    }
}
