"""GitHub Copilot CLI session adapter."""

import orjson
import re
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, COPILOT_DIR, MAX_PREVIEW_LENGTH
from ..logging_config import log_parse_error
from .base import BaseSessionAdapter, ErrorCallback, ParseError, Session, truncate_title


class CopilotAdapter(BaseSessionAdapter):
    """Adapter for GitHub Copilot CLI sessions."""

    name = "copilot-cli"
    color = AGENTS["copilot-cli"]["color"]
    badge = AGENTS["copilot-cli"]["badge"]
    supports_yolo = True

    def __init__(self, sessions_dir: Path | None = None) -> None:
        self._sessions_dir = sessions_dir if sessions_dir is not None else COPILOT_DIR

    def find_sessions(self) -> list[Session]:
        """Find all Copilot CLI sessions."""
        if not self.is_available():
            return []

        sessions = []
        for session_file in self._sessions_dir.glob("*.jsonl"):
            session = self._parse_session_file(session_file)
            if session:
                sessions.append(session)

        return sessions

    def _parse_session_file(
        self, session_file: Path, on_error: ErrorCallback = None
    ) -> Session | None:
        """Parse a Copilot CLI session file."""
        try:
            session_id = session_file.stem
            first_user_message = ""
            directory = ""
            timestamp = datetime.fromtimestamp(session_file.stat().st_mtime)
            messages: list[str] = []
            turn_count = 0

            with open(session_file, "r", encoding="utf-8") as f:
                for line in f:
                    if not line.strip():
                        continue
                    try:
                        entry = orjson.loads(line)
                    except orjson.JSONDecodeError:
                        # Skip malformed lines within the file
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
                            turn_count += 1
                            if not first_user_message and len(content) > 10:
                                first_user_message = content

                    # Process assistant messages
                    if msg_type == "assistant.message":
                        content = data.get("content", "")
                        if content:
                            messages.append(f"  {content}")
                            turn_count += 1

            # Skip sessions with no actual user message
            if not first_user_message:
                return None

            # Use first user message as title
            title = truncate_title(first_user_message)

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

    def _scan_session_files(self) -> dict[str, tuple[Path, float]]:
        """Scan all Copilot CLI session files."""
        current_files: dict[str, tuple[Path, float]] = {}

        for session_file in self._sessions_dir.glob("*.jsonl"):
            session_id = self._get_session_id_from_file(session_file)
            mtime = session_file.stat().st_mtime
            current_files[session_id] = (session_file, mtime)

        return current_files

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to resume a Copilot CLI session."""
        cmd = ["copilot"]
        if yolo:
            cmd.extend(["--allow-all-tools", "--allow-all-paths"])
        cmd.extend(["--resume", session.id])
        return cmd
