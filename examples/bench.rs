use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use fast_resume::index::SessionIndex;
use fast_resume::search::SearchEngine;
use serde_json::json;
use walkdir::WalkDir;

const WORDS: [&str; 12] = [
    "authentication",
    "token",
    "login",
    "backend",
    "database",
    "deploy",
    "refactor",
    "websocket",
    "pagination",
    "cache",
    "parser",
    "renderer",
];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn word(&mut self) -> &'static str {
        WORDS[(self.next() % WORDS.len() as u64) as usize]
    }
}

fn message_text(rng: &mut Rng, seed: usize) -> String {
    let mut text = format!("note {seed} about {}", rng.word());
    while text.len() < 480 {
        text.push(' ');
        if rng.next().is_multiple_of(4) {
            text.push_str(rng.word());
        } else {
            let len = 4 + (rng.next() % 9) as usize;
            for _ in 0..len {
                text.push((b'a' + (rng.next() % 26) as u8) as char);
            }
        }
    }
    text
}

fn write_claude_session(home: &Path, id: &str, project: &str, seed: usize) {
    let dir = home
        .join(".claude/projects")
        .join(format!("proj-{}", seed % 10));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{id}.jsonl"));
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ seed as u64);
    let mut rows = Vec::new();
    let cwd = format!("/work/proj-{}", seed % 10);
    for i in 0..60 {
        let text = message_text(&mut rng, seed * 1000 + i);
        if i % 2 == 0 {
            rows.push(json!({
                "type": "user",
                "cwd": cwd,
                "message": {"content": text},
            }));
        } else {
            rows.push(json!({
                "type": "assistant",
                "message": {"content": [{"type": "text", "text": text}]},
            }));
        }
    }
    let body: String = rows
        .iter()
        .map(|row| row.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(path, body).unwrap();
    let _ = project;
}

fn write_codex_session(home: &Path, id: &str, seed: usize) {
    let dir = home.join(".codex/sessions/2026/06/28");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("rollout-2026-06-28T12-00-00-{id}.jsonl"));
    let mut rng = Rng(0x2545_f491_4f6c_dd1d ^ (seed as u64) << 1);
    let mut rows = vec![json!({
        "type": "session_meta",
        "payload": {"id": id, "cwd": format!("/work/codex-{}", seed % 7)},
    })];
    for i in 0..60 {
        let text = message_text(&mut rng, seed * 1000 + i + 7);
        if i % 2 == 0 {
            rows.push(json!({
                "type": "event_msg",
                "payload": {"type": "user_message", "message": text},
            }));
        } else {
            rows.push(json!({
                "type": "response_item",
                "payload": {"role": "assistant", "content": [{"text": text}]},
            }));
        }
    }
    let body: String = rows
        .iter()
        .map(|row| row.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(path, body).unwrap();
}

fn uuid(seed: usize) -> String {
    format!(
        "{:08x}-{:04x}-4{:03x}-a{:03x}-{:012x}",
        seed,
        seed % 65536,
        seed % 4096,
        seed % 4096,
        seed as u64 * 2654435761 % 0xffffffffffff
    )
}

fn generate(home: &Path, sessions: usize) {
    let half = sessions / 2;
    for i in 0..half {
        write_claude_session(home, &uuid(i), "", i);
        write_codex_session(home, &uuid(1_000_000 + i), i);
    }
    let total: u64 = WalkDir::new(home)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.metadata().unwrap().len())
        .sum();
    println!(
        "generated {sessions} sessions, {total} bytes under {}",
        home.display()
    );
}

fn touch_some(home: &Path, count: usize) {
    let dir = home.join(".claude/projects");
    let mut files: Vec<PathBuf> = WalkDir::new(&dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && e.path().extension().is_some_and(|e| e == "jsonl"))
        .map(|e| e.path().to_path_buf())
        .collect();
    files.sort();
    std::thread::sleep(std::time::Duration::from_millis(20));
    for path in files
        .iter()
        .step_by(files.len().max(1) / count.max(1))
        .take(count)
    {
        let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
        use std::io::Write;
        write!(
            file,
            "\n{}",
            json!({"type": "user", "message": {"content": "appended during benchmark change"}})
        )
        .unwrap();
    }
}

fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.metadata().unwrap().len())
        .sum()
}

fn time1(name: &str, f: impl FnOnce() -> usize) {
    let start = Instant::now();
    let n = f();
    println!(
        "BENCH1\t{name}\t{:.2}\tms\t{n}",
        start.elapsed().as_secs_f64() * 1000.0
    );
}

fn bench_search(engine: &SearchEngine, name: &str, query: &str, iters: usize) {
    for _ in 0..3 {
        let _ = engine
            .search_result_with_offset(query, None, None, 0, 100)
            .unwrap();
    }
    let mut times = Vec::with_capacity(iters);
    for _ in 0..iters {
        let start = Instant::now();
        let hits = engine
            .search_result_with_offset(query, None, None, 0, 100)
            .unwrap();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(hits.len());
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
    println!(
        "BENCH\t{name}\tmin={:.2}\tp50={:.2}\tp90={:.2}\tmax={:.2}\tms",
        times[0],
        p(0.5),
        p(0.9),
        times[times.len() - 1]
    );
}

fn run(home: &Path, index_dir: &Path, changed: bool) {
    unsafe {
        env::set_var("HOME", home);
        env::set_var("XDG_CACHE_HOME", home.join(".cache"));
        env::remove_var("GROK_HOME");
        env::remove_var("KIMI_CODE_HOME");
        env::remove_var("PI_CODING_AGENT_SESSION_DIR");
    }
    let iters: usize = env::var("BENCH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15);
    let force_rebuild = env::var("BENCH_REBUILD").is_ok();

    time1("open", || {
        let index = SessionIndex::open(index_dir.to_path_buf()).unwrap();
        std::hint::black_box(index.total_len().unwrap())
    });
    let index = SessionIndex::open(index_dir.to_path_buf()).unwrap();
    let engine = SearchEngine::from_index(index.clone());

    if force_rebuild || engine.total_len() == 0 {
        time1("scan_all_sessions", || {
            SessionIndex::scan_all_sessions().len()
        });
        let sessions = SessionIndex::scan_all_sessions();
        time1("rebuild", || index.rebuild(sessions).unwrap().sessions);
    }

    time1("known_sessions", || index.known_sessions().unwrap().len());
    time1("refresh_no_change", || {
        index.refresh_incremental().unwrap().sessions
    });

    bench_search(&engine, "search_empty", "", iters);
    bench_search(
        &engine,
        "search_exact_authentication",
        "authentication",
        iters,
    );
    bench_search(&engine, "search_fuzzy_typo", "authentcation", iters);
    bench_search(&engine, "search_multiword", "login token backend", iters);
    bench_search(
        &engine,
        "search_dir_filter",
        "dir:proj-3 authentication",
        iters,
    );
    time1("count_fuzzy_typo", || {
        engine.count_result("authentcation", None, None).unwrap()
    });

    time1("all_sessions", || index.all_sessions().unwrap().len());
    time1("stats", || {
        std::hint::black_box(index.stats().unwrap().total_sessions)
    });
    time1("agents_with_sessions", || {
        engine.agents_with_sessions().len()
    });

    println!("BENCH1\tindex_dir_bytes\t{}\tbytes", dir_size(index_dir));

    if changed {
        touch_some(home, 50);
        time1("refresh_50_changed", || {
            index.refresh_incremental().unwrap().new_or_modified
        });
        time1("refresh_no_change_after", || {
            index.refresh_incremental().unwrap().sessions
        });
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let Some(command) = args.get(1).map(String::as_str) else {
        eprintln!("usage: bench <gen|run|changed> <home> <index> [sessions]");
        std::process::exit(2);
    };
    let home = PathBuf::from(args.get(2).unwrap_or_else(|| {
        eprintln!("usage: bench <gen|run|changed> <home> <index> [sessions]");
        std::process::exit(2);
    }));
    let index_dir = PathBuf::from(args.get(3).unwrap_or_else(|| {
        eprintln!("usage: bench <gen|run|changed> <home> <index> [sessions]");
        std::process::exit(2);
    }));
    match command {
        "gen" => {
            let sessions: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(2000);
            generate(&home, sessions);
        }
        "run" => run(&home, &index_dir, false),
        "changed" => run(&home, &index_dir, true),
        _ => {
            eprintln!("unknown command {command}");
            std::process::exit(2);
        }
    }
}
