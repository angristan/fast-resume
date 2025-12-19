"""OpenCode session adapter."""

import json
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, MAX_CONTENT_LENGTH, MAX_PREVIEW_LENGTH, OPENCODE_DIR
from .base import Session


class OpenCodeAdapter:
    """Adapter for OpenCode sessions."""

    name = "opencode"
    color = AGENTS["opencode"]["color"]
    badge = AGENTS["opencode"]["badge"]

    def is_available(self) -> bool:
        """Check if OpenCode data directory exists."""
        return OPENCODE_DIR.exists()

    def find_sessions(self) -> list[Session]:
        """Find all OpenCode sessions."""
        if not self.is_available():
            return []

        sessions = []
        session_dir = OPENCODE_DIR / "session"
        message_dir = OPENCODE_DIR / "message"
        part_dir = OPENCODE_DIR / "part"

        if not session_dir.exists():
            return []

        # OpenCode stores sessions in project-hash subdirectories
        for project_dir in session_dir.iterdir():
            if not project_dir.is_dir():
                continue

            for session_file in project_dir.glob("ses_*.json"):
                session = self._parse_session(
                    session_file, message_dir, part_dir
                )
                if session:
                    sessions.append(session)

        return sessions

    def _parse_session(
        self, session_file: Path, message_dir: Path, part_dir: Path
    ) -> Session | None:
        """Parse an OpenCode session file."""
        try:
            with open(session_file, "r", encoding="utf-8") as f:
                data = json.load(f)

            session_id = data.get("id", "")
            title = data.get("title", "Untitled session")
            directory = data.get("directory", "")

            # Parse timestamp from milliseconds
            time_data = data.get("time", {})
            created = time_data.get("created", 0)
            if created:
                timestamp = datetime.fromtimestamp(created / 1000)
            else:
                timestamp = datetime.fromtimestamp(session_file.stat().st_mtime)

            # Get message content from parts
            messages = self._get_session_messages(session_id, message_dir, part_dir)

            full_content = "\n".join(messages)[:MAX_CONTENT_LENGTH]
            preview = full_content[:MAX_PREVIEW_LENGTH]

            return Session(
                id=session_id,
                agent=self.name,
                title=title,
                directory=directory,
                timestamp=timestamp,
                preview=preview,
                content=full_content,
            )
        except Exception:
            return None

    def _get_session_messages(
        self, session_id: str, message_dir: Path, part_dir: Path
    ) -> list[str]:
        """Get all messages for a session from parts."""
        messages: list[str] = []

        # Find message directory for this session
        session_msg_dir = message_dir / session_id
        if not session_msg_dir.exists():
            return messages

        # Get all message files
        for msg_file in session_msg_dir.glob("msg_*.json"):
            try:
                with open(msg_file, "r", encoding="utf-8") as f:
                    msg_data = json.load(f)

                msg_id = msg_data.get("id", "")
                if not msg_id:
                    continue

                # Get parts for this message
                msg_part_dir = part_dir / msg_id
                if msg_part_dir.exists():
                    for part_file in sorted(msg_part_dir.glob("*.json")):
                        try:
                            with open(part_file, "r", encoding="utf-8") as f:
                                part_data = json.load(f)

                            part_type = part_data.get("type", "")
                            if part_type == "text":
                                text = part_data.get("text", "")
                                if text:
                                    messages.append(text)
                        except Exception:
                            continue
            except Exception:
                continue

        return messages

    def get_resume_command(self, session: Session) -> list[str]:
        """Get command to resume an OpenCode session."""
        return ["opencode", session.directory, "--session", session.id]
