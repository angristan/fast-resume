use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use fs4::fs_std::FileExt;
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{AllQuery, TermQuery};
use tantivy::schema::{IndexRecordOption, TantivyDocument};
use tantivy::{DocAddress, Index, IndexReader, IndexWriter, Order, ReloadPolicy, Score, Term};

use crate::adapters::KnownSessions;
use crate::config::index_dir;
use crate::model::{Session, sort_and_dedupe_sessions};
use crate::query::{Filter, parse_query};

mod document;
mod queries;
mod schema;
mod stats;

pub use stats::IndexStats;

pub const INDEX_REFRESH_BATCH_SIZE: usize = 500;

/// How often a streaming refresh commits, so TUI results update while long
/// refreshes run without paying a commit (fsync plus a new segment) per batch.
const STREAMING_COMMIT_INTERVAL: Duration = Duration::from_secs(1);

struct IndexLock {
    _file: File,
}

impl IndexLock {
    fn acquire(index_path: &Path, purpose: &str) -> Result<Self> {
        Self::acquire_notify(index_path, purpose, || {})
    }

    /// Acquire the lock, calling `on_wait` once first if another process
    /// already holds it and this acquisition is going to block.
    fn acquire_notify(index_path: &Path, purpose: &str, on_wait: impl FnOnce()) -> Result<Self> {
        let lock_path = coordination_lock_path(index_path, purpose);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| {
                format!(
                    "failed to open index {purpose} lock {}",
                    lock_path.display()
                )
            })?;
        let acquired = file.try_lock_exclusive().with_context(|| {
            format!(
                "failed to acquire index {purpose} lock {}",
                lock_path.display()
            )
        })?;
        if !acquired {
            on_wait();
            file.lock_exclusive().with_context(|| {
                format!(
                    "failed to acquire index {purpose} lock {}",
                    lock_path.display()
                )
            })?;
        }
        Ok(Self { _file: file })
    }
}

fn coordination_lock_path(index_path: &Path, purpose: &str) -> PathBuf {
    let name = index_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("index");
    index_path.with_file_name(format!("{name}.{purpose}.lock"))
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub session: Session,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct RefreshSummary {
    pub sessions: usize,
    pub new_or_modified: usize,
    pub deleted: usize,
}

#[derive(Clone)]
pub struct SessionIndex {
    index: Index,
    reader: Arc<IndexReader>,
    path: PathBuf,
    fields: schema::IndexFields,
}

impl SessionIndex {
    pub fn open_default() -> Result<Self> {
        Self::open(index_dir())
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let _initialization_lock = IndexLock::acquire(&path, "init")?;

        if path.exists() && !schema::schema_version_matches(&path) {
            // The init lock does not exclude writers, so take the write lock
            // before wiping: a refresh in another process must not lose its
            // index directory under a live IndexWriter.
            let _write_lock = IndexLock::acquire(&path, "write")?;
            fs::remove_dir_all(&path)
                .with_context(|| format!("failed to clear stale index {}", path.display()))?;
        }

        let index_schema = schema::build_schema();
        let index = if path.exists() {
            Index::open_in_dir(&path)
                .with_context(|| format!("failed to open Tantivy index {}", path.display()))?
        } else {
            fs::create_dir_all(&path)
                .with_context(|| format!("failed to create {}", path.display()))?;
            let index = Index::create_in_dir(&path, index_schema)
                .with_context(|| format!("failed to create Tantivy index {}", path.display()))?;
            schema::write_schema_version(&path)?;
            index
        };

        let fields = schema::IndexFields::from_schema(&index.schema())?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        Ok(Self {
            index,
            reader: Arc::new(reader),
            path,
            fields,
        })
    }

    pub fn rebuild(&self, sessions: Vec<Session>) -> Result<RefreshSummary> {
        let _write_lock = self.acquire_write_lock()?;
        self.reader.reload()?;
        self.rebuild_unlocked(sessions)
    }

    fn rebuild_unlocked(&self, sessions: Vec<Session>) -> Result<RefreshSummary> {
        let mut writer: IndexWriter<TantivyDocument> =
            self.index.writer_with_num_threads(1, 128_000_000)?;
        writer.delete_all_documents()?;
        for session in &sessions {
            writer.add_document(document::session_document(self.fields, session))?;
        }
        writer.commit()?;
        self.reader.reload()?;
        Ok(RefreshSummary {
            sessions: sessions.len(),
            new_or_modified: sessions.len(),
            deleted: 0,
        })
    }

    pub fn refresh_incremental(&self) -> Result<RefreshSummary> {
        self.refresh_incremental_notify(|| {})
    }

    /// Refresh incrementally, calling `on_wait` once first if the refresh has
    /// to wait for another process's refresh to finish.
    pub fn refresh_incremental_notify(&self, on_wait: impl FnOnce()) -> Result<RefreshSummary> {
        let _write_lock = IndexLock::acquire_notify(&self.path, "write", on_wait)?;
        self.reader.reload()?;
        crate::refresh::refresh_incremental(self)
    }

    pub fn refresh_incremental_streaming<F>(
        &self,
        batch_size: usize,
        on_progress: F,
    ) -> Result<RefreshSummary>
    where
        F: FnMut(RefreshSummary),
    {
        let _write_lock = self.acquire_write_lock()?;
        self.reader.reload()?;
        crate::refresh::refresh_incremental_streaming(
            self,
            batch_size,
            Some(STREAMING_COMMIT_INTERVAL),
            on_progress,
        )
    }

    pub fn scan_all_sessions() -> Vec<Session> {
        crate::refresh::scan_all_sessions()
    }

    pub fn reload(&self) -> Result<()> {
        self.reader.reload()?;
        Ok(())
    }

    pub fn known_sessions(&self) -> Result<KnownSessions> {
        let searcher = self.searcher()?;
        let mut known = KnownSessions::new();
        for (_, address) in self.search_all_addresses(&searcher)? {
            let doc = searcher.doc::<TantivyDocument>(address)?;
            let Some(id) = document::text(&doc, self.fields.id) else {
                continue;
            };
            let Some(agent) = document::text(&doc, self.fields.agent) else {
                continue;
            };
            let mtime = document::number(&doc, self.fields.mtime).unwrap_or(0.0);
            known.insert((agent.to_string(), id.to_string()), mtime);
        }
        Ok(known)
    }

    pub fn all_sessions(&self) -> Result<Vec<Session>> {
        let searcher = self.searcher()?;
        let mut sessions = Vec::new();
        for (_, address) in self.search_all_addresses(&searcher)? {
            let doc = searcher.doc::<TantivyDocument>(address)?;
            if let Some(session) = document::doc_to_session(self.fields, &doc) {
                sessions.push(session);
            }
        }
        Ok(sort_and_dedupe_sessions(sessions))
    }

    pub fn total_len(&self) -> Result<usize> {
        let searcher = self.searcher()?;
        Ok(searcher.num_docs() as usize)
    }

    pub fn count_for_agent(&self, agent: Option<&str>) -> Result<usize> {
        let searcher = self.searcher()?;
        let count = match agent {
            Some(agent) => {
                let term = Term::from_field_text(self.fields.agent, agent);
                let query = TermQuery::new(term, IndexRecordOption::Basic);
                searcher.search(&query, &Count)?
            }
            None => searcher.search(&AllQuery, &Count)?,
        };
        Ok(count)
    }

    pub fn agents_with_sessions(&self) -> Result<Vec<String>> {
        let mut agents: Vec<_> = self
            .all_sessions()?
            .into_iter()
            .map(|session| session.agent)
            .collect();
        agents.sort();
        agents.dedup();
        Ok(agents)
    }

    pub fn search(
        &self,
        query: &str,
        agent_filter: Option<&str>,
        directory_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        self.search_with_offset(query, agent_filter, directory_filter, 0, limit)
    }

    pub fn search_with_offset(
        &self,
        query: &str,
        agent_filter: Option<&str>,
        directory_filter: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let searcher = self.searcher()?;
        let (query, has_text) = self.build_search_query(query, agent_filter, directory_filter)?;
        if !has_text {
            let collector = TopDocs::with_limit(limit)
                .and_offset(offset)
                .order_by_fast_field::<f64>("timestamp", Order::Desc);
            let hits: Vec<(Option<f64>, DocAddress)> = searcher.search(&query, &collector)?;
            self.hits_to_sessions(
                &searcher,
                hits.into_iter()
                    .map(|(score, addr)| (score.unwrap_or_default() as f32, addr)),
            )
        } else {
            let hits: Vec<(Score, DocAddress)> = searcher.search(
                &query,
                &TopDocs::with_limit(limit)
                    .and_offset(offset)
                    .order_by_score(),
            )?;
            self.hits_to_sessions(&searcher, hits.into_iter())
        }
    }

    pub fn search_count(
        &self,
        query: &str,
        agent_filter: Option<&str>,
        directory_filter: Option<&str>,
    ) -> Result<usize> {
        let searcher = self.searcher()?;
        let (query, _) = self.build_search_query(query, agent_filter, directory_filter)?;
        Ok(searcher.search(&query, &Count)?)
    }

    pub fn stats(&self) -> Result<IndexStats> {
        Ok(stats::build(self.all_sessions()?, &self.path))
    }

    /// Test-only convenience: production writes go through `updater` so one
    /// `IndexWriter` serves a whole refresh.
    #[cfg(test)]
    pub(crate) fn update_sessions(&self, sessions: &[Session]) -> Result<()> {
        let mut updater = self.updater(None);
        updater.update_sessions(sessions)?;
        updater.finish()
    }

    /// Start a batched update pass that reuses one `IndexWriter`. With a
    /// commit interval, dirty changes commit at most that often; without one,
    /// everything commits once in `finish`.
    pub(crate) fn updater(&self, commit_interval: Option<Duration>) -> IndexUpdater<'_> {
        IndexUpdater {
            index: self,
            writer: None,
            dirty: false,
            commit_interval,
            last_commit: Instant::now(),
        }
    }

    fn acquire_write_lock(&self) -> Result<IndexLock> {
        IndexLock::acquire(&self.path, "write")
    }

    fn searcher(&self) -> Result<tantivy::Searcher> {
        Ok(self.reader.searcher())
    }

    fn build_search_query(
        &self,
        query: &str,
        agent_filter: Option<&str>,
        directory_filter: Option<&str>,
    ) -> Result<(Box<dyn tantivy::query::Query>, bool)> {
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

        let search_text = parsed.text.trim().to_string();
        let has_text = !search_text.is_empty();
        let query = queries::build(
            &self.index,
            self.fields,
            &search_text,
            effective_agent,
            effective_dir,
            parsed.date,
        )?;
        Ok((query, has_text))
    }

    fn search_all_addresses(
        &self,
        searcher: &tantivy::Searcher,
    ) -> Result<Vec<(Option<f64>, DocAddress)>> {
        let total = searcher.num_docs() as usize;
        if total == 0 {
            return Ok(Vec::new());
        }
        let collector =
            TopDocs::with_limit(total).order_by_fast_field::<f64>("timestamp", Order::Desc);
        Ok(searcher.search(&AllQuery, &collector)?)
    }

    fn hits_to_sessions(
        &self,
        searcher: &tantivy::Searcher,
        hits: impl Iterator<Item = (f32, DocAddress)>,
    ) -> Result<Vec<SearchHit>> {
        let mut sessions = Vec::new();
        for (score, address) in hits {
            let doc = searcher.doc::<TantivyDocument>(address)?;
            if let Some(session) = document::doc_to_session(self.fields, &doc) {
                sessions.push(SearchHit { session, score });
            }
        }
        Ok(sessions)
    }
}

pub(crate) struct IndexUpdater<'a> {
    index: &'a SessionIndex,
    writer: Option<IndexWriter<TantivyDocument>>,
    dirty: bool,
    commit_interval: Option<Duration>,
    last_commit: Instant,
}

impl IndexUpdater<'_> {
    pub(crate) fn update_sessions(&mut self, sessions: &[Session]) -> Result<()> {
        if sessions.is_empty() {
            return Ok(());
        }
        let fields = self.index.fields;
        let writer = self.writer()?;
        for session in sessions {
            writer.delete_term(Term::from_field_text(
                fields.session_key,
                &document::session_key(&session.agent, &session.id),
            ));
        }
        for session in sessions {
            writer.add_document(document::session_document(fields, session))?;
        }
        self.dirty = true;
        self.maybe_commit()
    }

    pub(crate) fn delete_sessions(&mut self, agent: &str, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let fields = self.index.fields;
        let writer = self.writer()?;
        for id in ids {
            writer.delete_term(Term::from_field_text(
                fields.session_key,
                &document::session_key(agent, id),
            ));
        }
        self.dirty = true;
        self.maybe_commit()
    }

    /// Commit outstanding changes and reload the reader. Dropping the updater
    /// without calling this discards uncommitted changes.
    pub(crate) fn finish(mut self) -> Result<()> {
        if self.dirty {
            self.commit()?;
        }
        Ok(())
    }

    /// The writer is created on first use so refreshes that find no changes
    /// never pay for its indexing arena.
    fn writer(&mut self) -> Result<&mut IndexWriter<TantivyDocument>> {
        if self.writer.is_none() {
            self.writer = Some(self.index.index.writer_with_num_threads(1, 128_000_000)?);
        }
        self.writer.as_mut().context("index writer was not created")
    }

    fn maybe_commit(&mut self) -> Result<()> {
        let Some(interval) = self.commit_interval else {
            return Ok(());
        };
        if self.dirty && self.last_commit.elapsed() >= interval {
            self.commit()?;
        }
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        if let Some(writer) = self.writer.as_mut() {
            writer.commit()?;
            self.index.reader.reload()?;
        }
        self.dirty = false;
        self.last_commit = Instant::now();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Local, Timelike};
    use tempfile::tempdir;

    use super::*;

    fn session(id: &str, agent: &str, title: &str, dir: &str, content: &str) -> Session {
        let mut session = Session::new(id, agent, title, dir, Local::now(), content, 2);
        session.mtime = 1.0;
        session
    }

    #[test]
    fn searches_and_filters_sessions_from_tantivy() {
        let temp = tempdir().unwrap();
        let index = SessionIndex::open(temp.path().join("index")).unwrap();
        index
            .update_sessions(&[
                session("a", "claude", "Auth bug", "/work/api", "token refresh"),
                session("b", "codex", "Other", "/work/frontend", "button"),
            ])
            .unwrap();

        let results = index
            .search("agent:claude dir:api token", None, None, 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session.id, "a");
    }

    #[test]
    fn known_sessions_reads_mtime_from_tantivy() {
        let temp = tempdir().unwrap();
        let index = SessionIndex::open(temp.path().join("index")).unwrap();
        index
            .update_sessions(&[session("a", "claude", "Auth bug", "/work/api", "token")])
            .unwrap();

        let known = index.known_sessions().unwrap();
        assert_eq!(
            known.get(&("claude".to_string(), "a".to_string())),
            Some(&1.0)
        );
    }

    #[test]
    fn updates_only_matching_agent_session_id() {
        let temp = tempdir().unwrap();
        let index = SessionIndex::open(temp.path().join("index")).unwrap();
        index
            .update_sessions(&[
                session(
                    "same",
                    "claude",
                    "Claude title",
                    "/work/a",
                    "claude content",
                ),
                session("same", "codex", "Codex title", "/work/b", "codex content"),
            ])
            .unwrap();
        let mut updated = session("same", "codex", "Updated Codex", "/work/b", "codex changed");
        updated.mtime = 2.0;

        index.update_sessions(&[updated]).unwrap();

        let sessions = index.all_sessions().unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(
            sessions
                .iter()
                .any(|s| s.agent == "claude" && s.title == "Claude title")
        );
        assert!(
            sessions
                .iter()
                .any(|s| s.agent == "codex" && s.title == "Updated Codex")
        );
    }

    #[test]
    fn fuzzy_search_handles_one_character_typo() {
        let temp = tempdir().unwrap();
        let index = SessionIndex::open(temp.path().join("index")).unwrap();
        index
            .update_sessions(&[session(
                "a",
                "claude",
                "Authentication bug",
                "/work/api",
                "refresh token failure",
            )])
            .unwrap();

        let results = index.search("authentcation", None, None, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session.id, "a");
    }

    #[test]
    fn fuzzy_search_handles_one_character_content_typo() {
        let temp = tempdir().unwrap();
        let index = SessionIndex::open(temp.path().join("index")).unwrap();
        index
            .update_sessions(&[session(
                "a",
                "claude",
                "Deployment notes",
                "/work/api",
                "refresh token failure",
            )])
            .unwrap();

        let results = index.search("tokem", None, None, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session.id, "a");
    }

    #[test]
    fn updater_without_interval_commits_only_on_finish() {
        let temp = tempdir().unwrap();
        let index = SessionIndex::open(temp.path().join("index")).unwrap();
        index
            .update_sessions(&[session("a", "claude", "Old", "/work/a", "old content")])
            .unwrap();

        let mut updater = index.updater(None);
        updater
            .update_sessions(&[session("b", "codex", "New", "/work/b", "new content")])
            .unwrap();
        updater
            .delete_sessions("claude", &["a".to_string()])
            .unwrap();

        let ids: Vec<_> = index
            .all_sessions()
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, vec!["a"], "changes must stay invisible before finish");

        updater.finish().unwrap();

        let ids: Vec<_> = index
            .all_sessions()
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, vec!["b"]);
    }

    #[test]
    fn updater_with_interval_commits_mid_stream() {
        let temp = tempdir().unwrap();
        let index = SessionIndex::open(temp.path().join("index")).unwrap();

        let mut updater = index.updater(Some(Duration::ZERO));
        updater
            .update_sessions(&[session("a", "claude", "Streamed", "/work/a", "content")])
            .unwrap();

        assert_eq!(
            index.total_len().unwrap(),
            1,
            "an elapsed interval must commit without finish"
        );
        updater.finish().unwrap();
    }

    #[test]
    fn stats_include_content_bytes_and_activity_buckets() {
        let temp = tempdir().unwrap();
        let index = SessionIndex::open(temp.path().join("index")).unwrap();
        let session = session(
            "a",
            "codex",
            "Stats test",
            "/work/api",
            "content bytes are counted",
        );
        let weekday = session.timestamp.weekday().to_string();
        let hour = session.timestamp.hour();
        let content_len = session.content.len() as u64;
        index.update_sessions(&[session]).unwrap();

        let stats = index.stats().unwrap();

        assert_eq!(
            stats.content_bytes_by_agent.get("codex"),
            Some(&content_len)
        );
        assert_eq!(stats.sessions_by_weekday.get(&weekday), Some(&1));
        assert_eq!(stats.sessions_by_hour.get(&hour), Some(&1));
    }
}
