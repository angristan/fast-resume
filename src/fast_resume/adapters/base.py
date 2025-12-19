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


class AgentAdapter(Protocol):
    """Protocol for agent-specific session adapters."""

    name: str
    color: str
    badge: str

    def find_sessions(self) -> list[Session]:
        """Find all sessions for this agent."""
        ...

    def get_resume_command(self, session: "Session") -> list[str]:
        """Get the command to resume a session."""
        ...

    def is_available(self) -> bool:
        """Check if this agent's data directory exists."""
        ...
