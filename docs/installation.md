# Installation

## Terminal support

[Ghostty](https://ghostty.org/) provides the best-tested experience, including terminal images. Other modern terminals work, but image protocols and some interactive behavior vary by terminal.

Use `fr --no-images` if artwork does not render correctly, or select a protocol explicitly with `--image-protocol kitty`, `sixel`, or `iterm2`.

## Homebrew

Homebrew packages are available for macOS and Linux:

```bash
brew tap angristan/tap
brew install fast-resume
```

Upgrade with:

```bash
brew update
brew upgrade fast-resume
```

## PyPI with uv

Run without installing:

```bash
uvx --from fast-resume fr
```

Or install permanently:

```bash
uv tool install fast-resume
fr
```

PyPI publishes Rust binary wheels for:

- macOS Apple Silicon and Intel
- Linux ARM64 and x86_64
- Windows x86_64

No source distribution is published yet. Platforms without a wheel should install through Cargo.

## Cargo

Install directly from the Git repository:

```bash
cargo install --locked --git https://github.com/angristan/fast-resume
fr
```

## Commands

The primary command is `fr`. The `fast-resume` command remains available as a compatibility wrapper.

Verify an installation with:

```bash
fr --version
fr --help
```

## First launch and upgrades

The first launch scans all supported local agent stores and builds a Tantivy index under:

```text
~/.cache/fast-resume/tantivy_index
```

Later launches search the existing index immediately and refresh changed sessions in the background. If an upgrade changes the index schema, fast-resume automatically discards the incompatible cache and rebuilds it.

To force a clean rebuild:

```bash
rm -rf ~/.cache/fast-resume
fr --rebuild
```

## Next steps

Continue with the [usage guide](usage.md), or read [how indexing and resume handoff work](how-it-works.md).
