use std::collections::HashSet;
use std::env;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use anyhow::Result;
use rayon::prelude::*;

use crate::adapters::all_adapters;
use crate::index::{INDEX_REFRESH_BATCH_SIZE, RefreshSummary, SessionIndex};
use crate::model::{Session, sort_and_dedupe_sessions};

enum AdapterEvent {
    Session(Session),
    Finished {
        agent: &'static str,
        deleted_ids: Vec<String>,
    },
}

pub fn scan_all_sessions() -> Vec<Session> {
    let trace_refresh = env::var_os("FAST_RESUME_TRACE_REFRESH").is_some();
    let sessions: Vec<_> = all_adapters()
        .into_par_iter()
        .flat_map(|adapter| {
            let started = Instant::now();
            let agent = adapter.name();
            let sessions = adapter.find_sessions();
            if trace_refresh {
                eprintln!(
                    "scan {agent}: {:.3}s, sessions={}",
                    started.elapsed().as_secs_f64(),
                    sessions.len()
                );
            }
            sessions
        })
        .collect();
    sort_and_dedupe_sessions(sessions)
}

pub fn refresh_incremental(index: &SessionIndex) -> Result<RefreshSummary> {
    refresh_incremental_streaming(index, INDEX_REFRESH_BATCH_SIZE, |_| {})
}

pub fn refresh_incremental_streaming<F>(
    index: &SessionIndex,
    batch_size: usize,
    mut on_progress: F,
) -> Result<RefreshSummary>
where
    F: FnMut(RefreshSummary),
{
    let known = index.known_sessions()?;
    if known.is_empty() {
        let sessions = scan_all_sessions();
        let summary = index.rebuild(sessions)?;
        on_progress(summary.clone());
        return Ok(summary);
    }

    let (tx, rx) = mpsc::channel();
    let trace_refresh = env::var_os("FAST_RESUME_TRACE_REFRESH").is_some();
    for adapter in all_adapters() {
        let tx = tx.clone();
        let known = known.clone();
        let trace_refresh = trace_refresh;
        thread::spawn(move || {
            let started = Instant::now();
            let agent = adapter.name();
            let scan = {
                let mut on_session = |session| {
                    let _ = tx.send(AdapterEvent::Session(session));
                };
                adapter.find_sessions_incremental_streaming(&known, &mut on_session)
            };
            if trace_refresh {
                eprintln!(
                    "refresh {agent}: {:.3}s, changed={}, deleted={}",
                    started.elapsed().as_secs_f64(),
                    scan.new_or_modified.len(),
                    scan.deleted_ids.len()
                );
            }
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
                    flush_refresh_batch(
                        index,
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
                    flush_refresh_batch(
                        index,
                        &mut batch,
                        &mut known_keys,
                        &mut total_sessions,
                        &mut changed,
                        deleted,
                        &mut on_progress,
                    )?;
                }
                if !deleted_ids.is_empty() {
                    index.delete_sessions(agent, &deleted_ids)?;
                    deleted += deleted_ids.len();
                    let agent = agent.to_string();
                    for id in &deleted_ids {
                        if known_keys.remove(&(agent.clone(), id.clone())) {
                            total_sessions = total_sessions.saturating_sub(1);
                        }
                    }
                    on_progress(RefreshSummary {
                        sessions: total_sessions,
                        new_or_modified: changed,
                        deleted,
                    });
                }
            }
        }
    }

    if !batch.is_empty() {
        flush_refresh_batch(
            index,
            &mut batch,
            &mut known_keys,
            &mut total_sessions,
            &mut changed,
            deleted,
            &mut on_progress,
        )?;
    }

    Ok(RefreshSummary {
        sessions: index.total_len()?,
        new_or_modified: changed,
        deleted,
    })
}

fn flush_refresh_batch<F>(
    index: &SessionIndex,
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

    index.update_sessions(batch)?;
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
