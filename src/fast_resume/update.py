"""Helpers for update notifications."""

from __future__ import annotations

import shutil
import sys
from collections.abc import Iterable
from pathlib import Path

PACKAGE_NAME = "fast-resume"


def _path_variants(path: Path) -> list[Path]:
    """Return the raw path and its symlink-resolved form when available."""
    paths = [path]
    try:
        resolved = path.resolve()
    except OSError:
        return paths
    if resolved != path:
        paths.append(resolved)
    return paths


def executable_paths() -> list[Path]:
    """Return candidate paths that can reveal how fast-resume was installed."""
    candidates: list[Path] = []

    for value in (sys.argv[0], sys.executable):
        if value:
            candidates.extend(_path_variants(Path(value).expanduser()))

    for executable in ("fr", "fast-resume"):
        path = shutil.which(executable)
        if path:
            candidates.extend(_path_variants(Path(path).expanduser()))

    return candidates


def _has_ordered_parts(parts: tuple[str, ...], expected: tuple[str, ...]) -> bool:
    return any(parts[i : i + len(expected)] == expected for i in range(len(parts)))


def detect_install_source(paths: Iterable[Path] | None = None) -> str | None:
    """Best-effort install source detection from executable path shapes."""
    candidates = list(paths) if paths is not None else executable_paths()

    for path in candidates:
        parts = path.parts
        if _has_ordered_parts(parts, ("Cellar", PACKAGE_NAME)) or _has_ordered_parts(
            parts, ("opt", PACKAGE_NAME)
        ):
            return "homebrew"

    for path in candidates:
        parts = path.parts
        if _has_ordered_parts(parts, ("uv", "tools", PACKAGE_NAME)):
            return "uv"

    for path in candidates:
        parts = path.parts
        if _has_ordered_parts(parts, ("pipx", "venvs", PACKAGE_NAME)):
            return "pipx"

    return None


def update_command(paths: Iterable[Path] | None = None) -> str | None:
    """Return the package-manager command for updating fast-resume."""
    source = detect_install_source(paths)
    if source == "homebrew":
        return f"brew upgrade {PACKAGE_NAME}"
    if source == "uv":
        return f"uv tool upgrade {PACKAGE_NAME}"
    if source == "pipx":
        return f"pipx upgrade {PACKAGE_NAME}"
    return None


def update_instruction(
    paths: Iterable[Path] | None = None, *, markup: bool = False
) -> str:
    """Return a user-facing update instruction."""
    command = update_command(paths)
    if command is None:
        return "Update with the package manager used to install fast-resume."
    if markup:
        return f"Run [bold]{command}[/bold] to update"
    return f"Run: {command}"
