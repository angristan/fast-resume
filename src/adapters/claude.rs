use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;
use walkdir::WalkDir;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    content_texts, incremental_from_files, incremental_from_files_streaming,
    parse_timestamp_seconds, raw_stats_for_tree, string_at, text_from_part,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

pub struct ClaudeAdapter {
    sessions_dir: PathBuf,
}

impl Default for ClaudeAdapter {
    fn default() -> Self {
        Self {
            sessions_dir: config::claude_dir(),
        }
    }
}

impl ClaudeAdapter {
    #[allow(dead_code)]
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir }
    }

    fn parse_session(&self, path: &Path) -> Option<Session> {
        let file = fs::File::open(path).ok()?;
        let mut directory = String::new();
        let mut first_user_message = String::new();
        let mut messages = Vec::new();
        let mut turns = 0usize;

        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(data) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let msg_type = data.get("type").and_then(Value::as_str).unwrap_or_default();

            if msg_type == "user" {
                if directory.is_empty() {
                    directory = string_at(&data, &["cwd"]);
                }
                let content = data
                    .pointer("/message/content")
                    .cloned()
                    .unwrap_or(Value::Null);
                let mut is_human_input = false;
                match content {
                    Value::String(text) => {
                        is_human_input = true;
                        let is_meta = data.get("isMeta").and_then(Value::as_bool).unwrap_or(false);
                        if !is_meta
                            && !text.starts_with("<command")
                            && !text.starts_with("<local-command")
                        {
                            messages.push(format!("» {text}"));
                            if first_user_message.is_empty() && text.chars().count() > 10 {
                                first_user_message = text;
                            }
                        }
                    }
                    Value::Array(parts) => {
                        if parts
                            .first()
                            .and_then(|part| part.get("type"))
                            .and_then(Value::as_str)
                            == Some("text")
                        {
                            is_human_input = true;
                        }
                        for part in parts {
                            if let Some(text) = text_from_part(&part) {
                                messages.push(format!("» {text}"));
                                if first_user_message.is_empty() {
                                    first_user_message = text;
                                }
                            } else if let Some(text) = part.as_str() {
                                messages.push(format!("» {text}"));
                            }
                        }
                    }
                    _ => {}
                }
                if is_human_input {
                    turns += 1;
                }
            } else if msg_type == "assistant" {
                let content = data
                    .pointer("/message/content")
                    .cloned()
                    .unwrap_or(Value::Null);
                let mut has_text = false;
                for text in content_texts(&content) {
                    messages.push(format!("  {text}"));
                    has_text = true;
                }
                if has_text {
                    turns += 1;
                }
            }
        }

        if first_user_message.is_empty() || messages.is_empty() {
            return None;
        }

        let title_source = claude_index_title(path).unwrap_or(first_user_message);
        let title = truncate_title(&title_source, 100, true);
        let mut session = Session::new(
            path.file_stem()?.to_string_lossy(),
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

    fn scan_session_files(&self) -> HashMap<String, (PathBuf, f64)> {
        let mut current_files = HashMap::new();
        let Ok(projects) = fs::read_dir(&self.sessions_dir) else {
            return current_files;
        };

        for project in projects.filter_map(Result::ok) {
            let project_dir = project.path();
            if !project_dir.is_dir() {
                continue;
            }
            let project_index = claude_project_index(&project_dir);
            let Ok(files) = fs::read_dir(&project_dir) else {
                continue;
            };
            for file in files.filter_map(Result::ok) {
                let path = file.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                if path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("agent-"))
                {
                    continue;
                }
                let Some(session_id) = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(ToString::to_string)
                else {
                    continue;
                };
                let mut mtime = file_mtime_seconds(&path);
                if let Some((_, index_mtime)) = project_index.get(&session_id) {
                    mtime = mtime.max(*index_mtime);
                }
                current_files.insert(session_id, (path, mtime));
            }
        }

        current_files
    }
}

impl Adapter for ClaudeAdapter {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn supports_yolo(&self) -> bool {
        true
    }

    fn find_sessions(&self) -> Vec<Session> {
        if !self.sessions_dir.exists() {
            return Vec::new();
        }
        let mut sessions = Vec::new();
        for entry in WalkDir::new(&self.sessions_dir)
            .min_depth(2)
            .max_depth(2)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("agent-"))
            {
                continue;
            }
            if let Some(session) = self.parse_session(path) {
                sessions.push(session);
            }
        }
        sessions
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
        let mut cmd = vec!["claude".to_string()];
        if yolo {
            cmd.push("--dangerously-skip-permissions".to_string());
        }
        cmd.extend(["--resume".to_string(), session.id.clone()]);
        cmd
    }

    fn raw_stats(&self) -> RawAdapterStats {
        raw_stats_for_tree(self.name(), &self.sessions_dir, "jsonl")
    }
}

fn claude_index_title(session_file: &Path) -> Option<String> {
    let session_id = session_file.file_stem()?.to_string_lossy();
    claude_project_index(session_file.parent()?)
        .get(session_id.as_ref())
        .map(|(title, _)| title.clone())
}

fn claude_project_index(project_dir: &Path) -> HashMap<String, (String, f64)> {
    let mut titles = HashMap::new();
    let index_file = project_dir.join("sessions-index.json");
    let index_mtime = file_mtime_seconds(&index_file);
    let Ok(data) = serde_json::from_slice::<Value>(&fs::read(index_file).unwrap_or_default())
    else {
        return titles;
    };
    let Some(entries) = data.get("entries").and_then(Value::as_array) else {
        return titles;
    };
    for entry in entries {
        let session_id = string_at(entry, &["sessionId"]);
        let summary = string_at(entry, &["summary"]);
        if session_id.is_empty() || summary.trim().is_empty() {
            continue;
        }
        let modified = parse_timestamp_seconds(&string_at(entry, &["modified"])).unwrap_or(0.0);
        let file_mtime = entry
            .get("fileMtime")
            .and_then(Value::as_f64)
            .map(|value| value / 1000.0)
            .unwrap_or(0.0);
        let mtime = index_mtime.max(modified).max(file_mtime);
        titles.insert(session_id, (summary.trim().to_string(), mtime));
    }
    titles
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::adapters::Adapter;

    use super::*;

    #[test]
    fn uses_sessions_index_title() {
        let temp = tempdir().unwrap();
        let projects = temp.path().join("projects");
        let project = projects.join("project-a");
        fs::create_dir_all(&project).unwrap();

        fs::write(
            project.join("session-rename.jsonl"),
            [
                json!({
                    "type": "user",
                    "cwd": "/work/app",
                    "message": {"content": "Original first prompt for this session"}
                })
                .to_string(),
                json!({"type": "assistant", "message": {"content": "Response"}}).to_string(),
            ]
            .join("\n"),
        )
        .unwrap();
        fs::write(
            project.join("sessions-index.json"),
            json!({
                "version": 1,
                "entries": [{
                    "sessionId": "session-rename",
                    "summary": "Renamed Claude thread"
                }]
            })
            .to_string(),
        )
        .unwrap();

        let adapter = ClaudeAdapter::new(projects);
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Renamed Claude thread");
        assert_eq!(sessions[0].directory, "/work/app");
    }
}
