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

## Nix

Run the source package without installing it:

```bash
nix run github:angristan/fast-resume
```

Install it into a user profile:

```bash
nix profile install github:angristan/fast-resume
```

For a declarative NixOS configuration, add fast-resume as a flake input:

```nix
inputs.fast-resume.url = "github:angristan/fast-resume";
```

Then include its package in the host module where `inputs` is available:

```nix
environment.systemPackages = [
  inputs.fast-resume.packages.${pkgs.system}.default
];
```

The flake builds `fr` from the committed Rust source and `Cargo.lock`. It
supports Linux ARM64/x86_64 and macOS Apple Silicon. Current Nixpkgs no longer
supports Intel macOS; use Homebrew, npm, or PyPI there.

The consumer's lock file pins both fast-resume and its tested Nixpkgs revision.
Advanced configurations can make the input follow the host's Nixpkgs after
verifying that its Rust toolchain satisfies fast-resume's minimum version.

## npm

Install globally with npm:

```bash
npm install --global fast-resume
fr
```

Or run without a permanent installation:

```bash
npx fast-resume
```

npm provides native binaries for macOS ARM64 and Intel, and Linux glibc ARM64 and x86_64.

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

The first launch scans all supported local agent stores and builds a Tantivy index under the XDG cache directory:

```text
$XDG_CACHE_HOME/fast-resume/tantivy_index
```

`XDG_CACHE_HOME` must be an absolute path. Fast-resume uses `~/.cache/fast-resume/tantivy_index` when the variable is unset or invalid.

Later launches search the existing index immediately and refresh changed sessions in the background. If an upgrade changes the index schema, fast-resume automatically discards the incompatible cache and rebuilds it.

To force a clean rebuild:

```bash
rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/fast-resume"
fr --rebuild
```

## Next steps

Continue with the [usage guide](usage.md), or read [how indexing and resume handoff work](how-it-works.md).
