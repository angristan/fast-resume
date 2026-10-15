//! fx stores one directory per session under `~/.fx/sessions/<id>/`.
//! `session.json` holds metadata and `events.jsonl` is the append-only event
//! log, which always starts at sequence 1 and so contains the full history.
//! Older (schema v3) sessions keep their title in `display.json`.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    IncrementalParse, build_resume_command, incremental_parse_jsonl, incremental_scan,
    json_file_has_parse_errors, timestamp_from_ms, value_i64_at,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

type SessionFiles = HashMap<String, (PathBuf, f64)>;

const FX_EVENTS_FILE: &str = "events.jsonl";
const FX_METADATA_FILE: &str = "session.json";
const FX_DISPLAY_FILE: &str = "display.json";

#[derive(Debug, Clone)]
pub struct FxAdapter {
    sessions_dir: PathBuf,
}

impl Default for FxAdapter {
    fn default() -> Self {
        Self {
            sessions_dir: config::fx_sessions_dir(),
        }
    }
}

impl FxAdapter {
    #[allow(dead_code)]
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir }
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
            |path| self.parse_incremental(path),
            on_session,
        )
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
        for entry in fs::read_dir(&self.sessions_dir).ok()? {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let session_dir = entry.path();
            if !session_dir.is_dir() {
                continue;
            }
            let Some(id) = session_dir
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|id| !id.is_empty() && !id.starts_with('.'))
            else {
                continue;
            };
            let events = session_dir.join(FX_EVENTS_FILE);
            if !events.is_file() {
                continue;
            }
            files.insert(id.to_string(), (events, fx_session_mtime(&session_dir)));
        }
        Some((files, complete))
    }

    fn parse_session(&self, events_path: &Path) -> Option<Session> {
        let session_dir = events_path.parent()?;
        let metadata: Value =
            serde_json::from_slice(&fs::read(session_dir.join(FX_METADATA_FILE)).ok()?).ok()?;
        if metadata.get("subagent_child").and_then(Value::as_bool) == Some(true) {
            return None;
        }
        let id = metadata
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(ToString::to_string)
            .or_else(|| Some(session_dir.file_name()?.to_str()?.to_string()))?;
        let directory = ["workspace_root", "origin_workspace_root"]
            .into_iter()
            .find_map(|field| non_empty_str(&metadata, field))
            .unwrap_or_default()
            .to_string();

        let file = fs::File::open(events_path).ok()?;
        let mut messages: Vec<(bool, String)> = Vec::new();
        let mut user_turns = 0;
        let mut last_event_ms = None;
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(record) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let Some(timestamp) = value_i64_at(&record, &["timestamp_ms"]) {
                last_event_ms = last_event_ms.max(Some(timestamp));
            }
            let Some(event) = record.get("event") else {
                continue;
            };
            if let Some(text) = event_text(event, "user", "text") {
                user_turns += 1;
                messages.push((true, text));
            } else if let Some(text) = event_text(event, "steering", "text") {
                messages.push((true, text));
            } else if let Some(text) = event_text(event, "assistant", "text")
                .or_else(|| event_text(event, "interrupted", "partial_text"))
            {
                match messages.last_mut() {
                    // One turn records an assistant event around every tool
                    // call; keep the reply together in the preview.
                    Some((false, current)) => {
                        current.push_str("\n\n");
                        current.push_str(&text);
                    }
                    _ => messages.push((false, text)),
                }
            }
        }

        let first_user = messages
            .iter()
            .find(|(user, _)| *user)
            .map(|(_, text)| text.clone())?;
        let display: Option<Value> = fs::read(session_dir.join(FX_DISPLAY_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let title = non_empty_str(&metadata, "title")
            .or_else(|| display.as_ref().and_then(|d| non_empty_str(d, "title")))
            .unwrap_or(&first_user);
        let timestamp = timestamp_from_ms(value_i64_at(&metadata, &["updated_at_ms"]))
            .or_else(|| timestamp_from_ms(last_event_ms))
            .or_else(|| timestamp_from_ms(value_i64_at(&metadata, &["created_at_ms"])))
            .unwrap_or_else(|| file_timestamp(events_path));
        let content = messages
            .into_iter()
            .map(|(user, text)| format!("{}{}", if user { "» " } else { "  " }, text))
            .collect::<Vec<_>>()
            .join("\n\n");

        let mut session = Session::new(
            id,
            self.name(),
            truncate_title(title, 100, true),
            directory,
            timestamp,
            content,
            user_turns,
        );
        session.mtime = fx_session_mtime(session_dir);
        Some(session)
    }

    fn parse_incremental(&self, path: &Path) -> IncrementalParse {
        let metadata = path
            .parent()
            .unwrap_or(Path::new(""))
            .join(FX_METADATA_FILE);
        // A missing or half-written session.json is usually a write in
        // progress; keep the indexed copy until it is readable again.
        if json_file_has_parse_errors(&metadata) {
            return IncrementalParse::Retain;
        }
        incremental_parse_jsonl(path, || self.parse_session(path))
    }
}

impl Adapter for FxAdapter {
    fn name(&self) -> &'static str {
        "fx"
    }

    fn find_sessions(&self) -> Vec<Session> {
        self.scan_session_files()
            .map(|(files, _)| {
                files
                    .into_values()
                    .filter_map(|(path, _)| self.parse_session(&path))
                    .collect()
            })
            .unwrap_or_default()
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
        // fx session ids can start with `-`, so pass them through `--id`.
        // fx has no permission flag for interactive resume, so yolo is a no-op.
        build_resume_command("fx", &[], yolo, &["resume", "--id"], &session.id)
    }

    fn raw_stats(&self) -> RawAdapterStats {
        let data_dir = self.sessions_dir.display().to_string();
        let Ok(entries) = fs::read_dir(&self.sessions_dir) else {
            return RawAdapterStats {
                agent: self.name(),
                data_dir,
                available: false,
                file_count: 0,
                total_bytes: 0,
            };
        };
        let (file_count, total_bytes) = entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.path().join(FX_EVENTS_FILE).metadata().ok())
            .fold((0, 0), |(files, bytes), metadata| {
                (files + 1, bytes + metadata.len())
            });
        RawAdapterStats {
            agent: self.name(),
            data_dir,
            available: true,
            file_count,
            total_bytes,
        }
    }
}

fn fx_session_mtime(session_dir: &Path) -> f64 {
    [FX_EVENTS_FILE, FX_METADATA_FILE, FX_DISPLAY_FILE]
        .into_iter()
        .map(|file| file_mtime_seconds(&session_dir.join(file)))
        .fold(0.0, f64::max)
}

fn event_text(event: &Value, kind: &str, field: &str) -> Option<String> {
    event
        .get(kind)?
        .get(field)?
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

fn non_empty_str<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use tempfile::{TempDir, tempdir};

    use super::*;

    fn write_session(root: &Path, id: &str, metadata: Value, events: &[Value]) -> PathBuf {
        let session_dir = root.join(id);
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(session_dir.join(FX_METADATA_FILE), metadata.to_string()).unwrap();
        let events_path = session_dir.join(FX_EVENTS_FILE);
        fs::write(
            &events_path,
            events
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        events_path
    }

    fn metadata(id: &str) -> Value {
        json!({
            "schema_version": 4,
            "id": id,
            "origin_workspace_root": "/work/origin",
            "workspace_root": "/work/fx",
            "created_at_ms": 1_792_043_605_269i64,
            "updated_at_ms": 1_792_043_664_676i64,
            "title": "Add fx support",
            "subagent_child": false
        })
    }

    fn event(seq: u64, event: Value) -> Value {
        json!({"schema_version": 3, "seq": seq, "timestamp_ms": 1_792_043_664_676i64, "event": event})
    }

    fn conversation() -> Vec<Value> {
        vec![
            event(
                1,
                json!({"user": {"text": "index fx sessions", "images": [], "work_id": null}}),
            ),
            event(
                2,
                json!({"assistant": {"text": "Reading the store.", "standalone_response": false}}),
            ),
            event(
                3,
                json!({"tool_call": {"call_id": "c1", "tool_name": "shell", "arguments_json": "{\"command\":\"secret tool args\"}"}}),
            ),
            event(
                4,
                json!({"tool_result": {"call_id": "c1", "tool_name": "shell", "preview": "large tool output"}}),
            ),
            event(
                5,
                json!({"assistant": {"text": "Added the adapter.", "standalone_response": false}}),
            ),
            event(6, json!({"steering": {"text": "also update docs"}})),
            event(
                7,
                json!({"interrupted": {"reason": "cancelled", "partial_text": "Updating docs"}}),
            ),
            event(
                8,
                json!({"context_checkpoint": {"covers_through_seq": 7, "summary": "compaction summary"}}),
            ),
            event(9, json!({"turn_completed": {"files": []}})),
        ]
    }

    fn adapter(temp: &TempDir) -> FxAdapter {
        FxAdapter::new(temp.path().to_path_buf())
    }

    #[test]
    fn parses_metadata_and_conversation_text() {
        let temp = tempdir().unwrap();
        let id = "-H_3CwaOxcI3";
        write_session(temp.path(), id, metadata(id), &conversation());

        let sessions = adapter(&temp).find_sessions();

        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.id, id);
        assert_eq!(session.agent, "fx");
        assert_eq!(session.title, "Add fx support");
        assert_eq!(session.directory, "/work/fx");
        assert_eq!(session.message_count, 1);
        assert_eq!(session.timestamp.timestamp_millis(), 1_792_043_664_676);
        for expected in [
            "» index fx sessions",
            "Reading the store.\n\nAdded the adapter.",
            "» also update docs",
            "Updating docs",
        ] {
            assert!(session.content.contains(expected), "missing {expected}");
        }
        for excluded in [
            "secret tool args",
            "large tool output",
            "compaction summary",
        ] {
            assert!(!session.content.contains(excluded), "indexed {excluded}");
        }
    }

    #[test]
    fn reads_legacy_display_title_and_falls_back_to_first_prompt() {
        let temp = tempdir().unwrap();
        let legacy = json!({
            "schema_version": 3,
            "id": "legacy",
            "workspace_root": "/work/legacy",
            "updated_at_ms": 1_782_838_183_918i64
        });
        let events = conversation();
        let legacy_events = write_session(temp.path(), "legacy", legacy.clone(), &events);
        fs::write(
            legacy_events.parent().unwrap().join(FX_DISPLAY_FILE),
            json!({"schema_version": 1, "title": "Legacy title"}).to_string(),
        )
        .unwrap();
        let mut untitled = legacy;
        untitled["id"] = json!("untitled");
        write_session(temp.path(), "untitled", untitled, &events);

        let sessions: HashMap<_, _> = adapter(&temp)
            .find_sessions()
            .into_iter()
            .map(|session| (session.id.clone(), session))
            .collect();

        assert_eq!(sessions["legacy"].title, "Legacy title");
        assert_eq!(sessions["untitled"].title, "index fx sessions");
    }

    #[test]
    fn skips_subagent_and_empty_sessions() {
        let temp = tempdir().unwrap();
        let mut child = metadata("child");
        child["subagent_child"] = json!(true);
        write_session(temp.path(), "child", child, &conversation());
        write_session(temp.path(), "empty", metadata("empty"), &[]);

        assert!(adapter(&temp).find_sessions().is_empty());
    }

    #[test]
    fn incremental_refresh_updates_changed_sessions_only() {
        let temp = tempdir().unwrap();
        let id = "abc123";
        let events_path = write_session(temp.path(), id, metadata(id), &conversation());
        let adapter = adapter(&temp);

        let first = adapter.find_sessions_incremental(&KnownSessions::new());
        assert_eq!(first.new_or_modified.len(), 1);
        let mtime = first.new_or_modified[0].mtime;
        assert_eq!(mtime, fx_session_mtime(events_path.parent().unwrap()));

        let mut known = KnownSessions::new();
        known.insert(("fx".to_string(), id.to_string()), mtime);
        let unchanged = adapter.find_sessions_incremental(&known);
        assert!(unchanged.new_or_modified.is_empty());
        assert!(unchanged.deleted_ids.is_empty());

        known.insert(("fx".to_string(), id.to_string()), mtime - 10.0);
        let changed = adapter.find_sessions_incremental(&known);
        assert_eq!(changed.new_or_modified.len(), 1);
    }

    #[test]
    fn incremental_refresh_retains_incomplete_sessions_and_deletes_removed_ones() {
        let temp = tempdir().unwrap();
        let torn = write_session(temp.path(), "torn", metadata("torn"), &conversation());
        fs::write(&torn, "{\"schema_version\":3,\"seq\":1,\"event\":{\"user\"").unwrap();
        let half_written = write_session(temp.path(), "meta", metadata("meta"), &conversation());
        fs::write(half_written.parent().unwrap().join(FX_METADATA_FILE), "{").unwrap();

        let mut known = KnownSessions::new();
        for id in ["torn", "meta", "gone"] {
            known.insert(("fx".to_string(), id.to_string()), 1.0);
        }
        let scan = adapter(&temp).find_sessions_incremental(&known);

        assert!(scan.new_or_modified.is_empty());
        assert_eq!(scan.deleted_ids, vec!["gone"]);
    }

    #[test]
    fn missing_store_reports_no_sessions_or_deletions() {
        let temp = tempdir().unwrap();
        let adapter = FxAdapter::new(temp.path().join("missing"));
        let mut known = KnownSessions::new();
        known.insert(("fx".to_string(), "abc".to_string()), 1.0);

        assert!(adapter.find_sessions().is_empty());
        assert!(!adapter.raw_stats().available);
        // A missing store is a complete scan of nothing.
        assert_eq!(
            adapter.find_sessions_incremental(&known).deleted_ids,
            vec!["abc"]
        );
    }

    #[test]
    fn resumes_by_exact_id() {
        let temp = tempdir().unwrap();
        let id = "-H_3CwaOxcI3";
        write_session(temp.path(), id, metadata(id), &conversation());
        let adapter = adapter(&temp);
        let session = adapter.find_sessions().pop().unwrap();

        assert!(!adapter.supports_yolo());
        for yolo in [false, true] {
            assert_eq!(
                adapter.resume_command(&session, yolo),
                ["fx", "resume", "--id", id]
            );
        }
    }
}
