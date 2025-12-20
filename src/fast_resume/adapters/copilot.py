"""GitHub Copilot CLI session adapter."""

import orjson
import re
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, COPILOT_DIR, MAX_PREVIEW_LENGTH
from .base import Session


class CopilotAdapter:
    """Adapter for GitHub Copilot CLI sessions."""

    name = "copilot"
    color = AGENTS["copilot"]["color"]
    badge = AGENTS["copilot"]["badge"]

    def is_available(self) -> bool:
        """Check if Copilot CLI data directory exists."""
        return COPILOT_DIR.exists()

    def find_sessions(self) -> list[Session]:
        """Find all Copilot CLI sessions."""
        if not self.is_available():
            return []

        sessions = []
        for session_file in COPILOT_DIR.glob("*.jsonl"):
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

    def get_resume_command(self, session: Session) -> list[str]:
        """Get command to resume a Copilot CLI session."""
        return ["copilot", "--resume", session.id]
