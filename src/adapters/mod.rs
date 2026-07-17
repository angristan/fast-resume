use std::collections::{HashMap, HashSet};

use crate::model::{RawAdapterStats, Session};

mod antigravity;
mod claude;
mod codex;
mod copilot_cli;
mod copilot_vscode;
mod crush;
mod cursor;
mod gemini;
mod grok;
mod opencode;
mod pi;
mod shared;
mod vibe;

pub use antigravity::AntigravityAdapter;
pub use claude::ClaudeAdapter;
pub use codex::CodexAdapter;
pub use copilot_cli::CopilotCliAdapter;
pub use copilot_vscode::CopilotVsCodeAdapter;
pub use crush::CrushAdapter;
pub use cursor::CursorAdapter;
pub use gemini::GeminiAdapter;
pub use grok::GrokAdapter;
pub use opencode::OpenCodeAdapter;
pub use pi::PiAdapter;
pub use vibe::VibeAdapter;

pub const MTIME_TOLERANCE: f64 = 0.001;

pub type KnownSessions = HashMap<(String, String), f64>;
pub type SessionCallback<'a> = dyn FnMut(Session) + Send + 'a;

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
                shared::session_needs_update(known, &session.agent, &session.id, session.mtime)
            })
            .collect();
        IncrementalScan {
            agent: self.name(),
            new_or_modified,
            deleted_ids: shared::deleted_ids_for_agent(known, self.name(), &current_ids),
        }
    }
    fn find_sessions_incremental_streaming(
        &self,
        known: &KnownSessions,
        on_session: &mut SessionCallback<'_>,
    ) -> IncrementalScan {
        let scan = self.find_sessions_incremental(known);
        for session in &scan.new_or_modified {
            on_session(session.clone());
        }
        scan
    }
    fn resume_command(&self, session: &Session, yolo: bool) -> Vec<String>;
    fn raw_stats(&self) -> RawAdapterStats;
}

pub fn all_adapters() -> Vec<Box<dyn Adapter>> {
    vec![
        Box::new(AntigravityAdapter::default()),
        Box::new(ClaudeAdapter::default()),
        Box::new(CodexAdapter::default()),
        Box::new(CopilotCliAdapter::default()),
        Box::new(CopilotVsCodeAdapter::default()),
        Box::new(CrushAdapter::default()),
        Box::new(CursorAdapter::default()),
        Box::new(GeminiAdapter::default()),
        Box::new(GrokAdapter::default()),
        Box::new(OpenCodeAdapter::default()),
        Box::new(PiAdapter::default()),
        Box::new(VibeAdapter::default()),
    ]
}

pub fn adapter_for(agent: &str) -> Option<Box<dyn Adapter>> {
    all_adapters()
        .into_iter()
        .find(|adapter| adapter.name() == agent)
}
