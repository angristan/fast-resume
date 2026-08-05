# Usage

## Interactive search

Open the TUI with every indexed session:

```bash
fr
```

Start with a free-text query or CLI filter:

```bash
fr "authentication bug"
fr -a claude
fr -a codex -d backend "api error"
```

Free text searches titles, conversation content, and bounded single-token directory/path matches. Exact matches are boosted, while fuzzy matching keeps common typos useful.

## Search filters

Filters work in the initial query and directly in the TUI search box.

### Agents

```text
agent:claude             # Include one agent
agent:claude,codex       # Include multiple agents
-agent:vibe              # Exclude an agent
agent:claude,!codex      # Include Claude and exclude Codex
```

### Directories

Directory filters use case-insensitive substring matching:

```text
dir:myproject
dir:backend,!test
```

### Dates

```text
date:today
date:yesterday
date:<1h
date:<2d
date:>1w
date:week
date:month
```

Relative units are `m`, `h`, `d`, `w`, `mo`, and `y`. `date:today` and `date:yesterday` use local civil-day boundaries, including daylight-saving transitions.

### Combining filters

```bash
fr "agent:claude date:<1d api bug"
fr "dir:backend -agent:vibe auth"
```

Type a partial filter such as `agent:cl` and press `Tab` to accept the suggestion.

## Non-interactive commands

```bash
# Print matching sessions without opening the TUI
fr --no-tui "api error"

# List sessions without offering to resume
fr --list "agent:codex"

# Return stable machine-readable results
fr --json --limit 10 "agent:codex api error"

# Rebuild the index from every source
fr --rebuild

# Show index and activity statistics
fr --stats
```

### JSON output

`--json` prints exactly one JSON object to stdout and implies non-interactive listing. Diagnostics and errors stay on stderr.

```json
{
  "schema_version": 1,
  "sessions": [
    {
      "id": "abc123",
      "agent": "codex",
      "title": "Review API authentication",
      "directory": "/work/backend",
      "timestamp": "2026-07-15T12:00:00+02:00",
      "message_count": 8,
      "resume_command": ["codex", "resume", "abc123"]
    }
  ],
  "meta": {
    "state": "more",
    "total": 24,
    "offset": 0,
    "limit": 10,
    "returned": 10,
    "next_offset": 10
  }
}
```

Continue with the same query and filters plus `--offset <next_offset>` only while `meta.state` is `more`. Stop on `complete` or `past_end`. `--all` returns every match from the requested offset; it conflicts with an explicit `--limit`.

The JSON session objects omit indexed conversation content and internal refresh fields. `--yolo` changes supported `resume_command` values but never starts a session in JSON mode.

Run `fr --agent-context` to print the bundled Agent Skill for coding-agent use.

## Command reference

```text
Usage: fr [OPTIONS] [QUERY]

Arguments:
  [QUERY]                 Search query

Options:
  -a, --agent <AGENT>     Filter by agent
  -d, --directory <DIR>   Filter by directory substring
      --no-tui            Output a list instead of opening the TUI
      --list              List sessions without resuming
      --json              Output a stable JSON session list
      --limit <N>         Maximum sessions to return (default: 50)
      --offset <N>        Skip matching sessions
      --all               Return all matches from the requested offset
      --rebuild           Rebuild the Tantivy index from a fresh scan
      --stats             Show index and session statistics
      --agent-context     Print concise instructions for coding agents
      --yolo              Force auto-approve flags where supported
      --images            Enable agent artwork when supported
      --no-images         Disable agent artwork
      --image-protocol <PROTOCOL>
                          auto, kitty, sixel, or iterm2
  -h, --help              Print help
  -V, --version           Print version
```

## Keybindings

### Search and navigation

| Key | Action |
| --- | --- |
| `↑` / `↓` | Move selection |
| `Ctrl+J` / `Ctrl+K` | Move selection |
| `Page Up` / `Page Down` | Move by 10 rows |
| `Tab` / `Shift+Tab` | Accept a suggestion or cycle the agent filter |
| `Enter` | Resume the selected session |

### Preview and actions

| Key | Action |
| --- | --- |
| `Ctrl+P` | Toggle the preview pane |
| `Alt`+`+` / `Alt`+`-` | Scroll the preview pane |
| Mouse wheel | Scroll the list or preview under the pointer |
| `Ctrl+Y` | Copy the complete resume command |
| `Esc` / `Ctrl+C` | Quit |

### Yolo confirmation

| Key | Action |
| --- | --- |
| `Tab` | Toggle the selected answer |
| `←` / `→` | Select No or Yolo |
| `Enter` | Confirm |
| `y` / `n` | Select Yolo or No directly |
| `Esc` | Cancel |

## Yolo mode

Yolo mode resumes an agent with its auto-approve or skip-permissions option when available.

| Agent | Added option | Detected from session |
| --- | --- | --- |
| Antigravity CLI | `--dangerously-skip-permissions` | No |
| Claude | `--dangerously-skip-permissions` | No |
| Codex | `--dangerously-bypass-approvals-and-sandbox` | Yes |
| Copilot CLI | `--yolo` | No |
| Crush | `--yolo` | No |
| Cursor CLI | `--yolo` | No |
| Grok Build | `--always-approve` | No |
| Kimi Code | `--yolo` | No |
| Vibe | `--agent auto-approve` | Yes |
| OpenCode | Configuration-based | — |
| Pi | Not applicable | — |
| Copilot in VS Code | Not applicable | — |

Codex and Vibe record their permission mode in session data, so fast-resume can preserve it automatically. Antigravity CLI, Claude, Copilot CLI, Crush, Cursor CLI, Grok Build, and Kimi Code do not; the TUI asks before resuming them. Pi has no fast-resume yolo variant. Pass `fr --yolo` to skip prompts and force supported options for agents that have one.

## Statistics

`fr --stats` reports:

- Total indexed sessions and messages
- Index size and date range
- Raw data size and indexed content by agent
- Activity by weekday and hour
- Most active directories

Example:

```text
Index Statistics

  Total sessions          751
  Total messages          13,799
  Avg messages/session    18.4
  Index size              15.5 MB
  Index location          ~/.cache/fast-resume/tantivy_index

Data by Agent

Agent              Files       Disk   Sessions   Messages    Content
------------------------------------------------------------------------
claude               477   312.9 MB        377      10415     3.1 MB
codex                107    23.6 MB         89        321   890.6 KB
opencode            9275    46.3 MB         72       1912   597.7 KB
```

## Terminal images

Artwork is enabled automatically when the terminal exposes a supported image protocol. Use:

```bash
fr --no-images
fr --image-protocol kitty
fr --image-protocol sixel
fr --image-protocol iterm2
```

See [installation](installation.md) for terminal guidance and [how it works](how-it-works.md) for adapter and index details.
