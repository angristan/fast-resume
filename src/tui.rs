use std::io::{self, Stdout};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::index::{INDEX_REFRESH_BATCH_SIZE, SessionIndex};
use crate::model::Session;
use crate::search::SearchEngine;

mod images;
mod input;
mod render;
mod state;
mod text;

use images::AgentImages;
use input::handle_key;
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
    let (search_tx, search_rx) = mpsc::channel::<SearchResult>();
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

fn start_search_if_requested(state: &mut AppState, tx: &Sender<SearchResult>) {
    let Some(request) = state.take_search_request() else {
        return;
    };
    spawn_search(state.engine.clone(), request, tx.clone());
}

fn spawn_search(engine: SearchEngine, request: SearchRequest, tx: Sender<SearchResult>) {
    thread::spawn(move || {
        let start = Instant::now();
        let visible = engine.search(&request.query, request.agent_filter.as_deref(), None, 100);
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        let _ = tx.send(SearchResult {
            generation: request.generation,
            visible,
            elapsed_ms,
        });
    });
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

#[cfg(test)]
mod tests {
    use chrono::Local;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tempfile::tempdir;

    use crate::index::SessionIndex;
    use crate::model::Session;
    use crate::search::SearchEngine;

    use super::input::handle_key;
    use super::state::AppState;

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
    fn typing_requests_search_without_blocking_visible_results() {
        let mut state = test_state(vec![session("a")]);

        handle_key(&mut state, key(KeyCode::Char('z'), KeyModifiers::NONE)).unwrap();

        assert_eq!(state.query, "z");
        assert_eq!(state.visible.len(), 1);
        let request = state.take_search_request().unwrap();
        assert_eq!(request.query, "z");
        assert_eq!(request.agent_filter, None);
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
    fn actions_force_current_search_before_using_selection() {
        let mut state = test_state(vec![session("a")]);

        handle_key(&mut state, key(KeyCode::Char('z'), KeyModifiers::NONE)).unwrap();
        let stale = state.take_search_request().unwrap();
        let exit = handle_key(&mut state, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();

        assert!(exit.is_none());
        assert!(state.visible.is_empty());
        assert!(state.modal.is_none());
        assert!(!state.apply_search_result(stale.generation, vec![session("stale")], 10.0));
        assert!(state.visible.is_empty());
    }
}
