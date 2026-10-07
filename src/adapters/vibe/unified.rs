//! Reader for sessions written by the Vibe Unified Harness.
//!
//! Vibe 2.x runs the Unified Harness by default; `--legacy-harness` is the
//! escape hatch that still writes the older `session_*/meta.json` and
//! `messages.jsonl` folders. Unified sessions live under
//! `<sessions_dir>/unified/<session_id>/`:
//!
//! ```text
//! CURRENT                              pointer to the published generation
//! generations/<generation>/
//!   manifest.json                      names the documents below and the journal segment
//!   runtime-state.json                 session_metadata (cwd, agent), identity, lifetime
//!   projection-state.json              public state: session title, timestamps, history
//! chunks/<sha256>.json                 pooled history runs when the manifest lists chunks
//! journal/<first-sequence>.jsonl       recovery journal appended after the generation
//! ```
//!
//! The store format (`mistral.vibe.unified-session-store/v1`) is private to
//! Vibe and may change. This reader therefore touches as little of it as
//! possible: it reads the public history projection, which mirrors what the
//! Vibe UI shows, plus the working directory and agent from the runtime state.
//! Vibe publishes a new generation only on store compaction or when a journal
//! record outgrows its page, so ordinary turns land in the journal as
//! `projection_delta` records. The reader replays those over the generation's
//! snapshot, the same way Vibe restores a session.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::model::{Session, file_mtime_seconds, file_timestamp, truncate_title};

use super::super::shared::{
    IncrementalParse, SessionFileScan, string_at, timestamp_from_ms, value_i64_at,
};

/// Directory, relative to the Vibe session root, that holds unified sessions.
pub(super) const UNIFIED_DIRNAME: &str = "unified";

/// Agent profile that Vibe resumes with `--agent auto-approve`.
const AUTO_APPROVE_AGENT: &str = "auto-approve";

/// Outcome of reading one unified session directory.
pub(super) enum UnifiedParse {
    /// A user-facing session with conversation content.
    Session(Session),
    /// The directory is readable but should not be listed: a subagent child,
    /// an ephemeral session, or a session without any conversation yet.
    Skip,
    /// The store could not be read consistently, for example because Vibe was
    /// rotating generations mid-read. Callers keep previously indexed data.
    Unreadable,
}

impl From<UnifiedParse> for IncrementalParse {
    fn from(parse: UnifiedParse) -> Self {
        match parse {
            UnifiedParse::Session(session) => Self::Session(session),
            UnifiedParse::Skip => Self::Delete,
            UnifiedParse::Unreadable => Self::Retain,
        }
    }
}

#[derive(Deserialize)]
struct CurrentPointer {
    session_id: String,
    generation: String,
}

#[derive(Deserialize)]
struct StoredFile {
    path: String,
    /// Ordered pool chunks whose concatenation is the document's transcript.
    #[serde(default)]
    chunks: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct JournalSegment {
    path: String,
}

#[derive(Deserialize)]
struct GenerationManifest {
    session_id: String,
    runtime_state: StoredFile,
    projection_state: StoredFile,
    recovery_journal_segment: JournalSegment,
}

#[derive(Deserialize)]
struct RuntimeState {
    #[serde(default)]
    storage_lifetime: Option<String>,
    #[serde(default)]
    session_metadata: SessionMetadata,
    #[serde(default)]
    identity: Option<SessionIdentity>,
}

#[derive(Deserialize, Default)]
struct SessionMetadata {
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent_name: Option<String>,
}

#[derive(Deserialize)]
struct SessionIdentity {
    /// `root`, `fork`, or `subagent`.
    kind: String,
}

/// The parts of Vibe's public session projection that fast-resume indexes.
struct Projection {
    /// `snapshot.session`: id, title, preview, and millisecond timestamps.
    session: Value,
    /// `snapshot.history.entries`, oldest first.
    entries: Vec<Value>,
}

impl Projection {
    fn from_snapshot(mut snapshot: Value) -> Self {
        let entries = match snapshot.pointer_mut("/history/entries").map(Value::take) {
            Some(Value::Array(entries)) => entries,
            _ => Vec::new(),
        };
        Self {
            session: snapshot
                .get_mut("session")
                .map(Value::take)
                .unwrap_or_default(),
            entries,
        }
    }

    /// Apply one `projection_delta` operation, mirroring Vibe's
    /// `apply_projection_delta`. Vibe treats a reference to an absent entry as
    /// corruption; a read-only indexer ignores it and keeps what it has.
    fn apply_op(&mut self, op: &Value) {
        let id = op.get("id").and_then(Value::as_str);
        match op.get("op").and_then(Value::as_str) {
            Some("append_entry") => {
                if let Some(entry) = op.get("entry") {
                    self.entries.push(entry.clone());
                }
            }
            Some("replace_entry") => {
                if let (Some(id), Some(entry)) = (id, op.get("entry"))
                    && let Some(slot) = self
                        .entries
                        .iter_mut()
                        .find(|item| entry_id(item) == Some(id))
                {
                    *slot = entry.clone();
                }
            }
            Some("remove_entry") => {
                if let Some(id) = id {
                    self.entries.retain(|item| entry_id(item) != Some(id));
                }
            }
            Some("set_history_entries") => {
                if let Some(Value::Array(entries)) = op.get("entries") {
                    self.entries.clone_from(entries);
                }
            }
            // The envelope carries every public field except history entries.
            Some("set_envelope") => {
                if let Some(session) = op.pointer("/state/session") {
                    self.session = session.clone();
                }
            }
            _ => {}
        }
    }
}

fn entry_id(entry: &Value) -> Option<&str> {
    entry.get("id").and_then(Value::as_str)
}

/// Whether `path` is a session directory inside the unified store.
pub(super) fn is_unified_session_dir(sessions_dir: &Path, path: &Path) -> bool {
    path.parent() == Some(sessions_dir.join(UNIFIED_DIRNAME).as_path())
}

/// List unified session directories keyed by session id. Returns `None` when
/// the store exists but cannot be listed; the completeness flag is false when
/// individual entries could not be read.
pub(super) fn scan_session_dirs(sessions_dir: &Path) -> Option<SessionFileScan> {
    let unified_dir = sessions_dir.join(UNIFIED_DIRNAME);
    let mut sessions = HashMap::new();
    if !unified_dir.exists() {
        return Some((sessions, true));
    }
    let entries = fs::read_dir(&unified_dir).ok()?;
    let mut complete = true;
    for entry in entries {
        let Ok(entry) = entry else {
            complete = false;
            continue;
        };
        let path = entry.path();
        let Some(session_id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        // `.session-index.json` and atomic-write temporaries share the
        // directory with session folders.
        if session_id.starts_with('.') || !path.is_dir() {
            continue;
        }
        // A folder without `CURRENT` has no published generation yet.
        if !path.join("CURRENT").is_file() {
            continue;
        }
        let mtime = session_mtime(&path);
        sessions.insert(session_id.to_string(), (path, mtime));
    }
    Some((sessions, complete))
}

/// Change marker for incremental refresh. Vibe rewrites `CURRENT` when it
/// publishes a generation and appends to a journal segment for everything
/// else, so the newest of those files moves whenever the session changes.
pub(super) fn session_mtime(session_dir: &Path) -> f64 {
    let current = file_mtime_seconds(&session_dir.join("CURRENT"));
    let Ok(segments) = fs::read_dir(session_dir.join("journal")) else {
        return current;
    };
    segments
        .filter_map(Result::ok)
        .map(|segment| file_mtime_seconds(&segment.path()))
        .fold(current, f64::max)
}

pub(super) fn parse_session(agent: &'static str, session_dir: &Path) -> UnifiedParse {
    let Some(stored) = load_store(session_dir) else {
        return UnifiedParse::Unreadable;
    };
    let StoredSession {
        session_id,
        runtime,
        projection,
    } = stored;

    // Subagent children appear inside their parent's conversation, and Vibe
    // hides them from its own session picker. Ephemeral sessions are not meant
    // to be resumed.
    if runtime
        .identity
        .as_ref()
        .is_some_and(|identity| identity.kind == "subagent")
        || runtime.storage_lifetime.as_deref() == Some("ephemeral")
    {
        return UnifiedParse::Skip;
    }

    let mut messages = Vec::new();
    let mut first_user = String::new();
    for entry in &projection.entries {
        // Reasoning, tool effects, callbacks, checkpoints, and notices are
        // not conversation text.
        if entry.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let role = entry
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let role_prefix = match role {
            "user" => "» ",
            "assistant" => "  ",
            _ => continue,
        };
        for text in message_texts(entry) {
            if role == "user" && first_user.is_empty() {
                first_user.clone_from(&text);
            }
            messages.push(format!("{role_prefix}{text}"));
        }
    }
    if messages.is_empty() {
        return UnifiedParse::Skip;
    }

    let title = Some(string_at(&projection.session, &["title"]))
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| truncate_title(&first_user, 80, false));
    let title = if title.is_empty() {
        "Vibe session".to_string()
    } else {
        title
    };
    let timestamp = timestamp_from_ms(value_i64_at(&projection.session, &["createdAt"]))
        .unwrap_or_else(|| file_timestamp(&session_dir.join("CURRENT")));

    let mut session = Session::new(
        session_id,
        agent,
        title,
        runtime.session_metadata.cwd.unwrap_or_default(),
        timestamp,
        messages.join("\n\n"),
        messages.len(),
    );
    session.mtime = session_mtime(session_dir);
    session.yolo = runtime.session_metadata.agent_name.as_deref() == Some(AUTO_APPROVE_AGENT);
    UnifiedParse::Session(session)
}

/// Text blocks of a public history message. Images, resource links, and
/// embedded file contents are not indexed.
fn message_texts(entry: &Value) -> Vec<String> {
    let Some(Value::Array(blocks)) = entry.get("content") else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .filter(|text| !text.trim().is_empty())
        .map(ToString::to_string)
        .collect()
}

struct StoredSession {
    session_id: String,
    runtime: RuntimeState,
    projection: Projection,
}

/// Load the published generation and replay its journal. Any missing or
/// inconsistent document yields `None` so callers keep indexed data; Vibe
/// keeps the previous generation on disk while it publishes the next one, so
/// a retry on the next refresh normally succeeds.
fn load_store(session_dir: &Path) -> Option<StoredSession> {
    let current: CurrentPointer = read_json(&session_dir.join("CURRENT"))?;
    let dir_name = session_dir.file_name()?.to_str()?;
    if current.session_id != dir_name || !is_plain_name(&current.generation) {
        return None;
    }
    let generation_dir = session_dir.join("generations").join(&current.generation);
    let manifest: GenerationManifest = read_json(&generation_dir.join("manifest.json"))?;
    if manifest.session_id != current.session_id {
        return None;
    }

    let runtime: RuntimeState =
        read_json(&generation_dir.join(document_name(&manifest.runtime_state)?))?;

    let snapshot =
        read_json::<Value>(&generation_dir.join(document_name(&manifest.projection_state)?))?
            .get_mut("snapshot")
            .map(Value::take)?;
    let mut projection = Projection::from_snapshot(snapshot);
    // A pooled projection stores its history entries in content-addressed
    // chunks and leaves the envelope's entry list empty.
    if let Some(chunks) = &manifest.projection_state.chunks {
        projection.entries = read_chunked_transcript(&session_dir.join("chunks"), chunks)?;
    }

    let journal = manifest.recovery_journal_segment.path;
    let mut parts = journal.split('/');
    let (Some("journal"), Some(segment), None) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    if !is_plain_name(segment) {
        return None;
    }
    replay_journal(&session_dir.join("journal").join(segment), &mut projection)?;

    Some(StoredSession {
        session_id: current.session_id,
        runtime,
        projection,
    })
}

/// Replay projection records from a journal segment. A missing segment means
/// nothing was appended yet. An unterminated final line is a write in
/// progress and is ignored, as Vibe does. A malformed complete record means
/// the segment cannot be trusted, so the load fails.
fn replay_journal(path: &Path, projection: &mut Projection) -> Option<()> {
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(()),
        Err(_) => return None,
    };
    let complete = match data.iter().rposition(|byte| *byte == b'\n') {
        Some(end) => &data[..end],
        None => return Some(()),
    };
    for line in complete.split(|byte| *byte == b'\n') {
        // Most records are Runtime bookkeeping (actions, receipts, callbacks).
        // Skip them without building a JSON tree; only projection records can
        // carry this marker.
        if !contains(line, b"\"projection_") {
            continue;
        }
        let record: Value = serde_json::from_slice(line).ok()?;
        match record.get("type").and_then(Value::as_str) {
            Some("projection_advanced") => {
                let snapshot = record.pointer("/payload/snapshot")?.clone();
                *projection = Projection::from_snapshot(snapshot);
            }
            Some("projection_delta") => {
                if let Some(Value::Array(ops)) = record.pointer("/payload/delta") {
                    for op in ops {
                        projection.apply_op(op);
                    }
                }
            }
            _ => {}
        }
    }
    Some(())
}

fn read_chunked_transcript(chunk_dir: &Path, digests: &[String]) -> Option<Vec<Value>> {
    let mut entries = Vec::new();
    for digest in digests {
        if !is_plain_name(digest) {
            return None;
        }
        let Value::Array(chunk) = read_json(&chunk_dir.join(format!("{digest}.json")))? else {
            return None;
        };
        entries.extend(chunk);
    }
    Some(entries)
}

/// Generation documents are referenced by plain file names. Reject anything
/// that could escape the generation directory.
fn document_name(file: &StoredFile) -> Option<&str> {
    is_plain_name(&file.path).then_some(file.path.as_str())
}

fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::Duration;

    use serde_json::{Value, json};
    use tempfile::tempdir;

    use crate::adapters::vibe::VibeAdapter;
    use crate::adapters::{Adapter, KnownSessions};

    const GENERATION: &str = "0000000000000003";
    const JOURNAL: &str = "journal/0000000000000004.jsonl";

    fn message(id: &str, role: &str, text: &str) -> Value {
        json!({
            "type": "message",
            "id": id,
            "role": role,
            "content": [{"type": "text", "text": text}],
            "createdAt": 1_784_110_800_000_i64,
        })
    }

    /// Write a unified session shaped like the files Vibe 2.25 writes: a
    /// published generation whose projection history is pooled into one
    /// chunk, plus an empty journal segment.
    fn write_session(sessions_dir: &Path, id: &str, runtime: Value, entries: &[Value]) -> PathBuf {
        let session_dir = sessions_dir.join("unified").join(id);
        let generation_dir = session_dir.join("generations").join(GENERATION);
        fs::create_dir_all(&generation_dir).unwrap();
        fs::create_dir_all(session_dir.join("chunks")).unwrap();
        fs::create_dir_all(session_dir.join("journal")).unwrap();

        let chunk = "a".repeat(64);
        fs::write(
            session_dir.join("chunks").join(format!("{chunk}.json")),
            format!("{}\n", Value::Array(entries.to_vec())),
        )
        .unwrap();
        let mut runtime_state = json!({
            "session_id": id,
            "storage_lifetime": "persistent",
            "session_metadata": {"cwd": "/work/vibe", "agent_name": null, "root_session_id": id},
            "identity": {"kind": "root", "session_id": id, "root_session_id": id},
        });
        json_merge(&mut runtime_state, runtime);
        fs::write(
            generation_dir.join("runtime-state.json"),
            runtime_state.to_string(),
        )
        .unwrap();
        fs::write(
            generation_dir.join("projection-state.json"),
            json!({
                "session_id": id,
                "snapshot": {
                    "session": {"id": id, "title": null, "createdAt": 1_784_110_800_000_i64},
                    "history": {"entries": [], "range": "latest"},
                },
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            generation_dir.join("manifest.json"),
            json!({
                "session_id": id,
                "generation": GENERATION,
                "runtime_state": {"path": "runtime-state.json", "chunks": null},
                "projection_state": {"path": "projection-state.json", "chunks": [chunk]},
                "recovery_journal_segment": {"first_sequence": 4, "path": JOURNAL},
            })
            .to_string(),
        )
        .unwrap();
        fs::write(session_dir.join(JOURNAL), "").unwrap();
        fs::write(
            session_dir.join("CURRENT"),
            json!({"session_id": id, "generation": GENERATION, "store_format_minor": 7})
                .to_string(),
        )
        .unwrap();
        session_dir
    }

    fn json_merge(base: &mut Value, patch: Value) {
        match (base, patch) {
            (Value::Object(base), Value::Object(patch)) => {
                for (key, value) in patch {
                    json_merge(base.entry(key).or_insert(Value::Null), value);
                }
            }
            (base, patch) => *base = patch,
        }
    }

    fn delta(sequence: u64, ops: Value) -> String {
        json!({
            "type": "projection_delta",
            "sequence": sequence,
            "payload": {"watermark": sequence, "delta": ops},
        })
        .to_string()
    }

    fn adapter(sessions_dir: &Path) -> VibeAdapter {
        VibeAdapter {
            sessions_dir: sessions_dir.to_path_buf(),
        }
    }

    #[test]
    fn parses_unified_session_from_chunks_and_journal() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_dir = write_session(
            &sessions_dir,
            "unified-1",
            json!({"session_metadata": {"agent_name": "auto-approve"}}),
            &[
                message("u1", "user", "Port the parser to the new store"),
                json!({"type": "reasoning", "id": "r1", "text": "private chain of thought"}),
                message("a1", "assistant", "Draft answer"),
            ],
        );
        // A live turn: the reply is rewritten in place, a later turn is
        // appended, a stale entry is removed, and the session is renamed. The
        // last line is still being written and must be ignored.
        let journal = [
            json!({"type": "core_input", "sequence": 4, "payload": {"text": "ignored"}})
                .to_string(),
            delta(
                5,
                json!([
                    {"op": "replace_entry", "id": "a1", "entry": message("a1", "assistant", "Final answer")},
                    {"op": "append_entry", "entry": message("u2", "user", "Follow-up question")},
                    {"op": "append_entry", "entry": message("tmp", "assistant", "Dropped draft")},
                    {"op": "remove_entry", "id": "tmp"},
                    {"op": "set_envelope", "state": {"session": {"id": "unified-1", "title": "Store port", "createdAt": 1_784_110_800_000_i64}, "history": {"entries": []}}},
                ]),
            ),
            r#"{"type":"projection_delta","sequence":6,"payload":{"delta":[{"op":"append_entry","#
                .to_string(),
        ];
        fs::write(session_dir.join(JOURNAL), journal.join("\n")).unwrap();

        let adapter = adapter(&sessions_dir);
        let sessions = adapter.find_sessions();

        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.id, "unified-1");
        assert_eq!(session.title, "Store port");
        assert_eq!(session.directory, "/work/vibe");
        assert!(session.yolo);
        assert_eq!(session.timestamp.timestamp_millis(), 1_784_110_800_000);
        assert_eq!(
            session.content,
            "» Port the parser to the new store\n\n  Final answer\n\n» Follow-up question"
        );
        assert_eq!(
            adapter.resume_command(session, false),
            vec!["vibe", "--resume", "unified-1"]
        );
    }

    #[test]
    fn projection_advanced_replaces_snapshot_and_inline_history_is_read() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_dir = write_session(&sessions_dir, "inline", json!({}), &[]);
        let generation_dir = session_dir.join("generations").join(GENERATION);
        // Stores written before history pooling keep entries inline.
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(generation_dir.join("manifest.json")).unwrap())
                .unwrap();
        manifest["projection_state"]["chunks"] = Value::Null;
        fs::write(generation_dir.join("manifest.json"), manifest.to_string()).unwrap();
        fs::write(
            generation_dir.join("projection-state.json"),
            json!({"snapshot": {"session": {"id": "inline"}, "history": {"entries": [message("u1", "user", "Inline prompt")]}}})
                .to_string(),
        )
        .unwrap();

        let sessions = adapter(&sessions_dir).find_sessions();
        assert_eq!(sessions[0].title, "Inline prompt");
        assert!(!sessions[0].yolo);

        let advanced = json!({
            "type": "projection_advanced",
            "sequence": 4,
            "payload": {"watermark": 9, "snapshot": {"session": {"id": "inline"}, "history": {"entries": [message("u9", "user", "Snapshot prompt")]}}},
        });
        fs::write(session_dir.join(JOURNAL), format!("{advanced}\n")).unwrap();

        let sessions = adapter(&sessions_dir).find_sessions();
        assert_eq!(sessions[0].content, "» Snapshot prompt");
    }

    #[test]
    fn skips_subagent_ephemeral_and_empty_sessions() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let prompt = [message("u1", "user", "Hidden prompt")];
        write_session(
            &sessions_dir,
            "child",
            json!({"identity": {"kind": "subagent"}}),
            &prompt,
        );
        write_session(
            &sessions_dir,
            "scratch",
            json!({"storage_lifetime": "ephemeral"}),
            &prompt,
        );
        write_session(&sessions_dir, "empty", json!({}), &[]);
        write_session(
            &sessions_dir,
            "fork",
            json!({"identity": {"kind": "fork"}}),
            &prompt,
        );
        // Vibe's listing cache sits next to the session folders.
        fs::write(sessions_dir.join("unified/.session-index.json"), "{}").unwrap();

        let sessions = adapter(&sessions_dir).find_sessions();

        let ids: Vec<_> = sessions.iter().map(|session| session.id.as_str()).collect();
        assert_eq!(ids, ["fork"]);
    }

    #[test]
    fn incremental_refresh_picks_up_journal_appends() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let session_dir = write_session(
            &sessions_dir,
            "live",
            json!({}),
            &[message("u1", "user", "First prompt")],
        );
        let adapter = adapter(&sessions_dir);
        let first = adapter.find_sessions_incremental(&KnownSessions::new());
        let indexed_mtime = first.new_or_modified[0].mtime;
        let mut known = KnownSessions::new();
        known.insert(("vibe".to_string(), "live".to_string()), indexed_mtime);

        thread::sleep(Duration::from_millis(20));
        let ops = json!([{"op": "append_entry", "entry": message("u2", "user", "Second prompt")}]);
        fs::write(session_dir.join(JOURNAL), format!("{}\n", delta(4, ops))).unwrap();

        let scan = adapter.find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert!(scan.new_or_modified[0].mtime > indexed_mtime);
        assert!(scan.new_or_modified[0].content.contains("Second prompt"));
        assert!(scan.deleted_ids.is_empty());
    }

    #[test]
    fn unreadable_stores_keep_indexed_sessions() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let prompt = [message("u1", "user", "Prompt")];
        // CURRENT names a generation that is missing, as during a rotation.
        let rotating = write_session(&sessions_dir, "rotating", json!({}), &prompt);
        fs::write(
            rotating.join("CURRENT"),
            json!({"session_id": "rotating", "generation": "0000000000000099"}).to_string(),
        )
        .unwrap();
        // A complete but malformed journal record cannot be trusted.
        let corrupt = write_session(&sessions_dir, "corrupt", json!({}), &prompt);
        fs::write(corrupt.join(JOURNAL), "{\"type\":\"projection_delta\",\n").unwrap();
        // A missing pooled chunk leaves the history unknown.
        let unpooled = write_session(&sessions_dir, "unpooled", json!({}), &prompt);
        fs::remove_dir_all(unpooled.join("chunks")).unwrap();
        // A newly created folder without CURRENT is not a session yet, and a
        // subagent child was never indexed in the first place.
        fs::create_dir_all(sessions_dir.join("unified/pending")).unwrap();
        write_session(
            &sessions_dir,
            "child",
            json!({"identity": {"kind": "subagent"}}),
            &prompt,
        );

        let mut known = KnownSessions::new();
        for id in ["rotating", "corrupt", "unpooled", "child"] {
            known.insert(("vibe".to_string(), id.to_string()), 0.0);
        }

        let scan = adapter(&sessions_dir).find_sessions_incremental(&known);

        assert!(scan.new_or_modified.is_empty());
        assert_eq!(scan.deleted_ids, ["child"]);
    }

    #[test]
    fn unlistable_unified_store_keeps_sessions_and_legacy_updates() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        let legacy_dir = sessions_dir.join("session_legacy");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(
            legacy_dir.join("meta.json"),
            json!({"session_id": "legacy"}).to_string(),
        )
        .unwrap();
        fs::write(
            legacy_dir.join("messages.jsonl"),
            json!({"role": "user", "content": "Legacy prompt"}).to_string(),
        )
        .unwrap();
        fs::write(sessions_dir.join("unified"), "not a directory").unwrap();
        let mut known = KnownSessions::new();
        known.insert(("vibe".to_string(), "unified-1".to_string()), 1.0);

        let scan = adapter(&sessions_dir).find_sessions_incremental(&known);

        assert_eq!(scan.new_or_modified.len(), 1);
        assert_eq!(scan.new_or_modified[0].id, "legacy");
        assert!(scan.deleted_ids.is_empty());
    }
}
