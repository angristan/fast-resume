<p align="center">
  <img src="assets/logo.png" alt="fast-resume" width="120" height="120">
</p>

# fast-resume

Search and resume conversations across Claude Code, Codex, and more, all from a single place.

## Why fast-resume?

Coding agents are really good right now, so I'm using a bunch of them. Sometimes I remember I, or the LLM, mentioned something specific in a previous session, and I want to go back to it.

The problem is that currently, agents do have a resume feature, but either they don't support searching, or the search is very basic (e.g., title only).

That's why I built `fast-resume`: a command-line tool that aggregates all your coding agent sessions into a single searchable index, so you can quickly find and resume any session.

![demo](https://github.com/user-attachments/assets/5ea9c2a5-a7c0-41bf-9357-394aeaaa0a06)

## Features

- **Unified Search**: One search box to find sessions across all your coding agents
- **Full-Text Search**: Search not just titles, but the entire conversation content (user messages and assistant responses)
- **Very fast**: Built on the Rust-powered Tantivy search engine for blazing-fast indexing and searching
- **Fuzzy Matching**: Typo-tolerant search with smart ranking (exact matches boosted)
- **Direct Resume**: Select, Enter, you're back in your session
- **Beautiful TUI**: fzf-style interface with agent icons, color-coded results, and live preview
- **Multi-Agent Support**: Works with Claude Code, Codex, Copilot, OpenCode, Vibe, Crush, and more

## Installation

### Recommended Terminal

For the best experience, [Ghostty 👻](https://ghostty.org/) is recommended. Other terminals may have issues with interactive features and displaying images.

### Homebrew

```bash
brew tap angristan/tap
brew install fast-resume
```

### Cargo

```bash
# Install from source
cargo install --git https://github.com/angristan/fast-resume
fr
```

## Usage

### Interactive TUI

```bash
# Open the TUI with all sessions
fr

# Pre-filter search query
fr "authentication bug"

# Filter by agent
fr -a claude
fr -a codex

# Filter by directory
fr -d myproject

# Combine filters
fr -a claude -d backend "api error"
```

### Keyword Search Syntax

Filter directly in the search box using keywords:

```bash
agent:claude             # Filter by agent
agent:claude,codex       # Multiple agents (OR)
-agent:vibe              # Exclude agent
agent:claude,!codex      # Include claude, exclude codex

dir:myproject            # Filter by directory (substring)
dir:backend,!test        # Include backend, exclude test

date:today               # Sessions from today
date:yesterday           # Sessions from yesterday
date:<1h                 # Within the last hour
date:<2d                 # Within the last 2 days
date:>1w                 # Older than 1 week
date:week                # Within the last week
date:month               # Within the last month
```

Combine keywords with free-text search:

```bash
fr "agent:claude date:<1d api bug"
fr "dir:backend -agent:vibe auth"
```

**Autocomplete**: Type `agent:cl` and press `Tab` to complete to `agent:claude`.

### Non-Interactive Mode

```bash
# List sessions in terminal (no TUI)
fr --no-tui

# Just list, don't offer to resume
fr --list

# Force rebuild the index
fr --rebuild

# View your usage statistics
fr --stats
```

### Yolo Mode

Resume sessions with auto-approve / skip-permissions flags:

| Agent           | Flag Added                                   | Auto-detected |
| --------------- | -------------------------------------------- | ------------- |
| Claude          | `--dangerously-skip-permissions`             | No            |
| Codex           | `--dangerously-bypass-approvals-and-sandbox` | Yes           |
| Copilot CLI     | `--yolo`                                     | No            |
| Vibe            | `--auto-approve`                             | Yes           |
| OpenCode        | _(config-based)_                             | —             |
| Crush           | `--yolo`                                     | No            |
| VS Code Copilot | _(n/a)_                                      | —             |

**Auto-detection:** Codex and Vibe store the permissions mode in their session files. Sessions originally started in yolo mode are automatically resumed in yolo mode.

**Interactive prompt:** For agents that support yolo but don't store it (Claude, Copilot CLI, Crush), you'll see a modal asking whether to resume in yolo mode. Use Tab to toggle, Enter to confirm.

**Force yolo:** Use `fr --yolo` to skip the prompt and always resume in yolo mode, if supported.

### Command Reference

```
Usage: fr [OPTIONS] [QUERY]

Arguments:
  QUERY                    Search query (optional)

Options:
  -a, --agent [claude|codex|copilot-cli|copilot-vscode|crush|opencode|vibe]
                          Filter by agent
  -d, --directory TEXT    Filter by directory (substring match)
  --no-tui                Output list to stdout instead of TUI
  --list                  Just list sessions, don't resume
  --rebuild               Force rebuild the session index
  --stats                 Show index statistics
  --yolo                  Resume with auto-approve/skip-permissions flags
  --version               Show version
  --help                  Show this message and exit
```

## Keybindings

### Navigation

| Key                     | Action                  |
| ----------------------- | ----------------------- |
| `↑` / `↓`               | Move selection up/down  |
| `Ctrl+J` / `Ctrl+K`     | Move selection up/down  |
| `Page Up` / `Page Down` | Move by 10 rows         |
| `Enter`                 | Resume selected session |

### Preview & Actions

| Key                 | Action                               |
| ------------------- | ------------------------------------ |
| `Ctrl+P`            | Toggle preview pane                  |
| `Alt`+`+` / `Alt`+`-` | Scroll preview pane                  |
| Mouse wheel         | Scroll list or preview under pointer |
| `Tab` / `Shift+Tab` | Accept suggestion or cycle agent filter |
| `Ctrl+Y`            | Copy full resume command to clipboard |
| `Esc` / `Ctrl+C`    | Quit                                 |

### Yolo Mode Modal

| Key             | Action            |
| --------------- | ----------------- |
| `Tab` / `←` `→` | Toggle selection  |
| `Enter`         | Confirm selection |
| `y`             | Select Yolo       |
| `n`             | Select No         |
| `Esc`           | Cancel            |

## Statistics Dashboard

Run `fr --stats` to see analytics about your coding sessions:

```
Index Statistics

  Total sessions          751
  Total messages          13,799
  Avg messages/session    18.4
  Index size              15.5 MB
  Index location          ~/.cache/fast-resume/tantivy_index
  Date range              2023-11-15 to 2025-12-22

Data by Agent

┌────────────────┬───────┬──────────┬──────────┬──────────┬──────────┬─────────────┐
│ Agent          │ Files │     Disk │ Sessions │ Messages │  Content │ Data Dir    │
├────────────────┼───────┼──────────┼──────────┼──────────┼──────────┼─────────────┤
│ claude         │   477 │ 312.9 MB │      377 │   10,415 │   3.1 MB │ ~/.claude/… │
│ copilot-vscode │   191 │ 146.0 MB │      189 │      954 │   1.4 MB │ ~/Library/… │
│ codex          │   107 │  23.6 MB │       89 │      321 │ 890.6 kB │ ~/.codex/…  │
│ opencode       │  9275 │  46.3 MB │       72 │    1,912 │ 597.7 kB │ ~/.local/…  │
│ vibe           │    12 │ 858.2 kB │       12 │      138 │ 380.0 kB │ ~/.vibe/…   │
│ crush          │     3 │   1.0 MB │        7 │       44 │  15.2 kB │ ~/.local/…  │
│ copilot-cli    │     5 │ 417.1 kB │        5 │       15 │   6.9 kB │ ~/.copilot… │
└────────────────┴───────┴──────────┴──────────┴──────────┴──────────┴─────────────┘

Activity by Day

 Mon   ██████████              89
 Tue   ██████████              86
 Wed   █████                   44
 Thu   ██████████████         115
 Fri   █████████████          112
 Sat   ████████████████████   163
 Sun   █████████████████      142

Activity by Hour

  0h ▄▁        ▄▄▅▂▂▂▂▂▃▃▃▅▅█ 23h
  Peak hours: 23:00 (99), 22:00 (63), 12:00 (63)

Top Directories

┌───────────────────────┬──────────┬──────────┐
│ Directory             │ Sessions │ Messages │
├───────────────────────┼──────────┼──────────┤
│ ~/git/openvpn-install │      234 │    5,597 │
│ ~/lab/larafeed        │      158 │    2,590 │
│ ~/lab/fast-resume     │       81 │    2,027 │
│ ...                   │          │          │
└───────────────────────┴──────────┴──────────┘
```

## How It Works

### Architecture

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                                 SessionSearch                                          │
│                                                                                        │
│   • Orchestrates adapters concurrently                                                  │
│   • Compares file mtimes to detect changes (incremental updates)                       │
│   • Delegates search queries to Tantivy index                                          │
└────────────────────────────────────────────────────────────────────────────────────────┘
                      │                                       │
         ┌────────────┴────────────┐                          │
         ▼                         ▼                          ▼
┌──────────────────┐    ┌───────────────────────────────────────────────────────────────────────────────┐
│  TantivyIndex    │    │                                 Adapters                                       │
│                  │    │  ┌────────┐ ┌───────┐ ┌───────┐ ┌─────────┐ ┌───────┐ ┌────────┐ ┌────┐        │
│ • Fuzzy search   │◄───│  │ Claude │ │ Codex │ │Copilot│ │ Copilot │ │ Crush │ │OpenCode│ │Vibe│        │
│ • mtime tracking │    │  │        │ │       │ │  CLI  │ │ VS Code │ │       │ │        │ │    │        │
│                  │    │  └───┬────┘ └───┬───┘ └───┬───┘ └────┬────┘ └───┬───┘ └───┬────┘ └─┬──┘        │
│ ~/.cache/        │    │      │          │         │          │          │         │        │           │
│   fast-resume/   │    └──────┼──────────┼─────────┼──────────┼──────────┼─────────┼────────┼───────────┘
└──────────────────┘           ▼          ▼         ▼          ▼          ▼         ▼        ▼
                          ~/.claude/ ~/.codex/ ~/.copilot/  VS Code/   crush.db opencode/ ~/.vibe/
```

### Session Parsing

Each agent stores sessions differently. Adapters normalize them into a common `Session` structure:

| Agent          | Format                                               | Parsing Strategy                                                                            |
| -------------- | ---------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| Claude Code    | JSONL in `~/.claude/projects/<project>/*.jsonl`      | Stream line-by-line, extract `user`/`assistant` messages, skip `agent-*` subprocess files   |
| Codex          | JSONL in `~/.codex/sessions/**/*.jsonl`              | Line-by-line parsing, extract from `session_meta`, `response_item`, and `event_msg` entries |
| Copilot CLI    | JSONL in `~/.copilot/session-state/*.jsonl`          | Line-by-line parsing, extract `user.message` and `assistant.message` types                  |
| Copilot VSCode | JSON in VS Code's `workspaceStorage/*/chatSessions/` | Parse `requests` array with message text and response values                                |
| Crush          | SQLite DB at `<project>/crush.db`                    | Query `sessions` and `messages` tables directly, parse JSON `parts` column                  |
| OpenCode       | Split JSON in `~/.local/share/opencode/storage/`     | Lazy-load `message/` and `part/` per session for progressive indexing                       |
| Vibe           | JSON in `~/.vibe/logs/session/session_*.json`        | Parse `messages` array with role-based content                                              |

**The normalized Session structure:**

```rust
pub struct Session {
    pub id: String,
    pub agent: String,
    pub title: String,
    pub directory: String,
    pub timestamp: DateTime<Local>,
    pub content: String,
    pub message_count: usize,
    pub mtime: f64,
    pub yolo: bool,
}
```

**What gets indexed:**

- User text messages (the actual prompts you typed)
- Assistant text responses

**What's excluded from indexing:**

- Tool results (file contents, command outputs, API responses)
- Tool use/calls (function invocations)
- Meta messages (system prompts, context summaries)
- Local command outputs (slash commands like `/context`)

This keeps the index focused on the actual conversation and avoids bloating it with large tool outputs that are rarely useful for search.

### Indexing

**Incremental updates** avoid re-parsing on every launch:

1. Load known sessions from Tantivy index with their `mtime` values
2. Scan session files, compare mtimes against known values
3. Only parse files where `current_mtime > known_mtime + 0.001`
4. Detect deleted sessions (in index but not on disk)
5. Apply changes atomically: delete removed, upsert modified

**Progressive indexing** with batched commits:

```rust
index.refresh_incremental_streaming(BATCH_SIZE, |summary| {
    scan_tx.send(ScanMessage::Progress {
        new_or_modified: summary.new_or_modified,
        deleted: summary.deleted,
        total: summary.sessions,
        elapsed: start.elapsed(),
    })?;
});
```

Sessions appear in the TUI progressively as they're parsed and batched. OpenCode uses parallel file I/O and processes smaller sessions first for faster initial results.

**Schema versioning**: A `.schema_version` file tracks the index schema. If it doesn't match the code's `SCHEMA_VERSION` constant, the entire index is deleted and rebuilt. This prevents deserialization errors after upgrades.

### Search

[Tantivy](https://github.com/quickwit-oss/tantivy) is a Rust full-text search library (powers Quickwit, similar to Lucene).

**Hybrid search** combines exact and fuzzy matching for best results:

```rust
// Exact terms are boosted, fuzzy terms provide typo tolerance.
let exact = QueryParser::for_index(&index, vec![title, content]).parse_query(query)?;
let fuzzy = fuzzy_term_queries(schema, query);
let combined = BooleanQuery::new(vec![
    (Occur::Should, Box::new(BoostQuery::new(Box::new(exact), 5.0))),
    (Occur::Should, Box::new(fuzzy)),
]);
```

This ensures exact matches rank first while still finding typos like `auth midleware` → "authentication middleware".

**Query lifecycle:**

```
┌─────────────┐   50ms    ┌─────────────┐  background  ┌─────────────┐
│  Keystroke  │ ────────► │  Debounce   │ ───────────► │   Worker    │
└─────────────┘  timer    └─────────────┘   thread     └──────┬──────┘
                                                              │
                          ┌─────────────┐              ┌──────▼──────┐
                          │   Render    │ ◄─────────── │   Tantivy   │
                          │   Table     │   results    │    Query    │
                          └─────────────┘              └─────────────┘
```

### TUI

**Streaming results**: Sessions appear as each adapter completes, not after all finish.

- **Fast path**: Index up-to-date → load synchronously, no spinner
- **Slow path**: Changes detected → spinner, stream results via `on_progress()` callback

**Preview context**: When searching, the preview pane jumps to the matching portion:

```rust
for term in query.to_lowercase().split_whitespace() {
    if let Some(pos) = content.to_lowercase().find(term) {
        let start = pos.saturating_sub(100);
        let preview = content[start..].chars().take(1500).collect::<String>();
        break;
    }
}
```

Matching terms are highlighted directly in the terminal UI.

### Resume Handoff

When you press Enter on a session, fast-resume hands off to the original agent:

```rust
match run_tui(query, agent_filter, yolo, image_protocol)? {
    TuiExit::Quit => Ok(()),
    TuiExit::Resume { command, directory } => exec_resume(command, directory),
}
```

`exec()` replaces the fast-resume process entirely with the agent CLI. This means:

- No subprocess overhead
- Shell history shows `claude --resume xyz`, not `fr`
- Agent inherits the correct working directory
- fast-resume process is gone after handoff

Each adapter returns the appropriate command:

| Agent          | Resume Command                  | With `--yolo`                                                  |
| -------------- | ------------------------------- | -------------------------------------------------------------- |
| Claude         | `claude --resume <id>`          | `claude --dangerously-skip-permissions --resume <id>`          |
| Codex          | `codex resume <id>`             | `codex --dangerously-bypass-approvals-and-sandbox resume <id>` |
| Copilot CLI    | `copilot --resume <id>`         | `copilot --yolo --resume <id>`                                 |
| Copilot VSCode | `code <directory>`              | _(no change)_                                                  |
| OpenCode       | `opencode <dir> --session <id>` | _(no change)_                                                  |
| Vibe           | `vibe --resume <id>`            | `vibe --auto-approve --resume <id>`                            |
| Crush          | `crush --session <id>`          | `crush --yolo --session <id>`                                  |

### Performance

Why fast-resume feels instant:

- **Tantivy**: Native Rust full-text search over 10k+ sessions in <10ms
- **Incremental updates**: Only re-parse files where `mtime` changed. Second launch with no changes: ~50ms total
- **Parallel adapters**: Adapters run concurrently. Total time = slowest adapter, not sum
- **Debounced search**: 50ms debounce prevents wasteful searches while typing
- **Background workers**: Index refresh runs off the UI thread
- **Streaming results**: Sessions appear as each adapter completes, not after all finish

Typical performance on a machine with ~500 sessions:

- Cold start (empty index): ~2s
- Warm start (no changes): ~50ms
- Search query: <10ms

## Development

```bash
# Clone and setup
git clone https://github.com/angristan/fast-resume.git
cd fast-resume

# Run locally
cargo run --

# Install pre-commit hooks
pre-commit install

# Run tests
cargo test

# Lint and format
cargo fmt --check
cargo check --all-targets --locked
```

### Project Structure

```
fast-resume/
├── src/
│   ├── main.rs             # Clap CLI entry point
│   ├── config.rs           # Constants, colors, paths
│   ├── index.rs            # Tantivy index and incremental refresh
│   ├── search.rs           # Search engine facade
│   ├── tui.rs              # Ratatui terminal UI
│   ├── adapters/           # Agent-specific parsers and resume commands
│   └── model.rs            # Shared session model
├── assets/                 # Logo and embedded agent icons
├── Cargo.toml              # Dependencies and build config
└── README.md
```

### Tech Stack

| Component           | Library                                                             |
| ------------------- | ------------------------------------------------------------------- |
| TUI Framework       | [Ratatui](https://ratatui.rs/)                                      |
| Terminal Handling   | [Crossterm](https://github.com/crossterm-rs/crossterm)              |
| CLI Framework       | [Clap](https://docs.rs/clap/latest/clap/)                           |
| Search Engine       | [Tantivy](https://github.com/quickwit-oss/tantivy)                  |
| JSON Parsing        | [serde_json](https://docs.rs/serde_json/latest/serde_json/)         |
| SQLite              | [rusqlite](https://docs.rs/rusqlite/latest/rusqlite/)               |

## Configuration

fast-resume uses sensible defaults and requires no configuration.

To clear the index and rebuild from scratch:

```bash
rm -rf ~/.cache/fast-resume/
fr --rebuild
```

## License

MIT
