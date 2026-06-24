use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use url::Url;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{deleted_ids_for_agent, session_needs_update, string_at, timestamp_from_ms};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

#[derive(Debug, Clone)]
pub struct CopilotVsCodeAdapter {
    chat_sessions_dir: PathBuf,
    workspace_storage_dir: PathBuf,
}

impl Default for CopilotVsCodeAdapter {
    fn default() -> Self {
        Self {
            chat_sessions_dir: config::vscode_empty_window_chat_dir(),
            workspace_storage_dir: config::vscode_workspace_storage_dir(),
        }
    }
}

impl Adapter for CopilotVsCodeAdapter {
    fn name(&self) -> &'static str {
        "copilot-vscode"
    }

    fn find_sessions(&self) -> Vec<Session> {
        self.session_files()
            .into_iter()
            .filter_map(|(path, workspace)| self.parse_session(&path, &workspace))
            .collect()
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        let mut current_files = HashMap::new();
        for (path, workspace) in self.session_files() {
            let Some(session_id) = vscode_session_id(&path) else {
                continue;
            };
            let mtime = file_mtime_seconds(&path);
            current_files.insert(session_id, (path, mtime, workspace));
        }

        let current_ids: HashSet<_> = current_files.keys().cloned().collect();
        let mut new_or_modified = Vec::new();
        for (session_id, (path, mtime, workspace)) in current_files {
            if !session_needs_update(known, self.name(), &session_id, mtime) {
                continue;
            }
            if let Some(mut session) = self.parse_session(&path, &workspace) {
                session.mtime = mtime;
                new_or_modified.push(session);
            }
        }

        IncrementalScan {
            agent: self.name(),
            new_or_modified,
            deleted_ids: deleted_ids_for_agent(known, self.name(), &current_ids),
        }
    }

    fn find_sessions_incremental_streaming(
        &self,
        known: &KnownSessions,
        on_session: &mut SessionCallback<'_>,
    ) -> IncrementalScan {
        let mut current_files = HashMap::new();
        for (path, workspace) in self.session_files() {
            let Some(session_id) = vscode_session_id(&path) else {
                continue;
            };
            let mtime = file_mtime_seconds(&path);
            current_files.insert(session_id, (path, mtime, workspace));
        }

        let current_ids: HashSet<_> = current_files.keys().cloned().collect();
        let mut new_or_modified = Vec::new();
        for (session_id, (path, mtime, workspace)) in current_files {
            if !session_needs_update(known, self.name(), &session_id, mtime) {
                continue;
            }
            if let Some(mut session) = self.parse_session(&path, &workspace) {
                session.mtime = mtime;
                on_session(session.clone());
                new_or_modified.push(session);
            }
        }

        IncrementalScan {
            agent: self.name(),
            new_or_modified,
            deleted_ids: deleted_ids_for_agent(known, self.name(), &current_ids),
        }
    }

    fn resume_command(&self, session: &Session, _yolo: bool) -> Vec<String> {
        if session.directory.is_empty() {
            vec!["code".to_string()]
        } else {
            vec!["code".to_string(), session.directory.clone()]
        }
    }

    fn raw_stats(&self) -> RawAdapterStats {
        let files = self.session_files();
        RawAdapterStats {
            agent: self.name(),
            data_dir: self.chat_sessions_dir.display().to_string(),
            available: !files.is_empty(),
            file_count: files.len(),
            total_bytes: files
                .iter()
                .filter_map(|(path, _)| path.metadata().ok().map(|m| m.len()))
                .sum(),
        }
    }
}

impl CopilotVsCodeAdapter {
    fn session_files(&self) -> Vec<(PathBuf, String)> {
        let mut files = Vec::new();
        if self.chat_sessions_dir.exists() {
            if let Ok(read_dir) = fs::read_dir(&self.chat_sessions_dir) {
                for entry in read_dir.filter_map(Result::ok) {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("json") {
                        files.push((path, String::new()));
                    }
                }
            }
        }
        if self.workspace_storage_dir.exists() {
            if let Ok(read_dir) = fs::read_dir(&self.workspace_storage_dir) {
                for entry in read_dir.filter_map(Result::ok) {
                    let ws_dir = entry.path();
                    let chat_dir = ws_dir.join("chatSessions");
                    if !chat_dir.exists() {
                        continue;
                    }
                    let workspace = workspace_directory(&ws_dir);
                    if let Ok(chat_files) = fs::read_dir(chat_dir) {
                        for chat_file in chat_files.filter_map(Result::ok) {
                            let path = chat_file.path();
                            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                                files.push((path, workspace.clone()));
                            }
                        }
                    }
                }
            }
        }
        files
    }

    fn parse_session(&self, path: &Path, workspace_directory: &str) -> Option<Session> {
        let data: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
        let session_id = string_at(&data, &["sessionId"]);
        let session_id = if session_id.is_empty() {
            path.file_stem()?.to_string_lossy().to_string()
        } else {
            session_id
        };
        let mut title = string_at(&data, &["customTitle"]);
        let requests = data.get("requests")?.as_array()?;
        if requests.is_empty() {
            return None;
        }

        let mut directory = workspace_directory.to_string();
        let mut messages = Vec::new();
        let mut turns = 0usize;

        for req in requests {
            let user_text = string_at(req, &["message", "text"]);
            if !user_text.is_empty() {
                messages.push(format!("» {user_text}"));
                turns += 1;
            }

            if directory.is_empty() {
                if let Some(refs) = req.get("contentReferences").and_then(Value::as_array) {
                    for reference in refs {
                        let fs_path = string_at(reference, &["reference", "uri", "fsPath"]);
                        if !fs_path.is_empty() {
                            directory = Path::new(&fs_path)
                                .parent()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default();
                            break;
                        }
                    }
                }
            }

            let mut has_response = false;
            if let Some(response) = req.get("response").and_then(Value::as_array) {
                for part in response {
                    let value = string_at(part, &["value"]);
                    if !value.is_empty() {
                        messages.push(format!("  {value}"));
                        has_response = true;
                    }
                }
            }
            if has_response {
                turns += 1;
            }
        }

        if messages.is_empty() {
            return None;
        }
        if title.is_empty() {
            title = truncate_title(messages[0].trim_start_matches("» ").trim(), 100, true);
        }

        let timestamp = timestamp_from_ms(
            data.get("lastMessageDate")
                .or_else(|| data.get("creationDate"))
                .and_then(Value::as_i64),
        )
        .unwrap_or_else(|| file_timestamp(path));

        let mut session = Session::new(
            session_id,
            self.name(),
            title,
            directory,
            timestamp,
            messages.join("\n\n"),
            turns,
        );
        session.mtime = file_mtime_seconds(path);
        Some(session)
    }
}

fn workspace_directory(workspace_dir: &Path) -> String {
    let workspace_json = workspace_dir.join("workspace.json");
    let Ok(data) = serde_json::from_slice::<Value>(&fs::read(workspace_json).unwrap_or_default())
    else {
        return String::new();
    };
    let folder = string_at(&data, &["folder"]);
    if let Ok(url) = Url::parse(&folder) {
        if url.scheme() == "file" {
            return url
                .to_file_path()
                .ok()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
        }
    }
    String::new()
}

fn vscode_session_id(path: &Path) -> Option<String> {
    let data: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    let id = string_at(&data, &["sessionId"]);
    if id.is_empty() {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .map(ToString::to_string)
    } else {
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::adapters::Adapter;

    use super::*;

    #[test]
    fn parses_session_and_opens_referenced_directory() {
        let temp = tempdir().unwrap();
        let chat_sessions_dir = temp.path().join("chat");
        fs::create_dir_all(&chat_sessions_dir).unwrap();
        fs::write(
            chat_sessions_dir.join("session.json"),
            json!({
                "sessionId": "vscode-1",
                "customTitle": "VS Code thread",
                "lastMessageDate": 1_720_000_000_000_i64,
                "requests": [{
                    "message": {"text": "Open the failing file"},
                    "contentReferences": [{
                        "reference": {"uri": {"fsPath": "/work/vscode/main.rs"}}
                    }],
                    "response": [{"value": "Opened"}]
                }]
            })
            .to_string(),
        )
        .unwrap();

        let adapter = CopilotVsCodeAdapter {
            chat_sessions_dir,
            workspace_storage_dir: temp.path().join("workspaceStorage"),
        };
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "vscode-1");
        assert_eq!(sessions[0].title, "VS Code thread");
        assert_eq!(sessions[0].directory, "/work/vscode");
        assert_eq!(sessions[0].message_count, 2);
        assert_eq!(
            adapter.resume_command(&sessions[0], false),
            vec!["code", "/work/vscode"]
        );
    }
}
