use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use url::Url;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    deleted_ids_for_agent, failed_incremental_scan, json_file_has_parse_errors,
    session_needs_update, string_at, timestamp_from_ms,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

#[derive(Debug, Clone)]
pub struct CopilotVsCodeAdapter {
    chat_sessions_dir: PathBuf,
    workspace_storage_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct VscodeSessionFile {
    path: PathBuf,
    workspace_dir: Option<PathBuf>,
}

impl VscodeSessionFile {
    fn session_id(&self) -> Option<String> {
        let data: Value = serde_json::from_slice(&fs::read(&self.path).ok()?).ok()?;
        let session_id = string_at(&data, &["sessionId"]);
        if !session_id.is_empty() {
            return Some(session_id);
        }
        self.fallback_session_id()
    }

    fn fallback_session_id(&self) -> Option<String> {
        self.path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(ToString::to_string)
    }

    fn workspace_directory(&self) -> String {
        self.workspace_dir
            .as_deref()
            .map(workspace_directory)
            .unwrap_or_default()
    }
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
            .map(|(files, _)| files)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|file| self.parse_session(&file))
            .collect()
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        self.find_sessions_incremental_with(known, |_| {})
    }

    fn find_sessions_incremental_streaming(
        &self,
        known: &KnownSessions,
        on_session: &mut SessionCallback<'_>,
    ) -> IncrementalScan {
        self.find_sessions_incremental_with(known, |session| on_session(session))
    }

    fn resume_command(&self, session: &Session, _yolo: bool) -> Vec<String> {
        if session.directory.is_empty() {
            vec!["code".to_string()]
        } else {
            vec!["code".to_string(), session.directory.clone()]
        }
    }

    fn raw_stats(&self) -> RawAdapterStats {
        let files = self
            .session_files()
            .map(|(files, _)| files)
            .unwrap_or_default();
        RawAdapterStats {
            agent: self.name(),
            data_dir: self.chat_sessions_dir.display().to_string(),
            available: !files.is_empty(),
            file_count: files.len(),
            total_bytes: files
                .iter()
                .filter_map(|file| file.path.metadata().ok().map(|m| m.len()))
                .sum(),
        }
    }
}

impl CopilotVsCodeAdapter {
    fn find_sessions_incremental_with<F>(
        &self,
        known: &KnownSessions,
        mut on_session: F,
    ) -> IncrementalScan
    where
        F: FnMut(Session),
    {
        let mut current_files = HashMap::new();
        let Some((files, mut complete)) = self.session_files() else {
            return failed_incremental_scan(self.name());
        };
        for file in files {
            if json_file_has_parse_errors(&file.path) {
                complete = false;
                continue;
            }
            let Some(session_id) = file.session_id() else {
                complete = false;
                continue;
            };
            let mtime = file_mtime_seconds(&file.path);
            current_files.insert(session_id, (file, mtime));
        }

        let mut current_ids = HashSet::new();
        let mut new_or_modified = Vec::new();
        for (session_id, (file, mtime)) in current_files {
            current_ids.insert(session_id.clone());
            if !session_needs_update(known, self.name(), &session_id, mtime) {
                continue;
            }
            if json_file_has_parse_errors(&file.path) {
                continue;
            } else if let Some(mut session) = self.parse_session(&file) {
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

    fn session_files(&self) -> Option<(Vec<VscodeSessionFile>, bool)> {
        let mut files = Vec::new();
        let mut complete = true;
        if self.chat_sessions_dir.exists() {
            if !self.chat_sessions_dir.is_dir() {
                return None;
            }
            let Ok(read_dir) = fs::read_dir(&self.chat_sessions_dir) else {
                return None;
            };
            for entry in read_dir {
                let Ok(entry) = entry else {
                    complete = false;
                    continue;
                };
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    files.push(VscodeSessionFile {
                        path,
                        workspace_dir: None,
                    });
                }
            }
        }
        if self.workspace_storage_dir.exists() {
            if !self.workspace_storage_dir.is_dir() {
                return None;
            }
            let Ok(read_dir) = fs::read_dir(&self.workspace_storage_dir) else {
                return None;
            };
            for entry in read_dir {
                let Ok(entry) = entry else {
                    complete = false;
                    continue;
                };
                let ws_dir = entry.path();
                let chat_dir = ws_dir.join("chatSessions");
                if !chat_dir.exists() {
                    continue;
                }
                if !chat_dir.is_dir() {
                    complete = false;
                    continue;
                }
                let Ok(chat_files) = fs::read_dir(chat_dir) else {
                    complete = false;
                    continue;
                };
                for chat_file in chat_files {
                    let Ok(chat_file) = chat_file else {
                        complete = false;
                        continue;
                    };
                    let path = chat_file.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("json") {
                        files.push(VscodeSessionFile {
                            path,
                            workspace_dir: Some(ws_dir.clone()),
                        });
                    }
                }
            }
        }
        Some((files, complete))
    }

    fn parse_session(&self, file: &VscodeSessionFile) -> Option<Session> {
        let data: Value = serde_json::from_slice(&fs::read(&file.path).ok()?).ok()?;
        let session_id = {
            let id = string_at(&data, &["sessionId"]);
            if id.is_empty() {
                file.fallback_session_id()?
            } else {
                id
            }
        };
        let mut title = string_at(&data, &["customTitle"]);
        let requests = data.get("requests")?.as_array()?;
        if requests.is_empty() {
            return None;
        }

        let mut directory = file.workspace_directory();
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
        .unwrap_or_else(|| file_timestamp(&file.path));

        let mut session = Session::new(
            session_id,
            self.name(),
            title,
            directory,
            timestamp,
            messages.join("\n\n"),
            turns,
        );
        session.mtime = file_mtime_seconds(&file.path);
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
            chat_sessions_dir.join("vscode-1.json"),
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

    #[test]
    fn parses_session_id_from_json_instead_of_filename() {
        let temp = tempdir().unwrap();
        let chat_sessions_dir = temp.path().join("chat");
        fs::create_dir_all(&chat_sessions_dir).unwrap();
        fs::write(
            chat_sessions_dir.join("random-uuid-filename.json"),
            json!({
                "sessionId": "actual-session-id-from-json",
                "requests": [{
                    "message": {"text": "Use the JSON identity"},
                    "response": [{"value": "Done"}]
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
        assert_eq!(sessions[0].id, "actual-session-id-from-json");
    }

    #[test]
    fn parses_filename_fallback_when_json_session_id_is_missing() {
        let temp = tempdir().unwrap();
        let chat_sessions_dir = temp.path().join("chat");
        fs::create_dir_all(&chat_sessions_dir).unwrap();
        fs::write(
            chat_sessions_dir.join("fallback-id-test.json"),
            json!({
                "requests": [{
                    "message": {"text": "No JSON identity"},
                    "response": [{"value": "Done"}]
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
        assert_eq!(sessions[0].id, "fallback-id-test");
    }

    #[test]
    fn unchanged_incremental_scan_uses_json_session_id() {
        let temp = tempdir().unwrap();
        let chat_sessions_dir = temp.path().join("chat");
        fs::create_dir_all(&chat_sessions_dir).unwrap();
        let path = chat_sessions_dir.join("uuid-filename-12345.json");
        fs::write(
            &path,
            json!({
                "sessionId": "json-session-id-abc",
                "requests": [{
                    "message": {"text": "Test for incremental"},
                    "response": [{"value": "Response"}]
                }]
            })
            .to_string(),
        )
        .unwrap();

        let adapter = CopilotVsCodeAdapter {
            chat_sessions_dir,
            workspace_storage_dir: temp.path().join("workspaceStorage"),
        };
        let mut known = KnownSessions::new();
        known.insert(
            (
                "copilot-vscode".to_string(),
                "json-session-id-abc".to_string(),
            ),
            file_mtime_seconds(&path),
        );

        let scan = adapter.find_sessions_incremental(&known);
        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn incremental_retains_malformed_changed_file() {
        let temp = tempdir().unwrap();
        let chat_sessions_dir = temp.path().join("chat");
        fs::create_dir_all(&chat_sessions_dir).unwrap();
        let path = chat_sessions_dir.join("vscode-malformed.json");
        fs::write(&path, "{").unwrap();

        let adapter = CopilotVsCodeAdapter {
            chat_sessions_dir,
            workspace_storage_dir: temp.path().join("workspaceStorage"),
        };
        let mut known = KnownSessions::new();
        known.insert(
            ("copilot-vscode".to_string(), "vscode-malformed".to_string()),
            0.0,
        );

        let scan = adapter.find_sessions_incremental(&known);

        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn malformed_file_does_not_block_other_incremental_updates() {
        let temp = tempdir().unwrap();
        let chat_sessions_dir = temp.path().join("chat");
        fs::create_dir_all(&chat_sessions_dir).unwrap();
        fs::write(chat_sessions_dir.join("malformed.json"), "{").unwrap();
        fs::write(
            chat_sessions_dir.join("good.json"),
            json!({
                "sessionId": "good",
                "requests": [{
                    "message": {"text": "Updated request"},
                    "response": [{"value": "Updated response"}]
                }]
            })
            .to_string(),
        )
        .unwrap();

        let adapter = CopilotVsCodeAdapter {
            chat_sessions_dir,
            workspace_storage_dir: temp.path().join("workspaceStorage"),
        };
        let mut known = KnownSessions::new();
        known.insert(("copilot-vscode".to_string(), "malformed".to_string()), 0.0);
        known.insert(("copilot-vscode".to_string(), "good".to_string()), 0.0);

        let scan = adapter.find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].id, "good");
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn incremental_deletes_valid_changed_file_that_no_longer_qualifies() {
        let temp = tempdir().unwrap();
        let chat_sessions_dir = temp.path().join("chat");
        fs::create_dir_all(&chat_sessions_dir).unwrap();
        let path = chat_sessions_dir.join("vscode-gone.json");
        fs::write(
            &path,
            json!({
                "sessionId": "vscode-gone",
                "requests": []
            })
            .to_string(),
        )
        .unwrap();

        let adapter = CopilotVsCodeAdapter {
            chat_sessions_dir,
            workspace_storage_dir: temp.path().join("workspaceStorage"),
        };
        let mut known = KnownSessions::new();
        known.insert(
            ("copilot-vscode".to_string(), "vscode-gone".to_string()),
            0.0,
        );

        let scan = adapter.find_sessions_incremental(&known);

        assert!(scan.new_or_modified.is_empty());
        assert_eq!(scan.deleted_ids, vec!["vscode-gone"]);
    }

    #[test]
    fn incremental_read_dir_errors_do_not_delete_known_sessions() {
        let temp = tempdir().unwrap();
        let chat_sessions_dir = temp.path().join("chat");
        fs::write(&chat_sessions_dir, "not a directory").unwrap();
        let adapter = CopilotVsCodeAdapter {
            chat_sessions_dir,
            workspace_storage_dir: temp.path().join("workspaceStorage"),
        };
        let mut known = KnownSessions::new();
        known.insert(("copilot-vscode".to_string(), "vscode-1".to_string()), 1.0);

        let scan = adapter.find_sessions_incremental(&known);

        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }
}
