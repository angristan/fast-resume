use anyhow::Result;
use arboard::Clipboard;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::adapters::adapter_for;

use super::TuiExit;
use super::state::{AppState, PendingAction, YoloModal};
use super::text::{shell_join, shell_quote};

pub(super) fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<Option<TuiExit>> {
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
        (KeyCode::Tab, _) => {
            if !state.accept_suggestion() {
                state.cycle_agent(false);
            }
        }
        (KeyCode::BackTab, _) => state.cycle_agent(true),
        (KeyCode::Backspace, _) => state.backspace(),
        (KeyCode::Delete, _) => state.delete(),
        (KeyCode::Left, _) => state.cursor = state.cursor.saturating_sub(1),
        (KeyCode::Right, _) => state.cursor = (state.cursor + 1).min(state.query.chars().count()),
        (KeyCode::Home, _) => state.cursor = 0,
        (KeyCode::End, _) => state.cursor = state.query.chars().count(),
        (KeyCode::Char('+'), modifiers) if modifiers.contains(KeyModifiers::ALT) => {
            state.scroll_preview(-3);
        }
        (KeyCode::Char('-'), modifiers) if modifiers.contains(KeyModifiers::ALT) => {
            state.scroll_preview(3);
        }
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
    if state.search_pending() {
        state.status = "searching; press again when results update".to_string();
        return Ok(None);
    }
    let Some(session) = state.selected_session().cloned() else {
        return Ok(None);
    };

    let supports_yolo = adapter_for(&session.agent)
        .as_ref()
        .is_some_and(|adapter| adapter.supports_yolo());
    if state.yolo || session.yolo || !supports_yolo {
        return finish_action(state, action, state.yolo || session.yolo, session);
    }

    state.modal = Some(YoloModal {
        action,
        session,
        selected: false,
    });
    Ok(None)
}

fn handle_modal_key(state: &mut AppState, key: KeyEvent) -> Result<Option<TuiExit>> {
    let Some(modal) = state.modal.as_mut() else {
        return Ok(None);
    };

    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => state.modal = None,
        (KeyCode::Left, _) => modal.selected = false,
        (KeyCode::Right, _) => modal.selected = true,
        (KeyCode::Tab | KeyCode::BackTab, _) => {
            modal.selected = !modal.selected;
        }
        (KeyCode::Char('y') | KeyCode::Char('Y'), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            let action = modal.action;
            let session = modal.session.clone();
            state.modal = None;
            return finish_action(state, action, true, session);
        }
        (KeyCode::Char('n') | KeyCode::Char('N'), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            let action = modal.action;
            let session = modal.session.clone();
            state.modal = None;
            return finish_action(state, action, false, session);
        }
        (KeyCode::Enter, _) => {
            let yolo = modal.selected;
            let action = modal.action;
            let session = modal.session.clone();
            state.modal = None;
            return finish_action(state, action, yolo, session);
        }
        _ => {}
    }

    Ok(None)
}

fn finish_action(
    state: &mut AppState,
    action: PendingAction,
    yolo: bool,
    session: crate::model::Session,
) -> Result<Option<TuiExit>> {
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
