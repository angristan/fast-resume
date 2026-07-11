use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    IncrementalParse, content_texts, failed_incremental_scan, incremental_from_files,
    incremental_from_files_streaming, incremental_parse_from_option,
    incremental_parse_jsonl_with_partial_check, json_file_has_parse_errors, parse_datetime,
    raw_stats_for_tree, string_at,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

#[derive(Debug, Clone)]
pub struct VibeAdapter {
    sessions_dir: PathBuf,
}

impl Default for VibeAdapter {
    fn default() -> Self {
        Self {
            sessions_dir: config::vibe_dir(),
        }
    }
}

impl Adapter for VibeAdapter {
    fn name(&self) -> &'static str {
        "vibe"
    }

    fn supports_yolo(&self) -> bool {
        true
    }

    fn find_sessions(&self) -> Vec<Session> {
        if !self.sessions_dir.exists() {
            return Vec::new();
        }
        let Ok(entries) = fs::read_dir(&self.sessions_dir) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_dir()
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|name| name.starts_with("session_"))
            })
            .filter_map(|path| self.parse_session(&path))
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

    fn resume_command(&self, session: &Session, yolo: bool) -> Vec<String> {
        let mut cmd = vec!["vibe".to_string()];
        if yolo {
            cmd.extend(["--agent".to_string(), "auto-approve".to_string()]);
        }
        cmd.extend(["--resume".to_string(), session.id.clone()]);
        cmd
    }

    fn raw_stats(&self) -> RawAdapterStats {
        raw_stats_for_tree(self.name(), &self.sessions_dir, "jsonl")
    }
}

impl VibeAdapter {
    fn scan_session_files(&self) -> Option<(HashMap<String, (PathBuf, f64)>, bool)> {
        let mut current_files = HashMap::new();
        let mut complete = true;
        if !self.sessions_dir.exists() {
            return Some((current_files, complete));
        }
        if !self.sessions_dir.is_dir() {
            return None;
        }
        let Ok(entries) = fs::read_dir(&self.sessions_dir) else {
            return None;
        };

        for entry in entries {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let session_dir = entry.path();
            if !session_dir.is_dir()
                || !session_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("session_"))
            {
                continue;
            }
            let metadata_file = session_dir.join("meta.json");
            let Ok(metadata_data) = fs::read(&metadata_file) else {
                complete = false;
                continue;
            };
            let Ok(metadata) = serde_json::from_slice::<Value>(&metadata_data) else {
                complete = false;
                continue;
            };
            let session_id = {
                let id = string_at(&metadata, &["session_id"]);
                if id.is_empty() {
                    session_dir
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default()
                        .to_string()
                } else {
                    id
                }
            };
            current_files.insert(
                session_id,
                (session_dir.clone(), vibe_session_mtime(&session_dir)),
            );
        }

        Some((current_files, complete))
    }

    fn parse_session(&self, session_dir: &Path) -> Option<Session> {
        let metadata_file = session_dir.join("meta.json");
        let messages_file = session_dir.join("messages.jsonl");
        let metadata: Value = serde_json::from_slice(&fs::read(&metadata_file).ok()?).ok()?;

        let session_id = {
            let id = string_at(&metadata, &["session_id"]);
            if id.is_empty() {
                session_dir.file_name()?.to_string_lossy().to_string()
            } else {
                id
            }
        };
        let directory = string_at(&metadata, &["environment", "working_directory"]);
        let yolo = metadata
            .pointer("/config/auto_approve")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || metadata
                .get("auto_approve")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        let timestamp = parse_datetime(&string_at(&metadata, &["start_time"]))
            .unwrap_or_else(|| file_timestamp(&metadata_file));
        let mut title = string_at(&metadata, &["title"]);

        let mut messages = Vec::new();
        let mut first_user = String::new();
        if messages_file.exists() {
            let file = fs::File::open(&messages_file).ok()?;
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let role = string_at(&msg, &["role"]);
                if role == "system" {
                    continue;
                }
                let role_prefix = if role == "user" { "» " } else { "  " };
                if let Some(content) = msg.get("content") {
                    for text in content_texts(content) {
                        if role == "user" && first_user.is_empty() {
                            first_user = text.clone();
                        }
                        messages.push(format!("{role_prefix}{text}"));
                    }
                }
            }
        }

        if title.is_empty() {
            title = if first_user.is_empty() {
                "Vibe session".to_string()
            } else {
                truncate_title(&first_user, 80, false)
            };
        }

        let mut session = Session::new(
            session_id,
            self.name(),
            title,
            directory,
            timestamp,
            messages.join("\n\n"),
            messages.len(),
        );
        session.mtime = vibe_session_mtime(session_dir);
        session.yolo = yolo;
        Some(session)
    }

    fn parse_session_incremental(&self, session_dir: &Path) -> IncrementalParse {
        let metadata_file = session_dir.join("meta.json");
        let messages_file = session_dir.join("messages.jsonl");
        if json_file_has_parse_errors(&metadata_file) {
            IncrementalParse::Retain
        } else if messages_file.exists() {
            incremental_parse_jsonl_with_partial_check(
                &messages_file,
                || self.parse_session(session_dir),
                |_| vibe_messages_have_user_content(&messages_file),
            )
        } else {
            incremental_parse_from_option(self.parse_session(session_dir))
        }
    }
}

fn vibe_session_mtime(session_dir: &Path) -> f64 {
    file_mtime_seconds(&session_dir.join("meta.json"))
        .max(file_mtime_seconds(&session_dir.join("messages.jsonl")))
}

fn vibe_messages_have_user_content(path: &Path) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .any(|line| {
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                return false;
            };
            string_at(&message, &["role"]) == "user"
                && message.get("content").is_some_and(|content| {
                    content_texts(content)
                        .iter()
                        .any(|text| !text.trim().is_empty())
                })
        })
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

    #[test]
    fn parses_session_and_auto_approve_resume_command() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_dir = sessions_dir.join("session_alpha");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("meta.json"),
            json!({
                "session_id": "vibe-1",
                "environment": {"working_directory": "/work/vibe"},
                "config": {"auto_approve": true},
                "start_time": "2026-01-01T00:00:00Z"
            })
            .to_string(),
        )
        .unwrap();
        write_jsonl(
            &session_dir.join("messages.jsonl"),
            &[
                json!({"role": "user", "content": "Please build the feature"}),
                json!({"role": "assistant", "content": [{"text": "Done"}]}),
            ],
        );

        let adapter = VibeAdapter { sessions_dir };
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "vibe-1");
        assert_eq!(sessions[0].title, "Please build the feature");
        assert_eq!(sessions[0].directory, "/work/vibe");
        assert!(sessions[0].yolo);
        assert_eq!(
            adapter.resume_command(&sessions[0], true),
            vec!["vibe", "--agent", "auto-approve", "--resume", "vibe-1"]
        );
    }

    #[test]
    fn parses_fractional_naive_start_time() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_dir = sessions_dir.join("session_alpha");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("meta.json"),
            json!({
                "session_id": "vibe-1",
                "environment": {"working_directory": "/work/vibe"},
                "start_time": "2025-01-10T14:00:00.123456"
            })
            .to_string(),
        )
        .unwrap();
        write_jsonl(
            &session_dir.join("messages.jsonl"),
            &[json!({"role": "user", "content": "Fractional timestamp"})],
        );

        let adapter = VibeAdapter { sessions_dir };
        let sessions = adapter.find_sessions();

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0]
                .timestamp
                .format("%Y-%m-%dT%H:%M:%S%.6f")
                .to_string(),
            "2025-01-10T14:00:00.123456"
        );
    }

    #[test]
    fn incremental_refresh_uses_messages_mtime() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_dir = sessions_dir.join("session_alpha");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("meta.json"),
            json!({
                "session_id": "vibe-1",
                "environment": {"working_directory": "/work/vibe"},
                "start_time": "2026-01-01T00:00:00Z"
            })
            .to_string(),
        )
        .unwrap();
        let meta_mtime = file_mtime_seconds(&session_dir.join("meta.json"));
        thread::sleep(Duration::from_millis(20));
        write_jsonl(
            &session_dir.join("messages.jsonl"),
            &[json!({"role": "user", "content": "Newer Vibe message"})],
        );

        let adapter = VibeAdapter { sessions_dir };
        let mut known = KnownSessions::new();
        known.insert(("vibe".to_string(), "vibe-1".to_string()), meta_mtime);

        let scan = adapter.find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert!(scan.new_or_modified[0].mtime > meta_mtime);
        assert!(
            scan.new_or_modified[0]
                .content
                .contains("Newer Vibe message")
        );
    }

    #[test]
    fn partial_jsonl_updates_while_invalid_jsonl_retains() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let partial_dir = sessions_dir.join("session_partial");
        let invalid_dir = sessions_dir.join("session_invalid");
        let unusable_partial_dir = sessions_dir.join("session_unusable");
        let whitespace_partial_dir = sessions_dir.join("session_whitespace");
        fs::create_dir_all(&partial_dir).unwrap();
        fs::create_dir_all(&invalid_dir).unwrap();
        fs::create_dir_all(&unusable_partial_dir).unwrap();
        fs::create_dir_all(&whitespace_partial_dir).unwrap();
        for (session_dir, session_id) in [
            (&partial_dir, "partial"),
            (&invalid_dir, "invalid"),
            (&unusable_partial_dir, "unusable"),
            (&whitespace_partial_dir, "whitespace"),
        ] {
            fs::write(
                session_dir.join("meta.json"),
                json!({
                    "session_id": session_id,
                    "environment": {"working_directory": "/work/vibe"},
                    "start_time": "2026-01-01T00:00:00Z"
                })
                .to_string(),
            )
            .unwrap();
        }
        fs::write(
            partial_dir.join("messages.jsonl"),
            [
                json!({"role": "user", "content": "Valid Vibe prompt after malformed history"})
                    .to_string(),
                "{".to_string(),
                json!({"role": "assistant", "content": "Updated response"}).to_string(),
            ]
            .join("\n"),
        )
        .unwrap();
        fs::write(invalid_dir.join("messages.jsonl"), "{").unwrap();
        fs::write(
            unusable_partial_dir.join("messages.jsonl"),
            [
                "{".to_string(),
                json!({
                    "role": "assistant",
                    "content": "Assistant text\n» looks like rendered user content"
                })
                .to_string(),
            ]
            .join("\n"),
        )
        .unwrap();
        fs::write(
            whitespace_partial_dir.join("messages.jsonl"),
            [
                "{".to_string(),
                json!({"role": "user", "content": "   "}).to_string(),
            ]
            .join("\n"),
        )
        .unwrap();
        let adapter = VibeAdapter { sessions_dir };
        let mut known = KnownSessions::new();
        known.insert(("vibe".to_string(), "partial".to_string()), 0.0);
        known.insert(("vibe".to_string(), "invalid".to_string()), 0.0);
        known.insert(("vibe".to_string(), "unusable".to_string()), 0.0);
        known.insert(("vibe".to_string(), "whitespace".to_string()), 0.0);

        let scan = adapter.find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].id, "partial");
        assert!(
            scan.new_or_modified[0]
                .content
                .contains("Valid Vibe prompt after malformed history")
        );
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn malformed_metadata_does_not_block_other_incremental_updates() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let malformed_dir = sessions_dir.join("session_malformed");
        fs::create_dir_all(&malformed_dir).unwrap();
        fs::write(malformed_dir.join("meta.json"), "{").unwrap();

        let good_dir = sessions_dir.join("session_good");
        fs::create_dir_all(&good_dir).unwrap();
        fs::write(
            good_dir.join("meta.json"),
            json!({
                "session_id": "good",
                "environment": {"working_directory": "/work/good"},
                "start_time": "2026-01-01T00:00:00Z"
            })
            .to_string(),
        )
        .unwrap();
        write_jsonl(
            &good_dir.join("messages.jsonl"),
            &[json!({"role": "user", "content": "Updated Vibe prompt"})],
        );

        let adapter = VibeAdapter { sessions_dir };
        let mut known = KnownSessions::new();
        known.insert(("vibe".to_string(), "malformed".to_string()), 0.0);
        known.insert(("vibe".to_string(), "good".to_string()), 0.0);

        let scan = adapter.find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].id, "good");
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn incremental_read_dir_errors_do_not_delete_known_sessions() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::write(&sessions_dir, "not a directory").unwrap();
        let adapter = VibeAdapter { sessions_dir };
        let mut known = KnownSessions::new();
        known.insert(("vibe".to_string(), "vibe-1".to_string()), 1.0);

        let scan = adapter.find_sessions_incremental(&known);

        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }
}
