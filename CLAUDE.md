# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`fast-resume` (`fr`) — a TUI for searching and resuming sessions across multiple coding-agent CLIs (Claude Code, Codex, Copilot CLI, Copilot VS Code, Crush, Gemini, Kiro, OpenCode, Vibe). Sessions are normalized via per-agent adapters and indexed in a Tantivy full-text search index.

Requires Python `>=3.14`. Managed with `uv`.

## Common commands

```bash
uv sync                          # install deps (incl. dev group)
uv run fr                        # run the CLI from source
uv run pytest                    # run full test suite (pytest-xdist auto-parallel via pyproject)
uv run pytest tests/test_index.py::test_name -v   # run a single test
uv run pytest -n 0               # disable xdist (helpful for debugging)
uv run ruff check . && uv run ruff format .
uv run ty check src/             # type check (Astral's `ty`)
uv run pre-commit run --all-files
```

Pre-commit hooks run `ruff` (with `--fix`), `ruff-format`, `ty check src/`, and the full `pytest` suite on every commit — expect commits to take a few seconds.

Commit messages follow Conventional Commits (`feat`, `fix`, `chore`, etc.); `commitlint` enforces a 72-char header. `semantic-release` (`.releaserc.json`) cuts versions from `master` — do not hand-edit `pyproject.toml`'s `version` or `CHANGELOG.md`.

## Architecture

The README has a thorough architecture section with diagrams; below are the non-obvious points worth knowing before editing.

**Adapter contract** — `src/fast_resume/adapters/base.py` defines two layers:
- `AgentAdapter` (Protocol): the public interface every adapter implements (`find_sessions`, `find_sessions_incremental`, `get_resume_command`, `is_available`, `get_raw_stats`, `supports_yolo`).
- `BaseSessionAdapter` (ABC): template-method base for file-based adapters (Claude, Codex, Copilot CLI/VS Code, Vibe). Subclasses only implement `_scan_session_files()` and `_parse_session_file()`; the base handles incremental scanning by comparing mtimes against `known` and emitting deleted IDs. `Crush` (SQLite) and `OpenCode` (split JSON, lazy parallel I/O) do not use this base — they implement the protocol directly.

**Incremental indexing** — `SessionSearch` (in `search.py`) loads `(session_id → (mtime, agent))` pairs from Tantivy, then dispatches adapters in a `ThreadPoolExecutor`. Adapters re-parse only files whose mtime exceeds the stored value by `MTIME_TOLERANCE` (1 ms — datetime precision). Sessions stream into the index via the `on_session` callback in batches; the TUI is wired to render them progressively.

**Schema versioning gotcha** — `config.SCHEMA_VERSION` is written to `~/.cache/fast-resume/tantivy_index/.schema_version`. **Any change to the Tantivy schema in `index.py` requires bumping `SCHEMA_VERSION`** or users get cryptic deserialization errors on upgrade. The index is auto-wiped and rebuilt when the version mismatches. The current version is 21 (bumped when the Gemini and Kiro adapters were added).

**Resume handoff** — `cli.py` does **not** subprocess the agent. After `run_tui()` returns a `(resume_cmd, resume_dir)` tuple, it `os.chdir(resume_dir)` then `os.execvp()` to replace the Python process entirely. Anything that needs cleanup (file handles, threads) must finish before `execvp`. When adding a new adapter, `get_resume_command()` must return an `argv` list that is safe to `execvp` directly.

**Yolo mode** — `supports_yolo` gates whether the TUI prompts. Codex and Vibe sniff their session files to set `Session.yolo` at parse time (auto-detect); Claude and Copilot CLI cannot, so the TUI shows a modal unless `--yolo` is passed.

**Query syntax** — `query.py` parses the keyword DSL (`agent:`, `dir:`, `date:`) into `Filter`/`DateFilter` objects before handing free-text terms to Tantivy. Search combines a 5x-boosted exact BM25 query with per-term `fuzzy_term_query(distance=1, prefix=True)` on `title` and `content` so typos still match while exact hits rank first.

**TUI layout** — `src/fast_resume/tui/` is a Textual app split into `app.py` (orchestration), `results_table.py`, `search_input.py`, `preview.py`, `filter_bar.py`, `modal.py` (yolo prompt), `query.py` (autocomplete), and `styles.py`. Searches are debounced 50 ms and run on a background worker; the preview pane jumps to and highlights the first matching term.

**What gets indexed** — only user prompts and assistant text responses. Tool calls, tool results, system/meta messages, and slash-command outputs are explicitly excluded to keep the index small and relevant.

## Tests

- `pytest-asyncio` is in `asyncio_mode = "auto"` — don't decorate async tests with `@pytest.mark.asyncio`.
- `pytest-xdist` runs `-n auto` by default; tests must be isolation-safe. Use `tmp_path` rather than fixed paths, and never touch the user's real `~/.cache/fast-resume/` from tests.
- TUI tests use Textual's `App.run_test()` pattern; see `tests/test_tui.py` for examples.
- Each adapter has its own `test_<agent>_adapter.py` — when adding an adapter, mirror the structure (fixtures with sample session files, mtime/incremental cases, resume-command assertions including yolo variants).
