"""Claude Code session adapter."""

import orjson
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, CLAUDE_DIR, MAX_PREVIEW_LENGTH
from ..logging_config import log_parse_error
from .base import BaseSessionAdapter, ErrorCallback, ParseError, Session, truncate_title


class ClaudeAdapter(BaseSessionAdapter):
    """Adapter for Claude Code sessions."""

    name = "claude"
    color = AGENTS["claude"]["color"]
    badge = AGENTS["claude"]["badge"]
    supports_yolo = True

    def __init__(self, sessions_dir: Path | None = None) -> None:
        self._sessions_dir = sessions_dir if sessions_dir is not None else CLAUDE_DIR

    def find_sessions(self) -> list[Session]:
        """Find all Claude Code sessions."""
        if not self.is_available():
            return []

        sessions = []
        for project_dir in self._sessions_dir.iterdir():
            if not project_dir.is_dir():
                continue

            for session_file in project_dir.glob("*.jsonl"):
                # Skip agent subprocesses
                if session_file.name.startswith("agent-"):
                    continue

                session = self._parse_session_file(session_file)
                if session:
                    sessions.append(session)

        return sessions

    def _parse_session_file(
        self, session_file: Path, on_error: ErrorCallback = None
    ) -> Session | None:
        """Parse a Claude Code session file."""
        try:
            title = ""
            first_user_message = ""
            directory = ""
            timestamp = datetime.fromtimestamp(session_file.stat().st_mtime)
            messages: list[str] = []
            # Count conversation turns (user + assistant, not tool results)
            turn_count = 0

            with open(session_file, "r", encoding="utf-8") as f:
                for line in f:
                    if not line.strip():
                        continue
                    try:
                        data = orjson.loads(line)
                    except orjson.JSONDecodeError:
                        # Skip malformed lines within the file
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
                            turn_count += 1

                    # Extract assistant content
                    if msg_type == "assistant":
                        msg = data.get("message", {})
                        content = msg.get("content", "")
                        has_text = False
                        if isinstance(content, str) and content:
                            messages.append(f"  {content}")
                            has_text = True
                        elif isinstance(content, list):
                            for part in content:
                                if (
                                    isinstance(part, dict)
                                    and part.get("type") == "text"
                                ):
                                    text = part.get("text", "")
                                    if text:
                                        messages.append(f"  {text}")
                                        has_text = True
                                elif isinstance(part, str):
                                    messages.append(f"  {part}")
                                    has_text = True
                        if has_text:
                            turn_count += 1

            # Skip sessions with no actual user message
            if not first_user_message:
                return None

            # Use first user message as title if no summary
            if not title:
                title = truncate_title(first_user_message)

            # Skip sessions with no actual conversation content
            if not messages:
                return None

            full_content = "\n\n".join(messages)
            preview = full_content[:MAX_PREVIEW_LENGTH]

            return Session(
                id=session_file.stem,
                agent=self.name,
                title=title,
                directory=directory,
                timestamp=timestamp,
                preview=preview,
                content=full_content,
                message_count=turn_count,
            )
        except OSError as e:
            error = ParseError(
                agent=self.name,
                file_path=str(session_file),
                error_type="OSError",
                message=str(e),
            )
            log_parse_error(
                error.agent, error.file_path, error.error_type, error.message
            )
            if on_error:
                on_error(error)
            return None
        except (KeyError, TypeError, AttributeError) as e:
            error = ParseError(
                agent=self.name,
                file_path=str(session_file),
                error_type=type(e).__name__,
                message=str(e),
            )
            log_parse_error(
                error.agent, error.file_path, error.error_type, error.message
            )
            if on_error:
                on_error(error)
            return None

    def _scan_session_files(self) -> dict[str, tuple[Path, float]]:
        """Scan all Claude Code session files."""
        current_files: dict[str, tuple[Path, float]] = {}

        for project_dir in self._sessions_dir.iterdir():
            if not project_dir.is_dir():
                continue

            for session_file in project_dir.glob("*.jsonl"):
                if session_file.name.startswith("agent-"):
                    continue

                session_id = session_file.stem
                mtime = session_file.stat().st_mtime
                current_files[session_id] = (session_file, mtime)

        return current_files

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to resume a Claude Code session."""
        cmd = ["claude"]
        if yolo:
            cmd.append("--dangerously-skip-permissions")
        cmd.extend(["--resume", session.id])
        return cmd
