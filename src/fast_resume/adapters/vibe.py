"""Vibe (Mistral) session adapter."""

import orjson
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, MAX_PREVIEW_LENGTH, VIBE_DIR
from ..logging_config import log_parse_error
from .base import BaseSessionAdapter, ErrorCallback, ParseError, Session, truncate_title


class VibeAdapter(BaseSessionAdapter):
    """Adapter for Vibe (Mistral) sessions."""

    name = "vibe"
    color = AGENTS["vibe"]["color"]
    badge = AGENTS["vibe"]["badge"]
    supports_yolo = True

    def __init__(self, sessions_dir: Path | None = None) -> None:
        self._sessions_dir = sessions_dir if sessions_dir is not None else VIBE_DIR

    def find_sessions(self) -> list[Session]:
        """Find all Vibe sessions."""
        if not self.is_available():
            return []

        sessions = []
        for session_file in self._sessions_dir.glob("session_*.json"):
            session = self._parse_session_file(session_file)
            if session:
                sessions.append(session)

        return sessions

    def _parse_session_file(
        self, session_file: Path, on_error: ErrorCallback = None
    ) -> Session | None:
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

            # Generate title from first user message (80-char hard truncate)
            user_messages = [
                m for i, m in enumerate(messages_data) if m.get("role") == "user"
            ]
            if user_messages:
                first_msg = user_messages[0].get("content", "")
                if isinstance(first_msg, str):
                    title = truncate_title(first_msg, max_length=80, word_break=False)
                else:
                    title = "Vibe session"
            else:
                title = "Vibe session"

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
        except orjson.JSONDecodeError as e:
            error = ParseError(
                agent=self.name,
                file_path=str(session_file),
                error_type="JSONDecodeError",
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
        """Scan all Vibe session files.

        Uses start_time from JSON metadata as mtime for consistency with parsing.
        """
        current_files: dict[str, tuple[Path, float]] = {}

        for session_file in self._sessions_dir.glob("session_*.json"):
            try:
                with open(session_file, "rb") as f:
                    data = orjson.loads(f.read())
                metadata = data.get("metadata", {})
                session_id = metadata.get("session_id", session_file.stem)

                # Use start_time to match what _parse_session_file stores
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

        return current_files

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to resume a Vibe session."""
        cmd = ["vibe"]
        if yolo:
            cmd.append("--auto-approve")
        cmd.extend(["--resume", session.id])
        return cmd
