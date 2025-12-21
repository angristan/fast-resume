"""Configuration and constants for fast-resume."""

from pathlib import Path

# Agent colors and badges (badge is the display name shown in UI)
AGENTS = {
    "claude": {"color": "#E87B35", "badge": "Claude Code"},
    "codex": {"color": "#00A67E", "badge": "Codex CLI"},
    "opencode": {"color": "#CFCECD", "badge": "OpenCode"},
    "vibe": {"color": "#FF6B35", "badge": "Vibe"},
    "crush": {"color": "#6B51FF", "badge": "Crush"},
    "copilot-cli": {"color": "#9CA3AF", "badge": "Copilot CLI"},
    "copilot-vscode": {"color": "#007ACC", "badge": "VS Code Copilot"},
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
SCHEMA_VERSION = 13  # Bump when schema changes

# Search settings
MAX_PREVIEW_LENGTH = 500
