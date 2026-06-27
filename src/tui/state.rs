use std::time::{Duration, Instant};

use crate::config::AGENT_ORDER;
use crate::model::Session;
use crate::search::SearchEngine;

use super::images::AgentImages;
use super::text::char_to_byte_idx;

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
        self.visible = self
            .engine
            .search(&self.query, self.agent_filter.as_deref(), None, 100);
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
            agent_filter: self.agent_filter.clone(),
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

    pub(super) fn cycle_agent(&mut self, reverse: bool) {
        let current = self
            .agent_filter
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
        self.agent_filter = if next == 0 {
            None
        } else {
            Some(AGENT_ORDER[next - 1].to_string())
        };
        self.request_search();
    }

    pub(super) fn insert_char(&mut self, ch: char) {
        let byte_idx = char_to_byte_idx(&self.query, self.cursor);
        self.query.insert(byte_idx, ch);
        self.cursor += 1;
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
        self.request_search();
    }

    pub(super) fn delete(&mut self) {
        if self.cursor >= self.query.chars().count() {
            return;
        }
        let start = char_to_byte_idx(&self.query, self.cursor);
        let end = char_to_byte_idx(&self.query, self.cursor + 1);
        self.query.replace_range(start..end, "");
        self.request_search();
    }
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
