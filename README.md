# fast-resume

A fuzzy finder TUI for quickly searching and resuming coding agent sessions across multiple AI assistants.

## Features

- **Unified search** across Claude Code, Codex CLI, OpenCode, and Vibe sessions
- **Fuzzy matching** with hybrid scoring (fuzzy + exact match bonuses)
- **Interactive TUI** built with [Textual](https://textual.textualize.io/) featuring search highlighting and session preview
- **Fast caching** for instant startup on subsequent runs
- **Direct resume** - select a session and immediately resume it with the appropriate agent

## Installation

Requires Python 3.11+.

```bash
uv tool install .
```

Or install in development mode:

```bash
uv pip install -e .
```

## Usage

```bash
# Open TUI with all sessions
fr

# Search for specific term
fr auth middleware

# Filter by agent
fr -a claude
fr -a codex
fr -a opencode
fr -a vibe

# Filter by directory
fr -d myproject

# List sessions without TUI
fr --no-tui

# Force rebuild the session cache
fr --rebuild
```

## Keybindings

| Key | Action |
|-----|--------|
| `↑/↓` or `j/k` | Navigate sessions |
| `Enter` | Resume selected session |
| `Tab` | Toggle preview pane |
| `/` | Focus search input |
| `q` or `Esc` | Quit |

## Supported Agents

| Agent | Data Location | Resume Command |
|-------|---------------|----------------|
| Claude Code | `~/.claude/projects/` | `claude --resume <id>` |
| Codex CLI | `~/.codex/sessions/` | `codex resume <id>` |
| OpenCode | `~/.local/share/opencode/storage/` | `opencode <dir> --session <id>` |
| Vibe | `~/.vibe/logs/session/` | `vibe --resume <id>` |

## How It Works

1. Scans session files from each supported agent's data directory
2. Extracts session metadata (title, directory, timestamp) and conversation content
3. Caches results in `~/.cache/fast-resume/` for fast subsequent loads
4. Uses [rapidfuzz](https://github.com/rapidfuzz/RapidFuzz) for fuzzy search with hybrid scoring that boosts exact and phrase matches
5. On selection, changes to the session's working directory and executes the agent's resume command
