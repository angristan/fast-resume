# How fast-resume works

## Overview

fast-resume normalizes local session data from each supported coding agent, stores searchable fields in Tantivy, and hands the selected session back to its original agent CLI.

```text
Agent stores ──► adapters ──► normalized sessions ──► Tantivy index
                                                        │
Terminal ◄──── resume handoff ◄──── TUI/search ◄────────┘
```

The TUI opens against the current index immediately. A background refresh scans for changes, commits updates in batches, reloads the search reader, and preserves the current selection where possible.

## Session adapters

Each adapter maps an agent-specific format into the shared `Session` model.

| Agent | Format | Parsing strategy |
| --- | --- | --- |
| Antigravity CLI | `~/.gemini/antigravity-cli/brain/<id>/.system_generated/logs/*.jsonl` | Reads user and model transcript steps and joins workspace metadata from Antigravity history |
| Claude Code | `~/.claude/projects/<project>/*.jsonl` | Reads user and assistant entries and skips agent subprocess files |
| Codex | `~/.codex/sessions/**/*.jsonl` | Reads `session_meta`, `response_item`, and `event_msg` records |
| Copilot CLI | `~/.copilot/session-state/**/*.jsonl` | Reads session identity, user messages, assistant messages, and titles |
| Copilot in VS Code | VS Code chat-session JSON | Reads request text, response values, and workspace references |
| Crush | Per-project SQLite database | Queries sessions and messages and parses JSON message parts |
| Cursor CLI | `~/.cursor/chats/*/*/store.db` | Reads session metadata and user/assistant records from Cursor's local SQLite stores |
| Gemini CLI | `~/.gemini/tmp/<project>/chats/session-*.{json,jsonl}` | Replays messages, metadata updates, checkpoints, and rewinds from legacy and current recordings |
| Grok Build | `$GROK_HOME/sessions/<workspace>/<id>/{summary.json,updates.jsonl}` | Reads session metadata and combines streamed ACP user and agent message chunks |
| OpenCode | SQLite or legacy split JSON | Joins sessions, messages, and text parts |
| Pi | `~/.pi/agent/sessions/**/*.jsonl` | Reads session headers, user and assistant messages, names, visible custom messages, and summaries |
| Vibe | `meta.json` and `messages.jsonl` | Reads metadata, role-based content, and auto-approve state |

Grok discovery respects `GROK_HOME`. Gemini resolves project directories from `.project_root` markers or `~/.gemini/projects.json`. Pi discovery respects `PI_CODING_AGENT_SESSION_DIR`, `PI_CODING_AGENT_DIR`, and the global `settings.json` `sessionDir`. Project-local `sessionDir` overrides outside that configured store cannot be discovered automatically.

The normalized model contains:

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

The index focuses on conversation text: prompts and assistant responses. Most large tool results, system metadata, context payloads, and local command output are excluded. Crush may include short tool-call or result summaries when they are stored as ordinary message parts.

## Indexing and refresh

The persistent index lives at:

```text
~/.cache/fast-resume/tantivy_index
```

On an incremental refresh, fast-resume:

1. Loads indexed session IDs and refresh markers.
2. Scans each adapter concurrently.
3. Parses new or changed sessions.
4. Retains old documents when a source is temporarily incomplete or malformed.
5. Infers deletions only when the relevant scan is complete.
6. Commits changes in batches and reports progress to the TUI.

File-backed adapters normally use modification times. Database-backed adapters include their relevant message and part activity; Crush also fingerprints the final indexed projection so same-second edits are detected.

JSONL sources distinguish three states:

- Clean logs follow normal update and deletion semantics.
- Recoverable partial logs can apply valid later records.
- Trailing or wholly invalid data retains the previous document until the writer completes it.

This prevents a transient write, inaccessible directory, or malformed companion file from erasing good indexed content.

The index schema has an explicit version. A mismatch triggers a complete rebuild instead of trying to read incompatible documents.

## Search

[Tantivy](https://github.com/quickwit-oss/tantivy) provides full-text indexing and ranking.

Search combines:

- Boosted exact matching over titles and conversation content
- Fuzzy prefix matching for common typos
- Bounded, single-token directory/path matching
- Structured agent, directory, and date filters

Exact matches rank first, but queries such as `auth midleware` can still find “authentication middleware,” and a token such as `backend` can match `/work/backend`.

Queries run in a worker thread. Each request receives a generation number; results from an older generation are ignored after the user types something newer.

```text
Keystroke ──► redraw input ──► search worker ──► Tantivy
    ▲                                                │
    └──────────── apply latest generation ◄──────────┘
```

## TUI refresh lifecycle

- **Warm path:** render and search the existing index immediately.
- **Refresh path:** scan adapters in the background and commit changed sessions in batches.
- **Reload path:** refresh the Tantivy reader and re-run the active query.
- **Failure path:** keep the current results and show the error instead of presenting an empty successful search.

The preview renders roles and code blocks, highlights query terms, and scrolls independently from the result list. Agent artwork uses the terminal's supported image protocol and compensates for terminal cell geometry.

## Resume handoff

Each adapter returns the command needed to continue its session:

| Agent | Resume command | Yolo variant |
| --- | --- | --- |
| Antigravity CLI | `agy --conversation <id>` | No change |
| Claude | `claude --resume <id>` | `claude --dangerously-skip-permissions --resume <id>` |
| Codex | `codex resume <id>` | `codex --dangerously-bypass-approvals-and-sandbox resume <id>` |
| Copilot CLI | `copilot --resume <id>` | `copilot --yolo --resume <id>` |
| Copilot in VS Code | `code <directory>` | No change |
| Crush | `crush --session <id>` | `crush --yolo --session <id>` |
| Cursor CLI | `agent --resume <id>` | `agent --yolo --resume <id>` |
| Gemini CLI | `gemini --resume <id>` | `gemini --approval-mode=yolo --resume <id>` |
| Grok Build | `grok --resume <id>` | `grok --always-approve --resume <id>` |
| OpenCode | `opencode <directory> --session <id>` | No change |
| Pi | `pi --session <id>` | No change |
| Vibe | `vibe --resume <id>` | `vibe --agent auto-approve --resume <id>` |

On Unix, `exec()` replaces fast-resume with the agent process. On Windows, fast-resume waits for the child and exits with the same status. In both cases the agent receives the session's working directory.

## Performance

fast-resume avoids a full parse on ordinary launches:

- Adapters scan concurrently.
- The current index is searchable before refresh finishes.
- Unchanged sessions are not re-parsed.
- Changed sessions are committed in batches.
- Search and index reloads stay off the input path.
- Obsolete search generations are discarded.

Cold-start time depends on the size and format of local agent history. Warm searches are typically measured in milliseconds once the Tantivy reader is loaded.

See the [usage guide](usage.md) for query syntax or [development](development.md) for the code layout and validation commands.
