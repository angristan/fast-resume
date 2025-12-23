"""Base protocol and abstract class for agent adapters."""

from abc import ABC, abstractmethod
from collections.abc import Callable
from dataclasses import dataclass
from datetime import datetime
import logging
from pathlib import Path
from typing import Protocol

logger = logging.getLogger(__name__)

# 1ms tolerance for mtime comparison due to datetime precision loss
MTIME_TOLERANCE = 0.001


def truncate_title(text: str, max_length: int = 100, word_break: bool = True) -> str:
    """Truncate title text with optional word-break.

    Args:
        text: The text to truncate
        max_length: Maximum length before truncation (default 100)
        word_break: If True, break at last word boundary; if False, hard truncate

    Returns:
        Truncated title with "..." suffix if text exceeded max_length
    """
    text = text.strip()
    if len(text) <= max_length:
        return text

    truncated = text[:max_length]
    if word_break:
        # Break at last word boundary
        truncated = truncated.rsplit(" ", 1)[0]
    return truncated + "..."


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
    yolo: bool = False  # Session was started with auto-approve/skip-permissions


@dataclass
class RawAdapterStats:
    """Raw statistics from an adapter's data folder."""

    agent: str
    data_dir: str  # Path to the data directory
    available: bool  # Whether the data directory exists
    file_count: int  # Number of session files
    total_bytes: int  # Total size in bytes


@dataclass
class ParseError:
    """Represents a session parsing error."""

    agent: str  # Which adapter encountered the error
    file_path: str  # Path to the problematic file
    error_type: str  # e.g., "JSONDecodeError", "KeyError", "OSError"
    message: str  # Human-readable error message


# Type alias for error callback
ErrorCallback = Callable[["ParseError"], None] | None


class AgentAdapter(Protocol):
    """Protocol for agent-specific session adapters."""

    name: str
    color: str
    badge: str

    def find_sessions(self) -> list[Session]:
        """Find all sessions for this agent."""
        ...

    def find_sessions_incremental(
        self,
        known: dict[str, tuple[float, str]],
        on_error: ErrorCallback = None,
    ) -> tuple[list[Session], list[str]]:
        """Find sessions incrementally, comparing against known sessions.

        Args:
            known: Dict mapping session_id to (mtime, agent_name) tuple
            on_error: Optional callback for parse errors

        Returns:
            Tuple of (new_or_modified sessions, deleted session IDs)
        """
        ...

    def get_resume_command(self, session: "Session", yolo: bool = False) -> list[str]:
        """Get the command to resume a session.

        Args:
            session: The session to resume
            yolo: If True, add auto-approve/skip-permissions flags
        """
        ...

    def is_available(self) -> bool:
        """Check if this agent's data directory exists."""
        ...

    def get_raw_stats(self) -> RawAdapterStats:
        """Get raw statistics from the adapter's data folder."""
        ...

    @property
    def supports_yolo(self) -> bool:
        """Whether this adapter supports yolo mode in resume command."""
        ...


class BaseSessionAdapter(ABC):
    """Base class for file-based session adapters.

    Provides a template method for find_sessions_incremental().
    Used by Claude, Copilot, Codex, and Vibe adapters.

    Subclasses implement:
        - _scan_session_files(): Return dict of session_id -> (path, mtime)
        - _parse_session_file(): Parse a single file into a Session
        - find_sessions(): Find all sessions
        - get_resume_command(): Command to resume a session
    """

    name: str
    color: str
    badge: str
    _sessions_dir: Path

    def is_available(self) -> bool:
        """Check if data directory exists."""
        return self._sessions_dir.exists()

    @property
    def supports_yolo(self) -> bool:
        """Whether this adapter supports yolo mode in resume command.

        Override in subclasses that support yolo flags.
        """
        return False

    @abstractmethod
    def _scan_session_files(self) -> dict[str, tuple[Path, float]]:
        """Scan session files and return current state.

        Returns:
            Dict mapping session_id to (file_path, mtime) tuple
        """
        ...

    @abstractmethod
    def _parse_session_file(
        self, session_file: Path, on_error: ErrorCallback = None
    ) -> Session | None:
        """Parse a single session file.

        Args:
            session_file: Path to the session file
            on_error: Optional callback for parse errors

        Returns:
            Session object or None if parsing failed
        """
        ...

    @abstractmethod
    def find_sessions(self) -> list[Session]:
        """Find all sessions for this agent."""
        ...

    @abstractmethod
    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to resume a session."""
        ...

    def find_sessions_incremental(
        self,
        known: dict[str, tuple[float, str]],
        on_error: ErrorCallback = None,
    ) -> tuple[list[Session], list[str]]:
        """Find sessions incrementally using template method pattern.

        Args:
            known: Dict mapping session_id to (mtime, agent_name) tuple
            on_error: Optional callback for parse errors

        Returns:
            Tuple of (new_or_modified sessions, deleted session IDs)
        """
        if not self.is_available():
            # All known sessions from this agent are deleted
            deleted_ids = [
                sid for sid, (_, agent) in known.items() if agent == self.name
            ]
            return [], deleted_ids

        # Scan current session files
        current_files = self._scan_session_files()

        # Find new and modified sessions
        new_or_modified = []
        for session_id, (path, mtime) in current_files.items():
            known_entry = known.get(session_id)
            if known_entry is None or mtime > known_entry[0] + MTIME_TOLERANCE:
                session = self._parse_session_file(path, on_error=on_error)
                if session:
                    session.mtime = mtime
                    new_or_modified.append(session)

        # Find deleted sessions (in known but not in current, for this agent only)
        current_ids = set(current_files.keys())
        deleted_ids = [
            sid
            for sid, (_, agent) in known.items()
            if agent == self.name and sid not in current_ids
        ]

        return new_or_modified, deleted_ids

    def get_raw_stats(self) -> RawAdapterStats:
        """Get raw statistics from the adapter's data folder."""
        if not self.is_available():
            return RawAdapterStats(
                agent=self.name,
                data_dir=str(self._sessions_dir),
                available=False,
                file_count=0,
                total_bytes=0,
            )

        # Use _scan_session_files to get file info
        files = self._scan_session_files()
        total_bytes = 0
        for path, _ in files.values():
            try:
                total_bytes += path.stat().st_size
            except OSError:
                pass

        return RawAdapterStats(
            agent=self.name,
            data_dir=str(self._sessions_dir),
            available=True,
            file_count=len(files),
            total_bytes=total_bytes,
        )
