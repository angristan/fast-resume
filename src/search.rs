use chrono::Duration;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

use crate::model::{Session, sort_and_dedupe_sessions};
use crate::query::{DateOp, Filter, parse_query};

#[derive(Debug, Clone)]
struct PreparedSession {
    session: Session,
    title_lc: String,
    dir_lc: String,
    content_lc: String,
}

#[derive(Debug, Clone)]
pub struct SearchEngine {
    sessions: Vec<PreparedSession>,
}

impl SearchEngine {
    pub fn new(mut sessions: Vec<Session>) -> Self {
        sessions = sort_and_dedupe_sessions(sessions);
        Self {
            sessions: sessions.into_iter().map(PreparedSession::new).collect(),
        }
    }

    pub fn update(&mut self, sessions: Vec<Session>) {
        *self = Self::new(sessions);
    }

    pub fn all_sessions(&self) -> Vec<Session> {
        self.sessions
            .iter()
            .map(|session| session.session.clone())
            .collect()
    }

    pub fn total_len(&self) -> usize {
        self.sessions.len()
    }

    pub fn count_for_agent(&self, agent: Option<&str>) -> usize {
        match agent {
            Some(agent) => self
                .sessions
                .iter()
                .filter(|s| s.session.agent == agent)
                .count(),
            None => self.sessions.len(),
        }
    }

    pub fn agents_with_sessions(&self) -> Vec<String> {
        let mut agents: Vec<_> = self
            .sessions
            .iter()
            .map(|s| s.session.agent.clone())
            .collect();
        agents.sort();
        agents.dedup();
        agents
    }

    pub fn search(
        &self,
        query: &str,
        agent_filter: Option<&str>,
        directory_filter: Option<&str>,
        limit: usize,
    ) -> Vec<Session> {
        let parsed = parse_query(query);
        let effective_agent = agent_filter
            .map(|agent| Filter {
                include: vec![agent.to_string()],
                exclude: Vec::new(),
            })
            .or(parsed.agent);
        let effective_dir = directory_filter
            .map(|dir| Filter {
                include: vec![dir.to_string()],
                exclude: Vec::new(),
            })
            .or(parsed.directory);

        let terms: Vec<String> = parsed
            .text
            .split_whitespace()
            .map(|term| term.to_lowercase())
            .filter(|term| !term.is_empty())
            .collect();

        let matcher = SkimMatcherV2::default().ignore_case();
        let mut scored = Vec::new();

        for prepared in &self.sessions {
            if let Some(filter) = &effective_agent {
                if !filter.matches(&prepared.session.agent, false) {
                    continue;
                }
            }
            if let Some(filter) = &effective_dir {
                if !filter.matches(&prepared.session.directory, true) {
                    continue;
                }
            }
            if let Some(date) = &parsed.date {
                let matched = match date.op {
                    DateOp::LessThan => prepared.session.timestamp >= date.cutoff,
                    DateOp::GreaterThan => prepared.session.timestamp < date.cutoff,
                    DateOp::Exact if date.value.eq_ignore_ascii_case("today") => {
                        prepared.session.timestamp >= date.cutoff
                    }
                    DateOp::Exact if date.value.eq_ignore_ascii_case("yesterday") => {
                        prepared.session.timestamp >= date.cutoff
                            && prepared.session.timestamp < date.cutoff + Duration::days(1)
                    }
                    DateOp::Exact => true,
                };
                if matched == date.negated {
                    continue;
                }
            }

            let Some(score) = prepared.score(&terms, &matcher) else {
                continue;
            };
            scored.push((score, prepared.session.clone()));
        }

        scored.sort_by(|(score_a, a), (score_b, b)| {
            score_b
                .cmp(score_a)
                .then_with(|| b.timestamp.cmp(&a.timestamp))
        });
        scored
            .into_iter()
            .take(limit)
            .map(|(_, session)| session)
            .collect()
    }
}

impl PreparedSession {
    fn new(session: Session) -> Self {
        Self {
            title_lc: session.title.to_lowercase(),
            dir_lc: session.directory.to_lowercase(),
            content_lc: session.content.to_lowercase(),
            session,
        }
    }

    fn score(&self, terms: &[String], matcher: &SkimMatcherV2) -> Option<i64> {
        if terms.is_empty() {
            return Some(self.session.timestamp.timestamp());
        }

        let mut total = 0i64;
        for term in terms {
            let mut term_score = 0i64;
            if self.title_lc.contains(term) {
                term_score += 12_000;
            }
            if self.dir_lc.contains(term) {
                term_score += 3_000;
            }
            if self.content_lc.contains(term) {
                term_score += 1_500;
            }
            if term_score == 0 {
                if let Some(score) = matcher.fuzzy_match(&self.title_lc, term) {
                    term_score += 500 + score;
                } else {
                    return None;
                }
            }
            total += term_score;
        }

        // Nudge recent sessions up without letting recency dominate a direct title hit.
        total += self.session.timestamp.timestamp() / 86_400;
        Some(total)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Local;

    use super::*;

    fn session(id: &str, agent: &str, title: &str, dir: &str, content: &str) -> Session {
        Session::new(id, agent, title, dir, Local::now(), content, 2)
    }

    #[test]
    fn empty_query_returns_newest_sessions() {
        let engine = SearchEngine::new(vec![
            session("a", "claude", "First", "/tmp/a", "one"),
            session("b", "codex", "Second", "/tmp/b", "two"),
        ]);
        assert_eq!(engine.search("", None, None, 10).len(), 2);
    }

    #[test]
    fn filters_by_agent_and_directory_keyword() {
        let engine = SearchEngine::new(vec![
            session("a", "claude", "Auth bug", "/work/api", "token"),
            session("b", "codex", "Other", "/work/frontend", "button"),
        ]);
        let results = engine.search("agent:claude dir:api token", None, None, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "a");
    }
}
