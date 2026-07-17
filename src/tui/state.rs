use std::time::{Duration, Instant};

use crate::config::{AGENT_ORDER, is_agent};
use crate::model::Session;
use crate::query::{Filter, parse_query};
use crate::search::SearchEngine;

use super::images::AgentImages;
use super::text::char_to_byte_idx;

const DATE_SUGGESTIONS: [&str; 4] = ["today", "yesterday", "week", "month"];

pub(super) const PENDING_SEARCH_STATUS: &str = "searching; press again when results update";

pub(super) enum ScanMessage {
    Progress {
        elapsed: Duration,
        new_or_modified: usize,
        deleted: usize,
        total: usize,
    },
    Finished {
        elapsed: Duration,
        new_or_modified: usize,
        deleted: usize,
        total: usize,
    },
    Failed {
        elapsed: Duration,
        error: String,
    },
}

pub(super) struct SearchRequest {
    pub(super) generation: u64,
    pub(super) query: String,
    pub(super) agent_filter: Option<String>,
    pub(super) directory_filter: Option<String>,
    pub(super) preserve_selection: Option<(String, String)>,
    pub(super) reload_index: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum PendingAction {
    Resume,
    Copy,
}

#[derive(Debug, Clone)]
pub(super) struct YoloModal {
    pub(super) action: PendingAction,
    pub(super) session: Session,
    pub(super) selected: bool,
}

pub(super) struct AppState {
    pub(super) engine: SearchEngine,
    pub(super) visible: Vec<Session>,
    pub(super) query: String,
    pub(super) cursor: usize,
    pub(super) selected: usize,
    pub(super) preview_scroll: u16,
    pub(super) agent_filter: Option<String>,
    pub(super) directory_filter: Option<String>,
    pub(super) yolo: bool,
    pub(super) scanning: bool,
    pub(super) status: String,
    pub(super) refresh_status: String,
    pub(super) last_search_ms: f64,
    pub(super) show_preview: bool,
    pub(super) modal: Option<YoloModal>,
    pub(super) images: Option<AgentImages>,
    search_generation: u64,
    applied_search_generation: u64,
    search_requested: bool,
    search_preserve_selection: Option<(String, String)>,
    search_reload_requested: bool,
}

impl AppState {
    pub(super) fn new(
        query: String,
        agent_filter: Option<String>,
        directory_filter: Option<String>,
        yolo: bool,
        engine: SearchEngine,
        images: Option<AgentImages>,
    ) -> Self {
        let mut state = Self {
            engine,
            visible: Vec::new(),
            cursor: query.chars().count(),
            query,
            selected: 0,
            preview_scroll: 0,
            agent_filter,
            directory_filter,
            yolo,
            scanning: true,
            status: String::new(),
            refresh_status: "refreshing session stores".to_string(),
            last_search_ms: 0.0,
            show_preview: true,
            modal: None,
            images,
            search_generation: 0,
            applied_search_generation: 0,
            search_requested: false,
            search_preserve_selection: None,
            search_reload_requested: false,
        };
        state.refresh_search();
        state
    }

    pub(super) fn refresh_search(&mut self) {
        self.refresh_search_inner(false);
    }

    fn refresh_search_inner(&mut self, preserve_selection: bool) {
        let selected_session = preserve_selection
            .then(|| self.selected_session_key())
            .flatten();
        self.search_requested = false;
        self.search_preserve_selection = None;
        self.search_reload_requested = false;
        self.search_generation = self.search_generation.saturating_add(1);
        self.applied_search_generation = self.search_generation;
        let start = Instant::now();
        let agent_filter = self.effective_agent_filter();
        let directory_filter = self.effective_directory_filter();
        self.visible = self.engine.search(
            &self.query,
            agent_filter.as_deref(),
            directory_filter.as_deref(),
            100,
        );
        self.last_search_ms = start.elapsed().as_secs_f64() * 1000.0;
        self.update_selection_after_search(selected_session.as_ref());
        self.preview_scroll = 0;
    }

    pub(super) fn request_search(&mut self) {
        self.search_preserve_selection = None;
        self.search_requested = true;
    }

    pub(super) fn request_search_preserving_selection(&mut self, reload_index: bool) {
        self.search_reload_requested |= reload_index;
        if self.search_preserve_selection.is_none()
            && !self.search_requested
            && self.applied_search_generation == self.search_generation
        {
            self.search_preserve_selection = self.selected_session_key();
        }
        self.search_requested = true;
    }

    pub(super) fn take_search_request(&mut self) -> Option<SearchRequest> {
        if !self.search_requested {
            return None;
        }
        self.search_requested = false;
        self.search_generation = self.search_generation.saturating_add(1);
        let preserve_selection = self.search_preserve_selection.clone();
        let reload_index = self.search_reload_requested;
        self.search_reload_requested = false;
        Some(SearchRequest {
            generation: self.search_generation,
            query: self.query.clone(),
            agent_filter: self.effective_agent_filter(),
            directory_filter: self.effective_directory_filter(),
            preserve_selection,
            reload_index,
        })
    }

    pub(super) fn apply_search_result(
        &mut self,
        generation: u64,
        visible: Vec<Session>,
        elapsed_ms: f64,
        preserve_selection: Option<&(String, String)>,
    ) -> bool {
        if generation != self.search_generation {
            return false;
        }
        let selected_session = preserve_selection.and_then(|_| self.selected_session_key());
        self.visible = visible;
        self.last_search_ms = elapsed_ms;
        self.applied_search_generation = generation;
        self.update_selection_after_search(selected_session.as_ref());
        self.search_preserve_selection = None;
        self.preview_scroll = 0;
        if self.status == PENDING_SEARCH_STATUS {
            self.status.clear();
        }
        true
    }

    pub(super) fn apply_search_error(&mut self, generation: u64, error: &str) -> bool {
        if generation != self.search_generation {
            return false;
        }
        self.applied_search_generation = generation;
        self.search_preserve_selection = None;
        self.status = format!("search failed: {error}");
        true
    }

    fn selected_session_key(&self) -> Option<(String, String)> {
        self.selected_session()
            .map(|session| (session.agent.clone(), session.id.clone()))
    }

    fn update_selection_after_search(&mut self, selected_session: Option<&(String, String)>) {
        if self.visible.is_empty() {
            self.selected = 0;
        } else if let Some((agent, id)) = selected_session {
            self.selected = self
                .visible
                .iter()
                .position(|session| session.agent == *agent && session.id == *id)
                .unwrap_or_else(|| self.selected.min(self.visible.len() - 1));
        } else {
            self.selected = 0;
        }
    }

    pub(super) fn search_pending(&self) -> bool {
        self.search_requested || self.applied_search_generation != self.search_generation
    }

    pub(super) fn selected_session(&self) -> Option<&Session> {
        self.visible.get(self.selected)
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        if self.visible.is_empty() {
            self.selected = 0;
            return;
        }
        let max = self.visible.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, max) as usize;
        self.preview_scroll = 0;
    }

    pub(super) fn scroll_preview(&mut self, delta: isize) {
        if delta < 0 {
            self.preview_scroll = self
                .preview_scroll
                .saturating_sub(delta.unsigned_abs() as u16);
        } else {
            self.preview_scroll = self.preview_scroll.saturating_add(delta as u16);
        }
    }

    pub(super) fn active_agent_filter(&self) -> Option<String> {
        if query_has_agent_filter(&self.query) {
            single_query_agent_filter(&self.query)
        } else {
            self.agent_filter.clone()
        }
    }

    pub(super) fn active_agent_filters(&self) -> Vec<String> {
        if query_has_agent_filter(&self.query) {
            query_agent_filters(&self.query)
        } else {
            self.agent_filter.iter().cloned().collect()
        }
    }

    pub(super) fn all_agent_filter_active(&self) -> bool {
        self.agent_filter.is_none() && !query_has_agent_filter(&self.query)
    }

    pub(super) fn agent_filters_with_sessions(&self) -> Vec<(&'static str, usize)> {
        AGENT_ORDER
            .iter()
            .filter_map(|agent| {
                let count = self.engine.count_for_agent(Some(agent));
                (count > 0).then_some((*agent, count))
            })
            .collect()
    }

    pub(super) fn count_agent_filter(&self) -> Option<String> {
        if let Some(agent) = single_query_agent_filter(&self.query) {
            return Some(agent);
        }
        if query_has_agent_filter(&self.query) {
            return None;
        }
        self.agent_filter.clone()
    }

    pub(super) fn suggestion_suffix(&self) -> Option<String> {
        let suggestion = search_suggestion(&self.query, self.cursor)?;
        suggestion
            .strip_prefix(&self.query)
            .filter(|suffix| !suffix.is_empty())
            .map(ToString::to_string)
    }

    pub(super) fn accept_suggestion(&mut self) -> bool {
        let Some(suggestion) = search_suggestion(&self.query, self.cursor) else {
            return false;
        };
        self.cursor = suggestion.chars().count();
        self.query = suggestion;
        self.clear_explicit_filter_if_query_has_agent();
        self.request_search();
        true
    }

    pub(super) fn cycle_agent(&mut self, reverse: bool) {
        let available = self.agent_filters_with_sessions();
        let active = self.active_agent_filter();
        let current = active
            .as_deref()
            .and_then(|agent| {
                available
                    .iter()
                    .position(|(candidate, _)| *candidate == agent)
            })
            .map(|idx| idx + 1)
            .unwrap_or(0);
        let len = available.len() + 1;
        let next = if reverse {
            (current + len - 1) % len
        } else {
            (current + 1) % len
        };
        let next_agent = if next == 0 {
            None
        } else {
            Some(available[next - 1].0.to_string())
        };
        self.query = update_agent_in_query(&self.query, next_agent.as_deref());
        self.cursor = self.query.chars().count();
        self.agent_filter = None;
        self.request_search();
    }

    pub(super) fn insert_char(&mut self, ch: char) {
        let byte_idx = char_to_byte_idx(&self.query, self.cursor);
        self.query.insert(byte_idx, ch);
        self.cursor += 1;
        self.clear_explicit_filter_if_query_has_agent();
        self.request_search();
    }

    pub(super) fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = char_to_byte_idx(&self.query, self.cursor - 1);
        let end = char_to_byte_idx(&self.query, self.cursor);
        self.query.replace_range(start..end, "");
        self.cursor -= 1;
        self.clear_explicit_filter_if_query_has_agent();
        self.request_search();
    }

    pub(super) fn delete(&mut self) {
        if self.cursor >= self.query.chars().count() {
            return;
        }
        let start = char_to_byte_idx(&self.query, self.cursor);
        let end = char_to_byte_idx(&self.query, self.cursor + 1);
        self.query.replace_range(start..end, "");
        self.clear_explicit_filter_if_query_has_agent();
        self.request_search();
    }

    fn effective_agent_filter(&self) -> Option<String> {
        if query_has_agent_filter(&self.query) {
            None
        } else {
            self.agent_filter.clone()
        }
    }

    fn effective_directory_filter(&self) -> Option<String> {
        if query_has_directory_filter(&self.query) {
            None
        } else {
            self.directory_filter.clone()
        }
    }

    fn clear_explicit_filter_if_query_has_agent(&mut self) {
        if query_has_agent_filter(&self.query) {
            self.agent_filter = None;
        }
    }
}

fn query_agent_filters(query: &str) -> Vec<String> {
    parse_query(query)
        .agent
        .map(valid_included_agents)
        .unwrap_or_default()
}

fn valid_included_agents(filter: Filter) -> Vec<String> {
    filter
        .include
        .into_iter()
        .filter_map(normalize_agent)
        .fold(Vec::new(), |mut agents, agent| {
            if !agents.contains(&agent) {
                agents.push(agent);
            }
            agents
        })
}

fn single_query_agent_filter(query: &str) -> Option<String> {
    let filter = parse_query(query).agent?;
    if !filter.exclude.is_empty() {
        return None;
    }
    let agents = valid_included_agents(filter);
    match agents.as_slice() {
        [agent] => Some(agent.clone()),
        _ => None,
    }
}

fn normalize_agent(agent: String) -> Option<String> {
    let agent = agent.to_ascii_lowercase();
    is_agent(&agent).then_some(agent)
}

fn query_has_agent_filter(query: &str) -> bool {
    parse_query(query).agent.is_some()
}

fn query_has_directory_filter(query: &str) -> bool {
    parse_query(query).directory.is_some()
}

fn update_agent_in_query(query: &str, agent: Option<&str>) -> String {
    let mut parts: Vec<_> = query
        .split_whitespace()
        .map(ToString::to_string)
        .filter(|part| !is_agent_keyword(part))
        .collect();
    if let Some(agent) = agent {
        parts.push(format!("agent:{agent}"));
    }
    parts.join(" ")
}

fn is_agent_keyword(token: &str) -> bool {
    token
        .strip_prefix('-')
        .unwrap_or(token)
        .starts_with("agent:")
}

fn search_suggestion(query: &str, cursor: usize) -> Option<String> {
    if cursor != query.chars().count() {
        return None;
    }
    let cursor_idx = char_to_byte_idx(query, cursor);
    let before_cursor = &query[..cursor_idx];
    let token_start = before_cursor
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);
    let token = &before_cursor[token_start..];
    let (negated, token) = token
        .strip_prefix('-')
        .map(|token| ("-", token))
        .unwrap_or(("", token));
    if let Some(value) = token.strip_prefix("agent:") {
        return complete_keyword(query, token_start, negated, "agent:", value, &AGENT_ORDER);
    }
    if let Some(value) = token.strip_prefix("date:") {
        return complete_keyword(
            query,
            token_start,
            negated,
            "date:",
            value,
            &DATE_SUGGESTIONS,
        );
    }
    None
}

fn complete_keyword(
    query: &str,
    token_start: usize,
    negated: &str,
    keyword: &str,
    value: &str,
    values: &[&str],
) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let (value_prefix, partial) = value
        .strip_prefix('!')
        .map(|partial| ("!", partial))
        .unwrap_or(("", value));
    if partial.is_empty() {
        return None;
    }
    let partial = partial.to_ascii_lowercase();
    let completion = values.iter().find(|candidate| {
        candidate.starts_with(&partial) && candidate.to_ascii_lowercase() != partial
    })?;
    Some(format!(
        "{}{}{}{}{}",
        &query[..token_start],
        negated,
        keyword,
        value_prefix,
        completion
    ))
}

pub(super) fn handle_scan_message(state: &mut AppState, message: ScanMessage) {
    match message {
        ScanMessage::Progress {
            elapsed,
            new_or_modified,
            deleted,
            total,
        } => {
            state.refresh_status =
                refresh_status("refreshing", total, new_or_modified, deleted, elapsed);
            state.request_search_preserving_selection(true);
        }
        ScanMessage::Finished {
            elapsed,
            new_or_modified,
            deleted,
            total,
        } => {
            let _ = state.engine.reload();
            state.scanning = false;
            state.refresh_status =
                refresh_status("refreshed", total, new_or_modified, deleted, elapsed);
            state.request_search_preserving_selection(true);
        }
        ScanMessage::Failed { elapsed, error } => {
            state.scanning = false;
            state.refresh_status =
                format!("refresh failed after {}: {error}", elapsed_label(elapsed));
        }
    }
}

fn refresh_status(
    label: &str,
    total: usize,
    new_or_modified: usize,
    deleted: usize,
    elapsed: Duration,
) -> String {
    let mut status = format!("{label}: {total} sessions, {new_or_modified} changed");
    if deleted > 0 {
        status.push_str(&format!(", {deleted} deleted"));
    }
    status.push_str(&format!(", {}", elapsed_label(elapsed)));
    status
}

fn elapsed_label(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    if seconds >= 1.0 {
        format!("{seconds:.1}s")
    } else {
        format!("{:.0}ms", seconds * 1000.0)
    }
}
