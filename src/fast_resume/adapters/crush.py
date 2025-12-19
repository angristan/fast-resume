"""Crush (charmbracelet) session adapter."""

import json
import sqlite3
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, CRUSH_PROJECTS_FILE, MAX_CONTENT_LENGTH, MAX_PREVIEW_LENGTH
from .base import Session


class CrushAdapter:
    """Adapter for Crush sessions."""

    name = "crush"
    color = AGENTS["crush"]["color"]
    badge = AGENTS["crush"]["badge"]

    def is_available(self) -> bool:
        """Check if Crush projects file exists."""
        return CRUSH_PROJECTS_FILE.exists()

    def find_sessions(self) -> list[Session]:
        """Find all Crush sessions across all projects."""
        if not self.is_available():
            return []

        sessions = []

        try:
            with open(CRUSH_PROJECTS_FILE, "r", encoding="utf-8") as f:
                projects_data = json.load(f)
        except (json.JSONDecodeError, OSError):
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

    def _load_sessions_from_db(self, db_path: Path, project_path: str) -> list[Session]:
        """Load sessions from a Crush SQLite database."""
        sessions = []

        try:
            conn = sqlite3.connect(str(db_path), timeout=5)
            conn.row_factory = sqlite3.Row
            cursor = conn.cursor()

            # Get all sessions
            cursor.execute("""
                SELECT id, title, message_count, updated_at, created_at
                FROM sessions
                WHERE message_count > 0
                ORDER BY updated_at DESC
            """)

            for row in cursor.fetchall():
                session = self._parse_session(
                    conn, row, project_path
                )
                if session:
                    sessions.append(session)

            conn.close()
        except sqlite3.Error:
            pass

        return sessions

    def _parse_session(
        self, conn: sqlite3.Connection, session_row: sqlite3.Row, project_path: str
    ) -> Session | None:
        """Parse a single session from the database."""
        try:
            session_id = session_row["id"]
            title = session_row["title"] or ""

            # Timestamps are in Unix seconds (or milliseconds - need to detect)
            updated_at = session_row["updated_at"]
            created_at = session_row["created_at"]

            # Detect if timestamp is in milliseconds (> year 3000 in seconds)
            if updated_at > 100_000_000_000:
                updated_at = updated_at / 1000
            if created_at > 100_000_000_000:
                created_at = created_at / 1000

            timestamp = datetime.fromtimestamp(updated_at)

            # Get messages for this session
            cursor = conn.cursor()
            cursor.execute("""
                SELECT role, parts, model
                FROM messages
                WHERE session_id = ?
                ORDER BY created_at ASC
            """, (session_id,))

            messages: list[str] = []
            first_user_message = ""

            for msg_row in cursor.fetchall():
                role = msg_row["role"]
                parts_json = msg_row["parts"]

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
                title = first_user_message.strip()[:100]
                if len(first_user_message) > 100:
                    title = title.rsplit(" ", 1)[0] + "..."

            full_content = "\n\n".join(messages)[:MAX_CONTENT_LENGTH]
            preview = full_content[:MAX_PREVIEW_LENGTH]

            return Session(
                id=session_id,
                agent=self.name,
                title=title,
                directory=project_path,
                timestamp=timestamp,
                preview=preview,
                content=full_content,
            )
        except Exception:
            return None

    def _extract_text_from_parts(self, parts_json: str) -> str:
        """Extract text content from message parts JSON."""
        try:
            parts = json.loads(parts_json)
        except json.JSONDecodeError:
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

    def get_resume_command(self, session: Session) -> list[str]:
        """Get command to resume a Crush session."""
        # Crush is interactive - it shows a session picker when launched in a project directory
        # fast-resume changes to session.directory before executing this command
        return ["crush"]
