"""Persistent user settings for fast-resume.

Settings are stored as a small JSON file in the cache directory. Reads and writes
are best-effort: a missing or corrupt file falls back to defaults, and write
failures are silently ignored so the TUI never breaks over a settings problem.
"""

import json
from pathlib import Path

from .config import CACHE_DIR

SETTINGS_FILE = CACHE_DIR / "settings.json"

# Default values for all known settings. Loaded settings are merged on top of
# these, so missing keys always resolve to a sensible default.
DEFAULTS: dict = {
    "preview_height": 12,
}


def load_settings(path: Path = SETTINGS_FILE) -> dict:
    """Load user settings, falling back to defaults for missing/invalid data."""
    merged = dict(DEFAULTS)
    try:
        data = json.loads(path.read_text())
    except (OSError, ValueError):
        return merged
    if isinstance(data, dict):
        merged.update(data)
    return merged


def save_settings(settings: dict, path: Path = SETTINGS_FILE) -> None:
    """Persist user settings to disk (best-effort)."""
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(settings))
    except OSError:
        pass
