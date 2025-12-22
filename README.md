# fast-resume

Search and resume conversations across Claude Code, Codex, and more, all from a single place.

## Why fast-resume?

Coding agents are really good right now, so I'm using a bunch of them. Sometimes I remember I, or the LLM, mentioned something specific in a previous session, and I want to go back to it.

The problem is that currently, agents do have a resume feature, but either they don't support searching, or the search is very basic (e.g., title only).

That's why I built `fast-resume`: a command-line tool that aggregates all your coding agent sessions into a single searchable index, so you can quickly find and resume any session.

![demo](https://github.com/user-attachments/assets/752c772e-c23f-4ed6-af3d-add43c7157da)

## Features

- **Unified Search**: One search box to find sessions across all your coding agents
- **Full-Text Search**: Search not just titles, but the entire conversation content (user messages and assistant responses)
- **Very fast**: Built on the Rust-powered Tantivy search engine for blazing-fast indexing and searching
- **Fuzzy Matching**: Typo-tolerant search with smart ranking (exact matches boosted)
- **Direct Resume**: Select, Enter, you're back in your session
- **Beautiful TUI**: fzf-style interface with agent icons, color-coded results, and live preview

## Supported Agents

| Agent               | Data Location                                 | Resume Command                  |
| ------------------- | --------------------------------------------- | ------------------------------- |
| **Claude Code**     | `~/.claude/projects/`                         | `claude --resume <id>`          |
| **Codex CLI**       | `~/.codex/sessions/`                          | `codex resume <id>`             |
| **Copilot CLI**     | `~/.copilot/session-state/`                   | `copilot --resume <id>`         |
| **VS Code Copilot** | `~/Library/Application Support/Code/` (macOS) | `code <directory>`              |
| **Crush**           | `~/.local/share/crush/projects.json`          | _(interactive only)_            |
| **OpenCode**        | `~/.local/share/opencode/storage/`            | `opencode <dir> --session <id>` |
| **Vibe**            | `~/.vibe/logs/session/`                       | `vibe --resume <id>`            |

## Installation

**Requirements:** Python 3.14+

```bash
# Run directly (no install needed)
uvx --from git+https://github.com/angristan/fast-resume fr

# Or install permanently
uv tool install git+https://github.com/angristan/fast-resume

# Then use either command:
fr
fast-resume
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
| Copilot CLI     | `--allow-all-tools --allow-all-paths`        | No            |
| Vibe            | `--auto-approve`                             | Yes           |
| OpenCode        | _(config-based)_                             | —             |
| Crush           | _(no CLI resume)_                            | —             |
| VS Code Copilot | _(n/a)_                                      | —             |

**Auto-detection:** Codex and Vibe store the permissions mode in their session files. Sessions originally started in yolo mode are automatically resumed in yolo mode—no flag needed.

**Force yolo:** Use `fr --yolo` to force yolo mode for all sessions, even if they weren't started that way. Useful for Claude and Copilot CLI which don't store this information.

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

| Key                     | Action                             |
| ----------------------- | ---------------------------------- |
| `↑` / `↓`               | Move selection up/down             |
| `j` / `k`               | Move selection up/down (vim-style) |
| `Page Up` / `Page Down` | Move by 10 rows                    |
| `Enter`                 | Resume selected session            |
| `/`                     | Focus search input                 |

### Preview & Actions

| Key         | Action                                |
| ----------- | ------------------------------------- |
| `Tab`       | Toggle preview pane                   |
| `+` / `-`   | Resize preview pane                   |
| `c`         | Copy full resume command to clipboard |
| `Ctrl+P`    | Open command palette                  |
| `q` / `Esc` | Quit                                  |

## Statistics Dashboard

Run `fr --stats` to see analytics about your coding sessions:

```
Index Statistics

Total sessions    247
Total messages    12,456
Avg messages/session  50.4
Index size        2.1 MB
Index location    ~/.cache/fast-resume/tantivy_index
Date range        2024-08-15 to 2024-12-20

Sessions by Agent

Agent      Sessions
claude          142  ████████████████████ (58%)
codex            45  ██████ (18%)
opencode         32  ████ (13%)
vibe             18  ██ (7%)
copilot           7  █ (3%)
crush             3  ▏ (1%)

Activity by Day

Mon  ████████████████████  89
Tue  ██████████████        62
Wed  ████████████          54
Thu  ██████████            45
Fri  ████████              36
Sat  ██                     8
Sun  █                      4

Activity by Hour

  0h ▁▁▁▁▁▁▁▂▃▅▆▇█▇▆▅▄▃▂▁▁▁▁▁ 23h
  Peak hours: 14:00 (23), 15:00 (21), 13:00 (19)

Top Directories

Directory                            Sessions
~/projects/fast-resume                     45
~/work/api-server                          32
~/projects/my-app                          28
...
```

## How It Works

### Architecture

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                                 SessionSearch                                          │
│                                                                                        │
│   • Orchestrates adapters in parallel (ThreadPoolExecutor)                             │
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
| OpenCode       | Split JSON in `~/.local/share/opencode/storage/`     | Join `session/<hash>/ses_*.json` + `message/<id>/msg_*.json` + `part/<id>/*.json`           |
| Vibe           | JSON in `~/.vibe/logs/session/session_*.json`        | Parse `messages` array with role-based content                                              |

**The normalized Session structure:**

```python
@dataclass
class Session:
    id: str              # Unique identifier (usually filename or UUID)
    agent: str           # "claude", "codex", "copilot-cli", "copilot-vscode", "crush", "opencode", "vibe"
    title: str           # Summary or first user message (max 100 chars)
    directory: str       # Working directory where session was created
    timestamp: datetime  # Last modified time
    preview: str         # First 500 chars for preview pane
    content: str         # Full conversation text (» user, ␣␣ assistant)
    message_count: int   # Conversation turns (user + assistant, excludes tool results)
    mtime: float         # File mtime for incremental update detection
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

**Parallel loading** via `ThreadPoolExecutor`:

```python
with ThreadPoolExecutor(max_workers=len(self.adapters)) as executor:
    futures = {executor.submit(get_incremental, a): a for a in self.adapters}
    for future in as_completed(futures):
        new_or_modified, deleted_ids = future.result()
        self._index.update_sessions(new_or_modified)
        on_progress()  # TUI updates as each adapter completes
```

**Schema versioning**: A `.schema_version` file tracks the index schema. If it doesn't match the code's `SCHEMA_VERSION` constant, the entire index is deleted and rebuilt. This prevents deserialization errors after upgrades.

### Search

[Tantivy](https://github.com/quickwit-oss/tantivy) is a Rust full-text search library (powers Quickwit, similar to Lucene). We use it via [tantivy-py](https://github.com/quickwit-oss/tantivy-py).

**Fuzzy matching** handles typos with edit distance 1 and prefix matching:

```python
for term in query.split():
    fuzzy_title = tantivy.Query.fuzzy_term_query(schema, "title", term, distance=1, prefix=True)
    fuzzy_content = tantivy.Query.fuzzy_term_query(schema, "content", term, distance=1, prefix=True)

    # Term can match in either field (OR), all terms must match (AND)
    term_query = tantivy.Query.boolean_query([
        (tantivy.Occur.Should, fuzzy_title),
        (tantivy.Occur.Should, fuzzy_content),
    ])
    query_parts.append((tantivy.Occur.Must, term_query))
```

So `auth midleware` (typo) matches "authentication middleware".

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

```python
for term in query.lower().split():
    pos = content.lower().find(term)
    if pos != -1:
        start = max(0, pos - 100)  # Show ~100 chars before match
        preview_text = content[start:start + 1500]
        break
```

Matching terms are highlighted with Rich's `Text.stylize()`.

### Resume Handoff

When you press Enter on a session, fast-resume hands off to the original agent:

```python
# In cli.py after TUI exits
resume_cmd, resume_dir = run_tui(query=query, agent_filter=agent)

if resume_cmd:
    # 1. Change to the session's original working directory
    os.chdir(resume_dir)

    # 2. Replace current process with agent's resume command
    os.execvp(resume_cmd[0], resume_cmd)
```

`os.execvp()` replaces the Python process entirely with the agent CLI. This means:

- No subprocess overhead
- Shell history shows `claude --resume xyz`, not `fr`
- Agent inherits the correct working directory
- fast-resume process is gone after handoff

Each adapter returns the appropriate command:

| Agent          | Resume Command                  | With `--yolo`                                                  |
| -------------- | ------------------------------- | -------------------------------------------------------------- |
| Claude         | `claude --resume <id>`          | `claude --dangerously-skip-permissions --resume <id>`          |
| Codex          | `codex resume <id>`             | `codex --dangerously-bypass-approvals-and-sandbox resume <id>` |
| Copilot CLI    | `copilot --resume <id>`         | `copilot --allow-all-tools --allow-all-paths --resume <id>`    |
| Copilot VSCode | `code <directory>`              | _(no change)_                                                  |
| OpenCode       | `opencode <dir> --session <id>` | _(no change)_                                                  |
| Vibe           | `vibe --resume <id>`            | `vibe --auto-approve --resume <id>`                            |
| Crush          | `crush`                         | _(no change)_                                                  |

### Performance

Why fast-resume feels instant:

- **Tantivy (Rust)**: Search engine written in Rust, accessed via Python bindings. Handles fuzzy queries over 10k+ sessions in <10ms
- **Incremental updates**: Only re-parse files where `mtime` changed. Second launch with no changes: ~50ms total
- **Parallel adapters**: All adapters run simultaneously in ThreadPoolExecutor. Total time = slowest adapter, not sum
- **Debounced search**: 50ms debounce prevents wasteful searches while typing
- **Background workers**: Search runs in thread, UI never blocks
- **orjson**: Rust-based JSON parsing, ~10x faster than stdlib json
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
uv sync

# Run locally
uv run fr

# Install pre-commit hooks
uv run pre-commit install

# Run tests
uv run pytest -v

# Lint and format
uv run ruff check .
uv run ruff format .
```

### Project Structure

```
fast-resume/
├── src/fast_resume/
│   ├── cli.py              # Click CLI entry point
│   ├── config.py           # Constants, colors, paths
│   ├── index.py            # TantivyIndex - search engine
│   ├── search.py           # SessionSearch - adapter orchestration
│   ├── tui.py              # Textual TUI application
│   ├── assets/             # Agent icons (PNG)
│   └── adapters/
│       ├── base.py         # Session dataclass, AgentAdapter protocol
│       ├── claude.py       # Claude Code adapter
│       ├── codex.py        # Codex CLI adapter
│       ├── copilot.py      # GitHub Copilot CLI adapter
│       ├── copilot_vscode.py # VS Code Copilot Chat adapter
│       ├── crush.py        # Crush adapter
│       ├── opencode.py     # OpenCode adapter
│       └── vibe.py         # Vibe adapter
├── tests/                  # pytest test suite
├── pyproject.toml          # Dependencies and build config
└── README.md
```

### Tech Stack

| Component           | Library                                                             |
| ------------------- | ------------------------------------------------------------------- |
| TUI Framework       | [Textual](https://textual.textualize.io/)                           |
| Terminal Formatting | [Rich](https://rich.readthedocs.io/)                                |
| CLI Framework       | [Click](https://click.palletsprojects.com/)                         |
| Search Engine       | [Tantivy](https://github.com/quickwit-oss/tantivy) (via tantivy-py) |
| JSON Parsing        | [orjson](https://github.com/ijl/orjson) (fast)                      |
| Date Formatting     | [humanize](https://python-humanize.readthedocs.io/)                 |

## Configuration

fast-resume uses sensible defaults and requires no configuration.

To clear the index and rebuild from scratch:

```bash
rm -rf ~/.cache/fast-resume/
fr --rebuild
```

## License

MIT
