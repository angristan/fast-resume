use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use rayon::prelude::*;
use serde_json::Value;
use walkdir::WalkDir;

use crate::harness::{FieldPath, Harness, Role};
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    JsonlHealth, JsonlHealthTracker, SessionFileScan, incremental_parse_with_health,
    incremental_scan, parse_datetime, timestamp_from_ms, timestamp_from_seconds,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

/// Epoch values above this are milliseconds; below, seconds. 10^11 seconds
/// is in the year 5138, and 10^11 milliseconds is in 1973.
const EPOCH_MS_THRESHOLD: i64 = 100_000_000_000;

/// Reads a custom harness declared in the config file: one JSONL file per
/// session, mapped to session fields by the harness format.
#[derive(Debug, Clone, Copy)]
pub struct CustomAdapter {
    harness: &'static Harness,
}

impl CustomAdapter {
    pub fn new(harness: &'static Harness) -> Self {
        Self { harness }
    }

    fn incremental(
        &self,
        known: &KnownSessions,
        on_session: Option<&mut SessionCallback<'_>>,
    ) -> IncrementalScan {
        incremental_scan(
            self.name(),
            known,
            self.scan_session_files(),
            |path| {
                let (session, health) = self.parse_file(path);
                incremental_parse_with_health(health, session, |_| true)
            },
            on_session,
        )
    }

    /// Session files keyed by file stem. Using the stem avoids opening
    /// unchanged files on every refresh, so it should match the session id.
    fn scan_session_files(&self) -> Option<SessionFileScan> {
        let dir = &self.harness.dir;
        let mut files = HashMap::new();
        if !dir.exists() {
            return Some((files, true));
        }
        if !dir.is_dir() {
            return None;
        }
        let mut complete = true;
        for entry in self.walk() {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let path = entry.path();
            if !self.is_session_file(path) {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            files.insert(
                stem.to_string(),
                (path.to_path_buf(), file_mtime_seconds(path)),
            );
        }
        Some((files, complete))
    }

    fn walk(&self) -> walkdir::IntoIter {
        let walk = WalkDir::new(&self.harness.dir).min_depth(1);
        if self.harness.recursive {
            walk.into_iter()
        } else {
            walk.max_depth(1).into_iter()
        }
    }

    fn is_session_file(&self, path: &Path) -> bool {
        path.is_file()
            && path.extension().and_then(|ext| ext.to_str())
                == Some(self.harness.extension.as_str())
    }

    fn session_paths(&self) -> Vec<PathBuf> {
        self.walk()
            .filter_map(Result::ok)
            .map(walkdir::DirEntry::into_path)
            .filter(|path| self.is_session_file(path))
            .collect()
    }

    /// Parse a session file and classify its JSONL health in one pass.
    fn parse_file(&self, path: &Path) -> (Option<Session>, JsonlHealth) {
        let Ok(file) = fs::File::open(path) else {
            return (None, JsonlHealth::Invalid);
        };
        let mut health = JsonlHealthTracker::default();
        let mut transcript = Transcript::default();
        for line in BufReader::new(file).lines() {
            // Unreadable bytes or invalid UTF-8: keep the indexed copy.
            let Ok(line) = line else {
                return (transcript.into_session(self, path), JsonlHealth::Invalid);
            };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(&line) {
                Ok(row) => {
                    health.valid_row();
                    transcript.add_row(self.harness, &row);
                }
                Err(_) => health.malformed_row(),
            }
        }
        (transcript.into_session(self, path), health.finish())
    }
}

impl Adapter for CustomAdapter {
    fn name(&self) -> &'static str {
        self.harness.name.as_str()
    }

    fn supports_yolo(&self) -> bool {
        self.harness.yolo_resume.is_some()
    }

    fn find_sessions(&self) -> Vec<Session> {
        self.session_paths()
            .par_iter()
            .filter_map(|path| self.parse_file(path).0)
            .collect()
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        self.incremental(known, None)
    }

    fn find_sessions_incremental_streaming(
        &self,
        known: &KnownSessions,
        on_session: &mut SessionCallback<'_>,
    ) -> IncrementalScan {
        self.incremental(known, Some(on_session))
    }

    fn resume_command(&self, session: &Session, yolo: bool) -> Vec<String> {
        self.harness
            .resume_command(&session.id, &session.directory, yolo)
    }

    fn raw_stats(&self) -> RawAdapterStats {
        let dir = &self.harness.dir;
        let paths = if dir.is_dir() {
            self.session_paths()
        } else {
            Vec::new()
        };
        RawAdapterStats {
            agent: self.name(),
            data_dir: dir.display().to_string(),
            available: dir.is_dir(),
            file_count: paths.len(),
            total_bytes: paths
                .iter()
                .filter_map(|path| path.metadata().ok())
                .map(|metadata| metadata.len())
                .sum(),
        }
    }
}

/// Session state accumulated row by row.
#[derive(Default)]
struct Transcript {
    id: Option<String>,
    directory: Option<String>,
    title: Option<String>,
    first_user_message: Option<String>,
    messages: Vec<String>,
    message_count: usize,
    first_timestamp: Option<DateTime<Local>>,
    last_activity: Option<DateTime<Local>>,
}

impl Transcript {
    fn add_row(&mut self, harness: &Harness, row: &Value) {
        let format = &harness.format;
        let Some(kind) = format.kind.string(row) else {
            return;
        };
        let Some(mapping) = format.rows.get(&kind) else {
            return;
        };
        let timestamp = mapping
            .timestamp
            .as_ref()
            .or(format.timestamp.as_ref())
            .and_then(|path| row_timestamp(path, row));
        if self.first_timestamp.is_none() {
            self.first_timestamp = timestamp;
        }

        if self.id.is_none()
            && let Some(id) = mapping
                .id
                .as_ref()
                .and_then(|path| non_empty(path.string(row)))
        {
            self.id = Some(id);
        }
        // The latest directory and title win: agents record changes as they happen.
        if let Some(cwd) = mapping
            .cwd
            .as_ref()
            .and_then(|path| non_empty(path.string(row)))
        {
            self.directory = Some(cwd);
        }
        if let Some(title) = mapping
            .title
            .as_ref()
            .and_then(|path| non_empty(path.string(row)))
        {
            self.title = Some(title.trim().to_string());
        }

        let Some(text_path) = &mapping.text else {
            return;
        };
        let role = match &mapping.role {
            Some(role_path) => {
                let Some(role) = role_path
                    .string(row)
                    .and_then(|role| mapping.roles.get(&role).copied())
                else {
                    return;
                };
                Some(role)
            }
            None => None,
        };
        let texts = text_path.texts(row);
        if texts.is_empty() {
            return;
        }
        if role.is_some()
            && let Some(timestamp) = timestamp
            && self.last_activity.is_none_or(|current| timestamp > current)
        {
            self.last_activity = Some(timestamp);
        }
        if role == Some(Role::User) {
            self.message_count += 1;
            if self.first_user_message.is_none() {
                self.first_user_message = Some(texts[0].to_string());
            }
        }
        // Rows without a role (summaries, notes) are indexed like assistant
        // text but do not count as messages.
        let prefix = if role == Some(Role::User) {
            "» "
        } else {
            "  "
        };
        self.messages
            .extend(texts.into_iter().map(|text| format!("{prefix}{text}")));
    }

    fn into_session(self, adapter: &CustomAdapter, path: &Path) -> Option<Session> {
        if self.messages.is_empty() {
            return None;
        }
        let id = self.id.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string()
        });
        let title = self
            .title
            .or(self.first_user_message)
            .unwrap_or_else(|| "(no messages)".to_string());
        let mut session = Session::new(
            id,
            adapter.name(),
            truncate_title(&title, 100, true),
            self.directory.unwrap_or_default(),
            self.last_activity
                .or(self.first_timestamp)
                .unwrap_or_else(|| file_timestamp(path)),
            self.messages.join("\n\n"),
            self.message_count,
        );
        session.mtime = file_mtime_seconds(path);
        Some(session)
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

/// Epoch seconds, epoch milliseconds, or an RFC 3339 string.
fn row_timestamp(path: &FieldPath, row: &Value) -> Option<DateTime<Local>> {
    path.values(row).into_iter().find_map(|value| match value {
        Value::Number(number) => {
            let value = number
                .as_i64()
                .or_else(|| number.as_f64().map(|value| value as i64))?;
            if value >= EPOCH_MS_THRESHOLD {
                timestamp_from_ms(Some(value))
            } else {
                timestamp_from_seconds(Some(value))
            }
        }
        Value::String(text) => parse_datetime(text),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;

    use serde_json::json;
    use tempfile::TempDir;

    use crate::harness::parse_config;

    use super::*;

    fn harness(dir: &Path, extra: &str) -> &'static Harness {
        let config = format!(
            r#"
[[harness]]
name = "piti"
dir = {dir:?}
resume = ["piti", "--resume", "{{id}}"]
yolo_resume = ["piti", "--yes-all", "--resume", "{{id}}"]
{extra}

[harness.format]
kind = "type"
timestamp = "timestamp"

[harness.format.rows.session]
id = "id"
cwd = "cwd"

[harness.format.rows.cwd]
cwd = "path"

[harness.format.rows.title]
title = "title"

[harness.format.rows.compaction]
text = "summary"

[harness.format.rows.message]
role = "message.role"
text = "message.content[type=text].text"
"#
        );
        let harnesses = parse_config(&config).expect("valid config");
        Box::leak(Box::new(harnesses.into_iter().next().expect("one harness")))
    }

    fn write_session(dir: &Path, name: &str, rows: &[Value]) -> PathBuf {
        let path = dir.join(name);
        let body: String = rows.iter().map(|row| format!("{row}\n")).collect();
        fs::write(&path, body).expect("write session");
        path
    }

    fn piti_rows(id: &str) -> Vec<Value> {
        vec![
            json!({"seq":0,"timestamp":1_790_000_000_000i64,"type":"session","id":id,"cwd":"/repo/app","version":1}),
            json!({"seq":1,"parent":0,"timestamp":1_790_000_000_100i64,"type":"model_change","provider":"p","model":"m"}),
            json!({"seq":2,"parent":1,"timestamp":1_790_000_001_000i64,"type":"message","message":{"role":"user","content":[{"type":"text","text":"Fix the login bug"}],"timestamp":1}}),
            json!({"seq":3,"parent":2,"timestamp":1_790_000_002_000i64,"type":"message","message":{"role":"assistant","content":[{"type":"thinking","thinking":"secret reasoning"},{"type":"text","text":"Patched the handler"},{"type":"tool_call","id":"c1","name":"bash","arguments":{"command":"rm -rf tmp"}}]}}),
            json!({"seq":4,"parent":3,"timestamp":1_790_000_003_000i64,"type":"message","message":{"role":"tool_result","content":[{"type":"text","text":"tool output stays out"}]}}),
            json!({"seq":5,"parent":4,"timestamp":1_790_000_004_000i64,"type":"usage","model":"m","usage":{}}),
        ]
    }

    #[test]
    fn parses_a_piti_style_session() {
        let temp = TempDir::new().expect("tempdir");
        let adapter = CustomAdapter::new(harness(temp.path(), ""));
        write_session(temp.path(), "s1.jsonl", &piti_rows("s1"));

        let sessions = adapter.find_sessions();
        let [session] = sessions.as_slice() else {
            panic!("one session, got {sessions:?}");
        };
        assert_eq!(session.id, "s1");
        assert_eq!(session.agent, "piti");
        assert_eq!(session.title, "Fix the login bug");
        assert_eq!(session.directory, "/repo/app");
        assert_eq!(session.message_count, 1);
        assert_eq!(
            session.content,
            "» Fix the login bug\n\n  Patched the handler"
        );
        assert_eq!(
            session.timestamp,
            timestamp_from_ms(Some(1_790_000_002_000)).expect("timestamp")
        );
    }

    #[test]
    fn latest_title_and_directory_win_and_unroled_text_is_indexed() {
        let temp = TempDir::new().expect("tempdir");
        let adapter = CustomAdapter::new(harness(temp.path(), ""));
        let mut rows = piti_rows("s1");
        rows.push(json!({"type":"title","title":"first name","timestamp":1}));
        rows.push(json!({"type":"title","title":"login-fix","timestamp":2}));
        rows.push(json!({"type":"cwd","path":"/repo/other","timestamp":3}));
        rows.push(json!({"type":"compaction","summary":"Earlier work summary","timestamp":4}));
        write_session(temp.path(), "s1.jsonl", &rows);

        let session = adapter.find_sessions().pop().expect("session");
        assert_eq!(session.title, "login-fix");
        assert_eq!(session.directory, "/repo/other");
        assert_eq!(session.message_count, 1);
        assert!(session.content.ends_with("\n\n  Earlier work summary"));
    }

    #[test]
    fn falls_back_to_file_stem_and_skips_sessions_without_messages() {
        let temp = TempDir::new().expect("tempdir");
        let adapter = CustomAdapter::new(harness(temp.path(), ""));
        write_session(
            temp.path(),
            "no-header.jsonl",
            &[json!({"type":"message","message":{"role":"user","content":"plain string prompt"}})],
        );
        write_session(temp.path(), "empty.jsonl", &piti_rows("empty")[..2]);

        let sessions = adapter.find_sessions();
        let [session] = sessions.as_slice() else {
            panic!("one session, got {sessions:?}");
        };
        assert_eq!(session.id, "no-header");
        assert_eq!(session.content, "» plain string prompt");
    }

    #[test]
    fn honors_extension_and_recursion() {
        let temp = TempDir::new().expect("tempdir");
        fs::create_dir(temp.path().join("archive")).expect("mkdir");
        write_session(temp.path(), "top.jsonl", &piti_rows("top"));
        write_session(temp.path(), "notes.txt", &piti_rows("notes"));
        write_session(&temp.path().join("archive"), "old.jsonl", &piti_rows("old"));

        let flat = CustomAdapter::new(harness(temp.path(), ""));
        let ids: HashSet<_> = flat.find_sessions().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, HashSet::from(["top".to_string()]));

        let recursive = CustomAdapter::new(harness(temp.path(), "recursive = true"));
        let ids: HashSet<_> = recursive
            .find_sessions()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, HashSet::from(["top".to_string(), "old".to_string()]));

        let stats = recursive.raw_stats();
        assert!(stats.available);
        assert_eq!(stats.file_count, 2);
    }

    #[test]
    fn incremental_refresh_tracks_changes_and_deletions() {
        let temp = TempDir::new().expect("tempdir");
        let adapter = CustomAdapter::new(harness(temp.path(), ""));
        let kept = write_session(temp.path(), "kept.jsonl", &piti_rows("kept"));
        write_session(temp.path(), "new.jsonl", &piti_rows("new"));
        fs::File::options()
            .write(true)
            .open(&kept)
            .and_then(|file| {
                file.set_times(
                    fs::FileTimes::new().set_modified(
                        std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000),
                    ),
                )
            })
            .expect("set mtime");

        let known = KnownSessions::from([
            (("piti".to_string(), "kept".to_string()), 1_000.0),
            (("piti".to_string(), "gone".to_string()), 1_000.0),
            (("claude".to_string(), "other".to_string()), 1_000.0),
        ]);
        let scan = adapter.find_sessions_incremental(&known);
        let changed: Vec<_> = scan.new_or_modified.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(changed, ["new"]);
        assert_eq!(scan.deleted_ids, ["gone"]);
    }

    #[test]
    fn retains_indexed_sessions_while_a_file_is_being_written() {
        let temp = TempDir::new().expect("tempdir");
        let adapter = CustomAdapter::new(harness(temp.path(), ""));
        let path = write_session(temp.path(), "s1.jsonl", &piti_rows("s1"));
        let mut body = fs::read_to_string(&path).expect("read");
        body.push_str("{\"type\":\"message\",\"mess");
        fs::write(&path, body).expect("write");

        let known = KnownSessions::from([(("piti".to_string(), "s1".to_string()), 1.0)]);
        let scan = adapter.find_sessions_incremental(&known);
        assert!(scan.new_or_modified.is_empty());
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn missing_directory_is_an_empty_complete_scan() {
        let temp = TempDir::new().expect("tempdir");
        let adapter = CustomAdapter::new(harness(&temp.path().join("missing"), ""));
        let known = KnownSessions::from([(("piti".to_string(), "s1".to_string()), 1.0)]);
        assert_eq!(
            adapter.find_sessions_incremental(&known).deleted_ids,
            ["s1"]
        );
        assert!(!adapter.raw_stats().available);
    }

    #[test]
    fn builds_resume_commands() {
        let temp = TempDir::new().expect("tempdir");
        let adapter = CustomAdapter::new(harness(temp.path(), ""));
        let session = Session::new("s1", "piti", "t", "/repo", Local::now(), "c", 1);
        assert!(adapter.supports_yolo());
        assert_eq!(
            adapter.resume_command(&session, false),
            ["piti", "--resume", "s1"]
        );
        assert_eq!(
            adapter.resume_command(&session, true),
            ["piti", "--yes-all", "--resume", "s1"]
        );
    }

    #[test]
    fn reads_epoch_seconds_and_rfc3339_timestamps() {
        let path = FieldPath::parse("t").expect("path");
        assert_eq!(
            row_timestamp(&path, &json!({"t": 1_790_000_000})),
            timestamp_from_seconds(Some(1_790_000_000))
        );
        assert_eq!(
            row_timestamp(&path, &json!({"t": 1_790_000_000_000i64})),
            timestamp_from_seconds(Some(1_790_000_000))
        );
        assert_eq!(
            row_timestamp(&path, &json!({"t": "2026-10-20T10:00:00Z"})),
            parse_datetime("2026-10-20T10:00:00Z")
        );
    }
}
