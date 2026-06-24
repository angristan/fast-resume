use std::io::{self, Stdout};
use std::sync::mpsc::{self, Receiver};
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
use crate::search::SearchEngine;

mod images;
mod input;
mod render;
mod state;
mod text;

use images::AgentImages;
use input::handle_key;
use render::draw;
use state::{AppState, ScanMessage, handle_scan_message};

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
