"""OpenCode session adapter."""

import orjson
from collections import defaultdict
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, MAX_PREVIEW_LENGTH, OPENCODE_DIR
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

        # Pre-index all messages by session_id: {session_id: [(msg_file, msg_id, role), ...]}
        messages_by_session: dict[str, list[tuple[Path, str, str]]] = defaultdict(list)
        if message_dir.exists():
            for msg_file in message_dir.glob("*/msg_*.json"):
                try:
                    with open(msg_file, "rb") as f:
                        msg_data = orjson.loads(f.read())
                    session_id = msg_file.parent.name
                    msg_id = msg_data.get("id", "")
                    role = msg_data.get("role", "")
                    if msg_id:
                        messages_by_session[session_id].append((msg_file, msg_id, role))
                except Exception:
                    continue

        # Pre-index all parts by message_id: {msg_id: [text, ...]}
        parts_by_message: dict[str, list[str]] = defaultdict(list)
        if part_dir.exists():
            for part_file in sorted(part_dir.glob("*/*.json")):
                try:
                    with open(part_file, "rb") as f:
                        part_data = orjson.loads(f.read())
                    msg_id = part_file.parent.name
                    if part_data.get("type") == "text":
                        text = part_data.get("text", "")
                        if text:
                            parts_by_message[msg_id].append(text)
                except Exception:
                    continue

        # OpenCode stores sessions in project-hash subdirectories
        for project_dir in session_dir.iterdir():
            if not project_dir.is_dir():
                continue

            for session_file in project_dir.glob("ses_*.json"):
                session = self._parse_session(
                    session_file, messages_by_session, parts_by_message
                )
                if session:
                    sessions.append(session)

        return sessions

    def _parse_session(
        self,
        session_file: Path,
        messages_by_session: dict[str, list[tuple[Path, str, str]]],
        parts_by_message: dict[str, list[str]],
    ) -> Session | None:
        """Parse an OpenCode session file."""
        try:
            with open(session_file, "rb") as f:
                data = orjson.loads(f.read())

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

            # Get message content from pre-indexed data
            messages = self._get_session_messages(
                session_id, messages_by_session, parts_by_message
            )

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
            )
        except Exception:
            return None

    def _get_session_messages(
        self,
        session_id: str,
        messages_by_session: dict[str, list[tuple[Path, str, str]]],
        parts_by_message: dict[str, list[str]],
    ) -> list[str]:
        """Get all messages for a session from pre-indexed parts."""
        messages: list[str] = []

        # Sort by filename to maintain order
        session_msgs = sorted(
            messages_by_session.get(session_id, []), key=lambda x: x[0].name
        )

        for _msg_file, msg_id, role in session_msgs:
            role_prefix = "» " if role == "user" else "  "
            for text in parts_by_message.get(msg_id, []):
                messages.append(f"{role_prefix}{text}")

        return messages

    def find_sessions_incremental(
        self, known: dict[str, tuple[float, str]]
    ) -> tuple[list[Session], list[str]]:
        """Find sessions incrementally, comparing against known sessions."""
        if not self.is_available():
            deleted_ids = [
                sid for sid, (_, agent) in known.items() if agent == self.name
            ]
            return [], deleted_ids

        session_dir = OPENCODE_DIR / "session"
        if not session_dir.exists():
            deleted_ids = [
                sid for sid, (_, agent) in known.items() if agent == self.name
            ]
            return [], deleted_ids

        # Scan session files and get timestamps
        # For OpenCode, we use the 'created' timestamp from the file content
        # (not file mtime) because that's what we store in the index
        current_sessions: dict[str, tuple[Path, float]] = {}

        for project_dir in session_dir.iterdir():
            if not project_dir.is_dir():
                continue

            for session_file in project_dir.glob("ses_*.json"):
                try:
                    with open(session_file, "rb") as f:
                        data = orjson.loads(f.read())
                    session_id = data.get("id", "")
                    if session_id:
                        # Use created timestamp to match what _parse_session stores
                        created = data.get("time", {}).get("created", 0)
                        if created:
                            mtime = datetime.fromtimestamp(created / 1000).timestamp()
                        else:
                            mtime = session_file.stat().st_mtime
                        current_sessions[session_id] = (session_file, mtime)
                except Exception:
                    continue

        # Check which sessions need parsing
        # Use 1ms tolerance for mtime comparison due to datetime precision loss
        sessions_to_parse: list[tuple[str, Path]] = []
        for session_id, (path, mtime) in current_sessions.items():
            known_entry = known.get(session_id)
            if known_entry is None or mtime > known_entry[0] + 0.001:
                sessions_to_parse.append((session_id, path))

        # Find deleted sessions
        current_ids = set(current_sessions.keys())
        deleted_ids = [
            sid
            for sid, (_, agent) in known.items()
            if agent == self.name and sid not in current_ids
        ]

        if not sessions_to_parse:
            return [], deleted_ids

        # Build indexes only for sessions we need to parse
        message_dir = OPENCODE_DIR / "message"
        part_dir = OPENCODE_DIR / "part"

        messages_by_session: dict[str, list[tuple[Path, str, str]]] = defaultdict(list)
        if message_dir.exists():
            for msg_file in message_dir.glob("*/msg_*.json"):
                try:
                    with open(msg_file, "rb") as f:
                        msg_data = orjson.loads(f.read())
                    session_id = msg_file.parent.name
                    msg_id = msg_data.get("id", "")
                    role = msg_data.get("role", "")
                    if msg_id:
                        messages_by_session[session_id].append((msg_file, msg_id, role))
                except Exception:
                    continue

        parts_by_message: dict[str, list[str]] = defaultdict(list)
        if part_dir.exists():
            for part_file in sorted(part_dir.glob("*/*.json")):
                try:
                    with open(part_file, "rb") as f:
                        part_data = orjson.loads(f.read())
                    msg_id = part_file.parent.name
                    if part_data.get("type") == "text":
                        text = part_data.get("text", "")
                        if text:
                            parts_by_message[msg_id].append(text)
                except Exception:
                    continue

        # Parse the changed sessions
        new_or_modified = []
        for session_id, path in sessions_to_parse:
            session = self._parse_session(path, messages_by_session, parts_by_message)
            if session:
                new_or_modified.append(session)

        return new_or_modified, deleted_ids

    def get_resume_command(self, session: Session) -> list[str]:
        """Get command to resume an OpenCode session."""
        return ["opencode", session.directory, "--session", session.id]
