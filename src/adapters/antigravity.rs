//! Antigravity CLI sessions combine generated JSONL transcripts with a
//! separate history file that maps conversation IDs back to workspaces.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    failed_incremental_scan, incremental_from_files, incremental_from_files_streaming,
    incremental_parse_jsonl, parse_datetime, raw_stats_for_tree, string_at,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

type SessionFiles = HashMap<String, (PathBuf, f64)>;

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
        let brain_dir = self.data_dir.join("brain");
        let mut files = HashMap::new();
        let mut complete = true;
        if !brain_dir.exists() {
            return Some((files, complete));
        }
        if !brain_dir.is_dir() {
            return None;
        }
        let entries = fs::read_dir(brain_dir).ok()?;
        let history_mtime = file_mtime_seconds(&self.data_dir.join("history.jsonl")).max(
            file_mtime_seconds(&self.data_dir.join("cache").join("last_conversations.json")),
        );
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
            let transcript = preferred_transcript(&logs);
            let Some(transcript) = transcript else {
                continue;
            };
            let mtime = file_mtime_seconds(&transcript).max(history_mtime);
            files.insert(id.to_string(), (transcript, mtime));
        }
        Some((files, complete))
    }

    fn parse_session(&self, path: &Path, workspaces: &HashMap<String, String>) -> Option<Session> {
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
        session.mtime = file_mtime_seconds(path)
            .max(file_mtime_seconds(&self.data_dir.join("history.jsonl")))
            .max(file_mtime_seconds(
                &self.data_dir.join("cache").join("last_conversations.json"),
            ));
        Some(session)
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
            incremental_parse_jsonl(path, || self.parse_session(path, &workspaces))
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
            |path| incremental_parse_jsonl(path, || self.parse_session(path, &workspaces)),
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
        raw_stats_for_tree(self.name(), &self.data_dir.join("brain"), "jsonl")
    }
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
    use std::fs;

    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::*;

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
