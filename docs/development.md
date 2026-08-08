# Development

## Setup

```bash
git clone https://github.com/angristan/fast-resume.git
cd fast-resume
cargo run --
```

Install the repository hooks if you have `pre-commit` available:

```bash
pre-commit install
```

## Validation

Run the same core checks used by CI:

```bash
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
git diff --check
```

CI fails on any Clippy warning. Fix warnings or add a narrow, explicit `#[allow]` with a reason.

## Project layout

```text
fast-resume/
├── src/
│   ├── main.rs             # Clap CLI and resume process handoff
│   ├── config.rs           # Agent metadata, paths, and schema version
│   ├── model.rs            # Normalized session model
│   ├── output.rs           # Human tables and stable JSON envelopes
│   ├── refresh.rs          # Concurrent incremental refresh orchestration
│   ├── index.rs            # Tantivy index facade and process locks
│   ├── index/              # Schema, documents, queries, and statistics
│   ├── query.rs            # User query and filter parsing
│   ├── search.rs           # Search engine facade
│   ├── stats.rs            # CLI statistics output
│   ├── adapters/           # Agent parsers and resume commands
│   ├── tui.rs              # Terminal lifecycle and event loop
│   └── tui/                # State, input, rendering, preview, layout, and images
├── tests/                  # CLI integration tests
├── assets/                 # Project and agent artwork
├── python/                 # Compatibility wrappers packaged in wheels
├── skills/                 # Portable Agent Skills embedded by the CLI
├── docs/                   # User and contributor documentation
├── Cargo.toml              # Rust dependencies and binary metadata
└── pyproject.toml          # Maturin/PyPI metadata
```

## Main components

| Component | Library |
| --- | --- |
| Terminal UI | [Ratatui](https://ratatui.rs/) |
| Terminal handling | [Crossterm](https://github.com/crossterm-rs/crossterm) |
| CLI | [Clap](https://docs.rs/clap/latest/clap/) |
| Search | [Tantivy](https://github.com/quickwit-oss/tantivy) |
| JSON | [serde_json](https://docs.rs/serde_json/latest/serde_json/) |
| SQLite | [rusqlite](https://docs.rs/rusqlite/latest/rusqlite/) |

## Packaging

`maturin` builds PyPI wheels containing the Rust binary and compatibility commands. The same wheel builds supply npm's native variants. All variants use the `fast-resume` package name with platform prerelease versions such as `2.7.0-linux-x64`. The launcher selects one through an npm alias in `optionalDependencies`; it does not download code from an install script.

Release automation also builds standalone macOS and Linux archives and dispatches the Homebrew formula update. Pull-request CI builds and installs wheels for macOS ARM64/Intel and Linux ARM64/x86_64. Release and publishing jobs run after a qualifying push to `master`, or through a manual workflow run that resumes an existing release.

### Resuming a partial release

If a publish job fails after semantic-release has created the tag, run the CI workflow manually (Actions → CI → Run workflow) with the release version, for example `2.8.0`. The run rebuilds the artifacts from the tag and retries every publish step. Publishing is idempotent: npm skips versions that already exist, `uv publish` checks PyPI before uploading, and release-asset uploads overwrite.

### npm publishing setup

npm requires a package to exist before it can have a trusted publisher. Claim the package once from an interactive npm session without making the bootstrap version `latest`:

```bash
npm login
npm publish ./npm/fast-resume --access public --tag bootstrap
npm trust github fast-resume \
  --repo angristan/fast-resume \
  --file workflow.yml \
  --allow-publish
```

The account must use 2FA. The trusted publisher is organization or user `angristan`, repository `fast-resume`, and workflow filename `workflow.yml`. The release workflow then publishes every native variant and the launcher through GitHub OIDC with provenance. It does not use an npm token. After a successful OIDC release, disallow token publishing in the npm package settings.

## Documentation

Keep the README focused on discovery and first use. Put detailed user workflows in [usage](usage.md), installation-specific material in [installation](installation.md), and implementation details in [how it works](how-it-works.md).
