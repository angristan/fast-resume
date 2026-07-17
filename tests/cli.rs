use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use tempfile::TempDir;

fn run_fr(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fr"))
        .args(args)
        .env_clear()
        .env("HOME", home)
        .output()
        .unwrap()
}

fn assert_success(output: Output) -> (String, String) {
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        output.status.success(),
        "fr failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout,
        stderr
    );
    (stdout, stderr)
}

fn write_codex_session(home: &Path, id: &str, directory: &str, prompt: &str) -> PathBuf {
    let session_dir = home.join(".codex/sessions/2026/06/28");
    fs::create_dir_all(&session_dir).unwrap();
    let session_file = session_dir.join(format!("rollout-2026-06-28T12-00-00-{id}.jsonl"));
    let rows = [
        json!({"type": "session_meta", "payload": {"id": id, "cwd": directory}}),
        json!({"type": "event_msg", "payload": {"type": "user_message", "message": prompt}}),
        json!({"type": "response_item", "payload": {"role": "assistant", "content": [{"text": "Done"}]}}),
    ];
    write_jsonl(&session_file, &rows);
    session_file
}

fn write_claude_session(home: &Path, id: &str, directory: &str, prompt: &str) -> PathBuf {
    let session_dir = home.join(".claude/projects/project");
    fs::create_dir_all(&session_dir).unwrap();
    let session_file = session_dir.join(format!("{id}.jsonl"));
    let rows = [
        json!({"type": "user", "cwd": directory, "message": {"content": prompt}}),
        json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "Done"}]}}),
    ];
    write_jsonl(&session_file, &rows);
    session_file
}

fn write_pi_session(home: &Path, id: &str, directory: &str, prompt: &str) -> PathBuf {
    let session_dir = home.join(".pi/agent/sessions/--repo-pi--");
    fs::create_dir_all(&session_dir).unwrap();
    let session_file = session_dir.join(format!("2026-07-15T10-00-00-000Z_{id}.jsonl"));
    let rows = [
        json!({"type": "session", "version": 3, "id": id, "timestamp": "2026-07-15T10:00:00.000Z", "cwd": directory}),
        json!({"type": "message", "id": "a1", "parentId": null, "timestamp": "2026-07-15T10:00:01.000Z", "message": {"role": "user", "content": prompt}}),
        json!({"type": "message", "id": "a2", "parentId": "a1", "timestamp": "2026-07-15T10:00:02.000Z", "message": {"role": "assistant", "content": [{"type": "text", "text": "Done"}]}}),
    ];
    write_jsonl(&session_file, &rows);
    session_file
}

fn write_new_agent_sessions(home: &Path) {
    let gemini_project = home.join(".gemini/tmp/project");
    let gemini_chats = gemini_project.join("chats");
    fs::create_dir_all(&gemini_chats).unwrap();
    fs::write(gemini_project.join(".project_root"), "/repo/gemini").unwrap();
    write_jsonl(
        &gemini_chats.join("session-gemini.jsonl"),
        &[
            json!({"sessionId":"gemini123","projectHash":"project","startTime":"2026-07-17T10:00:00Z"}),
            json!({"id":"u1","type":"user","content":"Gemini binary coverage"}),
        ],
    );

    let antigravity_id = "52d82992-7695-4d38-8d02-9747eecba839";
    let antigravity = home
        .join(".gemini/antigravity-cli/brain")
        .join(antigravity_id)
        .join(".system_generated/logs");
    fs::create_dir_all(&antigravity).unwrap();
    write_jsonl(
        &antigravity.join("transcript.jsonl"),
        &[
            json!({"source":"USER_EXPLICIT","type":"USER_INPUT","content":"<USER_REQUEST>Antigravity binary coverage</USER_REQUEST>"}),
        ],
    );

    let grok_id = "019edf9c-0000-7000-8000-000000000001";
    let grok = home.join(".grok/sessions/%2Frepo%2Fgrok").join(grok_id);
    fs::create_dir_all(&grok).unwrap();
    fs::write(
        grok.join("summary.json"),
        json!({"info":{"id":grok_id,"cwd":"/repo/grok"},"created_at":"2026-07-17T10:00:00Z"})
            .to_string(),
    )
    .unwrap();
    write_jsonl(
        &grok.join("updates.jsonl"),
        &[
            json!({"params":{"update":{"sessionUpdate":"user_message_chunk","content":{"text":"Grok binary coverage"}}}}),
        ],
    );

    let cursor = home.join(".cursor/chats/%2Frepo%2Fcursor/cursor123");
    fs::create_dir_all(&cursor).unwrap();
    let connection = rusqlite::Connection::open(cursor.join("store.db")).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE meta (key TEXT, value BLOB); CREATE TABLE blobs (key TEXT, value BLOB);",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO blobs (key, value) VALUES (?1, ?2)",
            (
                "u1",
                json!({"role":"user","content":"Cursor binary coverage"}).to_string(),
            ),
        )
        .unwrap();
}

fn write_jsonl(path: &Path, rows: &[Value]) {
    fs::write(
        path,
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
}

#[test]
fn list_stats_and_rebuild_work_through_the_binary() {
    let temp = TempDir::new().unwrap();
    write_codex_session(
        temp.path(),
        "abc123",
        "/repo/backend",
        "Review binary CLI coverage",
    );

    let (list_stdout, list_stderr) = assert_success(run_fr(temp.path(), &["--list"]));
    assert!(list_stderr.is_empty());
    assert!(list_stdout.contains("Agent"));
    assert!(list_stdout.contains("Title"));
    assert!(list_stdout.contains("Directory"));
    assert!(list_stdout.contains("ID"));
    assert!(list_stdout.contains("codex"));
    assert!(list_stdout.contains("Review binary CLI coverage"));
    assert!(list_stdout.contains("/repo/backend"));
    assert!(list_stdout.contains("abc123"));
    assert!(list_stdout.contains("Showing 1 of 1 sessions"));

    let (stats_stdout, stats_stderr) = assert_success(run_fr(temp.path(), &["--stats"]));
    assert!(stats_stderr.is_empty());
    assert!(stats_stdout.contains("Index Statistics"));
    assert!(stats_stdout.contains("Total sessions          1"));
    assert!(stats_stdout.contains("Data by Agent"));
    assert!(stats_stdout.contains("codex"));

    let (rebuild_stdout, rebuild_stderr) =
        assert_success(run_fr(temp.path(), &["--rebuild", "--list"]));
    assert!(rebuild_stdout.contains("Review binary CLI coverage"));
    assert!(rebuild_stdout.contains("Showing 1 of 1 sessions"));
    assert!(rebuild_stderr.contains("Indexed 1 sessions"));
}

#[test]
fn list_removes_stale_sessions_on_incremental_refresh() {
    let temp = TempDir::new().unwrap();
    let session_file = write_codex_session(
        temp.path(),
        "gone123",
        "/repo/backend",
        "Delete stale indexed session",
    );

    let (first_stdout, _) = assert_success(run_fr(temp.path(), &["--list"]));
    assert!(first_stdout.contains("gone123"));

    fs::remove_file(session_file).unwrap();

    let (second_stdout, second_stderr) = assert_success(run_fr(temp.path(), &["--list"]));
    assert!(second_stderr.is_empty());
    assert!(second_stdout.contains("No sessions found."));
    assert!(!second_stdout.contains("gone123"));
}

#[test]
fn stats_reports_empty_index_friendly_message() {
    let temp = TempDir::new().unwrap();

    let (stdout, stderr) = assert_success(run_fr(temp.path(), &["--stats"]));

    assert!(stderr.is_empty());
    assert!(stdout.contains("No sessions indexed."));
    assert!(!stdout.contains("Index Statistics"));
}

#[test]
fn list_footer_counts_filtered_matches() {
    let temp = TempDir::new().unwrap();
    write_codex_session(
        temp.path(),
        "backend123",
        "/repo/backend",
        "Needle backend investigation",
    );
    write_codex_session(
        temp.path(),
        "frontend123",
        "/repo/frontend",
        "Frontend polish session",
    );
    write_claude_session(
        temp.path(),
        "claude123",
        "/repo/backend",
        "Claude backend architecture review",
    );
    write_pi_session(
        temp.path(),
        "pi123",
        "/repo/pi",
        "Pi adapter integration coverage",
    );

    let (query_stdout, _) = assert_success(run_fr(temp.path(), &["--list", "Needle"]));
    assert!(query_stdout.contains("backend123"));
    assert!(!query_stdout.contains("frontend123"));
    assert!(!query_stdout.contains("claude123"));
    assert!(!query_stdout.contains("pi123"));
    assert!(query_stdout.contains("Showing 1 of 1 sessions"));

    let (directory_stdout, _) = assert_success(run_fr(temp.path(), &["--list", "-d", "backend"]));
    assert!(directory_stdout.contains("backend123"));
    assert!(directory_stdout.contains("claude123"));
    assert!(!directory_stdout.contains("frontend123"));
    assert!(!directory_stdout.contains("pi123"));
    assert!(directory_stdout.contains("Showing 2 of 2 sessions"));

    let (agent_stdout, _) = assert_success(run_fr(temp.path(), &["--list", "-a", "claude"]));
    assert!(agent_stdout.contains("claude123"));
    assert!(!agent_stdout.contains("backend123"));
    assert!(!agent_stdout.contains("frontend123"));
    assert!(!agent_stdout.contains("pi123"));
    assert!(agent_stdout.contains("Showing 1 of 1 sessions"));

    let (pi_stdout, _) = assert_success(run_fr(temp.path(), &["--list", "agent:pi"]));
    assert!(pi_stdout.contains("pi123"));
    assert!(pi_stdout.contains("Pi adapter integration coverage"));
    assert!(!pi_stdout.contains("backend123"));
    assert!(pi_stdout.contains("Showing 1 of 1 sessions"));
}

#[test]
fn lists_gemini_antigravity_cursor_and_grok_sessions() {
    let temp = TempDir::new().unwrap();
    write_new_agent_sessions(temp.path());

    let (stdout, stderr) = assert_success(run_fr(temp.path(), &["--list"]));

    assert!(stderr.is_empty());
    for expected in [
        "antigravity",
        "Antigravity binary coverage",
        "cursor",
        "Cursor binary coverage",
        "gemini",
        "Gemini binary coverage",
        "grok",
        "Grok binary coverage",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected} in:\n{stdout}"
        );
    }
    assert!(stdout.contains("Showing 4 of 4 sessions"));
}
