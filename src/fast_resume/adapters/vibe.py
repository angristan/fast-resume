"""Vibe (Mistral) session adapter.

Supports the Vibe 2.0 session format with per-session folders containing
meta.json and messages.jsonl files.
"""

import orjson
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, VIBE_DIR
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
        for session_dir in self._sessions_dir.glob("session_*"):
            if session_dir.is_dir():
                session = self._parse_session_file(session_dir)
                if session:
                    sessions.append(session)

        return sessions

    def _parse_session_file(
        self, session_file: Path, on_error: ErrorCallback = None
    ) -> Session | None:
        """Parse a Vibe session folder (meta.json + messages.jsonl).

        Note: For Vibe 2.0, session_file is actually a directory containing
        meta.json and messages.jsonl files.
        """
        session_dir = session_file  # Vibe uses directories, not files
        metadata_file = session_dir / "meta.json"
        messages_file = session_dir / "messages.jsonl"

        if not metadata_file.exists():
            return None

        try:
            # Read metadata
            with open(metadata_file, "rb") as f:
                metadata = orjson.loads(f.read())

            session_id = metadata.get("session_id", session_dir.name)

            # Get directory from environment
            env = metadata.get("environment", {})
            directory = env.get("working_directory", "")

            # Check if session was started with auto_approve
            # New format: config.auto_approve, Old format: auto_approve at root
            config = metadata.get("config", {})
            yolo = config.get("auto_approve", False) or metadata.get(
                "auto_approve", False
            )

            # Parse timestamps
            start_time = metadata.get("start_time", "")
            if start_time:
                try:
                    timestamp = datetime.fromisoformat(start_time)
                except ValueError:
                    timestamp = datetime.fromtimestamp(metadata_file.stat().st_mtime)
            else:
                timestamp = datetime.fromtimestamp(metadata_file.stat().st_mtime)

            # Get title from metadata if available
            title = metadata.get("title", "")

            # Read messages from JSONL file
            messages: list[str] = []
            messages_data: list[dict] = []

            if messages_file.exists():
                with open(messages_file, "rb") as f:
                    for line in f:
                        line = line.strip()
                        if not line:
                            continue
                        try:
                            msg = orjson.loads(line)
                            messages_data.append(msg)

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
                        except orjson.JSONDecodeError:
                            continue

            # Generate title from first user message if not in metadata
            if not title:
                user_messages = [m for m in messages_data if m.get("role") == "user"]
                if user_messages:
                    first_msg = user_messages[0].get("content", "")
                    if isinstance(first_msg, str):
                        title = truncate_title(
                            first_msg, max_length=80, word_break=False
                        )
                    else:
                        title = "Vibe session"
                else:
                    title = "Vibe session"

            full_content = "\n\n".join(messages)

            return Session(
                id=session_id,
                agent=self.name,
                title=title,
                directory=directory,
                timestamp=timestamp,
                content=full_content,
                message_count=len(messages),
                yolo=yolo,
            )
        except OSError as e:
            error = ParseError(
                agent=self.name,
                file_path=str(session_dir),
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
                file_path=str(metadata_file),
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
                file_path=str(session_dir),
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
        """Scan all Vibe session folders.

        Uses meta.json file mtime for incremental update detection.
        Vibe updates meta.json on every turn, so this correctly detects changes.
        """
        current_files: dict[str, tuple[Path, float]] = {}

        for session_dir in self._sessions_dir.glob("session_*"):
            if not session_dir.is_dir():
                continue
            metadata_file = session_dir / "meta.json"
            if not metadata_file.exists():
                continue
            try:
                with open(metadata_file, "rb") as f:
                    metadata = orjson.loads(f.read())
                session_id = metadata.get("session_id", session_dir.name)
                mtime = metadata_file.stat().st_mtime

                current_files[session_id] = (session_dir, mtime)
            except Exception:
                continue

        return current_files

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to resume a Vibe session."""
        cmd = ["vibe"]
        if yolo:
            cmd.extend(["--agent", "auto-approve"])
        cmd.extend(["--resume", session.id])
        return cmd
