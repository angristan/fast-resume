use std::collections::HashMap;
use std::env;
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use arboard::Clipboard;
use chrono::{DateTime, Local};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use image::ImageReader;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect, Size};
use ratatui::prelude::Frame;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui_image::{
    Image as TuiImage, Resize,
    picker::{Picker, ProtocolType},
    protocol::Protocol,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::adapters::adapter_for;
use crate::config::{AGENT_ORDER, AGENTS, VERSION};
use crate::index::{INDEX_REFRESH_BATCH_SIZE, SessionIndex};
use crate::model::Session;
use crate::search::SearchEngine;

pub enum TuiExit {
    Quit,
    Resume {
        command: Vec<String>,
        directory: String,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ImageProtocol {
    Auto,
    Kitty,
    Sixel,
    Iterm2,
}

enum ScanMessage {
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

#[derive(Debug, Clone, Copy)]
enum PendingAction {
    Resume,
    Copy,
}

#[derive(Debug, Clone)]
struct YoloModal {
    action: PendingAction,
    selected: bool,
}

struct AppState {
    engine: SearchEngine,
    visible: Vec<Session>,
    query: String,
    cursor: usize,
    selected: usize,
    preview_scroll: u16,
    agent_filter: Option<String>,
    yolo: bool,
    scanning: bool,
    status: String,
    last_search_ms: f64,
    show_preview: bool,
    modal: Option<YoloModal>,
    images: Option<AgentImages>,
}

#[derive(Default)]
struct AgentImages {
    row: HashMap<String, Protocol>,
    preview: HashMap<String, Protocol>,
}

impl AppState {
    fn new(
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
        };
        state.refresh_search();
        state
    }

    fn refresh_search(&mut self) {
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

    fn selected_session(&self) -> Option<&Session> {
        self.visible.get(self.selected)
    }

    fn move_selection(&mut self, delta: isize) {
        if self.visible.is_empty() {
            self.selected = 0;
            return;
        }
        let max = self.visible.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, max) as usize;
        self.preview_scroll = 0;
    }

    fn cycle_agent(&mut self, reverse: bool) {
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
        self.refresh_search();
    }

    fn insert_char(&mut self, ch: char) {
        let byte_idx = char_to_byte_idx(&self.query, self.cursor);
        self.query.insert(byte_idx, ch);
        self.cursor += 1;
        self.refresh_search();
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = char_to_byte_idx(&self.query, self.cursor - 1);
        let end = char_to_byte_idx(&self.query, self.cursor);
        self.query.replace_range(start..end, "");
        self.cursor -= 1;
        self.refresh_search();
    }

    fn delete(&mut self) {
        if self.cursor >= self.query.chars().count() {
            return;
        }
        let start = char_to_byte_idx(&self.query, self.cursor);
        let end = char_to_byte_idx(&self.query, self.cursor + 1);
        self.query.replace_range(start..end, "");
        self.refresh_search();
    }
}

impl AgentImages {
    fn load(protocol: ImageProtocol) -> Option<Self> {
        let protocol_type = detect_image_protocol(protocol)?;
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(protocol_type);

        let row = load_agent_protocols(&picker, Size::new(2, 1));
        let preview = load_agent_protocols(&picker, Size::new(10, 5));
        if preview.is_empty() {
            return None;
        }

        Some(Self { row, preview })
    }
}

fn detect_image_protocol(protocol: ImageProtocol) -> Option<ProtocolType> {
    match protocol {
        ImageProtocol::Kitty => return Some(ProtocolType::Kitty),
        ImageProtocol::Sixel => return Some(ProtocolType::Sixel),
        ImageProtocol::Iterm2 => return Some(ProtocolType::Iterm2),
        ImageProtocol::Auto => {}
    }

    if env_present("KITTY_WINDOW_ID")
        || env_present("GHOSTTY_BIN_DIR")
        || env_eq("TERM_PROGRAM", "ghostty")
    {
        return Some(ProtocolType::Kitty);
    }

    if env_present("ITERM_SESSION_ID")
        || env_contains("TERM_PROGRAM", "iTerm")
        || env_contains("TERM_PROGRAM", "WezTerm")
        || env_present("WEZTERM_EXECUTABLE")
    {
        return Some(ProtocolType::Iterm2);
    }

    if env_contains("TERM", "sixel") {
        return Some(ProtocolType::Sixel);
    }

    None
}

fn env_present(key: &str) -> bool {
    env::var(key).is_ok_and(|value| !value.is_empty())
}

fn env_eq(key: &str, expected: &str) -> bool {
    env::var(key).is_ok_and(|value| value.eq_ignore_ascii_case(expected))
}

fn env_contains(key: &str, needle: &str) -> bool {
    env::var(key).is_ok_and(|value| value.contains(needle))
}

fn load_agent_protocols(picker: &Picker, size: Size) -> HashMap<String, Protocol> {
    let mut protocols = HashMap::new();
    for agent in AGENT_ORDER {
        let path = agent_asset_path(agent);
        let Ok(reader) = ImageReader::open(&path) else {
            continue;
        };
        let Ok(image) = reader.decode() else {
            continue;
        };
        if let Ok(protocol) = picker.new_protocol(image, size, Resize::Fit(None)) {
            protocols.insert(agent.to_string(), protocol);
        }
    }
    protocols
}

fn agent_asset_path(agent: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("fast_resume")
        .join("assets")
        .join(format!("{agent}.png"))
}

pub fn run_tui(
    query: String,
    agent_filter: Option<String>,
    yolo: bool,
    image_protocol: Option<ImageProtocol>,
) -> Result<TuiExit> {
    let engine = SearchEngine::open_default()?;
    let (scan_tx, scan_rx) = mpsc::channel();
    thread::spawn(move || {
        let start = Instant::now();
        let progress_tx = scan_tx.clone();
        let refreshed = SessionIndex::open_default().and_then(|index| {
            index.refresh_incremental_streaming(INDEX_REFRESH_BATCH_SIZE, |summary| {
                let _ = progress_tx.send(ScanMessage::Progress {
                    elapsed: start.elapsed(),
                    new_or_modified: summary.new_or_modified,
                    deleted: summary.deleted,
                    total: summary.sessions,
                });
            })
        });
        let (new_or_modified, deleted, total) = refreshed
            .map(|summary| (summary.new_or_modified, summary.deleted, summary.sessions))
            .unwrap_or((0, 0, 0));
        let _ = scan_tx.send(ScanMessage::Finished {
            elapsed: start.elapsed(),
            new_or_modified,
            deleted,
            total,
        });
    });

    let mut terminal = setup_terminal()?;
    let images = image_protocol.and_then(AgentImages::load);
    let mut state = AppState::new(query, agent_filter, yolo, engine, images);
    let result = run_loop(&mut terminal, &mut state, scan_rx);
    restore_terminal(&mut terminal)?;
    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &mut AppState,
    scan_rx: Receiver<ScanMessage>,
) -> Result<TuiExit> {
    let mut needs_draw = true;
    loop {
        let mut latest_scan_message = None;
        while let Ok(message) = scan_rx.try_recv() {
            let finished = matches!(message, ScanMessage::Finished { .. });
            latest_scan_message = Some(message);
            if finished {
                break;
            }
        }
        if let Some(message) = latest_scan_message {
            handle_scan_message(state, message);
            needs_draw = true;
        }

        if needs_draw {
            terminal.draw(|frame| draw(frame, state))?;
            needs_draw = false;
        }

        if event::poll(Duration::from_millis(24))? {
            if let Event::Key(key) = event::read()? {
                if let Some(exit) = handle_key(state, key)? {
                    return Ok(exit);
                }
                needs_draw = true;
            }
        }
    }
}

fn handle_scan_message(state: &mut AppState, message: ScanMessage) {
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

fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<Option<TuiExit>> {
    if state.modal.is_some() {
        return handle_modal_key(state, key);
    }

    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(Some(TuiExit::Quit)),
        (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
            if let Some(exit) = begin_action(state, PendingAction::Copy)? {
                return Ok(Some(exit));
            }
        }
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => state.show_preview = !state.show_preview,
        (KeyCode::Esc, _) => return Ok(Some(TuiExit::Quit)),
        (KeyCode::Enter, _) => {
            if let Some(exit) = begin_action(state, PendingAction::Resume)? {
                return Ok(Some(exit));
            }
        }
        (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::CONTROL) => state.move_selection(-1),
        (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::CONTROL) => state.move_selection(1),
        (KeyCode::PageUp, _) => state.move_selection(-10),
        (KeyCode::PageDown, _) => state.move_selection(10),
        (KeyCode::Tab, _) => state.cycle_agent(false),
        (KeyCode::BackTab, _) => state.cycle_agent(true),
        (KeyCode::Backspace, _) => state.backspace(),
        (KeyCode::Delete, _) => state.delete(),
        (KeyCode::Left, _) => state.cursor = state.cursor.saturating_sub(1),
        (KeyCode::Right, _) => state.cursor = (state.cursor + 1).min(state.query.chars().count()),
        (KeyCode::Home, _) => state.cursor = 0,
        (KeyCode::End, _) => state.cursor = state.query.chars().count(),
        (KeyCode::Char('+'), _) => state.preview_scroll = state.preview_scroll.saturating_sub(3),
        (KeyCode::Char('-'), _) => state.preview_scroll = state.preview_scroll.saturating_add(3),
        (KeyCode::Char(ch), KeyModifiers::NONE) | (KeyCode::Char(ch), KeyModifiers::SHIFT) => {
            if ch != '\n' && ch != '\r' {
                state.insert_char(ch);
            }
        }
        _ => {}
    }

    Ok(None)
}

fn begin_action(state: &mut AppState, action: PendingAction) -> Result<Option<TuiExit>> {
    let Some(session) = state.selected_session().cloned() else {
        return Ok(None);
    };

    if matches!(action, PendingAction::Resume) && session.agent == "crush" {
        state.status =
            "Crush sessions are searchable; open crush in the project and use its session picker"
                .to_string();
        return Ok(None);
    }

    let supports_yolo = adapter_for(&session.agent)
        .as_ref()
        .is_some_and(|adapter| adapter.supports_yolo());
    if state.yolo || session.yolo || !supports_yolo {
        return finish_action(state, action, state.yolo || session.yolo);
    }

    state.modal = Some(YoloModal {
        action,
        selected: false,
    });
    Ok(None)
}

fn handle_modal_key(state: &mut AppState, key: KeyEvent) -> Result<Option<TuiExit>> {
    let Some(modal) = state.modal.as_mut() else {
        return Ok(None);
    };

    match key.code {
        KeyCode::Esc => state.modal = None,
        KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
            modal.selected = !modal.selected;
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            let action = modal.action;
            state.modal = None;
            return finish_action(state, action, true);
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            let action = modal.action;
            state.modal = None;
            return finish_action(state, action, false);
        }
        KeyCode::Enter => {
            let yolo = modal.selected;
            let action = modal.action;
            state.modal = None;
            return finish_action(state, action, yolo);
        }
        _ => {}
    }

    Ok(None)
}

fn finish_action(
    state: &mut AppState,
    action: PendingAction,
    yolo: bool,
) -> Result<Option<TuiExit>> {
    let Some(session) = state.selected_session().cloned() else {
        return Ok(None);
    };
    let Some(adapter) = adapter_for(&session.agent) else {
        state.status = "No resume command available for selected session".to_string();
        return Ok(None);
    };
    let command = adapter.resume_command(&session, yolo);
    match action {
        PendingAction::Resume => Ok(Some(TuiExit::Resume {
            command,
            directory: session.directory,
        })),
        PendingAction::Copy => {
            let command = shell_join(&command);
            let full = if session.directory.is_empty() {
                command
            } else {
                format!("cd {} && {}", shell_quote(&session.directory), command)
            };
            match Clipboard::new().and_then(|mut clipboard| clipboard.set_text(full.clone())) {
                Ok(()) => state.status = format!("copied: {full}"),
                Err(_) => state.status = format!("clipboard unavailable: {full}"),
            }
            Ok(None)
        }
    }
}

fn draw(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(frame, outer[0], state);
    draw_search(frame, outer[1], state);
    draw_filters(frame, outer[2], state);
    draw_main(frame, outer[3], state);
    draw_footer(frame, outer[4]);

    if let Some(modal) = &state.modal {
        draw_yolo_modal(frame, area, modal);
    }
}

fn draw_header(frame: &mut Frame, area: Rect, state: &AppState) {
    let scan = if state.scanning { " refreshing" } else { "" };
    let left = Line::from(vec![
        Span::styled(
            "fast-resume",
            Style::new().bold().fg(Color::Rgb(80, 220, 170)),
        ),
        Span::raw(format!(" v{VERSION}")),
        Span::styled(scan, Style::new().fg(Color::Rgb(240, 180, 80))),
    ]);
    let count = state.engine.count_for_agent(state.agent_filter.as_deref());
    let right = format!(
        "{} shown / {} indexed   {:.1}ms",
        state.visible.len(),
        count,
        state.last_search_ms
    );
    let mut spans = left.spans;
    let pad = area
        .width
        .saturating_sub(line_width(&Line::from(spans.clone())) as u16)
        .saturating_sub(right.width() as u16);
    spans.push(Span::raw(" ".repeat(pad as usize)));
    spans.push(Span::styled(right, Style::new().fg(Color::DarkGray)));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_search(frame: &mut Frame, area: Rect, state: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Rgb(80, 220, 170)))
        .title(" Search titles and messages ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let prompt = Span::styled(" / ", Style::new().fg(Color::Rgb(80, 220, 170)).bold());
    let query = Span::raw(state.query.as_str());
    frame.render_widget(Paragraph::new(Line::from(vec![prompt, query])), inner);

    let cursor_x = inner.x + 3 + display_width_until(&state.query, state.cursor) as u16;
    if cursor_x < inner.right() {
        frame.set_cursor_position((cursor_x, inner.y));
    }
}

fn draw_filters(frame: &mut Frame, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let mut x = area.x;
    x = draw_filter_tab(
        frame,
        area,
        x,
        "All",
        state.agent_filter.is_none(),
        Color::White,
        None,
    );
    for agent in AGENT_ORDER {
        let config = AGENTS.get(agent).expect("known agent");
        let active = state.agent_filter.as_deref() == Some(agent);
        let icon = state
            .images
            .as_ref()
            .and_then(|images| images.row.get(agent));
        x = draw_filter_tab(frame, area, x, config.badge, active, config.color, icon);
    }
}

fn draw_filter_tab(
    frame: &mut Frame,
    area: Rect,
    x: u16,
    label: &str,
    active: bool,
    color: Color,
    icon: Option<&Protocol>,
) -> u16 {
    let has_icon = icon.is_some();
    let label_width = label.width() as u16;
    let tab_width = label_width + if has_icon { 5 } else { 3 };
    if x >= area.right() {
        return x.saturating_add(tab_width);
    }

    let style = if active {
        Style::new()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(color)
    };

    let visible_width = tab_width.min(area.right().saturating_sub(x));
    frame.render_widget(
        Paragraph::new(" ".repeat(visible_width as usize)).style(style),
        Rect::new(x, area.y, visible_width, 1),
    );

    if let Some(protocol) = icon {
        if x + 1 < area.right() {
            let icon_width = 2.min(area.right().saturating_sub(x + 1));
            frame.render_widget(
                TuiImage::new(protocol).allow_clipping(true),
                Rect::new(x + 1, area.y, icon_width, 1),
            );
        }
        if x + 4 < area.right() {
            frame.render_widget(
                Paragraph::new(truncate(label, area.right().saturating_sub(x + 4) as usize))
                    .style(style),
                Rect::new(x + 4, area.y, label_width.min(area.right() - (x + 4)), 1),
            );
        }
    } else if x + 1 < area.right() {
        frame.render_widget(
            Paragraph::new(truncate(label, area.right().saturating_sub(x + 1) as usize))
                .style(style),
            Rect::new(x + 1, area.y, label_width.min(area.right() - (x + 1)), 1),
        );
    }

    x.saturating_add(tab_width)
}

fn draw_main(frame: &mut Frame, area: Rect, state: &AppState) {
    if state.show_preview {
        if area.width >= 116 {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
                .split(area);
            draw_results(frame, chunks[0], state);
            draw_preview(frame, chunks[1], state);
        } else {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(8), Constraint::Length(12)])
                .split(area);
            draw_results(frame, chunks[0], state);
            draw_preview(frame, chunks[1], state);
        }
    } else {
        draw_results(frame, area, state);
    }
}

fn draw_results(frame: &mut Frame, area: Rect, state: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Rgb(70, 80, 95)))
        .title(" Sessions ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let columns = result_columns(inner.width);
    draw_result_header(frame, inner, columns);

    let rows_area = Rect::new(
        inner.x,
        inner.y.saturating_add(1),
        inner.width,
        inner.height.saturating_sub(1),
    );

    if state.visible.is_empty() {
        frame.render_widget(
            Paragraph::new("  No sessions found").style(Style::new().fg(Color::DarkGray).italic()),
            rows_area,
        );
        return;
    }

    let max_rows = rows_area.height as usize;
    if max_rows == 0 {
        return;
    }
    let start = state
        .selected
        .saturating_sub(max_rows.saturating_sub(1))
        .min(state.visible.len().saturating_sub(1));
    let end = (start + max_rows).min(state.visible.len());

    for (screen_row, session) in state.visible[start..end].iter().enumerate() {
        let row_y = rows_area.y + screen_row as u16;
        let selected = start + screen_row == state.selected;
        draw_result_row(frame, rows_area, row_y, columns, session, selected, state);
    }
}

#[derive(Clone, Copy)]
struct ResultColumns {
    agent_x: u16,
    agent_w: u16,
    title_x: u16,
    title_w: u16,
    dir_x: u16,
    dir_w: u16,
    turns_x: u16,
    turns_w: u16,
    age_x: u16,
    age_w: u16,
}

fn result_columns(width: u16) -> ResultColumns {
    let (agent_w, dir_w, turns_w, age_w) = if width >= 100 {
        (15, 32, 7, 10)
    } else if width >= 72 {
        (13, 22, 6, 9)
    } else {
        (11, 0, 5, 8)
    };
    let fixed = agent_w + dir_w + turns_w + age_w + 4;
    let title_w = width.saturating_sub(fixed).max(16);
    let agent_x = 0;
    let title_x = agent_x + agent_w + 1;
    let dir_x = title_x + title_w + 1;
    let turns_x = dir_x + dir_w + 1;
    let age_x = turns_x + turns_w + 1;
    ResultColumns {
        agent_x,
        agent_w,
        title_x,
        title_w,
        dir_x,
        dir_w,
        turns_x,
        turns_w,
        age_x,
        age_w,
    }
}

fn draw_result_header(frame: &mut Frame, inner: Rect, columns: ResultColumns) {
    let style = Style::new().fg(Color::Gray).bold();
    draw_cell(
        frame,
        inner,
        columns.agent_x,
        0,
        columns.agent_w,
        "  Agent",
        style,
    );
    draw_cell(
        frame,
        inner,
        columns.title_x,
        0,
        columns.title_w,
        "Title",
        style,
    );
    if columns.dir_w > 0 {
        draw_cell(
            frame,
            inner,
            columns.dir_x,
            0,
            columns.dir_w,
            "Directory",
            style,
        );
    }
    draw_cell(
        frame,
        inner,
        columns.turns_x,
        0,
        columns.turns_w,
        "Turns",
        style,
    );
    draw_cell(frame, inner, columns.age_x, 0, columns.age_w, "Age", style);
}

fn draw_result_row(
    frame: &mut Frame,
    rows_area: Rect,
    row_y: u16,
    columns: ResultColumns,
    session: &Session,
    selected: bool,
    state: &AppState,
) {
    let row_style = if selected {
        Style::new().bg(Color::Rgb(36, 57, 52)).fg(Color::White)
    } else {
        Style::new()
    };
    frame.render_widget(
        Paragraph::new(" ".repeat(rows_area.width as usize)).style(row_style),
        Rect::new(rows_area.x, row_y, rows_area.width, 1),
    );

    let agent_color = AGENTS
        .get(session.agent.as_str())
        .map(|agent| agent.color)
        .unwrap_or(Color::White);
    let pointer = if selected { "▸" } else { " " };
    draw_cell(
        frame,
        rows_area,
        0,
        row_y - rows_area.y,
        1,
        pointer,
        row_style,
    );

    let label_x = if let Some(protocol) = state
        .images
        .as_ref()
        .and_then(|images| images.row.get(&session.agent))
    {
        frame.render_widget(
            TuiImage::new(protocol).allow_clipping(true),
            Rect::new(rows_area.x + 2, row_y, 2, 1),
        );
        5
    } else {
        2
    };

    draw_cell(
        frame,
        rows_area,
        label_x,
        row_y - rows_area.y,
        columns.agent_w.saturating_sub(label_x),
        &truncate(
            &session.agent,
            columns.agent_w.saturating_sub(label_x) as usize,
        ),
        row_style.fg(agent_color).add_modifier(Modifier::BOLD),
    );
    draw_cell(
        frame,
        rows_area,
        columns.title_x,
        row_y - rows_area.y,
        columns.title_w,
        &truncate(&session.title, columns.title_w as usize),
        row_style,
    );
    if columns.dir_w > 0 {
        draw_cell(
            frame,
            rows_area,
            columns.dir_x,
            row_y - rows_area.y,
            columns.dir_w,
            &truncate(&session.display_directory(), columns.dir_w as usize),
            row_style.fg(Color::DarkGray),
        );
    }
    draw_cell(
        frame,
        rows_area,
        columns.turns_x,
        row_y - rows_area.y,
        columns.turns_w,
        &session.message_count.to_string(),
        row_style,
    );
    draw_cell(
        frame,
        rows_area,
        columns.age_x,
        row_y - rows_area.y,
        columns.age_w,
        &time_ago(session.timestamp),
        age_style(session.timestamp).bg(row_style.bg.unwrap_or(Color::Reset)),
    );
}

fn draw_cell(frame: &mut Frame, area: Rect, x: u16, y: u16, width: u16, text: &str, style: Style) {
    if width == 0 || x >= area.width || y >= area.height {
        return;
    }
    frame.render_widget(
        Paragraph::new(truncate(text, width as usize)).style(style),
        Rect::new(area.x + x, area.y + y, width.min(area.width - x), 1),
    );
}

fn draw_preview(frame: &mut Frame, area: Rect, state: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Rgb(70, 80, 95)))
        .title(" Preview ");
    let inner = block.inner(area).inner(Margin {
        vertical: 0,
        horizontal: 1,
    });
    frame.render_widget(block, area);

    let Some(session) = state.selected_session() else {
        frame.render_widget(Paragraph::new("No session selected").dark_gray(), inner);
        return;
    };

    let agent_color = AGENTS
        .get(session.agent.as_str())
        .map(|agent| agent.color)
        .unwrap_or(Color::White);
    let header_lines = vec![
        Line::from(vec![
            Span::styled(&session.agent, Style::new().fg(agent_color).bold()),
            Span::raw("  "),
            Span::styled(&session.title, Style::new().bold()),
        ]),
        Line::from(vec![
            Span::styled(
                session.display_directory(),
                Style::new().fg(Color::DarkGray),
            ),
            Span::raw("  "),
            Span::styled(
                session.timestamp.format("%Y-%m-%d %H:%M").to_string(),
                Style::new().fg(Color::DarkGray),
            ),
        ]),
    ];

    let mut body_area = inner;
    if let Some(protocol) = state
        .images
        .as_ref()
        .and_then(|images| images.preview.get(&session.agent))
    {
        if inner.width > 48 && inner.height > 7 {
            let logo_area = Rect::new(inner.right().saturating_sub(8), inner.y, 8, 4);
            let text_area = Rect::new(inner.x, inner.y, inner.width.saturating_sub(9), 3);
            frame.render_widget(Paragraph::new(Text::from(header_lines.clone())), text_area);
            frame.render_widget(TuiImage::new(protocol).allow_clipping(true), logo_area);
            body_area = Rect::new(
                inner.x,
                inner.y + 4,
                inner.width,
                inner.height.saturating_sub(4),
            );
        }
    }

    let mut lines = if body_area.y == inner.y {
        let mut lines = header_lines;
        lines.push(Line::raw(""));
        lines
    } else {
        Vec::new()
    };

    let snippet = preview_snippet(session, &state.query);
    for line in snippet.lines().take(220) {
        lines.push(render_preview_line(line, &state.query));
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .scroll((state.preview_scroll, 0)),
        body_area,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect) {
    let footer = Line::from(vec![
        Span::styled(
            " Enter ",
            Style::new().fg(Color::Black).bg(Color::Rgb(80, 220, 170)),
        ),
        Span::raw(" resume  "),
        Span::styled(" Ctrl+Y ", Style::new().fg(Color::Black).bg(Color::Gray)),
        Span::raw(" copy  "),
        Span::styled(" Tab ", Style::new().fg(Color::Black).bg(Color::Gray)),
        Span::raw(" agent  "),
        Span::styled(" Ctrl+P ", Style::new().fg(Color::Black).bg(Color::Gray)),
        Span::raw(" preview  "),
        Span::styled(" Esc ", Style::new().fg(Color::Black).bg(Color::Gray)),
        Span::raw(" quit"),
    ]);
    frame.render_widget(Paragraph::new(footer), area);
}

fn draw_yolo_modal(frame: &mut Frame, area: Rect, modal: &YoloModal) {
    let popup = centered_rect(48, 8, area);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Rgb(240, 180, 80)))
        .title(" Yolo mode ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let text = vec![
        Line::from("Resume with auto-approve / skip-permissions flags?"),
        Line::raw(""),
        Line::from(vec![
            button_span(" No ", !modal.selected),
            Span::raw("  "),
            button_span(" Yolo ", modal.selected),
        ])
        .alignment(Alignment::Center),
    ];
    frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), inner);
}

fn button_span(label: &'static str, selected: bool) -> Span<'static> {
    if selected {
        Span::styled(
            label,
            Style::new()
                .fg(Color::Black)
                .bg(Color::Rgb(240, 180, 80))
                .bold(),
        )
    } else {
        Span::styled(label, Style::new().fg(Color::Gray))
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn char_to_byte_idx(value: &str, char_idx: usize) -> usize {
    value
        .char_indices()
        .nth(char_idx)
        .map(|(idx, _)| idx)
        .unwrap_or(value.len())
}

fn display_width_until(value: &str, char_idx: usize) -> usize {
    value.chars().take(char_idx).collect::<String>().width()
}

fn line_width(line: &Line) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.as_ref().width())
        .sum()
}

fn truncate(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if value.width() <= width {
        return value.to_string();
    }
    let keep = width.saturating_sub(3);
    let mut out = String::new();
    for ch in value.chars() {
        if out.width() + ch.width().unwrap_or(0) > keep {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn time_ago(timestamp: DateTime<Local>) -> String {
    let delta = Local::now().signed_duration_since(timestamp);
    if delta.num_minutes() < 1 {
        "now".to_string()
    } else if delta.num_hours() < 1 {
        format!("{}m", delta.num_minutes())
    } else if delta.num_days() < 1 {
        format!("{}h", delta.num_hours())
    } else if delta.num_days() < 30 {
        format!("{}d", delta.num_days())
    } else if delta.num_days() < 365 {
        format!("{}mo", delta.num_days() / 30)
    } else {
        format!("{}y", delta.num_days() / 365)
    }
}

fn age_style(timestamp: DateTime<Local>) -> Style {
    let hours = Local::now()
        .signed_duration_since(timestamp)
        .num_hours()
        .max(0) as f64;
    let t = (1.0 - (-0.0149 * hours).exp()).clamp(0.0, 1.0);
    let (r, g, b) = if t < 0.3 {
        let s = t / 0.3;
        (100.0 + s * 100.0, 200.0 - s * 20.0, 50.0 - s * 50.0)
    } else if t < 0.6 {
        let s = (t - 0.3) / 0.3;
        (200.0, 180.0 - s * 80.0, s * 50.0)
    } else {
        let s = (t - 0.6) / 0.4;
        (200.0 - s * 100.0, 100.0, 50.0 + s * 50.0)
    };
    Style::new().fg(Color::Rgb(r as u8, g as u8, b as u8))
}

fn preview_snippet(session: &Session, query: &str) -> String {
    if query.trim().is_empty() {
        return truncate_chars(&session.content, 6_000);
    }

    let content_lc = session.content.to_ascii_lowercase();
    let mut best = None;
    for term in query
        .split_whitespace()
        .map(|term| term.to_ascii_lowercase())
    {
        if let Some(pos) = content_lc.find(&term) {
            best = Some(best.map_or(pos, |current: usize| current.min(pos)));
        }
    }

    if let Some(pos) = best {
        let start = session.content[..pos]
            .char_indices()
            .rev()
            .nth(220)
            .map(|(idx, _)| idx)
            .unwrap_or(0);
        let end = session.content[pos..]
            .char_indices()
            .nth(5_000)
            .map(|(idx, _)| pos + idx)
            .unwrap_or(session.content.len());
        let mut snippet = String::new();
        if start > 0 {
            snippet.push_str("...\n");
        }
        snippet.push_str(&session.content[start..end]);
        if end < session.content.len() {
            snippet.push_str("\n...");
        }
        snippet
    } else {
        truncate_chars(&session.content, 6_000)
    }
}

fn render_preview_line(line: &str, query: &str) -> Line<'static> {
    let style = if line.starts_with("» ") {
        Style::new().fg(Color::Rgb(120, 210, 255)).bold()
    } else if line.starts_with("  ") {
        Style::new().fg(Color::White)
    } else if line.starts_with("...") {
        Style::new().fg(Color::DarkGray)
    } else {
        Style::new().fg(Color::Gray)
    };

    if query.trim().is_empty() {
        return Line::from(Span::styled(line.to_string(), style));
    }

    let terms: Vec<_> = query
        .split_whitespace()
        .map(|term| term.to_ascii_lowercase())
        .collect();
    let lower = line.to_ascii_lowercase();
    let mut spans = Vec::new();
    let mut idx = 0usize;
    while idx < line.len() {
        let next = terms
            .iter()
            .filter_map(|term| lower[idx..].find(term).map(|pos| (idx + pos, term.len())))
            .min_by_key(|(pos, _)| *pos);
        let Some((hit, len)) = next else {
            spans.push(Span::styled(line[idx..].to_string(), style));
            break;
        };
        if hit > idx {
            spans.push(Span::styled(line[idx..hit].to_string(), style));
        }
        let end = (hit + len).min(line.len());
        spans.push(Span::styled(
            line[hit..end].to_string(),
            Style::new()
                .fg(Color::Black)
                .bg(Color::Rgb(250, 220, 110))
                .bold(),
        ));
        idx = end;
    }
    Line::from(spans)
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max).collect();
    out.push_str("\n...");
    out
}

fn shell_join(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| shell_quote(part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "-_./:=+".contains(ch))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(area.width.saturating_sub(width) / 2),
            Constraint::Length(width.min(area.width)),
            Constraint::Min(0),
        ])
        .split(area);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height.min(area.height)),
            Constraint::Min(0),
        ])
        .split(horizontal[1]);
    vertical[1]
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::index::SessionIndex;

    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn session(id: &str) -> Session {
        Session::new(
            id,
            "codex",
            format!("Session {id}"),
            "/tmp/fast-resume",
            Local::now(),
            "message",
            1,
        )
    }

    fn test_state(sessions: Vec<Session>) -> AppState {
        let temp = tempdir().unwrap();
        let path = temp.keep();
        let index = SessionIndex::open(path.join("index")).unwrap();
        index.rebuild(sessions).unwrap();
        AppState::new(
            String::new(),
            None,
            false,
            SearchEngine::from_index(index),
            None,
        )
    }

    #[test]
    fn plain_j_and_k_type_into_search() {
        let mut state = test_state(Vec::new());

        handle_key(&mut state, key(KeyCode::Char('j'), KeyModifiers::NONE)).unwrap();
        handle_key(&mut state, key(KeyCode::Char('k'), KeyModifiers::NONE)).unwrap();

        assert_eq!(state.query, "jk");
        assert_eq!(state.cursor, 2);
    }

    #[test]
    fn ctrl_j_and_ctrl_k_keep_navigation_shortcuts() {
        let mut state = test_state(vec![session("a"), session("b")]);

        handle_key(&mut state, key(KeyCode::Char('j'), KeyModifiers::CONTROL)).unwrap();
        assert_eq!(state.selected, 1);
        assert!(state.query.is_empty());

        handle_key(&mut state, key(KeyCode::Char('k'), KeyModifiers::CONTROL)).unwrap();
        assert_eq!(state.selected, 0);
        assert!(state.query.is_empty());
    }
}
