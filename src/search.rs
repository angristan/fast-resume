use anyhow::Result;

use crate::index::SessionIndex;
use crate::model::Session;

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
    use chrono::Local;
    use tempfile::tempdir;

    use super::*;

    fn session(id: &str, agent: &str, title: &str, dir: &str, content: &str) -> Session {
        Session::new(id, agent, title, dir, Local::now(), content, 2)
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
}
