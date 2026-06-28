use std::io::{self, Stdout};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;

use crate::index::{INDEX_REFRESH_BATCH_SIZE, SessionIndex};
use crate::model::Session;
use crate::search::SearchEngine;

mod images;
mod input;
mod layout;
mod preview;
mod render;
mod state;
mod text;

use images::AgentImages;
use input::handle_key;
use layout::ScrollTarget;
use render::draw;
use state::{AppState, ScanMessage, SearchRequest, handle_scan_message};

pub use images::ImageProtocol;

pub enum TuiExit {
    Quit,
    Resume {
        command: Vec<String>,
        directory: String,
    },
}

pub fn run_tui(
    query: String,
    agent_filter: Option<String>,
    directory_filter: Option<String>,
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
    let mut state = AppState::new(query, agent_filter, directory_filter, yolo, engine, images);
    let result = run_loop(&mut terminal, &mut state, scan_rx);
    restore_terminal(&mut terminal)?;
    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &mut AppState,
    scan_rx: Receiver<ScanMessage>,
) -> Result<TuiExit> {
    let (search_tx, search_rx) = spawn_search_worker(state.engine.clone());
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

        while let Ok(result) = search_rx.try_recv() {
            if state.apply_search_result(result.generation, result.visible, result.elapsed_ms) {
                needs_draw = true;
            }
        }

        if needs_draw {
            terminal.draw(|frame| draw(frame, state))?;
            needs_draw = false;
        }

        if event::poll(Duration::from_millis(24))? {
            match event::read()? {
                Event::Key(key) => {
                    if let Some(exit) = handle_key(state, key)? {
                        return Ok(exit);
                    }
                    start_search_if_requested(state, &search_tx);
                    needs_draw = true;
                }
                Event::Resize(_, _) => {
                    terminal.autoresize()?;
                    needs_draw = true;
                }
                Event::Mouse(mouse) => {
                    let size = terminal.size()?;
                    let area = Rect::new(0, 0, size.width, size.height);
                    if handle_mouse(state, mouse, area) {
                        needs_draw = true;
                    }
                }
                _ => {}
            }
        }
    }
}

struct SearchResult {
    generation: u64,
    visible: Vec<Session>,
    elapsed_ms: f64,
}

fn start_search_if_requested(state: &mut AppState, tx: &Sender<SearchRequest>) {
    let Some(request) = state.take_search_request() else {
        return;
    };
    let _ = tx.send(request);
}

fn spawn_search_worker(engine: SearchEngine) -> (Sender<SearchRequest>, Receiver<SearchResult>) {
    let (request_tx, request_rx) = mpsc::channel::<SearchRequest>();
    let (result_tx, result_rx) = mpsc::channel::<SearchResult>();
    thread::spawn(move || {
        while let Ok(request) = request_rx.recv() {
            let result = run_search(&engine, latest_search_request(&request_rx, request));
            if result_tx.send(result).is_err() {
                break;
            }
        }
    });
    (request_tx, result_rx)
}

fn latest_search_request(
    request_rx: &Receiver<SearchRequest>,
    mut request: SearchRequest,
) -> SearchRequest {
    while let Ok(latest) = request_rx.try_recv() {
        request = latest;
    }
    request
}

fn run_search(engine: &SearchEngine, request: SearchRequest) -> SearchResult {
    let start = Instant::now();
    let visible = engine.search(
        &request.query,
        request.agent_filter.as_deref(),
        request.directory_filter.as_deref(),
        100,
    );
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    SearchResult {
        generation: request.generation,
        visible,
        elapsed_ms,
    }
}

const MOUSE_SCROLL_LINES: isize = 3;

fn handle_mouse(state: &mut AppState, mouse: MouseEvent, area: Rect) -> bool {
    let delta = match mouse.kind {
        MouseEventKind::ScrollUp => -MOUSE_SCROLL_LINES,
        MouseEventKind::ScrollDown => MOUSE_SCROLL_LINES,
        _ => return false,
    };

    match layout::scroll_target(area, state.show_preview, mouse.column, mouse.row) {
        Some(ScrollTarget::Results) => {
            state.move_selection(delta);
            true
        }
        Some(ScrollTarget::Preview) => {
            state.scroll_preview(delta);
            true
        }
        None => false,
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{Duration as ChronoDuration, Local};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;
    use tempfile::tempdir;

    use crate::index::SessionIndex;
    use crate::model::Session;
    use crate::search::SearchEngine;

    use super::input::handle_key;
    use super::state::{AppState, SearchRequest};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn search_request(generation: u64, query: &str) -> SearchRequest {
        SearchRequest {
            generation,
            query: query.to_string(),
            agent_filter: None,
            directory_filter: None,
        }
    }

    fn session(id: &str) -> Session {
        session_in(id, "/tmp/fast-resume")
    }

    fn session_in(id: &str, directory: &str) -> Session {
        Session::new(
            id,
            "codex",
            format!("Session {id}"),
            directory,
            Local::now(),
            "message",
            1,
        )
    }

    fn test_state(sessions: Vec<Session>) -> AppState {
        test_state_with_directory_filter(sessions, None)
    }

    #[test]
    fn pending_search_requests_coalesce_to_latest() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(search_request(2, "second")).unwrap();
        tx.send(search_request(3, "third")).unwrap();

        let latest = super::latest_search_request(&rx, search_request(1, "first"));

        assert_eq!(latest.generation, 3);
        assert_eq!(latest.query, "third");
    }

    fn test_state_with_directory_filter(
        sessions: Vec<Session>,
        directory_filter: Option<String>,
    ) -> AppState {
        test_state_and_index(sessions, directory_filter).0
    }

    fn test_state_and_index(
        sessions: Vec<Session>,
        directory_filter: Option<String>,
    ) -> (AppState, SessionIndex) {
        let temp = tempdir().unwrap();
        let path = temp.keep();
        let index = SessionIndex::open(path.join("index")).unwrap();
        index.rebuild(sessions).unwrap();
        let state = AppState::new(
            String::new(),
            None,
            directory_filter,
            false,
            SearchEngine::from_index(index.clone()),
            None,
        );
        (state, index)
    }

    fn type_query(state: &mut AppState, query: &str) {
        for ch in query.chars() {
            handle_key(state, key(KeyCode::Char(ch), KeyModifiers::NONE)).unwrap();
        }
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
    fn plus_and_minus_type_into_search() {
        let mut state = test_state(Vec::new());

        handle_key(&mut state, key(KeyCode::Char('-'), KeyModifiers::NONE)).unwrap();
        handle_key(&mut state, key(KeyCode::Char('+'), KeyModifiers::SHIFT)).unwrap();

        assert_eq!(state.query, "-+");
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

    #[test]
    fn alt_plus_and_minus_scroll_preview() {
        let mut state = test_state(Vec::new());
        state.preview_scroll = 3;

        handle_key(
            &mut state,
            key(KeyCode::Char('+'), KeyModifiers::ALT | KeyModifiers::SHIFT),
        )
        .unwrap();
        assert_eq!(state.preview_scroll, 0);

        handle_key(&mut state, key(KeyCode::Char('-'), KeyModifiers::ALT)).unwrap();
        assert_eq!(state.preview_scroll, 3);
        assert!(state.query.is_empty());
    }

    #[test]
    fn tab_accepts_agent_suggestion_before_cycling_filter() {
        let mut state = test_state(Vec::new());

        type_query(&mut state, "agent:c");
        assert_eq!(state.suggestion_suffix().as_deref(), Some("laude"));

        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE)).unwrap();

        assert_eq!(state.query, "agent:claude");
        assert_eq!(state.cursor, "agent:claude".chars().count());
        assert_eq!(state.active_agent_filter().as_deref(), Some("claude"));
    }

    #[test]
    fn tab_accepts_date_suggestion() {
        let mut state = test_state(Vec::new());

        type_query(&mut state, "date:y");

        assert_eq!(state.suggestion_suffix().as_deref(), Some("esterday"));
        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE)).unwrap();

        assert_eq!(state.query, "date:yesterday");
    }

    #[test]
    fn tab_cycles_filter_into_query_when_there_is_no_suggestion() {
        let mut state = test_state(Vec::new());

        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE)).unwrap();

        assert_eq!(state.query, "agent:claude");
        assert_eq!(state.active_agent_filter().as_deref(), Some("claude"));
        let request = state.take_search_request().unwrap();
        assert_eq!(request.query, "agent:claude");
        assert_eq!(request.agent_filter, None);
        assert_eq!(request.directory_filter, None);
    }

    #[test]
    fn deleting_cycled_filter_keyword_clears_filter() {
        let mut state = test_state(Vec::new());

        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE)).unwrap();
        for _ in 0.."agent:claude".chars().count() {
            handle_key(&mut state, key(KeyCode::Backspace, KeyModifiers::NONE)).unwrap();
        }

        assert!(state.query.is_empty());
        assert_eq!(state.active_agent_filter(), None);
        let request = state.take_search_request().unwrap();
        assert_eq!(request.agent_filter, None);
        assert_eq!(request.directory_filter, None);
    }

    #[test]
    fn reverse_filter_cycle_removes_agent_keyword_for_all() {
        let mut state = test_state(Vec::new());

        type_query(&mut state, "api agent:claude");
        handle_key(&mut state, key(KeyCode::BackTab, KeyModifiers::SHIFT)).unwrap();

        assert_eq!(state.query, "api");
        assert_eq!(state.active_agent_filter(), None);
    }

    #[test]
    fn typed_agent_keyword_syncs_filter_without_overriding_query_filter() {
        let mut state = test_state(Vec::new());

        type_query(&mut state, "agent:claude,codex");

        assert_eq!(state.active_agent_filter(), None);
        assert_eq!(
            state.active_agent_filters(),
            vec!["claude".to_string(), "codex".to_string()]
        );
        assert!(!state.all_agent_filter_active());
        let request = state.take_search_request().unwrap();
        assert_eq!(request.query, "agent:claude,codex");
        assert_eq!(request.agent_filter, None);
        assert_eq!(request.directory_filter, None);
    }

    #[test]
    fn negated_agent_keyword_does_not_activate_all_filter() {
        let mut state = test_state(Vec::new());

        type_query(&mut state, "-agent:claude");

        assert!(state.active_agent_filters().is_empty());
        assert!(!state.all_agent_filter_active());
        let request = state.take_search_request().unwrap();
        assert_eq!(request.query, "-agent:claude");
        assert_eq!(request.agent_filter, None);
        assert_eq!(request.directory_filter, None);
    }

    #[test]
    fn cli_directory_filter_limits_tui_search_until_query_overrides_it() {
        let mut state = test_state_with_directory_filter(
            vec![
                session_in("backend", "/work/backend"),
                session_in("frontend", "/work/frontend"),
            ],
            Some("backend".to_string()),
        );

        assert_eq!(state.visible.len(), 1);
        assert_eq!(state.visible[0].id, "backend");

        handle_key(&mut state, key(KeyCode::Char('a'), KeyModifiers::NONE)).unwrap();
        let request = state.take_search_request().unwrap();
        assert_eq!(request.directory_filter.as_deref(), Some("backend"));

        state.query = "dir:frontend".to_string();
        state.cursor = state.query.chars().count();
        state.refresh_search();

        assert_eq!(state.visible.len(), 1);
        assert_eq!(state.visible[0].id, "frontend");
    }

    #[test]
    fn mouse_wheel_over_results_moves_selection() {
        let mut state = test_state((0..10).map(|idx| session(&idx.to_string())).collect());

        assert!(super::handle_mouse(
            &mut state,
            mouse(MouseEventKind::ScrollDown, 10, 6),
            Rect::new(0, 0, 120, 40),
        ));

        assert_eq!(state.selected, 3);
        assert_eq!(state.preview_scroll, 0);
    }

    #[test]
    fn mouse_wheel_over_preview_scrolls_preview() {
        let mut state = test_state(vec![session("a")]);

        assert!(super::handle_mouse(
            &mut state,
            mouse(MouseEventKind::ScrollDown, 100, 6),
            Rect::new(0, 0, 120, 40),
        ));

        assert_eq!(state.selected, 0);
        assert_eq!(state.preview_scroll, 3);
    }

    #[test]
    fn mouse_wheel_outside_main_area_is_ignored() {
        let mut state = test_state(vec![session("a")]);

        assert!(!super::handle_mouse(
            &mut state,
            mouse(MouseEventKind::ScrollDown, 10, 1),
            Rect::new(0, 0, 120, 40),
        ));

        assert_eq!(state.selected, 0);
        assert_eq!(state.preview_scroll, 0);
    }

    #[test]
    fn typing_requests_search_without_blocking_visible_results() {
        let mut state = test_state(vec![session("a")]);

        handle_key(&mut state, key(KeyCode::Char('z'), KeyModifiers::NONE)).unwrap();

        assert_eq!(state.query, "z");
        assert_eq!(state.visible.len(), 1);
        let request = state.take_search_request().unwrap();
        assert_eq!(request.query, "z");
        assert_eq!(request.agent_filter, None);
        assert_eq!(request.directory_filter, None);
        assert!(state.take_search_request().is_none());
    }

    #[test]
    fn stale_search_results_are_ignored() {
        let mut state = test_state(vec![session("a")]);

        handle_key(&mut state, key(KeyCode::Char('a'), KeyModifiers::NONE)).unwrap();
        let stale = state.take_search_request().unwrap();
        handle_key(&mut state, key(KeyCode::Char('b'), KeyModifiers::NONE)).unwrap();
        let latest = state.take_search_request().unwrap();

        assert!(!state.apply_search_result(stale.generation, Vec::new(), 10.0));
        assert_eq!(state.visible.len(), 1);
        assert!(state.apply_search_result(latest.generation, Vec::new(), 1.0));
        assert!(state.visible.is_empty());
    }

    #[test]
    fn new_search_results_reset_selection_to_top() {
        let mut state = test_state(vec![session("a"), session("b"), session("c")]);
        state.move_selection(2);
        assert_eq!(state.selected, 2);

        handle_key(&mut state, key(KeyCode::Char('z'), KeyModifiers::NONE)).unwrap();
        let request = state.take_search_request().unwrap();
        assert!(state.apply_search_result(
            request.generation,
            vec![session("b"), session("c")],
            1.0
        ));

        assert_eq!(state.selected, 0);
        assert_eq!(state.selected_session().unwrap().id, "b");
    }

    #[test]
    fn background_refresh_preserves_selected_session_identity() {
        let base = Local::now();
        let mut first = session("a");
        first.timestamp = base + ChronoDuration::seconds(2);
        let mut selected = session("b");
        selected.timestamp = base + ChronoDuration::seconds(1);
        let mut last = session("c");
        last.timestamp = base;
        let (mut state, index) =
            test_state_and_index(vec![first.clone(), selected.clone(), last.clone()], None);
        state.selected = 1;
        assert_eq!(state.selected_session().unwrap().id, "b");

        let mut newer = session("newer");
        newer.timestamp = base + ChronoDuration::seconds(3);
        index.rebuild(vec![newer, first, selected, last]).unwrap();

        super::state::handle_scan_message(
            &mut state,
            super::state::ScanMessage::Progress {
                elapsed: Duration::ZERO,
                new_or_modified: 1,
                deleted: 0,
                total: 4,
            },
        );

        assert_eq!(state.selected, 2);
        assert_eq!(state.selected_session().unwrap().id, "b");
    }

    #[test]
    fn actions_wait_for_pending_search_results_before_using_selection() {
        let mut state = test_state(vec![session("a")]);

        handle_key(&mut state, key(KeyCode::Char('z'), KeyModifiers::NONE)).unwrap();
        let stale = state.take_search_request().unwrap();
        let exit = handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();

        assert!(exit.is_none());
        assert_eq!(state.visible.len(), 1);
        assert!(state.modal.is_none());
        assert!(state.status.contains("searching"));
        assert!(state.apply_search_result(stale.generation, Vec::new(), 10.0));
        assert!(state.visible.is_empty());
    }

    #[test]
    fn actions_do_not_use_matching_results_before_redraw() {
        let mut state = test_state(vec![session("a"), session("b")]);
        state.selected = state
            .visible
            .iter()
            .position(|session| session.id == "a")
            .unwrap();
        assert_eq!(state.selected_session().unwrap().id, "a");

        handle_key(&mut state, key(KeyCode::Char('b'), KeyModifiers::NONE)).unwrap();
        let request = state.take_search_request().unwrap();
        let exit = handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();

        assert!(exit.is_none());
        assert!(state.modal.is_none());
        assert_eq!(state.selected_session().unwrap().id, "a");
        assert!(state.status.contains("searching"));

        assert!(state.apply_search_result(request.generation, vec![session("b")], 10.0));
        assert_eq!(state.selected_session().unwrap().id, "b");
    }

    #[test]
    fn yolo_modal_confirms_original_session_after_selection_changes() {
        let mut state = test_state(vec![session("a"), session("b")]);
        let original_id = state.selected_session().unwrap().id.clone();

        let exit = handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();
        assert!(exit.is_none());
        assert_eq!(state.modal.as_ref().unwrap().session.id, original_id);

        state.selected = if state.selected == 0 { 1 } else { 0 };
        let exit = handle_key(&mut state, key(KeyCode::Char('y'), KeyModifiers::NONE))
            .unwrap()
            .unwrap();

        match exit {
            super::TuiExit::Resume { command, directory } => {
                assert_eq!(
                    command.last().map(String::as_str),
                    Some(original_id.as_str())
                );
                assert_eq!(directory, "/tmp/fast-resume");
            }
            super::TuiExit::Quit => panic!("expected resume exit"),
        }
    }

    #[test]
    fn yolo_modal_arrows_select_buttons_directionally() {
        let mut state = test_state(vec![session("a")]);

        handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();
        assert!(!state.modal.as_ref().unwrap().selected);

        handle_key(&mut state, key(KeyCode::Left, KeyModifiers::NONE)).unwrap();
        assert!(!state.modal.as_ref().unwrap().selected);

        handle_key(&mut state, key(KeyCode::Right, KeyModifiers::NONE)).unwrap();
        assert!(state.modal.as_ref().unwrap().selected);

        handle_key(&mut state, key(KeyCode::Left, KeyModifiers::NONE)).unwrap();
        assert!(!state.modal.as_ref().unwrap().selected);

        handle_key(&mut state, key(KeyCode::Tab, KeyModifiers::NONE)).unwrap();
        assert!(state.modal.as_ref().unwrap().selected);
    }

    #[test]
    fn enter_resumes_crush_sessions() {
        let mut crush = session("crush-1");
        crush.agent = "crush".to_string();
        let mut state = test_state(vec![crush]);
        state.yolo = true;

        let exit = handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap()
            .unwrap();

        match exit {
            super::TuiExit::Resume { command, directory } => {
                assert_eq!(command, vec!["crush", "--yolo", "--session", "crush-1"]);
                assert_eq!(directory, "/tmp/fast-resume");
            }
            super::TuiExit::Quit => panic!("expected resume exit"),
        }
    }
}
