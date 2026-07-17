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

# Rebuild the index from every source
fr --rebuild

# Show index and activity statistics
fr --stats
```

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
      --rebuild           Rebuild the Tantivy index from a fresh scan
      --stats             Show index and session statistics
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
| `Ctrl+N` | Toggle showing only named sessions (renamed or AI-titled) |
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
| Claude | `--dangerously-skip-permissions` | No |
| Codex | `--dangerously-bypass-approvals-and-sandbox` | Yes |
| Copilot CLI | `--yolo` | No |
| Vibe | `--agent auto-approve` | Yes |
| Crush | `--yolo` | No |
| OpenCode | Configuration-based | — |
| Pi | Not applicable | — |
| Copilot in VS Code | Not applicable | — |

Codex and Vibe record their permission mode in session data, so fast-resume can preserve it automatically. Claude, Copilot CLI, and Crush do not; the TUI asks before resuming them. Pi has no fast-resume yolo variant. Pass `fr --yolo` to skip prompts and force supported options for agents that have one.

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
