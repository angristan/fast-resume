# fast-resume

Search and resume conversations across Claude Code, Codex, and more, all from a single place.

## Why fast-resume?

Coding agents are really good right now, so I'm using a bunch of them. Sometimes I remember I, or the LLM, mentioned something specific in a previous session, and I want to go back to it.

The problem is that currently, agents do have a resume feature, but either they don't support searching, or the search is very basic (e.g., title only).

That's why I built `fast-resume`: a command-line tool that aggregates all your coding agent sessions into a single searchable index, so you can quickly find and resume any session.

![demo](https://github.com/user-attachments/assets/752c772e-c23f-4ed6-af3d-add43c7157da)

## Features

- **Unified Search**: One search box to find sessions across all your coding agents
- **Full-Text Search**: Search not just titles, but the entire conversation content, including tool outputs
- **Very fast**: Built on the Rust-powered Tantivy search engine for blazing-fast indexing and searching
- **Fuzzy Matching**: Typo-tolerant search with smart ranking (exact matches boosted)
- **Direct Resume**: Select, Enter, you're back in your session
- **Beautiful TUI**: fzf-style interface with agent icons, color-coded results, and live preview

## Supported Agents

| Agent              | Data Location                        | Resume Command                  |
| ------------------ | ------------------------------------ | ------------------------------- |
| **Claude Code**    | `~/.claude/projects/`                | `claude --resume <id>`          |
| **Codex CLI**      | `~/.codex/sessions/`                 | `codex resume <id>`             |
| **GitHub Copilot** | `~/.copilot/session-state/`          | `copilot --resume <id>`         |
| **Crush**          | `~/.local/share/crush/projects.json` | _(interactive only)_            |
| **OpenCode**       | `~/.local/share/opencode/storage/`   | `opencode <dir> --session <id>` |
| **Vibe**           | `~/.vibe/logs/session/`              | `vibe --resume <id>`            |

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

### Command Reference

```
Usage: fr [OPTIONS] [QUERY]

Arguments:
  QUERY                    Search query (optional)

Options:
  -a, --agent [claude|codex|copilot|crush|opencode|vibe]
                          Filter by agent
  -d, --directory TEXT    Filter by directory (substring match)
  --no-tui                Output list to stdout instead of TUI
  --list                  Just list sessions, don't resume
  --rebuild               Force rebuild the session index
  --stats                 Show index statistics
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
┌─────────────────────────────────────────────────────────────┐
│                         CLI (cli.py)                        │
│            fr [query] [-a agent] [-d dir] [flags]           │
└─────────────────────────────────────────────────────────────┘
                              │
              ┌───────────────┴───────────────┐
              ▼                               ▼
┌─────────────────────────┐     ┌─────────────────────────────┐
│      TUI (tui.py)       │     │    Non-TUI Output (--no-tui)│
│  Textual-based fuzzy    │     │    Rich table to stdout     │
│  finder interface       │     │                             │
└─────────────────────────┘     └─────────────────────────────┘
              │                               │
              └───────────────┬───────────────┘
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                  SessionSearch (search.py)                  │
│         Orchestrates adapters, manages streaming            │
└─────────────────────────────────────────────────────────────┘
              │                               │
              ▼                               ▼
┌─────────────────────────┐     ┌─────────────────────────────┐
│   TantivyIndex          │     │     Adapter Layer           │
│   (index.py)            │     │     (adapters/*.py)         │
│                         │     │                             │
│ • Full-text search      │     │ • ClaudeAdapter             │
│ • Fuzzy matching        │     │ • CodexAdapter              │
│ • Incremental updates   │     │ • CopilotAdapter            │
│ • Stats computation     │     │ • CrushAdapter              │
│                         │     │ • OpenCodeAdapter           │
│ ~/.cache/fast-resume/   │     │ • VibeAdapter               │
└─────────────────────────┘     └─────────────────────────────┘
```

### Search Flow

1. **Index Check**: On startup, checks if Tantivy index exists and schema version matches
2. **Incremental Load**: Compares file mtimes against known sessions, only parses changed files
3. **Parallel Loading**: Uses ThreadPoolExecutor to load from all agents simultaneously
4. **Tantivy Search**: Builds fuzzy queries with edit distance 1, searches title + content
5. **Results Ranking**: Exact matches boosted, results sorted by score then timestamp

### Index Details

- **Engine:** [Tantivy](https://github.com/quickwit-oss/tantivy) (Rust full-text search library)
- **Location:** `~/.cache/fast-resume/tantivy_index/`
- **Schema Version:** Tracked in `.schema_version` file, auto-rebuilds on schema changes
- **Fields:** `id`, `title`, `directory`, `agent`, `content`, `timestamp`, `message_count`, `mtime`

### Adapter Protocol

Each agent adapter implements:

```python
class AgentAdapter(Protocol):
    def find_sessions(self) -> list[Session]: ...
    def find_sessions_incremental(self, known: dict) -> tuple[list[Session], set[str]]: ...
    def get_resume_command(self, session: Session) -> list[str] | None: ...
    def is_available(self) -> bool: ...
```

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
│       ├── copilot.py      # GitHub Copilot adapter
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
