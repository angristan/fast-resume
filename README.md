<p align="center">
  <img src="assets/logo.png" alt="fast-resume" width="120" height="120">
</p>

# fast-resume

Search and resume conversations across Claude Code, Codex, Pi, and more, all from one terminal UI.

Coding agents can resume previous sessions, but searching those sessions is often limited or title-only. `fast-resume` builds a single full-text index over your local agent history so you can find a prompt, response, project, or session and jump straight back in.

<https://github.com/user-attachments/assets/60f6e128-2ae4-431d-8a87-097f600b6d04>

## Highlights

- Search titles, directories, user messages, and assistant responses across every supported agent.
- Find imperfect matches with typo-tolerant Tantivy search and exact-match ranking.
- Filter by agent, directory, and date from the command line or directly in the search box.
- Preview conversations, copy resume commands, or hand off directly to the original agent.
- Start immediately from the existing index while changed sessions refresh in the background.
- Use agent artwork, mouse controls, responsive filters, and compact layouts in the Ratatui TUI.

## Supported agents

Antigravity CLI, Claude Code, Codex, Copilot CLI, Copilot in VS Code, Crush, Cursor CLI, Grok Build, Kimi Code, OpenCode, Pi, and Vibe. See [how it works](docs/how-it-works.md#session-adapters) for storage formats and resume behavior.

## Install

Homebrew is the simplest option on macOS and Linux:

```bash
brew tap angristan/tap
brew install fast-resume
```

You can also install a binary wheel with `uv`:

```bash
uv tool install fast-resume
```

See the [installation guide](docs/installation.md) for `uvx`, Cargo, supported platforms, and terminal recommendations.

## Quick start

```bash
# Search all sessions interactively
fr

# Start with a query or filters
fr "authentication bug"
fr -a codex -d backend "api error"

# Search without opening the TUI
fr --list "agent:claude date:<2d auth"

# Inspect or rebuild the local index
fr --stats
fr --rebuild
```

Inside the TUI, use the arrow keys to select a session and press `Enter` to resume it. `Tab` completes filters or cycles agents, `Ctrl+P` toggles the preview, and `Ctrl+Y` copies the resume command.

## Documentation

- [Installation](docs/installation.md) — packages, platforms, terminals, and upgrades
- [Usage](docs/usage.md) — search syntax, CLI options, keybindings, yolo mode, and statistics
- [How it works](docs/how-it-works.md) — adapters, indexing, search, refresh safety, and resume handoff
- [Development](docs/development.md) — local setup, validation, project layout, and release packaging

## Configuration

No configuration is required. The Tantivy index lives at `~/.cache/fast-resume/tantivy_index` and is rebuilt automatically when its schema changes.

To reset it manually:

```bash
rm -rf ~/.cache/fast-resume
fr --rebuild
```

## License

MIT
