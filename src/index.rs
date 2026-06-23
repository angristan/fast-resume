use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use rayon::prelude::*;
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{
    AllQuery, BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, Query, QueryParser, RangeQuery,
    RegexQuery, TermQuery, TermSetQuery,
};
use tantivy::schema::{
    Field, IndexRecordOption, NumericOptions, STORED, Schema, TEXT, TantivyDocument,
    TextFieldIndexing, TextOptions, Value,
};
use tantivy::{DocAddress, Index, IndexWriter, Order, Score, Term, doc};

use crate::adapters::{KnownSessions, all_adapters};
use crate::config::{INDEX_SCHEMA_VERSION, index_dir};
use crate::model::{Session, sort_and_dedupe_sessions};
use crate::query::{DateOp, Filter, parse_query};

const VERSION_FILE: &str = ".schema_version";
pub const INDEX_REFRESH_BATCH_SIZE: usize = 500;

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

enum AdapterEvent {
    Session(Session),
    Finished {
        agent: &'static str,
        deleted_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct IndexStats {
    pub total_sessions: usize,
    pub total_messages: usize,
    pub sessions_by_agent: BTreeMap<String, usize>,
    pub messages_by_agent: BTreeMap<String, usize>,
    pub top_directories: Vec<(String, usize, usize)>,
    pub oldest: Option<DateTime<Local>>,
    pub newest: Option<DateTime<Local>>,
    pub sessions_by_weekday: BTreeMap<String, usize>,
    pub sessions_by_hour: BTreeMap<u32, usize>,
    pub index_size_bytes: u64,
    pub index_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SessionIndex {
    index: Index,
    path: PathBuf,
    fields: IndexFields,
}

#[derive(Debug, Clone, Copy)]
struct IndexFields {
    id: Field,
    session_key: Field,
    title: Field,
    directory: Field,
    agent: Field,
    content: Field,
    timestamp: Field,
    message_count: Field,
    mtime: Field,
    yolo: Field,
}

impl SessionIndex {
    pub fn open_default() -> Result<Self> {
        Self::open(index_dir())
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        if path.exists() && !schema_version_matches(&path) {
            fs::remove_dir_all(&path)
                .with_context(|| format!("failed to clear stale index {}", path.display()))?;
        }

        let schema = build_schema();
        let index = if path.exists() {
            Index::open_in_dir(&path)
                .with_context(|| format!("failed to open Tantivy index {}", path.display()))?
        } else {
            fs::create_dir_all(&path)
                .with_context(|| format!("failed to create {}", path.display()))?;
            let index = Index::create_in_dir(&path, schema.clone())
                .with_context(|| format!("failed to create Tantivy index {}", path.display()))?;
            write_schema_version(&path)?;
            index
        };

        let fields = IndexFields::from_schema(&index.schema())?;
        Ok(Self {
            index,
            path,
            fields,
        })
    }

    pub fn rebuild(&self, sessions: Vec<Session>) -> Result<RefreshSummary> {
        let mut writer: IndexWriter<TantivyDocument> =
            self.index.writer_with_num_threads(1, 128_000_000)?;
        writer.delete_all_documents()?;
        for session in &sessions {
            writer.add_document(self.session_document(session))?;
        }
        writer.commit()?;
        Ok(RefreshSummary {
            sessions: sessions.len(),
            new_or_modified: sessions.len(),
            deleted: 0,
        })
    }

    pub fn refresh_incremental(&self) -> Result<RefreshSummary> {
        self.refresh_incremental_streaming(INDEX_REFRESH_BATCH_SIZE, |_| {})
    }

    pub fn refresh_incremental_streaming<F>(
        &self,
        batch_size: usize,
        mut on_progress: F,
    ) -> Result<RefreshSummary>
    where
        F: FnMut(RefreshSummary),
    {
        let known = self.known_sessions()?;
        let (tx, rx) = mpsc::channel();
        for adapter in all_adapters() {
            let tx = tx.clone();
            let known = known.clone();
            thread::spawn(move || {
                let scan = {
                    let mut on_session = |session| {
                        let _ = tx.send(AdapterEvent::Session(session));
                    };
                    adapter.find_sessions_incremental_streaming(&known, &mut on_session)
                };
                let _ = tx.send(AdapterEvent::Finished {
                    agent: scan.agent,
                    deleted_ids: scan.deleted_ids,
                });
            });
        }
        drop(tx);

        let batch_size = batch_size.max(1);
        let mut batch = Vec::new();
        let mut changed = 0usize;
        let mut deleted = 0usize;
        let mut known_keys: HashSet<(String, String)> = known.keys().cloned().collect();
        let mut total_sessions = known_keys.len();

        for event in rx {
            match event {
                AdapterEvent::Session(session) => {
                    batch.push(session);
                    if batch.len() >= batch_size {
                        self.flush_refresh_batch(
                            &mut batch,
                            &mut known_keys,
                            &mut total_sessions,
                            &mut changed,
                            deleted,
                            &mut on_progress,
                        )?;
                    }
                }
                AdapterEvent::Finished { agent, deleted_ids } => {
                    if !batch.is_empty() {
                        self.flush_refresh_batch(
                            &mut batch,
                            &mut known_keys,
                            &mut total_sessions,
                            &mut changed,
                            deleted,
                            &mut on_progress,
                        )?;
                    }
                    if !deleted_ids.is_empty() {
                        self.delete_sessions(agent, &deleted_ids)?;
                        deleted += deleted_ids.len();
                        let agent = agent.to_string();
                        for id in &deleted_ids {
                            if known_keys.remove(&(agent.clone(), id.clone())) {
                                total_sessions = total_sessions.saturating_sub(1);
                            }
                        }
                        let summary = RefreshSummary {
                            sessions: total_sessions,
                            new_or_modified: changed,
                            deleted,
                        };
                        on_progress(summary);
                    }
                }
            }
        }

        if !batch.is_empty() {
            self.flush_refresh_batch(
                &mut batch,
                &mut known_keys,
                &mut total_sessions,
                &mut changed,
                deleted,
                &mut on_progress,
            )?;
        }

        Ok(RefreshSummary {
            sessions: self.total_len()?,
            new_or_modified: changed,
            deleted,
        })
    }

    fn flush_refresh_batch<F>(
        &self,
        batch: &mut Vec<Session>,
        known_keys: &mut HashSet<(String, String)>,
        total_sessions: &mut usize,
        changed: &mut usize,
        deleted: usize,
        on_progress: &mut F,
    ) -> Result<()>
    where
        F: FnMut(RefreshSummary),
    {
        if batch.is_empty() {
            return Ok(());
        }

        self.update_sessions(batch)?;
        *changed += batch.len();
        for session in batch.iter() {
            if known_keys.insert((session.agent.clone(), session.id.clone())) {
                *total_sessions += 1;
            }
        }
        batch.clear();
        on_progress(RefreshSummary {
            sessions: *total_sessions,
            new_or_modified: *changed,
            deleted,
        });
        Ok(())
    }

    pub fn scan_all_sessions() -> Vec<Session> {
        let sessions: Vec<_> = all_adapters()
            .into_par_iter()
            .flat_map(|adapter| adapter.find_sessions())
            .collect();
        sort_and_dedupe_sessions(sessions)
    }

    pub fn known_sessions(&self) -> Result<KnownSessions> {
        let searcher = self.searcher()?;
        let mut known = KnownSessions::new();
        for (_, address) in self.search_all_addresses(&searcher)? {
            let doc = searcher.doc::<TantivyDocument>(address)?;
            let Some(id) = text(&doc, self.fields.id) else {
                continue;
            };
            let Some(agent) = text(&doc, self.fields.agent) else {
                continue;
            };
            let mtime = number(&doc, self.fields.mtime).unwrap_or(0.0);
            known.insert((agent.to_string(), id.to_string()), mtime);
        }
        Ok(known)
    }

    pub fn all_sessions(&self) -> Result<Vec<Session>> {
        let searcher = self.searcher()?;
        let mut sessions = Vec::new();
        for (_, address) in self.search_all_addresses(&searcher)? {
            let doc = searcher.doc::<TantivyDocument>(address)?;
            if let Some(session) = self.doc_to_session(&doc) {
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
        if limit == 0 {
            return Ok(Vec::new());
        }

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

        let search_text = parsed.text.trim();
        let searcher = self.searcher()?;
        let query = self.build_query(search_text, effective_agent, effective_dir, parsed.date)?;
        if search_text.is_empty() {
            let collector =
                TopDocs::with_limit(limit).order_by_fast_field::<f64>("timestamp", Order::Desc);
            let hits: Vec<(Option<f64>, DocAddress)> = searcher.search(&query, &collector)?;
            self.hits_to_sessions(
                &searcher,
                hits.into_iter()
                    .map(|(score, addr)| (score.unwrap_or_default() as f32, addr)),
            )
        } else {
            let hits: Vec<(Score, DocAddress)> =
                searcher.search(&query, &TopDocs::with_limit(limit).order_by_score())?;
            self.hits_to_sessions(&searcher, hits.into_iter())
        }
    }

    pub fn stats(&self) -> Result<IndexStats> {
        let sessions = self.all_sessions()?;
        let mut stats = IndexStats {
            total_sessions: sessions.len(),
            index_size_bytes: index_size(&self.path),
            index_path: self.path.clone(),
            ..IndexStats::default()
        };

        let mut dir_counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for session in sessions {
            stats.total_messages += session.message_count;
            *stats
                .sessions_by_agent
                .entry(session.agent.clone())
                .or_default() += 1;
            *stats
                .messages_by_agent
                .entry(session.agent.clone())
                .or_default() += session.message_count;

            let dir = if session.directory.is_empty() {
                "n/a".to_string()
            } else {
                session.display_directory()
            };
            let dir_entry = dir_counts.entry(dir).or_default();
            dir_entry.0 += 1;
            dir_entry.1 += session.message_count;

            stats.oldest = Some(match stats.oldest {
                Some(oldest) => oldest.min(session.timestamp),
                None => session.timestamp,
            });
            stats.newest = Some(match stats.newest {
                Some(newest) => newest.max(session.timestamp),
                None => session.timestamp,
            });

            let weekday = session.timestamp.weekday().to_string();
            *stats.sessions_by_weekday.entry(weekday).or_default() += 1;
            *stats
                .sessions_by_hour
                .entry(session.timestamp.hour())
                .or_default() += 1;
        }

        let mut dirs: Vec<_> = dir_counts
            .into_iter()
            .map(|(dir, (sessions, messages))| (dir, sessions, messages))
            .collect();
        dirs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.2.cmp(&a.2)));
        dirs.truncate(10);
        stats.top_directories = dirs;

        Ok(stats)
    }

    fn update_sessions(&self, sessions: &[Session]) -> Result<()> {
        if sessions.is_empty() {
            return Ok(());
        }
        let mut writer: IndexWriter<TantivyDocument> =
            self.index.writer_with_num_threads(1, 128_000_000)?;
        for session in sessions {
            writer.delete_term(Term::from_field_text(
                self.fields.session_key,
                &session_key(&session.agent, &session.id),
            ));
        }
        for session in sessions {
            writer.add_document(self.session_document(session))?;
        }
        writer.commit()?;
        Ok(())
    }

    fn delete_sessions(&self, agent: &str, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut writer: IndexWriter<TantivyDocument> =
            self.index.writer_with_num_threads(1, 64_000_000)?;
        for id in ids {
            writer.delete_term(Term::from_field_text(
                self.fields.session_key,
                &session_key(agent, id),
            ));
        }
        writer.commit()?;
        Ok(())
    }

    fn session_document(&self, session: &Session) -> TantivyDocument {
        doc!(
            self.fields.id => session.id.clone(),
            self.fields.session_key => session_key(&session.agent, &session.id),
            self.fields.title => session.title.clone(),
            self.fields.directory => session.directory.clone(),
            self.fields.agent => session.agent.clone(),
            self.fields.content => session.content.clone(),
            self.fields.timestamp => datetime_to_seconds(session.timestamp),
            self.fields.message_count => session.message_count as i64,
            self.fields.mtime => session.mtime,
            self.fields.yolo => session.yolo,
        )
    }

    fn searcher(&self) -> Result<tantivy::Searcher> {
        let reader = self.index.reader()?;
        reader.reload()?;
        Ok(reader.searcher())
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
            if let Some(session) = self.doc_to_session(&doc) {
                sessions.push(SearchHit { session, score });
            }
        }
        Ok(sessions)
    }

    fn build_query(
        &self,
        search_text: &str,
        agent_filter: Option<Filter>,
        directory_filter: Option<Filter>,
        date_filter: Option<crate::query::DateFilter>,
    ) -> Result<Box<dyn Query>> {
        let mut parts: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        if !search_text.is_empty() {
            parts.push((Occur::Must, self.text_query(search_text)?));
        }

        if let Some(query) = self.agent_query(agent_filter) {
            parts.push((Occur::Must, query));
        }
        if let Some(query) = self.directory_query(directory_filter)? {
            parts.push((Occur::Must, query));
        }
        if let Some(date) = date_filter {
            let query = self.date_query(&date);
            if date.negated {
                if parts.is_empty() {
                    parts.push((Occur::Must, Box::new(AllQuery)));
                }
                parts.push((Occur::MustNot, query));
            } else {
                parts.push((Occur::Must, query));
            }
        }

        Ok(if parts.is_empty() {
            Box::new(AllQuery)
        } else {
            Box::new(BooleanQuery::new(parts))
        })
    }

    fn text_query(&self, search_text: &str) -> Result<Box<dyn Query>> {
        let parser = QueryParser::for_index(
            &self.index,
            vec![
                self.fields.title,
                self.fields.content,
                self.fields.directory,
            ],
        );
        let exact = parser.parse_query(search_text)?;
        let boosted_exact = BoostQuery::new(exact, 5.0);

        let mut alternatives: Vec<(Occur, Box<dyn Query>)> =
            vec![(Occur::Should, Box::new(boosted_exact))];
        let fuzzy_parts: Vec<(Occur, Box<dyn Query>)> = search_text
            .split_whitespace()
            .filter(|term| !term.is_empty())
            .map(|term| {
                let term = term.to_lowercase();
                let title = FuzzyTermQuery::new_prefix(
                    Term::from_field_text(self.fields.title, &term),
                    1,
                    true,
                );
                let content = FuzzyTermQuery::new_prefix(
                    Term::from_field_text(self.fields.content, &term),
                    1,
                    true,
                );
                (
                    Occur::Must,
                    Box::new(BooleanQuery::new(vec![
                        (Occur::Should, Box::new(title)),
                        (Occur::Should, Box::new(content)),
                    ])) as Box<dyn Query>,
                )
            })
            .collect();

        if !fuzzy_parts.is_empty() {
            alternatives.push((Occur::Should, Box::new(BooleanQuery::new(fuzzy_parts))));
        }

        Ok(Box::new(BooleanQuery::new(alternatives)))
    }

    fn agent_query(&self, filter: Option<Filter>) -> Option<Box<dyn Query>> {
        let filter = filter?;
        let mut parts: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        if !filter.include.is_empty() {
            let terms = filter
                .include
                .iter()
                .map(|agent| Term::from_field_text(self.fields.agent, agent));
            parts.push((Occur::Must, Box::new(TermSetQuery::new(terms))));
        }
        for excluded in filter.exclude {
            let term = Term::from_field_text(self.fields.agent, &excluded);
            parts.push((
                Occur::MustNot,
                Box::new(TermQuery::new(term, IndexRecordOption::Basic)),
            ));
        }
        if parts.is_empty() {
            return None;
        }
        if parts.iter().all(|(occur, _)| *occur == Occur::MustNot) {
            parts.insert(0, (Occur::Must, Box::new(AllQuery)));
        }
        Some(Box::new(BooleanQuery::new(parts)))
    }

    fn directory_query(&self, filter: Option<Filter>) -> Result<Option<Box<dyn Query>>> {
        let Some(filter) = filter else {
            return Ok(None);
        };
        let mut parts: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        if !filter.include.is_empty() {
            let include_parts: Result<Vec<_>> = filter
                .include
                .iter()
                .map(|dir| {
                    let pattern = format!("(?i).*{}.*", regex::escape(dir));
                    Ok((
                        Occur::Should,
                        Box::new(RegexQuery::from_pattern(&pattern, self.fields.directory)?)
                            as Box<dyn Query>,
                    ))
                })
                .collect();
            parts.push((Occur::Must, Box::new(BooleanQuery::new(include_parts?))));
        }

        for excluded in filter.exclude {
            let pattern = format!("(?i).*{}.*", regex::escape(&excluded));
            let query = RegexQuery::from_pattern(&pattern, self.fields.directory)?;
            parts.push((Occur::MustNot, Box::new(query)));
        }
        if parts.is_empty() {
            return Ok(None);
        }
        if parts.iter().all(|(occur, _)| *occur == Occur::MustNot) {
            parts.insert(0, (Occur::Must, Box::new(AllQuery)));
        }
        Ok(Some(Box::new(BooleanQuery::new(parts))))
    }

    fn date_query(&self, date: &crate::query::DateFilter) -> Box<dyn Query> {
        let cutoff = datetime_to_seconds(date.cutoff);
        match date.op {
            DateOp::LessThan => Box::new(RangeQuery::new(
                Bound::Included(Term::from_field_f64(self.fields.timestamp, cutoff)),
                Bound::Unbounded,
            )),
            DateOp::GreaterThan => Box::new(RangeQuery::new(
                Bound::Unbounded,
                Bound::Excluded(Term::from_field_f64(self.fields.timestamp, cutoff)),
            )),
            DateOp::Exact if date.value.eq_ignore_ascii_case("today") => Box::new(RangeQuery::new(
                Bound::Included(Term::from_field_f64(self.fields.timestamp, cutoff)),
                Bound::Unbounded,
            )),
            DateOp::Exact if date.value.eq_ignore_ascii_case("yesterday") => {
                let end = cutoff + 86_400.0;
                Box::new(RangeQuery::new(
                    Bound::Included(Term::from_field_f64(self.fields.timestamp, cutoff)),
                    Bound::Excluded(Term::from_field_f64(self.fields.timestamp, end)),
                ))
            }
            DateOp::Exact => Box::new(AllQuery),
        }
    }

    fn doc_to_session(&self, doc: &TantivyDocument) -> Option<Session> {
        let timestamp = number(doc, self.fields.timestamp)?;
        let mut session = Session::new(
            text(doc, self.fields.id)?.to_string(),
            text(doc, self.fields.agent).unwrap_or_default().to_string(),
            text(doc, self.fields.title).unwrap_or_default().to_string(),
            text(doc, self.fields.directory)
                .unwrap_or_default()
                .to_string(),
            Local.timestamp_opt(timestamp as i64, 0).single()?,
            text(doc, self.fields.content)
                .unwrap_or_default()
                .to_string(),
            integer(doc, self.fields.message_count).unwrap_or(0) as usize,
        );
        session.mtime = number(doc, self.fields.mtime).unwrap_or(0.0);
        session.yolo = boolean(doc, self.fields.yolo).unwrap_or(false);
        Some(session)
    }
}

impl IndexFields {
    fn from_schema(schema: &Schema) -> Result<Self> {
        Ok(Self {
            id: schema.get_field("id")?,
            session_key: schema.get_field("session_key")?,
            title: schema.get_field("title")?,
            directory: schema.get_field("directory")?,
            agent: schema.get_field("agent")?,
            content: schema.get_field("content")?,
            timestamp: schema.get_field("timestamp")?,
            message_count: schema.get_field("message_count")?,
            mtime: schema.get_field("mtime")?,
            yolo: schema.get_field("yolo")?,
        })
    }
}

fn build_schema() -> Schema {
    let mut schema = Schema::builder();
    schema.add_text_field("id", raw_text_options());
    schema.add_text_field("session_key", raw_text_options());
    schema.add_text_field("title", TEXT | STORED);
    schema.add_text_field("directory", raw_text_options());
    schema.add_text_field("agent", raw_text_options());
    schema.add_text_field("content", TEXT | STORED);
    schema.add_f64_field(
        "timestamp",
        NumericOptions::default()
            .set_stored()
            .set_indexed()
            .set_fast(),
    );
    schema.add_i64_field("message_count", STORED);
    schema.add_f64_field("mtime", STORED);
    schema.add_bool_field("yolo", STORED);
    schema.build()
}

fn raw_text_options() -> TextOptions {
    TextOptions::default().set_stored().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("raw")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    )
}

fn schema_version_matches(path: &Path) -> bool {
    let version_file = path.join(VERSION_FILE);
    fs::read_to_string(version_file)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        == Some(INDEX_SCHEMA_VERSION)
}

fn write_schema_version(path: &Path) -> Result<()> {
    fs::write(path.join(VERSION_FILE), INDEX_SCHEMA_VERSION.to_string())
        .with_context(|| format!("failed to write schema version in {}", path.display()))
}

fn datetime_to_seconds(timestamp: DateTime<Local>) -> f64 {
    timestamp.timestamp() as f64 + f64::from(timestamp.timestamp_subsec_nanos()) / 1e9
}

fn session_key(agent: &str, id: &str) -> String {
    format!("{agent}::{id}")
}

fn text(doc: &TantivyDocument, field: Field) -> Option<&str> {
    doc.get_first(field).and_then(|value| value.as_str())
}

fn number(doc: &TantivyDocument, field: Field) -> Option<f64> {
    doc.get_first(field).and_then(|value| value.as_f64())
}

fn integer(doc: &TantivyDocument, field: Field) -> Option<i64> {
    doc.get_first(field).and_then(|value| value.as_i64())
}

fn boolean(doc: &TantivyDocument, field: Field) -> Option<bool> {
    doc.get_first(field).and_then(|value| value.as_bool())
}

fn index_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.path().metadata().ok())
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use chrono::Local;
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
}
