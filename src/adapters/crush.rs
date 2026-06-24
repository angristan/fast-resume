use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Local;
use rusqlite::Connection;
use serde_json::Value;

use crate::config;
use crate::model::{RawAdapterStats, Session, truncate_title};

use super::Adapter;
use super::shared::{normalize_seconds, string_at, timestamp_from_seconds};

#[derive(Debug, Clone)]
pub struct CrushAdapter {
    projects_file: PathBuf,
}

impl Default for CrushAdapter {
    fn default() -> Self {
        Self {
            projects_file: config::crush_projects_file(),
        }
    }
}

impl Adapter for CrushAdapter {
    fn name(&self) -> &'static str {
        "crush"
    }

    fn supports_yolo(&self) -> bool {
        true
    }

    fn find_sessions(&self) -> Vec<Session> {
        let projects = crush_projects(&self.projects_file);
        projects
            .into_iter()
            .flat_map(|(project_path, db_path)| load_crush_db(self.name(), &db_path, &project_path))
            .collect()
    }

    fn resume_command(&self, session: &Session, yolo: bool) -> Vec<String> {
        let mut cmd = vec!["crush".to_string()];
        if yolo {
            cmd.push("--yolo".to_string());
        }
        cmd.extend(["--session".to_string(), session.id.clone()]);
        cmd
    }

    fn raw_stats(&self) -> RawAdapterStats {
        let projects = crush_projects(&self.projects_file);
        RawAdapterStats {
            agent: self.name(),
            data_dir: self
                .projects_file
                .parent()
                .unwrap_or(Path::new(""))
                .display()
                .to_string(),
            available: self.projects_file.exists(),
            file_count: projects.len(),
            total_bytes: projects
                .iter()
                .filter_map(|(_, path)| path.metadata().ok().map(|m| m.len()))
                .sum(),
        }
    }
}

fn load_crush_db(agent: &'static str, db_path: &Path, project_path: &str) -> Vec<Session> {
    let Ok(conn) = Connection::open(db_path) else {
        return Vec::new();
    };
    let mut stmt = match conn.prepare(
        r#"
        SELECT
            s.id, s.title, s.message_count, s.updated_at, s.created_at,
            m.role, m.parts, m.created_at as msg_created_at
        FROM sessions s
        LEFT JOIN messages m ON m.session_id = s.id
        WHERE s.message_count > 0
        ORDER BY s.updated_at DESC, m.created_at ASC
        "#,
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };

    let mut data: HashMap<String, (String, i64, i64)> = HashMap::new();
    let mut messages: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(3)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(4)?.unwrap_or_default(),
            row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };

    for row in rows.filter_map(Result::ok) {
        let (id, title, updated_at, created_at, role, parts) = row;
        data.entry(id.clone())
            .or_insert((title, updated_at, created_at));
        if !role.is_empty() {
            messages.entry(id).or_default().push((role, parts));
        }
    }

    let mut sessions = Vec::new();
    for (id, (title, updated_at, created_at)) in data {
        let mut rendered = Vec::new();
        let mut first_user = String::new();
        for (role, parts) in messages.remove(&id).unwrap_or_default() {
            let text = crush_parts_text(&parts);
            if text.is_empty() {
                continue;
            }
            if role == "user" && first_user.is_empty() && text.chars().count() > 5 {
                first_user = text.clone();
            }
            let prefix = if role == "user" { "» " } else { "  " };
            rendered.push(format!("{prefix}{text}"));
        }
        if rendered.is_empty() || first_user.is_empty() {
            continue;
        }
        let final_title = if title.is_empty() {
            truncate_title(&first_user, 100, true)
        } else {
            title
        };
        let timestamp =
            timestamp_from_seconds(normalize_seconds(updated_at).or(normalize_seconds(created_at)));
        let mut session = Session::new(
            id,
            agent,
            final_title,
            project_path,
            timestamp.unwrap_or_else(Local::now),
            rendered.join("\n\n"),
            rendered.len(),
        );
        session.mtime = session.timestamp.timestamp() as f64;
        sessions.push(session);
    }
    sessions
}

fn crush_projects(projects_file: &Path) -> Vec<(String, PathBuf)> {
    let Ok(data) = serde_json::from_slice::<Value>(&fs::read(projects_file).unwrap_or_default())
    else {
        return Vec::new();
    };
    let mut projects = Vec::new();
    if let Some(items) = data.get("projects").and_then(Value::as_array) {
        for project in items {
            let project_path = string_at(project, &["path"]);
            let data_dir = string_at(project, &["data_dir"]);
            if !data_dir.is_empty() {
                let db = PathBuf::from(data_dir).join("crush.db");
                if db.exists() {
                    projects.push((project_path, db));
                }
            }
        }
    }
    projects
}

fn crush_parts_text(parts_json: &str) -> String {
    let Ok(parts) = serde_json::from_str::<Value>(parts_json) else {
        return String::new();
    };
    let mut out = Vec::new();
    if let Some(parts) = parts.as_array() {
        for part in parts {
            let part_type = string_at(part, &["type"]);
            match part_type.as_str() {
                "text" => {
                    let text = string_at(part, &["data", "text"]);
                    if !text.is_empty() {
                        out.push(text);
                    }
                }
                "tool_result" => {
                    let content = string_at(part, &["data", "content"]);
                    if !content.is_empty() && content.chars().count() < 500 {
                        let name = string_at(part, &["data", "name"]);
                        let name = if name.is_empty() { "tool" } else { &name };
                        let short: String = content.chars().take(200).collect();
                        out.push(format!("[{name}]: {short}"));
                    }
                }
                "tool_call" => {
                    let name = string_at(part, &["data", "name"]);
                    if !name.is_empty() {
                        out.push(format!("[calling {name}]"));
                    }
                }
                _ => {}
            }
        }
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use chrono::Local;
    use serde_json::json;

    use crate::adapters::Adapter;

    use super::*;

    #[test]
    fn resume_command_supports_yolo() {
        let adapter = CrushAdapter {
            projects_file: PathBuf::from("projects.json"),
        };
        let session = Session::new(
            "crush-1",
            "crush",
            "Title",
            "/work/crush",
            Local::now(),
            "",
            0,
        );

        assert_eq!(
            adapter.resume_command(&session, true),
            vec!["crush", "--yolo", "--session", "crush-1"]
        );
    }

    #[test]
    fn renders_crush_parts_text() {
        let text = crush_parts_text(
            &json!([
                {"type": "text", "data": {"text": "hello"}},
                {"type": "tool_call", "data": {"name": "edit"}},
                {"type": "tool_result", "data": {"name": "edit", "content": "ok"}}
            ])
            .to_string(),
        );

        assert!(text.contains("hello"));
        assert!(text.contains("[calling edit]"));
        assert!(text.contains("[edit]: ok"));
    }
}
