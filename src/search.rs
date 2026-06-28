use anyhow::Result;

use crate::index::SessionIndex;
use crate::model::Session;

#[derive(Clone)]
pub struct SearchEngine {
    index: SessionIndex,
}

impl SearchEngine {
    pub fn open_default() -> Result<Self> {
        Ok(Self {
            index: SessionIndex::open_default()?,
        })
    }

    #[allow(dead_code)]
    pub fn from_index(index: SessionIndex) -> Self {
        Self { index }
    }

    pub fn reload(&mut self) -> Result<()> {
        self.index.reload()
    }

    pub fn all_sessions(&self) -> Vec<Session> {
        self.index.all_sessions().unwrap_or_default()
    }

    pub fn total_len(&self) -> usize {
        self.index.total_len().unwrap_or(0)
    }

    pub fn count_matches(
        &self,
        query: &str,
        agent_filter: Option<&str>,
        directory_filter: Option<&str>,
    ) -> usize {
        self.index
            .search_count(query, agent_filter, directory_filter)
            .unwrap_or(0)
    }

    pub fn count_for_agent(&self, agent: Option<&str>) -> usize {
        self.index.count_for_agent(agent).unwrap_or(0)
    }

    pub fn agents_with_sessions(&self) -> Vec<String> {
        self.index.agents_with_sessions().unwrap_or_default()
    }

    pub fn search(
        &self,
        query: &str,
        agent_filter: Option<&str>,
        directory_filter: Option<&str>,
        limit: usize,
    ) -> Vec<Session> {
        self.index
            .search(query, agent_filter, directory_filter, limit)
            .map(|hits| hits.into_iter().map(|hit| hit.session).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration as ChronoDuration, Local};
    use tempfile::tempdir;

    use super::*;

    fn session(id: &str, agent: &str, title: &str, dir: &str, content: &str) -> Session {
        Session::new(id, agent, title, dir, Local::now(), content, 2)
    }

    fn session_at(
        id: &str,
        agent: &str,
        title: &str,
        dir: &str,
        content: &str,
        age: ChronoDuration,
    ) -> Session {
        Session::new(id, agent, title, dir, Local::now() - age, content, 2)
    }

    fn result_ids(engine: &SearchEngine, query: &str) -> Vec<String> {
        let mut ids: Vec<_> = engine
            .search(query, None, None, 10)
            .into_iter()
            .map(|session| session.id)
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn empty_query_returns_newest_sessions() {
        let temp = tempdir().unwrap();
        let index = SessionIndex::open(temp.path().join("index")).unwrap();
        index
            .rebuild(vec![
                session("a", "claude", "First", "/tmp/a", "one"),
                session("b", "codex", "Second", "/tmp/b", "two"),
            ])
            .unwrap();
        let engine = SearchEngine::from_index(index);
        assert_eq!(engine.search("", None, None, 10).len(), 2);
    }

    #[test]
    fn filters_by_agent_and_directory_keyword() {
        let temp = tempdir().unwrap();
        let index = SessionIndex::open(temp.path().join("index")).unwrap();
        index
            .rebuild(vec![
                session("a", "claude", "Auth bug", "/work/api", "token"),
                session("b", "codex", "Other", "/work/frontend", "button"),
            ])
            .unwrap();
        let engine = SearchEngine::from_index(index);
        let results = engine.search("agent:claude dir:api token", None, None, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "a");
    }

    #[test]
    fn normalizes_typed_agent_filter_case() {
        let temp = tempdir().unwrap();
        let index = SessionIndex::open(temp.path().join("index")).unwrap();
        index
            .rebuild(vec![
                session("a", "claude", "Auth bug", "/work/api", "token"),
                session("b", "codex", "Other", "/work/frontend", "button"),
            ])
            .unwrap();
        let engine = SearchEngine::from_index(index);

        let results = engine.search("agent:CoDeX button", None, None, 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "b");
    }

    #[test]
    fn malformed_text_query_still_returns_lenient_matches() {
        let temp = tempdir().unwrap();
        let index = SessionIndex::open(temp.path().join("index")).unwrap();
        index
            .rebuild(vec![session(
                "a",
                "codex",
                "Fast resume search",
                "/work/fast-resume",
                "fast resume should keep matching while a quote is half typed",
            )])
            .unwrap();
        let engine = SearchEngine::from_index(index);

        let results = engine.search("\"fast", None, None, 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "a");
    }

    #[test]
    fn executes_keyword_filter_combinations() {
        let temp = tempdir().unwrap();
        let index = SessionIndex::open(temp.path().join("index")).unwrap();
        index
            .rebuild(vec![
                session_at(
                    "claude-recent",
                    "claude",
                    "Auth UI",
                    "/work/web-app",
                    "token",
                    ChronoDuration::hours(1),
                ),
                session_at(
                    "codex-recent",
                    "codex",
                    "API UI",
                    "/work/web-api",
                    "button",
                    ChronoDuration::hours(2),
                ),
                session_at(
                    "vibe-old",
                    "vibe",
                    "Old UI",
                    "/work/web-app",
                    "archive",
                    ChronoDuration::days(10),
                ),
                session_at(
                    "opencode-recent",
                    "opencode",
                    "CLI",
                    "/work/cli",
                    "terminal",
                    ChronoDuration::minutes(30),
                ),
            ])
            .unwrap();
        let engine = SearchEngine::from_index(index);

        assert_eq!(
            result_ids(&engine, "agent:claude,codex"),
            vec!["claude-recent", "codex-recent"]
        );
        assert_eq!(
            result_ids(&engine, "-agent:claude"),
            vec!["codex-recent", "opencode-recent", "vibe-old"]
        );
        assert_eq!(
            result_ids(&engine, "dir:!web-app"),
            vec!["codex-recent", "opencode-recent"]
        );
        assert_eq!(
            result_ids(&engine, "date:<3h"),
            vec!["claude-recent", "codex-recent", "opencode-recent"]
        );
        assert_eq!(result_ids(&engine, "date:>3d"), vec!["vibe-old"]);
        assert_eq!(result_ids(&engine, "date:!today"), vec!["vibe-old"]);
        assert_eq!(
            result_ids(&engine, "agent:claude,codex dir:web date:<5h"),
            vec!["claude-recent", "codex-recent"]
        );
    }
}
