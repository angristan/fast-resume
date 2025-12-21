"""Vibe (Mistral) session adapter."""

import orjson
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, MAX_PREVIEW_LENGTH, VIBE_DIR
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
            with open(session_file, "rb") as f:
                data = orjson.loads(f.read())

            metadata = data.get("metadata", {})
            session_id = metadata.get("session_id", session_file.stem)

            # Get directory from environment
            env = metadata.get("environment", {})
            directory = env.get("working_directory", "")

            # Check if session was started with auto_approve
            yolo = metadata.get("auto_approve", False)

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
                m for i, m in enumerate(messages_data) if m.get("role") == "user"
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

            full_content = "\n\n".join(messages)
            preview = full_content[:MAX_PREVIEW_LENGTH]

            return Session(
                id=session_id,
                agent=self.name,
                title=title,
                directory=directory,
                timestamp=timestamp,
                preview=preview,
                content=full_content,
                message_count=len(messages),
                yolo=yolo,
            )
        except Exception:
            return None

    def find_sessions_incremental(
        self, known: dict[str, tuple[float, str]]
    ) -> tuple[list[Session], list[str]]:
        """Find sessions incrementally, comparing against known sessions."""
        if not self.is_available():
            deleted_ids = [
                sid for sid, (_, agent) in known.items() if agent == self.name
            ]
            return [], deleted_ids

        # Scan all session files and build current state
        # For Vibe, we use the 'start_time' timestamp from the file content
        # (not file mtime) because that's what we store in the index
        current_files: dict[str, tuple[Path, float]] = {}

        for session_file in VIBE_DIR.glob("session_*.json"):
            try:
                with open(session_file, "rb") as f:
                    data = orjson.loads(f.read())
                metadata = data.get("metadata", {})
                session_id = metadata.get("session_id", session_file.stem)

                # Use start_time to match what _parse_session stores
                start_time = metadata.get("start_time", "")
                if start_time:
                    try:
                        mtime = datetime.fromisoformat(start_time).timestamp()
                    except ValueError:
                        mtime = session_file.stat().st_mtime
                else:
                    mtime = session_file.stat().st_mtime

                current_files[session_id] = (session_file, mtime)
            except Exception:
                continue

        # Find new and modified sessions
        # Use 1ms tolerance for mtime comparison due to datetime precision loss
        new_or_modified = []
        for session_id, (path, mtime) in current_files.items():
            known_entry = known.get(session_id)
            if known_entry is None or mtime > known_entry[0] + 0.001:
                session = self._parse_session(path)
                if session:
                    session.mtime = mtime
                    new_or_modified.append(session)

        # Find deleted sessions
        current_ids = set(current_files.keys())
        deleted_ids = [
            sid
            for sid, (_, agent) in known.items()
            if agent == self.name and sid not in current_ids
        ]

        return new_or_modified, deleted_ids

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to resume a Vibe session."""
        cmd = ["vibe"]
        if yolo:
            cmd.append("--auto-approve")
        cmd.extend(["--resume", session.id])
        return cmd
