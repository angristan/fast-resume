//! Gemini CLI records project-scoped chats as legacy JSON or append-only
//! JSONL. JSONL metadata updates, checkpoints, and rewind markers are replayed
//! before the normalized conversation is built.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;
use walkdir::WalkDir;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    IncrementalParse, failed_incremental_scan, incremental_from_files,
    incremental_from_files_streaming, incremental_parse_jsonl, json_file_has_parse_errors,
    parse_datetime, string_at,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

type SessionFiles = HashMap<String, (PathBuf, f64)>;

#[derive(Debug, Clone)]
pub struct GeminiAdapter {
    sessions_dir: PathBuf,
    projects_file: PathBuf,
}

impl Default for GeminiAdapter {
    fn default() -> Self {
        let gemini_dir = config::gemini_dir();
        Self {
            sessions_dir: config::gemini_sessions_dir(),
            projects_file: gemini_dir.join("projects.json"),
        }
    }
}

impl GeminiAdapter {
    #[allow(dead_code)]
    pub fn new(sessions_dir: PathBuf, projects_file: PathBuf) -> Self {
        Self {
            sessions_dir,
            projects_file,
        }
    }

    fn project_directories(&self) -> HashMap<String, String> {
        let Ok(bytes) = fs::read(&self.projects_file) else {
            return HashMap::new();
        };
        let Ok(data) = serde_json::from_slice::<Value>(&bytes) else {
            return HashMap::new();
        };
        data.get("projects")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(directory, slug)| {
                slug.as_str()
                    .map(|slug| (slug.to_string(), directory.to_string()))
            })
            .collect()
    }

    fn directory_for(&self, path: &Path, projects: &HashMap<String, String>) -> String {
        let Some(project_dir) = path.parent().and_then(Path::parent) else {
            return String::new();
        };
        if let Ok(directory) = fs::read_to_string(project_dir.join(".project_root")) {
            let directory = directory.trim();
            if !directory.is_empty() {
                return directory.to_string();
            }
        }
        project_dir
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|slug| projects.get(slug))
            .cloned()
            .unwrap_or_default()
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
        for entry in WalkDir::new(&self.sessions_dir) {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let path = entry.path();
            if !is_gemini_session_file(path) {
                continue;
            }
            let id = gemini_session_id(path).unwrap_or_else(|| fallback_id(path));
            files.insert(id, (path.to_path_buf(), file_mtime_seconds(path)));
        }
        Some((files, complete))
    }

    fn parse_session(&self, path: &Path, projects: &HashMap<String, String>) -> Option<Session> {
        let records = load_records(path)?;
        let mut metadata = serde_json::Map::new();
        let mut messages: Vec<Value> = Vec::new();

        for record in records {
            if let Some(rewind_to) = record.get("$rewindTo").and_then(Value::as_str) {
                if rewind_to.is_empty() {
                    messages.clear();
                } else if let Some(index) = messages
                    .iter()
                    .position(|message| string_at(message, &["id"]) == rewind_to)
                {
                    messages.truncate(index);
                }
                continue;
            }
            if let Some(updates) = record.get("$set").and_then(Value::as_object) {
                if let Some(checkpoint) = updates.get("messages").and_then(Value::as_array) {
                    messages = checkpoint.clone();
                }
                for (key, value) in updates {
                    if key != "messages" {
                        metadata.insert(key.clone(), value.clone());
                    }
                }
                continue;
            }
            if record.get("sessionId").is_some() && record.get("projectHash").is_some() {
                if let Some(checkpoint) = record.get("messages").and_then(Value::as_array) {
                    messages = checkpoint.clone();
                }
                if let Some(object) = record.as_object() {
                    for (key, value) in object {
                        if key != "messages" {
                            metadata.insert(key.clone(), value.clone());
                        }
                    }
                }
                continue;
            }
            if record.get("id").is_some() {
                messages.push(record);
            }
        }

        let mut rendered = Vec::new();
        let mut first_user = String::new();
        let mut user_turns = 0usize;
        let mut last_activity = None;
        for message in messages {
            let role = string_at(&message, &["type"]);
            if role != "user" && role != "gemini" {
                continue;
            }
            let texts = gemini_content_texts(message.get("content").unwrap_or(&Value::Null));
            if texts.is_empty() {
                continue;
            }
            if role == "user" {
                user_turns += 1;
                if first_user.is_empty() {
                    first_user = texts.join("\n");
                }
            }
            if let Some(timestamp) = parse_datetime(&string_at(&message, &["timestamp"]))
                && last_activity.is_none_or(|current| timestamp > current)
            {
                last_activity = Some(timestamp);
            }
            let prefix = if role == "user" { "» " } else { "  " };
            rendered.extend(texts.into_iter().map(|text| format!("{prefix}{text}")));
        }
        if first_user.is_empty() {
            return None;
        }

        let session_id = metadata
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| fallback_id(path));
        let summary = metadata
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let title = if summary.is_empty() {
            &first_user
        } else {
            summary
        };
        let timestamp = last_activity
            .or_else(|| {
                metadata
                    .get("lastUpdated")
                    .and_then(Value::as_str)
                    .and_then(parse_datetime)
            })
            .or_else(|| {
                metadata
                    .get("startTime")
                    .and_then(Value::as_str)
                    .and_then(parse_datetime)
            })
            .unwrap_or_else(|| file_timestamp(path));
        let mut session = Session::new(
            session_id,
            self.name(),
            truncate_title(title, 100, true),
            self.directory_for(path, projects),
            timestamp,
            rendered.join("\n\n"),
            user_turns,
        );
        session.mtime = file_mtime_seconds(path);
        Some(session)
    }

    fn parse_incremental(
        &self,
        path: &Path,
        projects: &HashMap<String, String>,
    ) -> IncrementalParse {
        if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            incremental_parse_jsonl(path, || self.parse_session(path, projects))
        } else if json_file_has_parse_errors(path) {
            IncrementalParse::Retain
        } else {
            self.parse_session(path, projects)
                .map_or(IncrementalParse::Delete, IncrementalParse::Session)
        }
    }
}

impl Adapter for GeminiAdapter {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn supports_yolo(&self) -> bool {
        true
    }

    fn find_sessions(&self) -> Vec<Session> {
        let projects = self.project_directories();
        self.scan_session_files()
            .map(|(files, _)| {
                files
                    .into_values()
                    .filter_map(|(path, _)| self.parse_session(&path, &projects))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        let Some((files, complete)) = self.scan_session_files() else {
            return failed_incremental_scan(self.name());
        };
        let projects = self.project_directories();
        let mut scan = incremental_from_files(self.name(), known, files, |path| {
            self.parse_incremental(path, &projects)
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
        let projects = self.project_directories();
        let mut scan = incremental_from_files_streaming(
            self.name(),
            known,
            files,
            |path| self.parse_incremental(path, &projects),
            on_session,
        );
        if !complete {
            scan.deleted_ids.clear();
        }
        scan
    }

    fn resume_command(&self, session: &Session, yolo: bool) -> Vec<String> {
        let mut command = vec!["gemini".to_string()];
        if yolo {
            command.push("--approval-mode=yolo".to_string());
        }
        command.extend(["--resume".to_string(), session.id.clone()]);
        command
    }

    fn raw_stats(&self) -> RawAdapterStats {
        raw_stats_for_gemini(self.name(), &self.sessions_dir)
    }
}

fn is_gemini_session_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("json" | "jsonl")
    ) && path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        == Some("chats")
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("session-"))
}

fn load_records(path: &Path) -> Option<Vec<Value>> {
    if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
        let file = fs::File::open(path).ok()?;
        Some(
            BufReader::new(file)
                .lines()
                .map_while(Result::ok)
                .filter_map(|line| serde_json::from_str(&line).ok())
                .collect(),
        )
    } else {
        let value = serde_json::from_slice::<Value>(&fs::read(path).ok()?).ok()?;
        Some(vec![value])
    }
}

fn gemini_session_id(path: &Path) -> Option<String> {
    for record in load_records(path)? {
        if let Some(id) = record.get("sessionId").and_then(Value::as_str)
            && !id.is_empty()
        {
            return Some(id.to_string());
        }
    }
    None
}

fn fallback_id(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .strip_prefix("session-")
        .unwrap_or_default()
        .to_string()
}

fn gemini_content_texts(content: &Value) -> Vec<String> {
    match content {
        Value::String(text) if !text.trim().is_empty() => vec![text.clone()],
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .filter(|text| !text.trim().is_empty())
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn raw_stats_for_gemini(agent: &'static str, dir: &Path) -> RawAdapterStats {
    if !dir.exists() {
        return RawAdapterStats {
            agent,
            data_dir: dir.display().to_string(),
            available: false,
            file_count: 0,
            total_bytes: 0,
        };
    }
    let mut file_count = 0;
    let mut total_bytes = 0;
    for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
        if is_gemini_session_file(entry.path()) {
            file_count += 1;
            total_bytes += entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        }
    }
    RawAdapterStats {
        agent,
        data_dir: dir.display().to_string(),
        available: true,
        file_count,
        total_bytes,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn parses_jsonl_checkpoints_rewinds_and_project_directory() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("tmp");
        let project_dir = sessions_dir.join("fast-resume");
        let chats = project_dir.join("chats");
        fs::create_dir_all(&chats).unwrap();
        fs::write(project_dir.join(".project_root"), "/work/fast-resume\n").unwrap();
        let path = chats.join("session-2026-07-17-gemini-id.jsonl");
        let rows = [
            json!({"sessionId":"gemini-id","projectHash":"hash","startTime":"2026-07-17T10:00:00Z"}),
            json!({"id":"u1","type":"user","timestamp":"2026-07-17T10:00:01Z","content":"Fix the Gemini adapter"}),
            json!({"id":"a1","type":"gemini","timestamp":"2026-07-17T10:00:02Z","content":[{"text":"First answer"}]}),
            json!({"$rewindTo":"a1"}),
            json!({"id":"a2","type":"gemini","timestamp":"2026-07-17T10:00:03Z","content":"Final answer"}),
            json!({"$set":{"summary":"Gemini adapter work","lastUpdated":"2026-07-17T10:00:04Z"}}),
        ];
        fs::write(
            &path,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();

        let adapter = GeminiAdapter::new(sessions_dir, temp.path().join("projects.json"));
        let sessions = adapter.find_sessions();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "gemini-id");
        assert_eq!(sessions[0].title, "Gemini adapter work");
        assert_eq!(sessions[0].directory, "/work/fast-resume");
        assert_eq!(sessions[0].message_count, 1);
        assert!(sessions[0].content.contains("Final answer"));
        assert!(!sessions[0].content.contains("First answer"));
        assert_eq!(
            adapter.resume_command(&sessions[0], true),
            vec!["gemini", "--approval-mode=yolo", "--resume", "gemini-id"]
        );
    }

    #[test]
    fn parses_legacy_json_and_registry_mapping() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("tmp");
        let chats = sessions_dir.join("project-slug/chats");
        fs::create_dir_all(&chats).unwrap();
        fs::write(
            temp.path().join("projects.json"),
            json!({"projects": {"/work/legacy": "project-slug"}}).to_string(),
        )
        .unwrap();
        fs::write(
            chats.join("session-legacy.json"),
            json!({
                "sessionId":"legacy-id",
                "projectHash":"hash",
                "startTime":"2026-07-17T10:00:00Z",
                "messages":[
                    {"id":"u1","type":"user","content":"Legacy prompt"},
                    {"id":"a1","type":"gemini","content":"Legacy answer"}
                ]
            })
            .to_string(),
        )
        .unwrap();

        let adapter = GeminiAdapter::new(sessions_dir, temp.path().join("projects.json"));
        let sessions = adapter.find_sessions();
        assert_eq!(sessions[0].directory, "/work/legacy");
        assert!(sessions[0].content.contains("Legacy answer"));
    }
}
