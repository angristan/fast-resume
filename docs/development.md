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
cargo check --all-targets --locked
cargo test --locked
cargo build --release --locked
git diff --check
```

Clippy is useful for additional cleanup, although the project does not currently fail CI on every style warning:

```bash
cargo clippy --all-targets --locked
```

## Project layout

```text
fast-resume/
├── src/
│   ├── main.rs             # Clap CLI and resume process handoff
│   ├── config.rs           # Agent metadata, paths, and schema version
│   ├── model.rs            # Normalized session model
│   ├── refresh.rs          # Concurrent incremental refresh orchestration
│   ├── index.rs            # Tantivy index facade
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

`maturin` builds PyPI wheels containing the Rust binary and compatibility commands. Release automation also builds standalone macOS and Linux archives and dispatches the Homebrew formula update.

Pull-request CI builds and installs wheels for macOS ARM64/Intel, Linux ARM64/x86_64, and Windows x86_64. Release and publishing jobs run only after a qualifying push to `master`.

## Documentation

Keep the README focused on discovery and first use. Put detailed user workflows in [usage](usage.md), installation-specific material in [installation](installation.md), and implementation details in [how it works](how-it-works.md).
