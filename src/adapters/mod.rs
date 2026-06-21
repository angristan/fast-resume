use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use regex::Regex;
use rusqlite::{Connection, params_from_iter};
use serde_json::Value;
use url::Url;
use walkdir::WalkDir;

use crate::config;
use crate::model::{RawAdapterStats, Session, file_mtime_seconds, file_timestamp, truncate_title};

pub const MTIME_TOLERANCE: f64 = 0.001;

pub type KnownSessions = HashMap<(String, String), f64>;

#[derive(Debug, Clone, Default)]
pub struct IncrementalScan {
    pub agent: &'static str,
    pub new_or_modified: Vec<Session>,
    pub deleted_ids: Vec<String>,
}

pub trait Adapter: Send {
    fn name(&self) -> &'static str;
    fn supports_yolo(&self) -> bool {
        false
    }
    fn find_sessions(&self) -> Vec<Session>;
    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        let sessions = self.find_sessions();
        let current_ids: HashSet<_> = sessions.iter().map(|session| session.id.clone()).collect();
        let new_or_modified = sessions
            .into_iter()
            .filter(|session| {
                session_needs_update(known, &session.agent, &session.id, session.mtime)
            })
            .collect();
        IncrementalScan {
            agent: self.name(),
            new_or_modified,
            deleted_ids: deleted_ids_for_agent(known, self.name(), &current_ids),
        }
    }
    fn resume_command(&self, session: &Session, yolo: bool) -> Vec<String>;
    fn raw_stats(&self) -> RawAdapterStats;
}

fn session_needs_update(known: &KnownSessions, agent: &str, id: &str, mtime: f64) -> bool {
    known
        .get(&(agent.to_string(), id.to_string()))
        .is_none_or(|known_mtime| mtime > *known_mtime + MTIME_TOLERANCE)
}

fn deleted_ids_for_agent(
    known: &KnownSessions,
    agent: &str,
    current_ids: &HashSet<String>,
) -> Vec<String> {
    known
        .iter()
        .filter_map(|((known_agent, id), _)| {
            (known_agent == agent && !current_ids.contains(id)).then(|| id.clone())
        })
        .collect()
}

fn incremental_from_files<F>(
    agent: &'static str,
    known: &KnownSessions,
    current_files: HashMap<String, (PathBuf, f64)>,
    mut parse: F,
) -> IncrementalScan
where
    F: FnMut(&Path) -> Option<Session>,
{
    let current_ids: HashSet<_> = current_files.keys().cloned().collect();
    let mut new_or_modified = Vec::new();

    for (session_id, (path, mtime)) in current_files {
        if !session_needs_update(known, agent, &session_id, mtime) {
            continue;
        }
        if let Some(mut session) = parse(&path) {
            session.mtime = mtime;
            new_or_modified.push(session);
        }
    }

    IncrementalScan {
        agent,
        new_or_modified,
        deleted_ids: deleted_ids_for_agent(known, agent, &current_ids),
    }
}

pub fn all_adapters() -> Vec<Box<dyn Adapter>> {
    vec![
        Box::new(ClaudeAdapter::default()),
        Box::new(CodexAdapter::default()),
        Box::new(CopilotCliAdapter::default()),
        Box::new(CopilotVsCodeAdapter::default()),
        Box::new(CrushAdapter::default()),
        Box::new(OpenCodeAdapter::default()),
        Box::new(VibeAdapter::default()),
    ]
}

pub fn adapter_for(agent: &str) -> Option<Box<dyn Adapter>> {
    all_adapters()
        .into_iter()
        .find(|adapter| adapter.name() == agent)
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct CodexAdapter {
    sessions_dir: PathBuf,
    session_index_file: PathBuf,
}

impl Default for CodexAdapter {
    fn default() -> Self {
        Self {
            sessions_dir: config::codex_dir(),
            session_index_file: config::codex_session_index_file(),
        }
    }
}

impl CodexAdapter {
    #[allow(dead_code)]
    pub fn new(sessions_dir: PathBuf, session_index_file: PathBuf) -> Self {
        Self {
            sessions_dir,
            session_index_file,
        }
    }

    fn parse_session(
        &self,
        path: &Path,
        thread_names: &HashMap<String, String>,
    ) -> Option<Session> {
        let file = fs::File::open(path).ok()?;
        let mut session_id = String::new();
        let mut directory = String::new();
        let mut messages = Vec::new();
        let mut user_prompts = Vec::new();
        let mut turns = 0usize;
        let mut yolo = false;

        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(data) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let msg_type = string_at(&data, &["type"]);
            let payload = data.get("payload").unwrap_or(&Value::Null);

            match msg_type.as_str() {
                "session_meta" => {
                    session_id = string_at(payload, &["id"]);
                    directory = string_at(payload, &["cwd"]);
                }
                "turn_context" => {
                    let approval = string_at(payload, &["approval_policy"]);
                    let sandbox_mode = payload
                        .pointer("/sandbox_policy/mode")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if approval == "never" || sandbox_mode == "danger-full-access" {
                        yolo = true;
                    }
                }
                "response_item" => {
                    let role = string_at(payload, &["role"]);
                    if role == "user" || role == "assistant" {
                        let role_prefix = if role == "user" { "» " } else { "  " };
                        let mut has_text = false;
                        if let Some(content) = payload.get("content") {
                            for text in content_texts(content) {
                                if !text.trim_start().starts_with("<environment_context>") {
                                    messages.push(format!("{role_prefix}{text}"));
                                    has_text = true;
                                }
                            }
                        }
                        if has_text {
                            turns += 1;
                        }
                    }
                }
                "event_msg" => match string_at(payload, &["type"]).as_str() {
                    "user_message" => {
                        let message = string_at(payload, &["message"]);
                        if !message.is_empty() {
                            messages.push(format!("» {message}"));
                            user_prompts.push(message);
                        }
                    }
                    "agent_reasoning" => {
                        let text = string_at(payload, &["text"]);
                        if !text.is_empty() {
                            messages.push(format!("  {text}"));
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        if session_id.is_empty() {
            session_id = fallback_session_id(path);
        }
        if user_prompts.is_empty() {
            return None;
        }

        let title_source = thread_names
            .get(&session_id)
            .cloned()
            .unwrap_or_else(|| user_prompts[0].clone());
        let mut session = Session::new(
            session_id,
            self.name(),
            truncate_title(&title_source, 80, false),
            directory,
            file_timestamp(path),
            messages.join("\n\n"),
            turns,
        );
        session.mtime = file_mtime_seconds(path);
        session.yolo = yolo;
        Some(session)
    }

    fn load_thread_index(&self) -> HashMap<String, (String, f64)> {
        let index_mtime = file_mtime_seconds(&self.session_index_file);
        let Ok(file) = fs::File::open(&self.session_index_file) else {
            return HashMap::new();
        };
        let mut out = HashMap::new();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(data) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let id = string_at(&data, &["id"]);
            let thread_name = string_at(&data, &["thread_name"]);
            if !id.is_empty() && !thread_name.trim().is_empty() {
                let updated_at = string_at(&data, &["updated_at"]);
                let mtime = parse_timestamp_seconds(&updated_at).unwrap_or(index_mtime);
                out.insert(id, (thread_name.trim().to_string(), mtime));
            }
        }
        out
    }

    fn load_thread_names(&self) -> HashMap<String, String> {
        self.load_thread_index()
            .into_iter()
            .map(|(id, (thread_name, _))| (id, thread_name))
            .collect()
    }

    fn session_id_from_file(&self, path: &Path) -> String {
        if let Some(session_id) = codex_session_id_from_path(path) {
            return session_id;
        }
        if let Ok(file) = fs::File::open(path) {
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(data) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if string_at(&data, &["type"]) == "session_meta" {
                    let id = string_at(data.get("payload").unwrap_or(&Value::Null), &["id"]);
                    if !id.is_empty() {
                        return id;
                    }
                    break;
                }
            }
        }
        fallback_session_id(path)
    }

    fn scan_session_files(&self) -> HashMap<String, (PathBuf, f64)> {
        let mut current_files = HashMap::new();
        if !self.sessions_dir.exists() {
            return current_files;
        }

        let thread_index = self.load_thread_index();
        for entry in WalkDir::new(&self.sessions_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
        {
            let path = entry.path();
            let session_id = self.session_id_from_file(path);
            let mut mtime = file_mtime_seconds(path);
            if let Some((_, index_mtime)) = thread_index.get(&session_id) {
                mtime = mtime.max(*index_mtime);
            }
            current_files.insert(session_id, (path.to_path_buf(), mtime));
        }
        current_files
    }
}

impl Adapter for CodexAdapter {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn supports_yolo(&self) -> bool {
        true
    }

    fn find_sessions(&self) -> Vec<Session> {
        if !self.sessions_dir.exists() {
            return Vec::new();
        }
        let thread_names = self.load_thread_names();
        WalkDir::new(&self.sessions_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
            .filter_map(|entry| self.parse_session(entry.path(), &thread_names))
            .collect()
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        let thread_names = self.load_thread_names();
        let current_files = self.scan_session_files();
        let current_ids: HashSet<_> = current_files.keys().cloned().collect();
        let cache_watermark = file_mtime_seconds(&config::cache_file());
        let mut new_or_modified = Vec::new();

        for (session_id, (path, mtime)) in current_files {
            if !session_needs_update(known, self.name(), &session_id, mtime) {
                continue;
            }

            // Some historical Codex logs have no user prompt in the shape this app indexes.
            // Once an unchanged file predates the cache, treat the previous parse miss as stable
            // so warm incremental refreshes do not keep rereading hundreds of MB.
            let known_key = (self.name().to_string(), session_id.clone());
            if !known.contains_key(&known_key)
                && cache_watermark > 0.0
                && mtime <= cache_watermark + MTIME_TOLERANCE
            {
                continue;
            }

            if let Some(mut session) = self.parse_session(&path, &thread_names) {
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

    fn resume_command(&self, session: &Session, yolo: bool) -> Vec<String> {
        let mut cmd = vec!["codex".to_string()];
        if yolo {
            cmd.push("--dangerously-bypass-approvals-and-sandbox".to_string());
        }
        cmd.extend(["resume".to_string(), session.id.clone()]);
        cmd
    }

    fn raw_stats(&self) -> RawAdapterStats {
        raw_stats_for_tree(self.name(), &self.sessions_dir, "jsonl")
    }
}

#[derive(Debug, Clone)]
pub struct CopilotCliAdapter {
    sessions_dir: PathBuf,
}

impl Default for CopilotCliAdapter {
    fn default() -> Self {
        Self {
            sessions_dir: config::copilot_dir(),
        }
    }
}

impl Adapter for CopilotCliAdapter {
    fn name(&self) -> &'static str {
        "copilot-cli"
    }

    fn supports_yolo(&self) -> bool {
        true
    }

    fn find_sessions(&self) -> Vec<Session> {
        if !self.sessions_dir.exists() {
            return Vec::new();
        }
        WalkDir::new(&self.sessions_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
            .filter_map(|entry| self.parse_session(entry.path()))
            .collect()
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        incremental_from_files(self.name(), known, self.scan_session_files(), |path| {
            self.parse_session(path)
        })
    }

    fn resume_command(&self, session: &Session, yolo: bool) -> Vec<String> {
        let mut cmd = vec!["copilot".to_string()];
        if yolo {
            cmd.push("--yolo".to_string());
        }
        cmd.extend(["--resume".to_string(), session.id.clone()]);
        cmd
    }

    fn raw_stats(&self) -> RawAdapterStats {
        raw_stats_for_tree(self.name(), &self.sessions_dir, "jsonl")
    }
}

impl CopilotCliAdapter {
    fn scan_session_files(&self) -> HashMap<String, (PathBuf, f64)> {
        let mut current_files = HashMap::new();
        if !self.sessions_dir.exists() {
            return current_files;
        }

        for entry in WalkDir::new(&self.sessions_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
        {
            let path = entry.path();
            let session_id = self.session_id_from_file(path);
            current_files.insert(session_id, (path.to_path_buf(), file_mtime_seconds(path)));
        }
        current_files
    }

    fn session_id_from_file(&self, path: &Path) -> String {
        if let Ok(file) = fs::File::open(path) {
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(entry) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if string_at(&entry, &["type"]) == "session.start" {
                    let id = string_at(entry.get("data").unwrap_or(&Value::Null), &["sessionId"]);
                    if !id.is_empty() {
                        return id;
                    }
                    break;
                }
            }
        }
        copilot_fallback_session_id(path, &self.sessions_dir)
    }

    fn parse_session(&self, path: &Path) -> Option<Session> {
        let file = fs::File::open(path).ok()?;
        let mut session_id = copilot_fallback_session_id(path, &self.sessions_dir);
        let mut directory = String::new();
        let mut first_user_message = String::new();
        let mut session_title = String::new();
        let mut messages = Vec::new();
        let mut turns = 0usize;
        let folder_re = Regex::new(r"Folder (/[^\s]+)").ok()?;

        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(entry) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let msg_type = string_at(&entry, &["type"]);
            let data = entry.get("data").unwrap_or(&Value::Null);

            match msg_type.as_str() {
                "session.start" => {
                    let id = string_at(data, &["sessionId"]);
                    if !id.is_empty() {
                        session_id = id;
                    }
                    if directory.is_empty() {
                        directory = string_at(data, &["context", "cwd"]);
                    }
                }
                "session.info" if directory.is_empty() => {
                    if string_at(data, &["infoType"]) == "folder_trust" {
                        let message = string_at(data, &["message"]);
                        if let Some(caps) = folder_re.captures(&message) {
                            directory = caps[1].to_string();
                        }
                    }
                }
                "session.title_changed" => {
                    let title = string_at(data, &["title"]);
                    if !title.trim().is_empty() {
                        session_title = title.trim().to_string();
                    }
                }
                "user.message" => {
                    let content = string_at(data, &["content"]);
                    if !content.is_empty() {
                        messages.push(format!("» {content}"));
                        turns += 1;
                        if first_user_message.is_empty() && content.chars().count() > 10 {
                            first_user_message = content;
                        }
                    }
                }
                "assistant.message" => {
                    let content = string_at(data, &["content"]);
                    if !content.is_empty() {
                        messages.push(format!("  {content}"));
                        turns += 1;
                    }
                }
                _ => {}
            }
        }

        if first_user_message.is_empty() || messages.is_empty() {
            return None;
        }

        let title = truncate_title(
            if session_title.is_empty() {
                &first_user_message
            } else {
                &session_title
            },
            100,
            true,
        );
        let mut session = Session::new(
            session_id,
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
}

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
        incremental_from_files(self.name(), known, self.scan_session_files(), |path| {
            self.parse_session(path)
        })
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
    fn scan_session_files(&self) -> HashMap<String, (PathBuf, f64)> {
        let mut current_files = HashMap::new();
        let Ok(entries) = fs::read_dir(&self.sessions_dir) else {
            return current_files;
        };

        for entry in entries.filter_map(Result::ok) {
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
            let Ok(metadata) =
                serde_json::from_slice::<Value>(&fs::read(&metadata_file).unwrap_or_default())
            else {
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
                (session_dir, file_mtime_seconds(&metadata_file)),
            );
        }

        current_files
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
        session.mtime = file_mtime_seconds(&metadata_file);
        session.yolo = yolo;
        Some(session)
    }
}

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

#[derive(Debug, Clone)]
pub struct OpenCodeAdapter {
    data_dir: PathBuf,
    db_path: PathBuf,
    legacy_dir: PathBuf,
}

impl Default for OpenCodeAdapter {
    fn default() -> Self {
        Self {
            data_dir: config::opencode_dir(),
            db_path: config::opencode_db(),
            legacy_dir: config::opencode_legacy_dir(),
        }
    }
}

impl Adapter for OpenCodeAdapter {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn find_sessions(&self) -> Vec<Session> {
        if self.db_path.exists() {
            return load_opencode_db(self.name(), &self.db_path);
        }
        load_opencode_legacy(self.name(), &self.legacy_dir)
    }

    fn find_sessions_incremental(&self, known: &KnownSessions) -> IncrementalScan {
        if self.db_path.exists() {
            return load_opencode_db_incremental(self.name(), &self.db_path, known);
        }
        load_opencode_legacy_incremental(self.name(), &self.legacy_dir, known)
    }

    fn resume_command(&self, session: &Session, _yolo: bool) -> Vec<String> {
        vec![
            "opencode".to_string(),
            session.directory.clone(),
            "--session".to_string(),
            session.id.clone(),
        ]
    }

    fn raw_stats(&self) -> RawAdapterStats {
        if self.db_path.exists() {
            let mut total_bytes = self.db_path.metadata().map(|m| m.len()).unwrap_or(0);
            let mut files = 1usize;
            for suffix in ["-wal", "-shm"] {
                let path = self.db_path.with_file_name(format!(
                    "{}{}",
                    self.db_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    suffix
                ));
                if let Ok(meta) = path.metadata() {
                    total_bytes += meta.len();
                    files += 1;
                }
            }
            return RawAdapterStats {
                agent: self.name(),
                data_dir: format!("{} (sqlite)", self.data_dir.display()),
                available: true,
                file_count: files,
                total_bytes,
            };
        }
        raw_stats_for_tree(self.name(), &self.legacy_dir, "json")
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

fn load_opencode_db(agent: &'static str, db_path: &Path) -> Vec<Session> {
    let Ok(conn) = Connection::open(db_path) else {
        return Vec::new();
    };

    let mut sessions_meta = Vec::new();
    let mut stmt = match conn.prepare(
        "SELECT id, title, directory, time_created, time_updated FROM session ORDER BY time_updated DESC",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(3)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(4)?.unwrap_or_default(),
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };
    for row in rows.filter_map(Result::ok) {
        sessions_meta.push(row);
    }
    drop(stmt);

    let mut messages_by_session: HashMap<String, Vec<(String, String)>> = HashMap::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT id, session_id, data FROM message ORDER BY time_created ASC")
    {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }) {
            for (msg_id, session_id, data) in rows.filter_map(Result::ok) {
                let role = serde_json::from_str::<Value>(&data)
                    .ok()
                    .map(|value| string_at(&value, &["role"]))
                    .unwrap_or_default();
                messages_by_session
                    .entry(session_id)
                    .or_default()
                    .push((msg_id, role));
            }
        }
    }

    let mut parts_by_message: HashMap<String, Vec<String>> = HashMap::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT message_id, data FROM part ORDER BY time_created ASC")
    {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
            for (message_id, data) in rows.filter_map(Result::ok) {
                let Ok(value) = serde_json::from_str::<Value>(&data) else {
                    continue;
                };
                if string_at(&value, &["type"]) == "text" {
                    let text = string_at(&value, &["text"]);
                    if !text.is_empty() {
                        parts_by_message.entry(message_id).or_default().push(text);
                    }
                }
            }
        }
    }

    let mut sessions = Vec::new();
    for (id, title, directory, time_created, time_updated) in sessions_meta {
        let mut rendered = Vec::new();
        let session_messages = messages_by_session.remove(&id).unwrap_or_default();
        for (message_id, role) in &session_messages {
            let prefix = if role == "user" { "» " } else { "  " };
            for text in parts_by_message
                .get(message_id)
                .cloned()
                .unwrap_or_default()
            {
                rendered.push(format!("{prefix}{text}"));
            }
        }
        let timestamp =
            timestamp_from_ms(Some(time_created.max(time_updated))).unwrap_or_else(Local::now);
        let mut session = Session::new(
            id,
            agent,
            if title.is_empty() {
                "Untitled session".to_string()
            } else {
                title
            },
            directory,
            timestamp,
            rendered.join("\n\n"),
            session_messages.len(),
        );
        session.mtime = session.timestamp.timestamp() as f64;
        sessions.push(session);
    }
    sessions
}

fn load_opencode_db_incremental(
    agent: &'static str,
    db_path: &Path,
    known: &KnownSessions,
) -> IncrementalScan {
    let Ok(conn) = Connection::open(db_path) else {
        return IncrementalScan {
            agent,
            new_or_modified: Vec::new(),
            deleted_ids: deleted_ids_for_agent(known, agent, &HashSet::new()),
        };
    };

    let mut stmt = match conn
        .prepare("SELECT id, title, directory, time_created, time_updated FROM session")
    {
        Ok(stmt) => stmt,
        Err(_) => {
            return IncrementalScan {
                agent,
                new_or_modified: Vec::new(),
                deleted_ids: deleted_ids_for_agent(known, agent, &HashSet::new()),
            };
        }
    };

    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(3)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(4)?.unwrap_or_default(),
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => {
            return IncrementalScan {
                agent,
                new_or_modified: Vec::new(),
                deleted_ids: deleted_ids_for_agent(known, agent, &HashSet::new()),
            };
        }
    };

    let mut current_ids = HashSet::new();
    let mut sessions_to_fetch = Vec::new();
    for row in rows.filter_map(Result::ok) {
        let (id, title, directory, time_created, time_updated) = row;
        current_ids.insert(id.clone());
        let timestamp_ms = time_created.max(time_updated);
        let mtime = timestamp_from_ms(Some(timestamp_ms))
            .map(datetime_to_seconds)
            .unwrap_or_else(|| file_mtime_seconds(db_path));
        if session_needs_update(known, agent, &id, mtime) {
            sessions_to_fetch.push((id, title, directory, time_created, time_updated, mtime));
        }
    }
    drop(stmt);

    let deleted_ids = deleted_ids_for_agent(known, agent, &current_ids);
    if sessions_to_fetch.is_empty() {
        return IncrementalScan {
            agent,
            new_or_modified: Vec::new(),
            deleted_ids,
        };
    }

    let fetch_ids: Vec<_> = sessions_to_fetch
        .iter()
        .map(|(id, _, _, _, _, _)| id.clone())
        .collect();
    let mut messages_by_session: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for chunk in fetch_ids.chunks(900) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let query = format!(
            "SELECT id, session_id, data FROM message WHERE session_id IN ({placeholders}) ORDER BY time_created ASC"
        );
        let Ok(mut stmt) = conn.prepare(&query) else {
            continue;
        };
        let Ok(rows) = stmt.query_map(params_from_iter(chunk.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }) else {
            continue;
        };
        for (msg_id, session_id, data) in rows.filter_map(Result::ok) {
            let role = serde_json::from_str::<Value>(&data)
                .ok()
                .map(|value| string_at(&value, &["role"]))
                .unwrap_or_default();
            messages_by_session
                .entry(session_id)
                .or_default()
                .push((msg_id, role));
        }
    }

    let mut parts_by_message: HashMap<String, Vec<String>> = HashMap::new();
    for chunk in fetch_ids.chunks(900) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let query = format!(
            "SELECT p.message_id, p.data FROM part p JOIN message m ON m.id = p.message_id WHERE m.session_id IN ({placeholders}) ORDER BY p.time_created ASC"
        );
        let Ok(mut stmt) = conn.prepare(&query) else {
            continue;
        };
        let Ok(rows) = stmt.query_map(params_from_iter(chunk.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) else {
            continue;
        };
        for (message_id, data) in rows.filter_map(Result::ok) {
            let Ok(value) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            if string_at(&value, &["type"]) == "text" {
                let text = string_at(&value, &["text"]);
                if !text.is_empty() {
                    parts_by_message.entry(message_id).or_default().push(text);
                }
            }
        }
    }

    let mut new_or_modified = Vec::new();
    for (id, title, directory, time_created, time_updated, mtime) in sessions_to_fetch {
        let mut rendered = Vec::new();
        let session_messages = messages_by_session.remove(&id).unwrap_or_default();
        for (message_id, role) in &session_messages {
            let prefix = if role == "user" { "» " } else { "  " };
            for text in parts_by_message
                .get(message_id)
                .cloned()
                .unwrap_or_default()
            {
                rendered.push(format!("{prefix}{text}"));
            }
        }
        let timestamp =
            timestamp_from_ms(Some(time_created.max(time_updated))).unwrap_or_else(Local::now);
        let mut session = Session::new(
            id,
            agent,
            if title.is_empty() {
                "Untitled session".to_string()
            } else {
                title
            },
            directory,
            timestamp,
            rendered.join("\n\n"),
            session_messages.len(),
        );
        session.mtime = mtime;
        new_or_modified.push(session);
    }

    IncrementalScan {
        agent,
        new_or_modified,
        deleted_ids,
    }
}

fn load_opencode_legacy(agent: &'static str, legacy_dir: &Path) -> Vec<Session> {
    let session_dir = legacy_dir.join("session");
    let message_dir = legacy_dir.join("message");
    let part_dir = legacy_dir.join("part");
    if !session_dir.exists() {
        return Vec::new();
    }

    let mut messages_by_session: HashMap<String, Vec<(PathBuf, String, String)>> = HashMap::new();
    if message_dir.exists() {
        for entry in WalkDir::new(&message_dir)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("msg_") && name.ends_with(".json"))
            {
                continue;
            }
            let Ok(data) = serde_json::from_slice::<Value>(&fs::read(path).unwrap_or_default())
            else {
                continue;
            };
            let msg_id = string_at(&data, &["id"]);
            let role = string_at(&data, &["role"]);
            let Some(session_id) = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
            else {
                continue;
            };
            if !msg_id.is_empty() {
                messages_by_session
                    .entry(session_id.to_string())
                    .or_default()
                    .push((path.to_path_buf(), msg_id, role));
            }
        }
    }

    let mut parts_by_message: HashMap<String, Vec<String>> = HashMap::new();
    if part_dir.exists() {
        for entry in WalkDir::new(&part_dir).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(data) = serde_json::from_slice::<Value>(&fs::read(path).unwrap_or_default())
            else {
                continue;
            };
            if string_at(&data, &["type"]) != "text" {
                continue;
            }
            let text = string_at(&data, &["text"]);
            let Some(message_id) = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
            else {
                continue;
            };
            if !text.is_empty() {
                parts_by_message
                    .entry(message_id.to_string())
                    .or_default()
                    .push(text);
            }
        }
    }

    let mut sessions = Vec::new();
    for entry in WalkDir::new(&session_dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("ses_") && name.ends_with(".json"))
        {
            continue;
        }
        let Ok(data) = serde_json::from_slice::<Value>(&fs::read(path).unwrap_or_default()) else {
            continue;
        };
        let id = string_at(&data, &["id"]);
        if id.is_empty() {
            continue;
        }
        let title = {
            let value = string_at(&data, &["title"]);
            if value.is_empty() {
                "Untitled session".to_string()
            } else {
                value
            }
        };
        let directory = string_at(&data, &["directory"]);
        let time_ms = value_i64_at(&data, &["time", "updated"])
            .or_else(|| value_i64_at(&data, &["time", "created"]));
        let timestamp = timestamp_from_ms(time_ms).unwrap_or_else(|| file_timestamp(path));

        let mut rendered = Vec::new();
        let mut session_messages = messages_by_session.remove(&id).unwrap_or_default();
        session_messages.sort_by(|a, b| a.0.cmp(&b.0));
        for (_path, msg_id, role) in &session_messages {
            let prefix = if role == "user" { "» " } else { "  " };
            for text in parts_by_message.get(msg_id).cloned().unwrap_or_default() {
                rendered.push(format!("{prefix}{text}"));
            }
        }

        let mut session = Session::new(
            id,
            agent,
            title,
            directory,
            timestamp,
            rendered.join("\n\n"),
            session_messages.len(),
        );
        session.mtime = file_mtime_seconds(path);
        sessions.push(session);
    }
    sessions
}

fn load_opencode_legacy_incremental(
    agent: &'static str,
    legacy_dir: &Path,
    known: &KnownSessions,
) -> IncrementalScan {
    let current_files = scan_opencode_legacy_sessions(legacy_dir);
    let current_ids: HashSet<_> = current_files.keys().cloned().collect();
    let deleted_ids = deleted_ids_for_agent(known, agent, &current_ids);
    let changed_ids: HashSet<_> = current_files
        .iter()
        .filter_map(|(id, (_, mtime))| {
            session_needs_update(known, agent, id, *mtime).then(|| id.clone())
        })
        .collect();

    if changed_ids.is_empty() {
        return IncrementalScan {
            agent,
            new_or_modified: Vec::new(),
            deleted_ids,
        };
    }

    let mut new_or_modified = Vec::new();
    for mut session in load_opencode_legacy(agent, legacy_dir) {
        if !changed_ids.contains(&session.id) {
            continue;
        }
        if let Some((_, mtime)) = current_files.get(&session.id) {
            session.mtime = *mtime;
        }
        new_or_modified.push(session);
    }

    IncrementalScan {
        agent,
        new_or_modified,
        deleted_ids,
    }
}

fn scan_opencode_legacy_sessions(legacy_dir: &Path) -> HashMap<String, (PathBuf, f64)> {
    let mut current_files = HashMap::new();
    let session_dir = legacy_dir.join("session");
    if !session_dir.exists() {
        return current_files;
    }

    for entry in WalkDir::new(&session_dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("ses_") && name.ends_with(".json"))
        {
            continue;
        }
        let Ok(data) = serde_json::from_slice::<Value>(&fs::read(path).unwrap_or_default()) else {
            continue;
        };
        let id = string_at(&data, &["id"]);
        if id.is_empty() {
            continue;
        }
        let time_ms = value_i64_at(&data, &["time", "updated"])
            .or_else(|| value_i64_at(&data, &["time", "created"]));
        let mtime = timestamp_from_ms(time_ms)
            .map(datetime_to_seconds)
            .unwrap_or_else(|| file_mtime_seconds(path));
        current_files.insert(id, (path.to_path_buf(), mtime));
    }

    current_files
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

fn content_texts(content: &Value) -> Vec<String> {
    match content {
        Value::String(text) if !text.is_empty() => vec![text.clone()],
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                text_from_part(part).or_else(|| part.as_str().map(ToString::to_string))
            })
            .filter(|text| !text.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn text_from_part(part: &Value) -> Option<String> {
    if let Some(text) = part.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = part.get("input_text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    None
}

fn string_at(value: &Value, path: &[&str]) -> String {
    let mut current = value;
    for key in path {
        current = current.get(*key).unwrap_or(&Value::Null);
    }
    current.as_str().unwrap_or_default().to_string()
}

fn value_i64_at(value: &Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_i64()
        .or_else(|| current.as_f64().map(|v| v as i64))
}

fn fallback_session_id(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    stem.split_once('-')
        .map(|(_, rest)| rest.to_string())
        .unwrap_or_else(|| stem.to_string())
}

fn codex_session_id_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let candidate = stem.get(stem.len().saturating_sub(36)..)?;
    is_uuid_like(candidate).then(|| candidate.to_string())
}

fn is_uuid_like(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    value.chars().enumerate().all(|(idx, ch)| match idx {
        8 | 13 | 18 | 23 => ch == '-',
        _ => ch.is_ascii_hexdigit(),
    })
}

fn copilot_fallback_session_id(path: &Path, sessions_dir: &Path) -> String {
    if path.parent() != Some(sessions_dir) {
        path.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string()
    } else {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string()
    }
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

fn parse_timestamp_seconds(value: &str) -> Option<f64> {
    if value.trim().is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.timestamp() as f64 + f64::from(dt.timestamp_subsec_nanos()) / 1e9)
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
                .ok()
                .and_then(|dt| Local.from_local_datetime(&dt).single())
                .map(datetime_to_seconds)
        })
}

fn datetime_to_seconds(timestamp: DateTime<Local>) -> f64 {
    timestamp.timestamp() as f64 + f64::from(timestamp.timestamp_subsec_nanos()) / 1e9
}

fn parse_datetime(value: &str) -> Option<DateTime<Local>> {
    if value.trim().is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Local))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
                .ok()
                .and_then(|dt| Local.from_local_datetime(&dt).single())
        })
}

fn timestamp_from_ms(value: Option<i64>) -> Option<DateTime<Local>> {
    let value = value?;
    if value <= 0 {
        return None;
    }
    Local.timestamp_millis_opt(value).single()
}

fn timestamp_from_seconds(value: Option<i64>) -> Option<DateTime<Local>> {
    let value = value?;
    if value <= 0 {
        return None;
    }
    Local.timestamp_opt(value, 0).single()
}

fn normalize_seconds(value: i64) -> Option<i64> {
    if value <= 0 {
        None
    } else if value > 100_000_000_000 {
        Some(value / 1000)
    } else {
        Some(value)
    }
}

fn raw_stats_for_tree(agent: &'static str, dir: &Path, extension: &str) -> RawAdapterStats {
    if !dir.exists() {
        return RawAdapterStats {
            agent,
            data_dir: dir.display().to_string(),
            available: false,
            file_count: 0,
            total_bytes: 0,
        };
    }
    let mut seen = HashSet::new();
    let mut total_bytes = 0;
    for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(extension) {
            continue;
        }
        if seen.insert(path.to_path_buf()) {
            total_bytes += path.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    RawAdapterStats {
        agent,
        data_dir: dir.display().to_string(),
        available: true,
        file_count: seen.len(),
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
    fn claude_uses_sessions_index_title() {
        let temp = tempdir().unwrap();
        let projects = temp.path().join("projects");
        let project = projects.join("project-a");
        fs::create_dir_all(&project).unwrap();
        let session_file = project.join("session-rename.jsonl");
        fs::write(
            &session_file,
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

    #[test]
    fn codex_uses_thread_name_and_detects_yolo() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(sessions_dir.join("2026/06/21")).unwrap();
        let session_file = sessions_dir.join("2026/06/21/rollout-2026-06-21T10-00-00-test.jsonl");
        fs::write(
            &session_file,
            [
                json!({"type": "session_meta", "payload": {"id": "abc123", "cwd": "/work/zeno"}})
                    .to_string(),
                json!({
                    "type": "turn_context",
                    "payload": {
                        "approval_policy": "never",
                        "sandbox_policy": {"mode": "danger-full-access"}
                    }
                })
                .to_string(),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "Original prompt"}})
                    .to_string(),
                json!({
                    "type": "response_item",
                    "payload": {
                        "role": "user",
                        "content": [{"text": "<environment_context>skip me</environment_context>"}]
                    }
                })
                .to_string(),
                json!({"type": "response_item", "payload": {"role": "assistant", "content": [{"text": "Answer"}]}})
                    .to_string(),
            ]
            .join("\n"),
        )
        .unwrap();
        let session_index = temp.path().join("session_index.jsonl");
        fs::write(
            &session_index,
            json!({"id": "abc123", "thread_name": "Renamed Codex thread"}).to_string(),
        )
        .unwrap();

        let adapter = CodexAdapter::new(sessions_dir, session_index);
        let sessions = adapter.find_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Renamed Codex thread");
        assert!(sessions[0].yolo);
        assert!(!sessions[0].content.contains("<environment_context>"));
        assert_eq!(
            adapter.resume_command(&sessions[0], true),
            vec![
                "codex",
                "--dangerously-bypass-approvals-and-sandbox",
                "resume",
                "abc123"
            ]
        );
    }

    #[test]
    fn codex_incremental_uses_session_index_mtime() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(sessions_dir.join("2026/06/21")).unwrap();
        let session_file = sessions_dir.join("2026/06/21/rollout-test123.jsonl");
        fs::write(
            &session_file,
            [
                json!({"type": "session_meta", "payload": {"id": "test123", "cwd": "/work/app"}})
                    .to_string(),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "Original prompt"}})
                    .to_string(),
                json!({"type": "response_item", "payload": {"role": "assistant", "content": [{"text": "Answer"}]}})
                    .to_string(),
            ]
            .join("\n"),
        )
        .unwrap();
        let session_index = temp.path().join("session_index.jsonl");
        fs::write(
            &session_index,
            json!({
                "id": "test123",
                "thread_name": "Renamed from index",
                "updated_at": "2030-01-01T00:00:00Z"
            })
            .to_string(),
        )
        .unwrap();

        let adapter = CodexAdapter::new(sessions_dir, session_index);
        let file_mtime = file_mtime_seconds(&session_file);
        let mut known = KnownSessions::new();
        known.insert(("codex".to_string(), "test123".to_string()), file_mtime);

        let scan = adapter.find_sessions_incremental(&known);
        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].title, "Renamed from index");
        assert!(scan.new_or_modified[0].mtime > file_mtime);

        let mut refreshed_known = KnownSessions::new();
        refreshed_known.insert(
            ("codex".to_string(), "test123".to_string()),
            scan.new_or_modified[0].mtime,
        );
        let unchanged = adapter.find_sessions_incremental(&refreshed_known);
        assert!(unchanged.new_or_modified.is_empty());
        assert!(unchanged.deleted_ids.is_empty());
    }
}
