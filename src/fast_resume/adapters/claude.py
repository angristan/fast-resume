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
            # Count actual human interactions (not tool results)
            human_turn_count = 0

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

                    # Process user messages
                    if msg_type == "user":
                        msg = data.get("message", {})
                        content = msg.get("content", "")

                        # Check if this is a real human input or automatic tool result
                        is_human_input = False
                        if isinstance(content, str):
                            is_human_input = True
                            if not data.get("isMeta") and not content.startswith(
                                ("<command", "<local-command")
                            ):
                                messages.append(f"» {content}")
                                if not first_user_message and len(content) > 10:
                                    first_user_message = content
                        elif isinstance(content, list):
                            # Check first part - if it's text (not tool_result), it's human
                            first_part = content[0] if content else {}
                            if isinstance(first_part, dict):
                                part_type = first_part.get("type", "")
                                if part_type == "text":
                                    is_human_input = True
                                # tool_result means automatic response, not human input

                            for part in content:
                                if (
                                    isinstance(part, dict)
                                    and part.get("type") == "text"
                                ):
                                    text = part.get("text", "")
                                    messages.append(f"» {text}")
                                    if not first_user_message:
                                        first_user_message = text
                                elif isinstance(part, str):
                                    messages.append(f"» {part}")

                        if is_human_input:
                            human_turn_count += 1

                    # Extract assistant content for preview (no need to count)
                    if msg_type == "assistant":
                        msg = data.get("message", {})
                        content = msg.get("content", "")
                        if isinstance(content, str) and content:
                            messages.append(f"  {content}")
                        elif isinstance(content, list):
                            for part in content:
                                if (
                                    isinstance(part, dict)
                                    and part.get("type") == "text"
                                ):
                                    text = part.get("text", "")
                                    if text:
                                        messages.append(f"  {text}")
                                elif isinstance(part, str):
                                    messages.append(f"  {part}")

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
                message_count=human_turn_count,
            )
        except Exception:
            return None

    def get_resume_command(self, session: Session) -> list[str]:
        """Get command to resume a Claude Code session."""
        return ["claude", "--resume", session.id]
