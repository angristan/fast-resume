# Repository instructions

## Project

- `fast-resume` is a Rust 2024 CLI and Ratatui application. The binary is `fr`.
- Read `docs/development.md` for the source map and validation workflow.
- Read `docs/how-it-works.md` before changing adapters, refresh behavior, search, or resume handoff.
- Keep the README focused on discovery. Put detailed user behavior in `docs/usage.md`.

## Behavior

- Preserve the human CLI unless a change explicitly replaces it.
- Machine output must remain one valid JSON value on stdout. Send diagnostics and errors to stderr.
- Do not serialize `model::Session` directly. It contains conversation content and internal refresh fields.
- Keep index writes behind the cross-process coordination in `index.rs`. Reload the manual Tantivy reader after another process can commit.
- Adapter scans must not delete good indexed data after partial files, malformed rows, inaccessible paths, or incomplete scans.
- Index user and assistant conversation text. Exclude system payloads, large tool results, and unrelated local command output.
- Tests must use synthetic session data under `TempDir`. Do not depend on a developer's real agent history or home directory.

## Adding or changing an adapter

- Implement the `Adapter` contract in `src/adapters/` and register it in `src/adapters/mod.rs`.
- Build file-backed incremental refresh on `shared::incremental_scan`: write a file scan and a parser, and pass both through it. It owns the deletion-safety rule (incomplete scans never delete). Use `shared::build_resume_command` for the standard binary/yolo/resume shape.
- Update agent metadata and source paths in `src/config.rs`.
- Add focused parser, incremental refresh, deletion-safety, and resume-command tests.
- Add binary coverage in `tests/cli.rs`.
- Update the supported-agent and resume-command documentation.
- Add and register an agent image only when an appropriate asset is available.

## Validation

Run the same core checks as CI:

```bash
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
git diff --check
```

CI fails on any Clippy warning, so fix or explicitly allow every warning before committing.

## Commits

- Follow Conventional Commits for every commit subject.
- Use `<type>(<optional-scope>): <description>`.
- Use one of the repository's allowed types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, or `revert`.
- Keep the complete commit header to 72 characters or fewer.
- Before committing, validate the exact final subject against `commitlint.config.mjs`.
- Keep PR titles compatible with the same rules so squash-merge commit titles pass commitlint. For example, use `feat(pi): add Pi session support (#71)`, not `Add Pi session support (#71)`.
