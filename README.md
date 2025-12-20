# fast-resume

A fuzzy finder TUI for quickly searching and resuming coding agent sessions across multiple AI assistants.

## Features

- **Unified search** across Claude Code, Codex CLI, Crush, OpenCode, and Vibe sessions
- **Fuzzy matching** with hybrid scoring (fuzzy + exact match bonuses)
- **Interactive TUI** built with [Textual](https://textual.textualize.io/) with fzf-style interface
- **Progressive loading** - sessions stream in as each agent loads
- **Fast caching** for instant startup on subsequent runs
- **Direct resume** - select a session and immediately resume it with the appropriate agent

## Installation

Requires Python 3.14+.

```bash
# Install from GitHub
uvx --from git+https://github.com/angristan/fast-resume fr

# Or install permanently
uv tool install git+https://github.com/angristan/fast-resume
```

For development:

```bash
git clone https://github.com/angristan/fast-resume.git
cd fast-resume
uv sync
```

## Usage

Both `fr` and `fast-resume` commands are available.

```bash
# Open TUI with all sessions
fr

# Search for specific term
fr auth middleware

# Filter by agent
fr -a claude
fr -a codex
fr -a crush
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
| `1-6` | Filter by agent (All/Claude/Codex/Crush/OpenCode/Vibe) |
| `Ctrl+P` | Open command palette |
| `q` or `Esc` | Quit |

## Supported Agents

| Agent | Data Location | Resume Command |
|-------|---------------|----------------|
| Claude Code | `~/.claude/projects/` | `claude --resume <id>` |
| Codex CLI | `~/.codex/sessions/` | `codex resume <id>` |
| Crush | `~/.local/share/crush/projects.json` | N/A (no CLI resume support) |
| OpenCode | `~/.local/share/opencode/storage/` | `opencode <dir> --session <id>` |
| Vibe | `~/.vibe/logs/session/` | `vibe --resume <id>` |

## Development

```bash
uv run pre-commit install
uv run pytest
```

## How It Works

1. Scans session files from each supported agent's data directory
2. Extracts session metadata (title, directory, timestamp) and conversation content
3. Caches results in `~/.cache/fast-resume/` for fast subsequent loads
4. Uses [rapidfuzz](https://github.com/rapidfuzz/RapidFuzz) for fuzzy search with hybrid scoring that boosts exact and phrase matches
5. On selection, changes to the session's working directory and executes the agent's resume command
