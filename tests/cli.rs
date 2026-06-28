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

    let (query_stdout, _) = assert_success(run_fr(temp.path(), &["--list", "Needle"]));
    assert!(query_stdout.contains("backend123"));
    assert!(!query_stdout.contains("frontend123"));
    assert!(!query_stdout.contains("claude123"));
    assert!(query_stdout.contains("Showing 1 of 1 sessions"));

    let (directory_stdout, _) = assert_success(run_fr(temp.path(), &["--list", "-d", "backend"]));
    assert!(directory_stdout.contains("backend123"));
    assert!(directory_stdout.contains("claude123"));
    assert!(!directory_stdout.contains("frontend123"));
    assert!(directory_stdout.contains("Showing 2 of 2 sessions"));

    let (agent_stdout, _) = assert_success(run_fr(temp.path(), &["--list", "-a", "claude"]));
    assert!(agent_stdout.contains("claude123"));
    assert!(!agent_stdout.contains("backend123"));
    assert!(!agent_stdout.contains("frontend123"));
    assert!(agent_stdout.contains("Showing 1 of 1 sessions"));
}
