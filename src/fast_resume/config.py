"""Configuration and constants for fast-resume."""

from pathlib import Path

# Agent colors and badges
AGENTS = {
    "claude": {"color": "#E87B35", "badge": "claude"},
    "codex": {"color": "#00A67E", "badge": "codex"},
    "opencode": {"color": "#CFCECD", "badge": "opencode"},
    "vibe": {"color": "#FF6B35", "badge": "vibe"},
    "crush": {"color": "#6B51FF", "badge": "crush"},
    "copilot": {"color": "#9CA3AF", "badge": "copilot"},
}

# Storage paths
CLAUDE_DIR = Path.home() / ".claude" / "projects"
CODEX_DIR = Path.home() / ".codex" / "sessions"
OPENCODE_DIR = Path.home() / ".local" / "share" / "opencode" / "storage"
VIBE_DIR = Path.home() / ".vibe" / "logs" / "session"
CRUSH_PROJECTS_FILE = Path.home() / ".local" / "share" / "crush" / "projects.json"
COPILOT_DIR = Path.home() / ".copilot" / "session-state"

# Storage location
INDEX_DIR = Path.home() / ".cache" / "fast-resume" / "tantivy_index"
SCHEMA_VERSION = 11  # Bump when schema changes

# Search settings
MAX_PREVIEW_LENGTH = 500
