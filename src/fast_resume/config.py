"""Configuration and constants for fast-resume."""

from pathlib import Path

# Agent colors and badges (badge is the display name shown in UI)
AGENTS = {
    "claude": {"color": "#E87B35", "badge": "claude"},
    "codex": {"color": "#00A67E", "badge": "codex"},
    "opencode": {"color": "#CFCECD", "badge": "opencode"},
    "vibe": {"color": "#FF6B35", "badge": "vibe"},
    "crush": {"color": "#6B51FF", "badge": "crush"},
    "copilot-cli": {"color": "#9CA3AF", "badge": "copilot"},
    "copilot-vscode": {"color": "#007ACC", "badge": "vscode"},
}

# Storage paths
CLAUDE_DIR = Path.home() / ".claude" / "projects"
CODEX_DIR = Path.home() / ".codex" / "sessions"
OPENCODE_DIR = Path.home() / ".local" / "share" / "opencode" / "storage"
VIBE_DIR = Path.home() / ".vibe" / "logs" / "session"
CRUSH_PROJECTS_FILE = Path.home() / ".local" / "share" / "crush" / "projects.json"
COPILOT_DIR = Path.home() / ".copilot" / "session-state"

# Storage location
CACHE_DIR = Path.home() / ".cache" / "fast-resume"
INDEX_DIR = CACHE_DIR / "tantivy_index"
LOG_FILE = CACHE_DIR / "parse-errors.log"
SCHEMA_VERSION = 16  # Bump when schema changes (16: raw tokenizer for agent)

# Search settings
MAX_PREVIEW_LENGTH = 500
