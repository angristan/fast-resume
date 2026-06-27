use std::time::{Duration, Instant};

use crate::config::{AGENT_ORDER, is_agent};
use crate::model::Session;
use crate::query::{Filter, parse_query};
use crate::search::SearchEngine;

use super::images::AgentImages;
use super::text::char_to_byte_idx;

const DATE_SUGGESTIONS: [&str; 4] = ["today", "yesterday", "week", "month"];

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
}

pub(super) struct SearchRequest {
    pub(super) generation: u64,
    pub(super) query: String,
    pub(super) agent_filter: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum PendingAction {
    Resume,
    Copy,
}

#[derive(Debug, Clone)]
pub(super) struct YoloModal {
    pub(super) action: PendingAction,
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
    pub(super) yolo: bool,
    pub(super) scanning: bool,
    pub(super) status: String,
    pub(super) last_search_ms: f64,
    pub(super) show_preview: bool,
    pub(super) modal: Option<YoloModal>,
    pub(super) images: Option<AgentImages>,
    search_generation: u64,
    applied_search_generation: u64,
    search_requested: bool,
}

impl AppState {
    pub(super) fn new(
        query: String,
        agent_filter: Option<String>,
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
            yolo,
            scanning: true,
            status: "loading Tantivy index; refreshing session stores".to_string(),
            last_search_ms: 0.0,
            show_preview: true,
            modal: None,
            images,
            search_generation: 0,
            applied_search_generation: 0,
            search_requested: false,
        };
        state.refresh_search();
        state
    }

    pub(super) fn refresh_search(&mut self) {
        self.search_requested = false;
        self.search_generation = self.search_generation.saturating_add(1);
        self.applied_search_generation = self.search_generation;
        let start = Instant::now();
        let agent_filter = self.effective_agent_filter();
        self.visible = self
            .engine
            .search(&self.query, agent_filter.as_deref(), None, 100);
        self.last_search_ms = start.elapsed().as_secs_f64() * 1000.0;
        if self.visible.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.visible.len() - 1);
        }
        self.preview_scroll = 0;
    }

    pub(super) fn request_search(&mut self) {
        self.search_requested = true;
    }

    pub(super) fn take_search_request(&mut self) -> Option<SearchRequest> {
        if !self.search_requested {
            return None;
        }
        self.search_requested = false;
        self.search_generation = self.search_generation.saturating_add(1);
        Some(SearchRequest {
            generation: self.search_generation,
            query: self.query.clone(),
            agent_filter: self.effective_agent_filter(),
        })
    }

    pub(super) fn apply_search_result(
        &mut self,
        generation: u64,
        visible: Vec<Session>,
        elapsed_ms: f64,
    ) -> bool {
        if generation != self.search_generation {
            return false;
        }
        self.visible = visible;
        self.last_search_ms = elapsed_ms;
        self.applied_search_generation = generation;
        if self.visible.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.visible.len() - 1);
        }
        self.preview_scroll = 0;
        true
    }

    pub(super) fn ensure_current_search(&mut self) {
        if self.search_requested || self.applied_search_generation != self.search_generation {
            self.refresh_search();
        }
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
        let active = self.active_agent_filter();
        let current = active
            .as_deref()
            .and_then(|agent| AGENT_ORDER.iter().position(|candidate| *candidate == agent))
            .map(|idx| idx + 1)
            .unwrap_or(0);
        let len = AGENT_ORDER.len() + 1;
        let next = if reverse {
            (current + len - 1) % len
        } else {
            (current + 1) % len
        };
        let next_agent = if next == 0 {
            None
        } else {
            Some(AGENT_ORDER[next - 1].to_string())
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
            let _ = state.engine.reload();
            state.status = format!(
                "refreshing: {total} sessions, {new_or_modified} changed, {deleted} deleted in {:.1}ms",
                elapsed.as_secs_f64() * 1000.0
            );
            state.refresh_search();
        }
        ScanMessage::Finished {
            elapsed,
            new_or_modified,
            deleted,
            total,
        } => {
            let _ = state.engine.reload();
            state.scanning = false;
            state.status = format!(
                "refresh complete: {total} sessions, {new_or_modified} changed, {deleted} deleted in {:.1}ms",
                elapsed.as_secs_f64() * 1000.0
            );
            state.refresh_search();
        }
    }
}
