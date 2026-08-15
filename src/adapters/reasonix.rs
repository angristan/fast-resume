use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;
use walkdir::WalkDir;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    IncrementalParse, SessionFileScan, content_texts, incremental_parse_jsonl_with_partial_check,
    incremental_scan, parse_datetime, raw_stats_for_tree, string_at,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

#[derive(Debug, Clone)]
pub struct ReasonixAdapter {
    sessions_dir: PathBuf,
}

impl Default for ReasonixAdapter {
    fn default() -> Self {
        Self {
            sessions_dir: config::reasonix_sessions_dir(),
        }
    }
}

impl ReasonixAdapter {
    #[allow(dead_code)]
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir }
    }

    /// Check if a filename is a primary session transcript, matching
    /// Reasonix's store.IsSessionTranscriptName logic.
    fn is_session_file(name: &str) -> bool {
        name.ends_with(".jsonl")
            && !name.ends_with(".events.jsonl")
            && !name.ends_with(".conflicts.jsonl")
            && !name.ends_with(".guardian.jsonl")
    }

    fn scan_session_files(&self) -> Option<SessionFileScan> {
        let mut current_files: HashMap<String, (PathBuf, f64)> = HashMap::new();
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
            if !path.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if !Self::is_session_file(name) {
                continue;
            }
            let session_id = self.session_id(path);
            let mtime = reasonix_session_mtime(path);
            current_files.insert(session_id, (path.to_path_buf(), mtime));
        }

        Some((current_files, complete))
    }

    fn session_id(&self, path: &Path) -> String {
        // Try to read 'id' from the .jsonl.meta sidecar
        let meta_path = meta_path(path);
        if let Ok(meta_data) = fs::read_to_string(&meta_path)
            && let Ok(meta) = serde_json::from_str::<Value>(&meta_data)
        {
            let id = string_at(&meta, &["id"]);
            if !id.is_empty() {
                return id;
            }
        }
        // Fallback to filename stem
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string()
    }

    fn parse_session(&self, path: &Path) -> Option<Session> {
        let file = fs::File::open(path).ok()?;
        let meta_path = meta_path(path);
        let meta_data = fs::read_to_string(&meta_path).ok()?;
        let meta: Value = serde_json::from_str(&meta_data).ok()?;

        let session_id = {
            let id = string_at(&meta, &["id"]);
            if id.is_empty() {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string()
            } else {
                id
            }
        };

        // Best-effort reverse of the project slug back to the real workspace
        // path. Reasonix's WorkspaceSlug replaces the path separator with '-'
        // (lossy for directory names containing '-'), so we disambiguate by
        // checking which interpretation exists on disk. Empty when nothing
        // matches: the TUI then shows n/a, chdir is skipped on resume, and
        // resume locates the session file by scanning instead.
        let slug = path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let directory = decode_slug(slug);

        // Timestamp from meta created_at
        let timestamp = parse_datetime(&string_at(&meta, &["created_at"]))
            .unwrap_or_else(|| file_timestamp(path));

        // Title from meta preview, fallback to first user message
        let preview = string_at(&meta, &["preview"]);
        let mut title = if preview.is_empty() {
            String::new()
        } else {
            preview
        };
        let message_count = super::shared::value_i64_at(&meta, &["turns"])
            .map(|v| v as usize)
            .unwrap_or(0);

        // Parse the JSONL transcript
        let mut first_user_message = String::new();
        let mut messages = Vec::new();

        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(data) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let role = string_at(&data, &["role"]);
            if role == "system" {
                continue;
            }
            let is_user = role == "user";
            let is_assistant = role == "assistant";
            // Include tool results only if they have meaningful content
            let is_tool = role == "tool";
            if !is_user && !is_assistant && !is_tool {
                continue;
            }

            let role_prefix = if is_user { "» " } else { "  " };
            if let Some(content) = data.get("content") {
                for text in content_texts(content) {
                    if is_user && first_user_message.is_empty() {
                        first_user_message = text.clone();
                    }
                    messages.push(format!("{role_prefix}{text}"));
                }
            }
        }

        if title.is_empty() {
            title = if first_user_message.is_empty() {
                "Reasonix session".to_string()
            } else {
                truncate_title(&first_user_message, 80, false)
            };
        }

        let mut session = Session::new(
            session_id,
            self.name(),
            title,
            directory,
            timestamp,
            messages.join("\n\n"),
            message_count,
        );
        session.mtime = reasonix_session_mtime(path);
        // YOLO detection is not yet supported in the meta format
        session.yolo = false;
        Some(session)
    }

    fn parse_session_incremental(&self, path: &Path) -> IncrementalParse {
        let meta_path = meta_path(path);
        if json_meta_has_parse_errors(&meta_path) {
            IncrementalParse::Retain
        } else {
            incremental_parse_jsonl_with_partial_check(path, || self.parse_session(path), |_| true)
        }
    }

    /// Locate the session transcript file for a session id by scanning the
    /// store. Used by resume when the workspace path could not be decoded.
    fn find_session_file(&self, id: &str) -> Option<String> {
        for entry in WalkDir::new(&self.sessions_dir)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if !Self::is_session_file(name) {
                continue;
            }
            if self.session_id(path) == id {
                return Some(path.display().to_string());
            }
        }
        None
    }
}

impl Adapter for ReasonixAdapter {
    fn name(&self) -> &'static str {
        "reasonix"
    }

    fn supports_yolo(&self) -> bool {
        false
    }

    fn find_sessions(&self) -> Vec<Session> {
        if !self.sessions_dir.exists() {
            return Vec::new();
        }
        WalkDir::new(&self.sessions_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.path().is_file()
                    && entry
                        .path()
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(Self::is_session_file)
            })
            .filter_map(|entry| self.parse_session(entry.path()))
            .collect()
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        incremental_scan(
            self.name(),
            known,
            self.scan_session_files(),
            |path| self.parse_session_incremental(path),
            None,
        )
    }

    fn find_sessions_incremental_streaming(
        &self,
        known: &KnownSessions,
        on_session: &mut SessionCallback<'_>,
    ) -> IncrementalScan {
        incremental_scan(
            self.name(),
            known,
            self.scan_session_files(),
            |path| self.parse_session_incremental(path),
            Some(on_session),
        )
    }

    fn resume_command(&self, session: &Session, _yolo: bool) -> Vec<String> {
        let path = if session.directory.is_empty() {
            // Workspace could not be decoded from the slug; locate the
            // session file by scanning the store.
            self.find_session_file(&session.id).unwrap_or_default()
        } else {
            // Re-encode the workspace path into Reasonix's slug form (the
            // inverse of the decode in parse_session). This reproduces the
            // original project slug directory name, so the session file path
            // is correct even when the slug was ambiguous to decode.
            let slug = session.directory.replace(['/', '\\'], "-");
            format!(
                "{}/{}/sessions/{}.jsonl",
                self.sessions_dir.display(),
                slug,
                session.id
            )
        };
        vec!["reasonix".to_string(), "--resume".to_string(), path]
    }

    fn raw_stats(&self) -> RawAdapterStats {
        raw_stats_for_tree(self.name(), &self.sessions_dir, "jsonl")
    }
}

/// Path to the .jsonl.meta sidecar for a session transcript.
fn meta_path(session_path: &Path) -> PathBuf {
    let mut path = session_path.to_path_buf();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    path.set_file_name(format!("{name}.meta"));
    path
}

/// Mtime is the max of the .jsonl and .jsonl.meta file times.
fn reasonix_session_mtime(session_path: &Path) -> f64 {
    file_mtime_seconds(session_path).max(file_mtime_seconds(&meta_path(session_path)))
}

/// Check if the meta JSON file has parse errors.
fn json_meta_has_parse_errors(path: &Path) -> bool {
    let Ok(data) = fs::read(path) else {
        return true;
    };
    serde_json::from_slice::<Value>(&data).is_err()
}

/// Best-effort reverse of Reasonix's WorkspaceSlug, which flattens an
/// absolute workspace path by replacing the path separator with '-'
/// (e.g. `/Users/dn/codeai/fast-resume` -> `-Users-dn-codeai-fast-resume`).
///
/// The encoding is lossy — a '-' in the slug may be either a path separator
/// or part of a directory name — so we try every interpretation and return
/// the first one that exists on disk. Returns an empty string when no
/// interpretation matches (e.g. the workspace was moved or deleted).
#[cfg(not(windows))]
fn decode_slug(slug: &str) -> String {
    decode_slug_in(slug, "/", '/')
}

/// Windows slugs additionally fold case and replace ':' and '\' with '-'
/// (e.g. `c:\users\dev\proj` -> `c--users-dev-proj`).
#[cfg(windows)]
fn decode_slug(slug: &str) -> String {
    let bytes = slug.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b'-' && bytes[2] == b'-' {
        let drive = &slug[..1];
        let rest = &slug[3..];
        decode_slug_in(rest, &format!("{drive}:\\"), '\\')
    } else {
        decode_slug_in(slug.trim_start_matches('-'), "\\", '\\')
    }
}

/// decode_slug with an injectable search root and separator (used by tests to
/// keep the search off the real filesystem).
fn decode_slug_in(slug: &str, root: &str, sep: char) -> String {
    let trimmed = slug.trim_start_matches('-');
    if trimmed.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = trimmed.split('-').collect();
    let separators = parts.len().saturating_sub(1);
    if separators > 16 {
        // Too ambiguous to guess; avoid an exponential search.
        return String::new();
    }
    // Each '-' between parts is either a separator (bit = 1) or part of a
    // directory name (bit = 0). Try the literal all-separator reading first.
    let total = 1usize << separators;
    for mask in (0..total).rev() {
        let mut path = String::from(root);
        if !path.is_empty() && !path.ends_with(sep) {
            path.push(sep);
        }
        for (i, part) in parts.iter().enumerate() {
            path.push_str(part);
            if i < separators {
                if mask & (1 << (separators - 1 - i)) != 0 {
                    path.push(sep);
                } else {
                    path.push('-');
                }
            }
        }
        if Path::new(&path).exists() {
            return path;
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::thread;
    use std::time::Duration;

    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::*;
    use crate::adapters::{Adapter, KnownSessions};

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

    fn write_meta(path: &Path, data: &Value) {
        fs::write(path, data.to_string()).unwrap();
    }

    fn session_meta(session_id: &str, turns: usize, preview: &str) -> Value {
        json!({
            "id": session_id,
            "created_at": "2026-07-23T03:25:35.557318Z",
            "updated_at": "2026-07-23T03:43:46.915110Z",
            "model": "deepseek-flash/deepseek-v4-flash",
            "revision": 15,
            "schema_version": 1,
            "turns": turns,
            "preview": preview,
        })
    }

    #[test]
    fn parses_reasonix_session() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp
            .path()
            .join("projects")
            .join("-Users-dn-fake-project")
            .join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let session_id = "20260723-032449.082229000-deepseek-v4-flash";
        let session_file = sessions_dir.join(format!("{session_id}.jsonl"));
        let meta_file = sessions_dir.join(format!("{session_id}.jsonl.meta"));

        write_jsonl(
            &session_file,
            &[
                json!({"role": "system", "content": "You are Reasonix, a coding agent."}),
                json!({"role": "user", "content": "Please implement the feature", "createdAt": 1784887550169i64}),
                json!({"role": "assistant", "content": "Here is the implementation"}),
                json!({"role": "tool", "content": "command output", "tool_call_id": "call_1", "name": "bash"}),
            ],
        );
        write_meta(
            &meta_file,
            &session_meta(session_id, 1, "Please implement the feature"),
        );

        let adapter = ReasonixAdapter::new(temp.path().join("projects"));
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.id, session_id);
        assert_eq!(session.agent, "reasonix");
        assert_eq!(session.title, "Please implement the feature");
        // The fake slug decodes to nothing on disk, so the directory is empty.
        assert_eq!(session.directory, "");
        assert_eq!(session.message_count, 1);
        assert!(session.content.contains("» Please implement the feature"));
        assert!(session.content.contains("Here is the implementation"));
        assert!(session.content.contains("command output"));
        assert!(!session.content.contains("You are Reasonix"));
    }

    #[test]
    fn decode_slug_recovers_existing_workspace_path() {
        let temp = tempdir().unwrap();
        // Simulate a workspace whose directory name contains a '-'.
        let workspace = temp.path().join("Users/dn/codeai/fast-resume");
        fs::create_dir_all(&workspace).unwrap();
        let root = format!("{}/", temp.path().display());

        assert_eq!(
            decode_slug_in("-Users-dn-codeai-fast-resume", &root, '/'),
            workspace.display().to_string()
        );
        // The literal all-separator reading would be .../fast/resume, which
        // must NOT exist for the assertion above to be meaningful.
        assert!(!temp.path().join("Users/dn/codeai/fast/resume").exists());
    }

    #[test]
    fn decode_slug_returns_empty_when_no_interpretation_exists() {
        let temp = tempdir().unwrap();
        let root = format!("{}/", temp.path().display());

        assert_eq!(decode_slug_in("-Users-dn-no-such-dir", &root, '/'), "");
        assert_eq!(decode_slug_in("", &root, '/'), "");
    }

    #[test]
    fn falls_back_to_preview_title_when_no_user_message() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp
            .path()
            .join("projects")
            .join("-Users-dn-test-workspace")
            .join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let session_id = "session-abc";
        let session_file = sessions_dir.join(format!("{session_id}.jsonl"));
        let meta_file = sessions_dir.join(format!("{session_id}.jsonl.meta"));

        write_jsonl(
            &session_file,
            &[json!({"role": "system", "content": "System prompt only"})],
        );
        write_meta(
            &meta_file,
            &session_meta(session_id, 0, "System prompt only session"),
        );

        let adapter = ReasonixAdapter::new(temp.path().join("projects"));
        let sessions = adapter.find_sessions();
        // No user messages, but meta has preview — should still produce a session
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "System prompt only session");
        assert_eq!(sessions[0].message_count, 0);
    }

    #[test]
    fn excludes_sidecar_files() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp
            .path()
            .join("projects")
            .join("-Users-dn-test")
            .join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let session_id = "test-session";
        let session_file = sessions_dir.join(format!("{session_id}.jsonl"));
        let meta_file = sessions_dir.join(format!("{session_id}.jsonl.meta"));
        let events_file = sessions_dir.join(format!("{session_id}.events.jsonl"));
        let conflicts_file = sessions_dir.join(format!("{session_id}.conflicts.jsonl"));

        write_jsonl(
            &session_file,
            &[json!({"role": "user", "content": "Hello"})],
        );
        write_meta(&meta_file, &session_meta(session_id, 1, "Hello"));
        // Write sidecars (should be excluded)
        fs::write(&events_file, "{}").unwrap();
        fs::write(&conflicts_file, "{}").unwrap();

        let adapter = ReasonixAdapter::new(temp.path().join("projects"));
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn incremental_detects_mtime_change_via_meta_file() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp
            .path()
            .join("projects")
            .join("-Users-dn-test")
            .join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let session_id = "test-123";
        let session_file = sessions_dir.join(format!("{session_id}.jsonl"));
        let meta_file = sessions_dir.join(format!("{session_id}.jsonl.meta"));

        write_jsonl(
            &session_file,
            &[json!({"role": "user", "content": "Original"})],
        );
        write_meta(&meta_file, &session_meta(session_id, 1, "Original"));

        let adapter = ReasonixAdapter::new(temp.path().join("projects"));
        let known_mtime = reasonix_session_mtime(&session_file);
        thread::sleep(Duration::from_millis(1100));

        // Update both meta and jsonl
        write_meta(&meta_file, &session_meta(session_id, 2, "Updated"));
        write_jsonl(
            &session_file,
            &[
                json!({"role": "user", "content": "Original"}),
                json!({"role": "user", "content": "Follow-up"}),
            ],
        );

        let mut known = KnownSessions::new();
        known.insert(
            ("reasonix".to_string(), session_id.to_string()),
            known_mtime,
        );

        let scan = adapter.find_sessions_incremental(&known);
        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].title, "Updated");
        assert_eq!(scan.new_or_modified[0].message_count, 2);
    }

    #[test]
    fn incremental_detects_deleted_sessions() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp
            .path()
            .join("projects")
            .join("-Users-dn-test")
            .join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let session_id = "delete-me";
        let session_file = sessions_dir.join(format!("{session_id}.jsonl"));
        let meta_file = sessions_dir.join(format!("{session_id}.jsonl.meta"));

        write_jsonl(
            &session_file,
            &[json!({"role": "user", "content": "Delete me"})],
        );
        write_meta(&meta_file, &session_meta(session_id, 1, "Delete me"));

        let adapter = ReasonixAdapter::new(temp.path().join("projects"));
        let known_mtime = reasonix_session_mtime(&session_file);

        // Delete the session
        fs::remove_file(&session_file).unwrap();
        fs::remove_file(&meta_file).unwrap();

        let mut known = KnownSessions::new();
        known.insert(
            ("reasonix".to_string(), session_id.to_string()),
            known_mtime,
        );

        let scan = adapter.find_sessions_incremental(&known);
        assert!(scan.new_or_modified.is_empty());
        assert_eq!(scan.deleted_ids, vec![session_id]);
    }

    #[test]
    fn malformed_meta_does_not_block_other_updates() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp
            .path()
            .join("projects")
            .join("-Users-dn-test")
            .join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();

        // Malformed meta
        let malformed_file = sessions_dir.join("bad.jsonl");
        let malformed_meta = sessions_dir.join("bad.jsonl.meta");
        write_jsonl(&malformed_file, &[json!({"role": "user", "content": "Hi"})]);
        fs::write(&malformed_meta, "{invalid json}").unwrap();

        // Good session
        let good_id = "good-session";
        let good_file = sessions_dir.join(format!("{good_id}.jsonl"));
        let good_meta = sessions_dir.join(format!("{good_id}.jsonl.meta"));
        write_jsonl(
            &good_file,
            &[json!({"role": "user", "content": "Good prompt"})],
        );
        write_meta(&good_meta, &session_meta(good_id, 1, "Good prompt"));

        let adapter = ReasonixAdapter::new(temp.path().join("projects"));
        let mut known = KnownSessions::new();
        known.insert(("reasonix".to_string(), "bad".to_string()), 0.0);
        known.insert(("reasonix".to_string(), good_id.to_string()), 0.0);

        let scan = adapter.find_sessions_incremental(&known);
        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].id, good_id);
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn resume_command_reencodes_decoded_directory() {
        // A decoded workspace path (as stored by parse_session) must be
        // re-encoded into the slug form to locate the session file.
        let session = Session::new(
            "test-session-123",
            "reasonix",
            "Test session",
            "/Users/dn/codeai/my-project",
            chrono::Local::now(),
            "",
            0,
        );
        let adapter = ReasonixAdapter::default();
        let cmd = adapter.resume_command(&session, false);
        assert_eq!(cmd[0], "reasonix");
        assert_eq!(cmd[1], "--resume");
        assert!(
            cmd[2].contains("projects/-Users-dn-codeai-my-project/sessions/test-session-123.jsonl")
        );
    }

    #[test]
    fn resume_command_accepts_legacy_slug_directory() {
        // Sessions indexed by an older version may carry the raw slug in the
        // directory field; the resume path must still be correct.
        let session = Session::new(
            "test-session-123",
            "reasonix",
            "Test session",
            "-Users-dn-codeai-my-project",
            chrono::Local::now(),
            "",
            0,
        );
        let adapter = ReasonixAdapter::default();
        let cmd = adapter.resume_command(&session, false);
        assert_eq!(cmd[0], "reasonix");
        assert_eq!(cmd[1], "--resume");
        assert!(
            cmd[2].contains("projects/-Users-dn-codeai-my-project/sessions/test-session-123.jsonl")
        );
    }

    #[test]
    fn resume_command_scans_when_directory_is_empty() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp
            .path()
            .join("projects")
            .join("-Users-dn-fake-project")
            .join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let session_id = "resume-me";
        write_jsonl(
            &sessions_dir.join(format!("{session_id}.jsonl")),
            &[json!({"role": "user", "content": "Hi"})],
        );
        write_meta(
            &sessions_dir.join(format!("{session_id}.jsonl.meta")),
            &session_meta(session_id, 1, "Hi"),
        );

        let session = Session::new(
            session_id,
            "reasonix",
            "Hi",
            "",
            chrono::Local::now(),
            "",
            1,
        );
        let adapter = ReasonixAdapter::new(temp.path().join("projects"));
        let cmd = adapter.resume_command(&session, false);
        assert_eq!(cmd[0], "reasonix");
        assert_eq!(cmd[1], "--resume");
        assert!(
            cmd[2].ends_with("projects/-Users-dn-fake-project/sessions/resume-me.jsonl"),
            "unexpected resume path: {}",
            cmd[2]
        );
    }
}
