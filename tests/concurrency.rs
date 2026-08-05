use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use fs4::fs_std::FileExt;
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
    fs::write(
        &session_file,
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    session_file
}

#[test]
fn parallel_list_calls_wait_for_the_index_writer() {
    let temp = TempDir::new().unwrap();
    write_codex_session(
        temp.path(),
        "parallel123",
        "/repo/parallel",
        "Concurrent index writer coverage",
    );

    let cache_dir = temp.path().join(".cache/fast-resume");
    fs::create_dir_all(&cache_dir).unwrap();
    let lock_path = cache_dir.join("tantivy_index.write.lock");
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .unwrap();
    lock_file.lock_exclusive().unwrap();

    let mut children: Vec<_> = (0..8)
        .map(|_| {
            Command::new(env!("CARGO_BIN_EXE_fr"))
                .arg("--list")
                .arg("Concurrent index writer")
                .env_clear()
                .env("HOME", temp.path())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();

    let index_meta = cache_dir.join("tantivy_index/meta.json");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !index_meta.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        index_meta.exists(),
        "parallel calls did not initialize the index"
    );
    FileExt::unlock(&lock_file).unwrap();

    for child in children.drain(..) {
        let output = child.wait_with_output().unwrap();
        let (stdout, stderr) = assert_success(output);
        assert!(stderr.is_empty());
        assert!(stdout.contains("parallel123"));
        assert!(stdout.contains("Showing 1 of 1 sessions"));
    }
}

#[test]
fn list_uses_committed_snapshot_while_refresh_is_busy() {
    let temp = TempDir::new().unwrap();
    write_codex_session(
        temp.path(),
        "committed123",
        "/repo/parallel",
        "Committed session",
    );
    assert_success(run_fr(temp.path(), &["--list"]));

    write_codex_session(
        temp.path(),
        "pending123",
        "/repo/parallel",
        "Pending session",
    );
    let lock_path = temp
        .path()
        .join(".cache/fast-resume/tantivy_index.write.lock");
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .unwrap();
    lock_file.lock_exclusive().unwrap();

    let started = Instant::now();
    let (busy_stdout, busy_stderr) = assert_success(run_fr(temp.path(), &["--list"]));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(busy_stderr.is_empty());
    assert!(busy_stdout.contains("committed123"));
    assert!(!busy_stdout.contains("pending123"));

    FileExt::unlock(&lock_file).unwrap();
    let (refreshed_stdout, refreshed_stderr) = assert_success(run_fr(temp.path(), &["--list"]));
    assert!(refreshed_stderr.is_empty());
    assert!(refreshed_stdout.contains("committed123"));
    assert!(refreshed_stdout.contains("pending123"));
}
