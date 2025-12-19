"""Claude Code session adapter."""

import json
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, CLAUDE_DIR, MAX_CONTENT_LENGTH, MAX_PREVIEW_LENGTH
from .base import Session


class ClaudeAdapter:
    """Adapter for Claude Code sessions."""

    name = "claude"
    color = AGENTS["claude"]["color"]
    badge = AGENTS["claude"]["badge"]

    def is_available(self) -> bool:
        """Check if Claude Code data directory exists."""
        return CLAUDE_DIR.exists()

    def find_sessions(self) -> list[Session]:
        """Find all Claude Code sessions."""
        if not self.is_available():
            return []

        sessions = []
        for project_dir in CLAUDE_DIR.iterdir():
            if not project_dir.is_dir():
                continue

            for session_file in project_dir.glob("*.jsonl"):
                # Skip agent subprocesses
                if session_file.name.startswith("agent-"):
                    continue

                session = self._parse_session(session_file)
                if session:
                    sessions.append(session)

        return sessions

    def _parse_session(self, session_file: Path) -> Session | None:
        """Parse a Claude Code session file."""
        try:
            title = ""
            first_user_message = ""
            directory = ""
            timestamp = datetime.fromtimestamp(session_file.stat().st_mtime)
            messages: list[str] = []

            with open(session_file, "r", encoding="utf-8") as f:
                for line in f:
                    if not line.strip():
                        continue
                    try:
                        data = json.loads(line)
                    except json.JSONDecodeError:
                        continue

                    msg_type = data.get("type", "")

                    # Get summary/title
                    if msg_type == "summary":
                        title = data.get("summary", "")

                    # Get directory from user message
                    if msg_type == "user" and not directory:
                        directory = data.get("cwd", "")

                    # Extract message content
                    if msg_type in ("user", "assistant"):
                        msg = data.get("message", {})
                        content = msg.get("content", "")
                        role_prefix = "» " if msg_type == "user" else "  "
                        if isinstance(content, str):
                            # Skip meta messages and commands
                            if not data.get("isMeta") and not content.startswith(("<command", "<local-command")):
                                messages.append(f"{role_prefix}{content}")
                                if msg_type == "user" and not first_user_message and len(content) > 10:
                                    first_user_message = content
                        elif isinstance(content, list):
                            for part in content:
                                if isinstance(part, dict):
                                    if part.get("type") == "text":
                                        text = part.get("text", "")
                                        messages.append(f"{role_prefix}{text}")
                                        if msg_type == "user" and not first_user_message:
                                            first_user_message = text
                                elif isinstance(part, str):
                                    messages.append(f"{role_prefix}{part}")

            # Skip sessions with no actual user message
            if not first_user_message:
                return None

            # Use first user message as title if no summary
            if not title:
                # Truncate and clean up
                title = first_user_message.strip()[:100]
                if len(first_user_message) > 100:
                    title = title.rsplit(" ", 1)[0] + "..."

            # Skip sessions with no actual conversation content
            if not messages:
                return None

            full_content = "\n\n".join(messages)[:MAX_CONTENT_LENGTH]
            preview = full_content[:MAX_PREVIEW_LENGTH]

            return Session(
                id=session_file.stem,
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
        """Get command to resume a Claude Code session."""
        return ["claude", "--resume", session.id]
