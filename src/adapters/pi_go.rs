use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::shared::{
    IncrementalParse, failed_incremental_scan, incremental_from_files,
    incremental_from_files_streaming, parse_datetime, raw_stats_for_tree, string_at,
};
use super::{Adapter, IncrementalScan, KnownSessions, SessionCallback};

type SessionDirs = HashMap<String, (PathBuf, f64)>;

#[derive(Debug, Clone)]
pub struct PiGoAdapter {
    sessions_dir: PathBuf,
}

impl Default for PiGoAdapter {
    fn default() -> Self {
        Self {
            sessions_dir: config::pi_go_sessions_dir(),
        }
    }
}

impl PiGoAdapter {
    #[allow(dead_code)]
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir }
    }

    fn scan_session_dirs(&self) -> Option<(SessionDirs, bool)> {
        let mut current = HashMap::new();
        if !self.sessions_dir.exists() {
            return Some((current, true));
        }
        if !self.sessions_dir.is_dir() {
            return None;
        }
        let mut complete = true;
        let entries = fs::read_dir(&self.sessions_dir).ok()?;
        for entry in entries {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !path.is_dir() || name.starts_with("sessions-trash-") {
                continue;
            }
            let mtime = file_mtime_seconds(&path.join("meta.json"))
                .max(file_mtime_seconds(&path.join("events.jsonl")));
            current.insert(name.to_string(), (path, mtime));
        }
        Some((current, complete))
    }

    fn parse_session(&self, dir: &Path) -> Option<Session> {
        let meta: Value = serde_json::from_reader(File::open(dir.join("meta.json")).ok()?).ok()?;
        let id = string_at(&meta, &["id"]);
        let id = if id.is_empty() {
            dir.file_name()?.to_string_lossy().into_owned()
        } else {
            id
        };
        let directory = string_at(&meta, &["workDir"]);
        let timestamp = parse_datetime(&string_at(&meta, &["updatedAt"]))
            .or_else(|| parse_datetime(&string_at(&meta, &["createdAt"])))
            .unwrap_or_else(|| file_timestamp(&dir.join("meta.json")));

        let events = File::open(dir.join("events.jsonl")).ok()?;
        let mut messages = Vec::new();
        let mut first_user = None;
        let mut message_count = 0;
        for line in BufReader::new(events).lines().map_while(Result::ok) {
            let Ok(event) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let author = string_at(&event, &["Author"]);
            if author != "user" && author != "pi" {
                continue;
            }
            let texts: Vec<String> = event
                .get("Content")
                .and_then(|content| content.get("parts"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .filter(|text| !text.is_empty())
                .map(ToString::to_string)
                .collect();
            if texts.is_empty() {
                continue;
            }
            if author == "user" {
                message_count += 1;
                if first_user.is_none() {
                    first_user = texts.first().cloned();
                }
            }
            messages.push(format!(
                "{}{}",
                if author == "user" { "» " } else { "  " },
                texts.join("\n")
            ));
        }
        if messages.is_empty() {
            return None;
        }
        let title = first_user.unwrap_or_else(|| "(no messages)".to_string());
        let mut session = Session::new(
            id,
            self.name(),
            truncate_title(&title, 100, true),
            directory,
            timestamp,
            messages.join("\n\n"),
            message_count,
        );
        session.mtime = file_mtime_seconds(&dir.join("meta.json"))
            .max(file_mtime_seconds(&dir.join("events.jsonl")));
        Some(session)
    }

    fn parse_session_incremental(&self, dir: &Path) -> IncrementalParse {
        let events = dir.join("events.jsonl");
        if !events.is_file() || !dir.join("meta.json").is_file() {
            return IncrementalParse::Retain;
        }
        super::shared::incremental_parse_jsonl(&events, || self.parse_session(dir))
    }
}

impl Adapter for PiGoAdapter {
    fn name(&self) -> &'static str {
        "pi-go"
    }

    fn find_sessions(&self) -> Vec<Session> {
        let Some((dirs, _)) = self.scan_session_dirs() else {
            return Vec::new();
        };
        dirs.into_values()
            .filter_map(|(dir, _)| self.parse_session(&dir))
            .collect()
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        let Some((dirs, complete)) = self.scan_session_dirs() else {
            return failed_incremental_scan(self.name());
        };
        let mut scan = incremental_from_files(self.name(), known, dirs, |dir| {
            self.parse_session_incremental(dir)
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
        let Some((dirs, complete)) = self.scan_session_dirs() else {
            return failed_incremental_scan(self.name());
        };
        let mut scan = incremental_from_files_streaming(
            self.name(),
            known,
            dirs,
            |dir| self.parse_session_incremental(dir),
            on_session,
        );
        if !complete {
            scan.deleted_ids.clear();
        }
        scan
    }

    fn resume_command(&self, session: &Session, _yolo: bool) -> Vec<String> {
        vec![
            "pi".to_string(),
            "--session".to_string(),
            session.id.clone(),
        ]
    }

    fn raw_stats(&self) -> RawAdapterStats {
        raw_stats_for_tree(self.name(), &self.sessions_dir, "jsonl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use tempfile::TempDir;

    fn write_session(temp: &TempDir) -> PathBuf {
        let dir = temp.path().join("260709-0908-test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("meta.json"),
            r#"{"id":"260709-0908-test","workDir":"/repo/app","updatedAt":"2026-07-09T13:12:27+02:00"}"#,
        )
        .unwrap();
        fs::write(
            dir.join("events.jsonl"),
            "{\"Author\":\"user\",\"Content\":{\"parts\":[{\"text\":\"Fix the parser\"}]}}\n{\"Author\":\"pi\",\"Content\":{\"parts\":[{\"text\":\"Done\"},{\"functionCall\":{}}]}}\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn parses_pi_go_session() {
        let temp = TempDir::new().unwrap();
        let session = PiGoAdapter::new(temp.path().to_path_buf())
            .find_sessions()
            .into_iter()
            .next();
        assert!(session.is_none());
        write_session(&temp);
        let session = PiGoAdapter::new(temp.path().to_path_buf())
            .find_sessions()
            .remove(0);
        assert_eq!(session.agent, "pi-go");
        assert_eq!(session.title, "Fix the parser");
        assert!(session.content.contains("Done"));
        assert!(!session.content.contains("functionCall"));
    }

    #[test]
    fn excludes_trash_sessions() {
        let temp = TempDir::new().unwrap();
        let trash = temp.path().join("sessions-trash-20260808");
        fs::create_dir_all(&trash).unwrap();
        let adapter = PiGoAdapter::new(temp.path().to_path_buf());
        assert!(adapter.find_sessions().is_empty());
    }

    #[test]
    fn resume_uses_pi_cli() {
        let adapter = PiGoAdapter::new(PathBuf::new());
        let session = Session::new("id", "pi-go", "title", "/repo", Local::now(), "", 0);
        assert_eq!(
            adapter.resume_command(&session, false),
            ["pi", "--session", "id"]
        );
    }
}
