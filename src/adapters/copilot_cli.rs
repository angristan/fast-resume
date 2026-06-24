use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;
use walkdir::WalkDir;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    copilot_fallback_session_id, incremental_from_files, incremental_from_files_streaming,
    raw_stats_for_tree, string_at,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

#[derive(Debug, Clone)]
pub struct CopilotCliAdapter {
    sessions_dir: PathBuf,
}

impl Default for CopilotCliAdapter {
    fn default() -> Self {
        Self {
            sessions_dir: config::copilot_dir(),
        }
    }
}

impl Adapter for CopilotCliAdapter {
    fn name(&self) -> &'static str {
        "copilot-cli"
    }

    fn supports_yolo(&self) -> bool {
        true
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
        incremental_from_files(self.name(), known, self.scan_session_files(), |path| {
            self.parse_session(path)
        })
    }

    fn find_sessions_incremental_streaming(
        &self,
        known: &KnownSessions,
        on_session: &mut SessionCallback<'_>,
    ) -> IncrementalScan {
        incremental_from_files_streaming(
            self.name(),
            known,
            self.scan_session_files(),
            |path| self.parse_session(path),
            on_session,
        )
    }

    fn resume_command(&self, session: &Session, yolo: bool) -> Vec<String> {
        let mut cmd = vec!["copilot".to_string()];
        if yolo {
            cmd.push("--yolo".to_string());
        }
        cmd.extend(["--resume".to_string(), session.id.clone()]);
        cmd
    }

    fn raw_stats(&self) -> RawAdapterStats {
        raw_stats_for_tree(self.name(), &self.sessions_dir, "jsonl")
    }
}

impl CopilotCliAdapter {
    fn scan_session_files(&self) -> HashMap<String, (PathBuf, f64)> {
        let mut current_files = HashMap::new();
        if !self.sessions_dir.exists() {
            return current_files;
        }

        for entry in WalkDir::new(&self.sessions_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
        {
            let path = entry.path();
            let session_id = self.session_id_from_file(path);
            current_files.insert(session_id, (path.to_path_buf(), file_mtime_seconds(path)));
        }
        current_files
    }

    fn session_id_from_file(&self, path: &Path) -> String {
        if let Ok(file) = fs::File::open(path) {
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(entry) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if string_at(&entry, &["type"]) == "session.start" {
                    let id = string_at(entry.get("data").unwrap_or(&Value::Null), &["sessionId"]);
                    if !id.is_empty() {
                        return id;
                    }
                    break;
                }
            }
        }
        copilot_fallback_session_id(path, &self.sessions_dir)
    }

    fn parse_session(&self, path: &Path) -> Option<Session> {
        let file = fs::File::open(path).ok()?;
        let mut session_id = copilot_fallback_session_id(path, &self.sessions_dir);
        let mut directory = String::new();
        let mut first_user_message = String::new();
        let mut session_title = String::new();
        let mut messages = Vec::new();
        let mut turns = 0usize;
        let folder_re = Regex::new(r"Folder (/[^\s]+)").ok()?;

        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(entry) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let msg_type = string_at(&entry, &["type"]);
            let data = entry.get("data").unwrap_or(&Value::Null);

            match msg_type.as_str() {
                "session.start" => {
                    let id = string_at(data, &["sessionId"]);
                    if !id.is_empty() {
                        session_id = id;
                    }
                    if directory.is_empty() {
                        directory = string_at(data, &["context", "cwd"]);
                    }
                }
                "session.info" if directory.is_empty() => {
                    if string_at(data, &["infoType"]) == "folder_trust" {
                        let message = string_at(data, &["message"]);
                        if let Some(caps) = folder_re.captures(&message) {
                            directory = caps[1].to_string();
                        }
                    }
                }
                "session.title_changed" => {
                    let title = string_at(data, &["title"]);
                    if !title.trim().is_empty() {
                        session_title = title.trim().to_string();
                    }
                }
                "user.message" => {
                    let content = string_at(data, &["content"]);
                    if !content.is_empty() {
                        messages.push(format!("» {content}"));
                        turns += 1;
                        if first_user_message.is_empty() && content.chars().count() > 10 {
                            first_user_message = content;
                        }
                    }
                }
                "assistant.message" => {
                    let content = string_at(data, &["content"]);
                    if !content.is_empty() {
                        messages.push(format!("  {content}"));
                        turns += 1;
                    }
                }
                _ => {}
            }
        }

        if first_user_message.is_empty() || messages.is_empty() {
            return None;
        }

        let title = truncate_title(
            if session_title.is_empty() {
                &first_user_message
            } else {
                &session_title
            },
            100,
            true,
        );
        let mut session = Session::new(
            session_id,
            self.name(),
            title,
            directory,
            file_timestamp(path),
            messages.join("\n\n"),
            turns,
        );
        session.mtime = file_mtime_seconds(path);
        Some(session)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use serde_json::{Value, json};
    use tempfile::tempdir;

    use crate::adapters::Adapter;

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
    fn parses_session_and_yolo_resume_command() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        write_jsonl(
            &sessions_dir.join("session.jsonl"),
            &[
                json!({"type": "session.start", "data": {"sessionId": "copilot-1", "context": {"cwd": "/work/copilot"}}}),
                json!({"type": "session.title_changed", "data": {"title": "Investigate failing test"}}),
                json!({"type": "user.message", "data": {"content": "Please fix this broken test"}}),
                json!({"type": "assistant.message", "data": {"content": "Done"}}),
            ],
        );

        let adapter = CopilotCliAdapter { sessions_dir };
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "copilot-1");
        assert_eq!(sessions[0].title, "Investigate failing test");
        assert_eq!(sessions[0].directory, "/work/copilot");
        assert_eq!(sessions[0].message_count, 2);
        assert_eq!(
            adapter.resume_command(&sessions[0], true),
            vec!["copilot", "--yolo", "--resume", "copilot-1"]
        );
    }
}
