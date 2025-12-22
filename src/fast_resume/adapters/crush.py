"""Crush (charmbracelet) session adapter."""

import orjson
import sqlite3
from collections import defaultdict
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, CRUSH_PROJECTS_FILE, MAX_PREVIEW_LENGTH
from ..logging_config import log_parse_error
from .base import ErrorCallback, ParseError, RawAdapterStats, Session, truncate_title


class CrushAdapter:
    """Adapter for Crush sessions."""

    name = "crush"
    color = AGENTS["crush"]["color"]
    badge = AGENTS["crush"]["badge"]

    def __init__(self, projects_file: Path | None = None) -> None:
        self._projects_file = (
            projects_file if projects_file is not None else CRUSH_PROJECTS_FILE
        )

    def is_available(self) -> bool:
        """Check if Crush projects file exists."""
        return self._projects_file.exists()

    def find_sessions(self) -> list[Session]:
        """Find all Crush sessions across all projects."""
        if not self.is_available():
            return []

        sessions = []

        try:
            with open(self._projects_file, "rb") as f:
                projects_data = orjson.loads(f.read())
        except (orjson.JSONDecodeError, OSError):
            return []

        for project in projects_data.get("projects", []):
            project_path = project.get("path", "")
            data_dir = project.get("data_dir", "")

            if not data_dir:
                continue

            db_path = Path(data_dir) / "crush.db"
            if not db_path.exists():
                continue

            project_sessions = self._load_sessions_from_db(db_path, project_path)
            sessions.extend(project_sessions)

        return sessions

    def _load_sessions_from_db(
        self, db_path: Path, project_path: str, on_error: ErrorCallback = None
    ) -> list[Session]:
        """Load sessions from a Crush SQLite database."""
        sessions = []

        try:
            conn = sqlite3.connect(str(db_path), timeout=5)
            conn.row_factory = sqlite3.Row
            cursor = conn.cursor()

            cursor.execute("""
                SELECT
                    s.id, s.title, s.message_count, s.updated_at, s.created_at,
                    m.role, m.parts, m.created_at as msg_created_at
                FROM sessions s
                LEFT JOIN messages m ON m.session_id = s.id
                WHERE s.message_count > 0
                ORDER BY s.updated_at DESC, m.created_at ASC
            """)

            # Group messages by session
            session_data: dict[str, dict] = {}
            session_messages: dict[str, list[tuple[str, str]]] = defaultdict(list)

            for row in cursor.fetchall():
                session_id = row["id"]

                # Store session metadata (first occurrence)
                if session_id not in session_data:
                    session_data[session_id] = {
                        "title": row["title"] or "",
                        "updated_at": row["updated_at"],
                        "created_at": row["created_at"],
                    }

                # Collect messages
                if row["role"] is not None:
                    session_messages[session_id].append((row["role"], row["parts"]))

            conn.close()

            # Build Session objects
            for session_id, data in session_data.items():
                session = self._build_session(
                    session_id,
                    data,
                    session_messages[session_id],
                    project_path,
                    on_error=on_error,
                )
                if session:
                    sessions.append(session)

        except sqlite3.Error as e:
            error = ParseError(
                agent=self.name,
                file_path=str(db_path),
                error_type="sqlite3.Error",
                message=str(e),
            )
            log_parse_error(
                error.agent, error.file_path, error.error_type, error.message
            )
            if on_error:
                on_error(error)

        return sessions

    def _build_session(
        self,
        session_id: str,
        data: dict,
        messages_raw: list[tuple[str, str]],
        project_path: str,
        on_error: ErrorCallback = None,
    ) -> Session | None:
        """Build a Session object from pre-fetched data."""
        try:
            title = data["title"]
            updated_at = data["updated_at"]
            created_at = data["created_at"]

            # Detect if timestamp is in milliseconds (> year 3000 in seconds)
            if updated_at > 100_000_000_000:
                updated_at = updated_at / 1000
            if created_at > 100_000_000_000:
                created_at = created_at / 1000

            timestamp = datetime.fromtimestamp(updated_at)

            messages: list[str] = []
            first_user_message = ""

            for role, parts_json in messages_raw:
                text_content = self._extract_text_from_parts(parts_json)
                if not text_content:
                    continue

                role_prefix = "» " if role == "user" else "  "
                messages.append(f"{role_prefix}{text_content}")

                if role == "user" and not first_user_message and len(text_content) > 5:
                    first_user_message = text_content

            # Skip sessions with no actual content
            if not messages or not first_user_message:
                return None

            # Use first user message as title if none set
            if not title:
                title = truncate_title(first_user_message)

            full_content = "\n\n".join(messages)
            preview = full_content[:MAX_PREVIEW_LENGTH]

            return Session(
                id=session_id,
                agent=self.name,
                title=title,
                directory=project_path,
                timestamp=timestamp,
                preview=preview,
                content=full_content,
                message_count=len(messages),
            )
        except (KeyError, TypeError, AttributeError, ValueError) as e:
            error = ParseError(
                agent=self.name,
                file_path=f"crush_db:{session_id}",
                error_type=type(e).__name__,
                message=str(e),
            )
            log_parse_error(
                error.agent, error.file_path, error.error_type, error.message
            )
            if on_error:
                on_error(error)
            return None

    def _extract_text_from_parts(self, parts_json: str) -> str:
        """Extract text content from message parts JSON."""
        try:
            parts = orjson.loads(parts_json)
        except orjson.JSONDecodeError:
            return ""

        text_parts = []
        for part in parts:
            if not isinstance(part, dict):
                continue

            part_type = part.get("type", "")
            data = part.get("data", {})

            if part_type == "text" and isinstance(data, dict):
                text = data.get("text", "")
                if text:
                    text_parts.append(text)
            elif part_type == "tool_result" and isinstance(data, dict):
                # Include tool results for context
                content = data.get("content", "")
                if content and len(content) < 500:  # Skip long tool outputs
                    text_parts.append(f"[{data.get('name', 'tool')}]: {content[:200]}")
            elif part_type == "tool_call" and isinstance(data, dict):
                # Include tool calls for context
                name = data.get("name", "")
                if name:
                    text_parts.append(f"[calling {name}]")

        return " ".join(text_parts)

    def find_sessions_incremental(
        self,
        known: dict[str, tuple[float, str]],
        on_error: ErrorCallback = None,
    ) -> tuple[list[Session], list[str]]:
        """Find sessions incrementally, comparing against known sessions."""
        if not self.is_available():
            deleted_ids = [
                sid for sid, (_, agent) in known.items() if agent == self.name
            ]
            return [], deleted_ids

        try:
            with open(self._projects_file, "rb") as f:
                projects_data = orjson.loads(f.read())
        except (orjson.JSONDecodeError, OSError):
            deleted_ids = [
                sid for sid, (_, agent) in known.items() if agent == self.name
            ]
            return [], deleted_ids

        # For Crush, we track db file mtimes and session IDs within
        # When a db changes, we reload all sessions from it and diff
        new_or_modified = []
        all_current_ids: set[str] = set()

        for project in projects_data.get("projects", []):
            project_path = project.get("path", "")
            data_dir = project.get("data_dir", "")

            if not data_dir:
                continue

            db_path = Path(data_dir) / "crush.db"
            if not db_path.exists():
                continue

            # Load all sessions from this db
            project_sessions = self._load_sessions_from_db(
                db_path, project_path, on_error=on_error
            )

            for session in project_sessions:
                all_current_ids.add(session.id)
                known_entry = known.get(session.id)
                # Use session timestamp for comparison since db doesn't have file mtime
                # Use 1ms tolerance for comparison due to datetime precision loss
                session_mtime = session.timestamp.timestamp()
                if known_entry is None or session_mtime > known_entry[0] + 0.001:
                    session.mtime = session_mtime
                    new_or_modified.append(session)

        # Find deleted sessions
        deleted_ids = [
            sid
            for sid, (_, agent) in known.items()
            if agent == self.name and sid not in all_current_ids
        ]

        return new_or_modified, deleted_ids

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to resume a Crush session."""
        # Crush is interactive - it shows a session picker when launched in a project directory
        # fast-resume changes to session.directory before executing this command
        return ["crush"]

    def get_raw_stats(self) -> RawAdapterStats:
        """Get raw statistics from Crush database files."""
        if not self.is_available():
            return RawAdapterStats(
                agent=self.name,
                data_dir=str(self._projects_file.parent),
                available=False,
                file_count=0,
                total_bytes=0,
            )

        try:
            with open(self._projects_file, "rb") as f:
                projects_data = orjson.loads(f.read())
        except (orjson.JSONDecodeError, OSError):
            return RawAdapterStats(
                agent=self.name,
                data_dir=str(self._projects_file.parent),
                available=True,
                file_count=0,
                total_bytes=0,
            )

        file_count = 0
        total_bytes = 0

        for project in projects_data.get("projects", []):
            data_dir = project.get("data_dir", "")
            if not data_dir:
                continue

            db_path = Path(data_dir) / "crush.db"
            if db_path.exists():
                try:
                    file_count += 1
                    total_bytes += db_path.stat().st_size
                except OSError:
                    pass

        return RawAdapterStats(
            agent=self.name,
            data_dir=str(self._projects_file.parent),
            available=True,
            file_count=file_count,
            total_bytes=total_bytes,
        )
