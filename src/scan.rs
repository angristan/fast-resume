use std::collections::HashMap;

use rayon::prelude::*;

use crate::adapters::{IncrementalScan, KnownSessions, all_adapters};
use crate::model::{Session, sort_and_dedupe_sessions};

#[derive(Debug, Clone)]
pub struct IncrementalRefresh {
    pub sessions: Vec<Session>,
    pub new_or_modified: usize,
    pub deleted: usize,
}

pub fn scan_all_sessions() -> Vec<Session> {
    let sessions: Vec<_> = all_adapters()
        .into_par_iter()
        .flat_map(|adapter| adapter.find_sessions())
        .collect();
    sort_and_dedupe_sessions(sessions)
}

pub fn refresh_sessions_incremental(cached: Vec<Session>) -> IncrementalRefresh {
    let known = known_sessions(&cached);
    let scans: Vec<_> = all_adapters()
        .into_par_iter()
        .map(|adapter| adapter.find_sessions_incremental(&known))
        .collect();
    merge_incremental_scans(cached, scans)
}

pub fn known_sessions(sessions: &[Session]) -> KnownSessions {
    sessions
        .iter()
        .map(|session| ((session.agent.clone(), session.id.clone()), session.mtime))
        .collect()
}

fn merge_incremental_scans(
    cached: Vec<Session>,
    scans: Vec<IncrementalScan>,
) -> IncrementalRefresh {
    let mut by_key: HashMap<(String, String), Session> = cached
        .into_iter()
        .map(|session| ((session.agent.clone(), session.id.clone()), session))
        .collect();

    let mut new_or_modified = 0usize;
    let mut deleted = 0usize;
    for scan in scans {
        for id in scan.deleted_ids {
            if by_key.remove(&(scan.agent.to_string(), id)).is_some() {
                deleted += 1;
            }
        }
        for session in scan.new_or_modified {
            by_key.insert((session.agent.clone(), session.id.clone()), session);
            new_or_modified += 1;
        }
    }

    IncrementalRefresh {
        sessions: sort_and_dedupe_sessions(by_key.into_values().collect()),
        new_or_modified,
        deleted,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Local;

    use super::*;

    fn session(agent: &str, id: &str, mtime: f64) -> Session {
        let mut session = Session::new(id, agent, id, "/tmp", Local::now(), "content", 1);
        session.mtime = mtime;
        session
    }

    #[test]
    fn merge_incremental_scans_applies_updates_and_deletions() {
        let cached = vec![session("codex", "old", 1.0), session("claude", "gone", 1.0)];
        let scans = vec![
            IncrementalScan {
                agent: "codex",
                new_or_modified: vec![session("codex", "old", 2.0), session("codex", "new", 1.0)],
                deleted_ids: Vec::new(),
            },
            IncrementalScan {
                agent: "claude",
                new_or_modified: Vec::new(),
                deleted_ids: vec!["gone".to_string()],
            },
        ];

        let refreshed = merge_incremental_scans(cached, scans);
        assert_eq!(refreshed.new_or_modified, 2);
        assert_eq!(refreshed.deleted, 1);
        assert_eq!(refreshed.sessions.len(), 2);
        assert!(refreshed.sessions.iter().any(|session| session.id == "new"));
        assert!(
            refreshed
                .sessions
                .iter()
                .any(|session| session.id == "old" && session.mtime == 2.0)
        );
    }
}
