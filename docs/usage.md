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

## TUI color themes

The TUI detects whether the terminal has a dark or light background. Override the detected theme when needed:

```bash
fr --theme light
fr --theme dark
FAST_RESUME_THEME=light fr
```

The command-line option takes precedence over `FAST_RESUME_THEME`. Automatic detection falls back to the dark theme when the terminal does not report its background color.

## Non-interactive commands

```bash
# List matching sessions without opening the TUI (--no-tui is an alias)
fr --list "agent:codex"

# Return stable machine-readable results
fr --json --limit 10 "agent:codex api error"

# Serve the existing index without scanning for changes
fr --json --no-refresh "agent:codex api error"

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

Non-interactive calls refresh the index first. If another `fr` process holds the refresh lock, the call prints a notice on stderr and waits. Pass `--no-refresh` to skip the scan and serve the last indexed state immediately.

Run `fr --agent-context` to print the bundled Agent Skill for coding-agent use.

## Command reference

```text
Usage: fr [OPTIONS] [QUERY]

Arguments:
  [QUERY]                 Search query

Options:
  -a, --agent <AGENT>     Filter by agent
  -d, --directory <DIR>   Filter by directory substring
      --list              List sessions to stdout instead of opening the TUI
                          [alias: --no-tui]
      --json              Output a stable JSON session list
      --limit <N>         Maximum sessions to return (default: 50)
      --offset <N>        Skip matching sessions
      --all               Return all matches from the requested offset
      --no-refresh        Serve the existing index without scanning for changes
      --rebuild           Rebuild the Tantivy index from a fresh scan
      --stats             Show index and session statistics
                          (honors -a and -d filters)
      --agent-context     Print concise instructions for coding agents
      --yolo              Force auto-approve flags where supported
      --theme <THEME>      Select auto, dark, or light TUI colors
                          [env: FAST_RESUME_THEME=] [default: auto]
      --images            Enable agent artwork when supported
      --no-images         Disable agent artwork
      --image-protocol <PROTOCOL>
                          auto, kitty, sixel, or iterm2
  -h, --help              Print help
  -V, --version           Print version
```

## Keybindings

Press `F1` in the TUI to show all keyboard shortcuts.

### Search input

| Key | Action |
| --- | --- |
| `←` / `→` or `Ctrl+B` / `Ctrl+F` | Move the cursor |
| `Home` / `End` or `Ctrl+A` / `Ctrl+E` | Move to the start or end |
| `Backspace` | Delete the previous character |
| `Delete` / `Ctrl+D` | Delete the next character |
| `Ctrl+W` | Delete the previous word |
| `Ctrl+U` | Delete to the start |
| `Tab` / `Shift+Tab` | Accept a suggestion or cycle the agent filter |

### Results navigation

| Key | Action |
| --- | --- |
| `↑` / `↓` | Move selection |
| `Ctrl+J` / `Ctrl+K` | Move selection |
| `Page Up` / `Page Down` | Move by 10 rows |
| `Enter` | Resume the selected session |

### Preview and actions

| Key | Action |
| --- | --- |
| `Ctrl+P` | Toggle the preview pane |
| `Alt`+`+` / `Alt`+`-` | Scroll the preview pane |
| Mouse wheel | Scroll the list or preview under the pointer |
| `Ctrl+Y` | Copy the complete resume command |
| `F1` | Show or close keyboard help |
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
| fx | Configuration-based | — |
| Pi | Not applicable | — |
| Copilot in VS Code | Not applicable | — |

Codex and Vibe record their permission mode in session data, so fast-resume can preserve it automatically. Antigravity CLI, Claude, Copilot CLI, Crush, Cursor CLI, Grok Build, and Kimi Code do not; the TUI asks before resuming them. fx reads its permission mode from settings or `FX_PERMISSION_MODE`, and Pi has no fast-resume yolo variant. Pass `fr --yolo` to skip prompts and force supported options for agents that have one.

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

The shown index location is the default. When `XDG_CACHE_HOME` is an absolute path, fast-resume uses `$XDG_CACHE_HOME/fast-resume/tantivy_index` instead.

## Custom harnesses

fast-resume can index an agent without a built-in adapter when the agent stores each session as one JSON Lines file. Declare the agent as a custom harness in the config file:

- `$FAST_RESUME_CONFIG`, when set
- `$XDG_CONFIG_HOME/fast-resume/config.toml`, when `XDG_CONFIG_HOME` is an absolute path
- `~/.config/fast-resume/config.toml` otherwise

A harness names its session directory, its resume command, and how rows map to session fields. This example reads a store where each row has a `type` field:

```toml
[[harness]]
name = "piti"                       # used in agent:piti and --agent piti
badge = "piti"                      # optional, defaults to name
color = "#9776ff"                   # optional; light_color is also accepted
dir = "~/.local/share/piti/sessions"
extension = "jsonl"                 # optional, default "jsonl"
recursive = false                   # optional; true also reads subdirectories
resume = ["piti", "--resume", "{id}"]
yolo_resume = ["piti", "--yes-all", "--resume", "{id}"]  # optional

[harness.format]
kind = "type"                       # field that identifies the row kind
timestamp = "timestamp"             # optional default timestamp for every row

[harness.format.rows.session]       # rows where type == "session"
id = "id"
cwd = "cwd"

[harness.format.rows.cwd]
cwd = "path"

[harness.format.rows.title]
title = "title"

[harness.format.rows.compaction]
text = "summary"                    # indexed without a role

[harness.format.rows.message]
role = "message.role"
text = "message.content[type=text].text"
```

Each key under `rows` is a value of the `kind` field. Rows of other kinds are skipped. A row mapping can read these fields:

| Field | Meaning |
| --- | --- |
| `id` | Session ID. The first value is used. Defaults to the file name without its extension |
| `cwd` | Working directory. The last value is used |
| `title` | Session title. The last value is used. Defaults to the first user message |
| `timestamp` | Overrides `format.timestamp` for this row kind |
| `role` | Message role. Rows with a role that is not listed in `roles` are skipped |
| `roles` | Maps source role values to `user` or `assistant`. Defaults to `{ user = "user", assistant = "assistant" }` |
| `text` | Text to index. Rows with `text` and no `role` are indexed but not counted as messages |

Paths are object keys separated by dots, such as `message.role`. Arrays are read element by element. `key[field=value]` keeps only the elements of the `key` array whose `field` equals `value`; `key[field=a|b]` accepts several values. When `key` holds a string instead of an array, that string is used. Timestamps can be epoch seconds, epoch milliseconds, or RFC 3339 strings. The session time is the latest user or assistant message, or the first row timestamp, or the file modification time.

`resume` and `yolo_resume` replace `{id}` with the session ID and `{directory}` with its working directory. The command runs in the session's working directory. Without `yolo_resume`, yolo mode does not apply to the harness.

Refresh compares file modification times keyed by file name, so a harness whose session IDs match its file names avoids reopening unchanged files. A file that ends in an incomplete row keeps its previously indexed version until the row is complete.

Harness names use lowercase letters, digits, and dashes, and cannot reuse a built-in agent name. If the config file cannot be parsed, fr prints the error, ignores every custom harness, and keeps their indexed sessions. Sessions of a harness removed from a valid config file are removed from the index on the next refresh.

## Terminal images

Artwork is enabled automatically when the terminal exposes a supported image protocol. Use:

```bash
fr --no-images
fr --image-protocol kitty
fr --image-protocol sixel
fr --image-protocol iterm2
```

See [installation](installation.md) for terminal guidance and [how it works](how-it-works.md) for adapter and index details.
