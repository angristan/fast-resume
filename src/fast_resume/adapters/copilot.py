"""GitHub Copilot CLI session adapter."""

import orjson
import re
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, COPILOT_DIR, MAX_PREVIEW_LENGTH
from .base import Session


class CopilotAdapter:
    """Adapter for GitHub Copilot CLI sessions."""

    name = "copilot-cli"
    color = AGENTS["copilot-cli"]["color"]
    badge = AGENTS["copilot-cli"]["badge"]

    def __init__(self, sessions_dir: Path | None = None) -> None:
        self._sessions_dir = sessions_dir if sessions_dir is not None else COPILOT_DIR

    def is_available(self) -> bool:
        """Check if Copilot CLI data directory exists."""
        return self._sessions_dir.exists()

    def find_sessions(self) -> list[Session]:
        """Find all Copilot CLI sessions."""
        if not self.is_available():
            return []

        sessions = []
        for session_file in self._sessions_dir.glob("*.jsonl"):
            session = self._parse_session(session_file)
            if session:
                sessions.append(session)

        return sessions

    def _parse_session(self, session_file: Path) -> Session | None:
        """Parse a Copilot CLI session file."""
        try:
            session_id = session_file.stem
            first_user_message = ""
            directory = ""
            timestamp = datetime.fromtimestamp(session_file.stat().st_mtime)
            messages: list[str] = []
            human_turn_count = 0

            with open(session_file, "r", encoding="utf-8") as f:
                for line in f:
                    if not line.strip():
                        continue
                    try:
                        entry = orjson.loads(line)
                    except orjson.JSONDecodeError:
                        continue

                    msg_type = entry.get("type", "")
                    data = entry.get("data", {})

                    # Get session ID from session.start
                    if msg_type == "session.start":
                        session_id = data.get("sessionId", session_id)

                    # Get directory from folder_trust info
                    if msg_type == "session.info" and not directory:
                        if data.get("infoType") == "folder_trust":
                            # Extract path from message like "Folder /path/to/dir has been added..."
                            message = data.get("message", "")
                            match = re.search(r"Folder (/[^\s]+)", message)
                            if match:
                                directory = match.group(1)

                    # Process user messages
                    if msg_type == "user.message":
                        content = data.get("content", "")
                        if content:
                            messages.append(f"» {content}")
                            human_turn_count += 1
                            if not first_user_message and len(content) > 10:
                                first_user_message = content

                    # Process assistant messages
                    if msg_type == "assistant.message":
                        content = data.get("content", "")
                        if content:
                            messages.append(f"  {content}")

            # Skip sessions with no actual user message
            if not first_user_message:
                return None

            # Use first user message as title
            title = first_user_message.strip()[:100]
            if len(first_user_message) > 100:
                title = title.rsplit(" ", 1)[0] + "..."

            # Skip sessions with no actual conversation content
            if not messages:
                return None

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
                message_count=human_turn_count,
            )
        except Exception:
            return None

    def _get_session_id_from_file(self, session_file: Path) -> str:
        """Extract session ID from file content or filename."""
        try:
            with open(session_file, "r", encoding="utf-8") as f:
                for line in f:
                    if not line.strip():
                        continue
                    try:
                        entry = orjson.loads(line)
                        if entry.get("type") == "session.start":
                            session_id = entry.get("data", {}).get("sessionId", "")
                            if session_id:
                                return session_id
                    except orjson.JSONDecodeError:
                        continue
        except Exception:
            pass
        return session_file.stem

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
        current_files: dict[str, tuple[Path, float]] = {}

        for session_file in self._sessions_dir.glob("*.jsonl"):
            session_id = self._get_session_id_from_file(session_file)
            mtime = session_file.stat().st_mtime
            current_files[session_id] = (session_file, mtime)

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
        """Get command to resume a Copilot CLI session."""
        cmd = ["copilot"]
        if yolo:
            cmd.extend(["--allow-all-tools", "--allow-all-paths"])
        cmd.extend(["--resume", session.id])
        return cmd
