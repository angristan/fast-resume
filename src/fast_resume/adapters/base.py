"""Base protocol for agent adapters."""

from dataclasses import dataclass
from datetime import datetime
from typing import Protocol


@dataclass
class Session:
    """Represents a coding agent session."""

    id: str
    agent: str  # "claude", "codex", "crush", "opencode", "vibe"
    title: str
    directory: str
    timestamp: datetime
    preview: str  # First ~500 chars of content for display
    content: str  # Full searchable content
    message_count: int = 0  # Number of user + assistant messages
    mtime: float = 0.0  # File modification time for incremental updates


class AgentAdapter(Protocol):
    """Protocol for agent-specific session adapters."""

    name: str
    color: str
    badge: str

    def find_sessions(self) -> list[Session]:
        """Find all sessions for this agent."""
        ...

    def find_sessions_incremental(
        self, known: dict[str, tuple[float, str]]
    ) -> tuple[list[Session], list[str]]:
        """Find sessions incrementally, comparing against known sessions.

        Args:
            known: Dict mapping session_id to (mtime, agent_name) tuple

        Returns:
            Tuple of (new_or_modified sessions, deleted session IDs)
        """
        ...

    def get_resume_command(self, session: "Session") -> list[str]:
        """Get the command to resume a session."""
        ...

    def is_available(self) -> bool:
        """Check if this agent's data directory exists."""
        ...
