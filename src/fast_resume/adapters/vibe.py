"""Vibe (Mistral) session adapter."""

import json
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, MAX_CONTENT_LENGTH, MAX_PREVIEW_LENGTH, VIBE_DIR
from .base import Session


class VibeAdapter:
    """Adapter for Vibe (Mistral) sessions."""

    name = "vibe"
    color = AGENTS["vibe"]["color"]
    badge = AGENTS["vibe"]["badge"]

    def is_available(self) -> bool:
        """Check if Vibe data directory exists."""
        return VIBE_DIR.exists()

    def find_sessions(self) -> list[Session]:
        """Find all Vibe sessions."""
        if not self.is_available():
            return []

        sessions = []
        for session_file in VIBE_DIR.glob("session_*.json"):
            session = self._parse_session(session_file)
            if session:
                sessions.append(session)

        return sessions

    def _parse_session(self, session_file: Path) -> Session | None:
        """Parse a Vibe session file."""
        try:
            with open(session_file, "r", encoding="utf-8") as f:
                data = json.load(f)

            metadata = data.get("metadata", {})
            session_id = metadata.get("session_id", session_file.stem)

            # Get directory from environment
            env = metadata.get("environment", {})
            directory = env.get("working_directory", "")

            # Parse timestamps
            start_time = metadata.get("start_time", "")
            if start_time:
                try:
                    timestamp = datetime.fromisoformat(start_time)
                except ValueError:
                    timestamp = datetime.fromtimestamp(session_file.stat().st_mtime)
            else:
                timestamp = datetime.fromtimestamp(session_file.stat().st_mtime)

            # Extract messages
            messages_data = data.get("messages", [])
            messages: list[str] = []

            for msg in messages_data:
                role = msg.get("role", "")
                content = msg.get("content", "")

                # Skip system messages
                if role == "system":
                    continue

                role_prefix = "» " if role == "user" else "  "

                if isinstance(content, str) and content:
                    messages.append(f"{role_prefix}{content}")
                elif isinstance(content, list):
                    for part in content:
                        if isinstance(part, dict):
                            text = part.get("text", "")
                            if text:
                                messages.append(f"{role_prefix}{text}")

            # Generate title from first user message
            user_messages = [
                m for i, m in enumerate(messages_data)
                if m.get("role") == "user"
            ]
            if user_messages:
                first_msg = user_messages[0].get("content", "")
                if isinstance(first_msg, str):
                    title = first_msg[:80]
                else:
                    title = "Vibe session"
            else:
                title = "Vibe session"

            if len(title) == 80:
                title += "..."

            full_content = "\n\n".join(messages)[:MAX_CONTENT_LENGTH]
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

    def get_resume_command(self, session: Session) -> list[str]:
        """Get command to resume a Vibe session."""
        return ["vibe", "--resume", session.id]
