"""Codex CLI session adapter."""

import orjson
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, CODEX_DIR, MAX_PREVIEW_LENGTH
from ..logging_config import log_parse_error
from .base import BaseSessionAdapter, ErrorCallback, ParseError, Session, truncate_title


class CodexAdapter(BaseSessionAdapter):
    """Adapter for Codex CLI sessions."""

    name = "codex"
    color = AGENTS["codex"]["color"]
    badge = AGENTS["codex"]["badge"]
    supports_yolo = True

    def __init__(self, sessions_dir: Path | None = None) -> None:
        self._sessions_dir = sessions_dir if sessions_dir is not None else CODEX_DIR

    def find_sessions(self) -> list[Session]:
        """Find all Codex CLI sessions."""
        if not self.is_available():
            return []

        sessions = []
        # Codex stores sessions in YYYY/MM/DD subdirectories
        for session_file in self._sessions_dir.rglob("*.jsonl"):
            session = self._parse_session_file(session_file)
            if session:
                sessions.append(session)

        return sessions

    def _parse_session_file(
        self, session_file: Path, on_error: ErrorCallback = None
    ) -> Session | None:
        """Parse a Codex CLI session file."""
        try:
            session_id = ""
            directory = ""
            timestamp = datetime.fromtimestamp(session_file.stat().st_mtime)
            messages: list[str] = []
            user_prompts: list[str] = []  # Actual human inputs for title
            turn_count = 0  # Count user + assistant turns
            yolo = False  # Track if session was started in yolo mode

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
                    payload = data.get("payload", {})

                    # Get session metadata
                    if msg_type == "session_meta":
                        session_id = payload.get("id", "")
                        directory = payload.get("cwd", "")

                    # Check turn_context for yolo mode
                    if msg_type == "turn_context":
                        approval_policy = payload.get("approval_policy", "")
                        sandbox_policy = payload.get("sandbox_policy", {})
                        sandbox_mode = (
                            sandbox_policy.get("mode", "")
                            if isinstance(sandbox_policy, dict)
                            else ""
                        )
                        if (
                            approval_policy == "never"
                            or sandbox_mode == "danger-full-access"
                        ):
                            yolo = True

                    # Extract response items for preview content
                    if msg_type == "response_item":
                        role = payload.get("role", "")
                        content = payload.get("content", [])
                        if role in ("user", "assistant"):
                            role_prefix = "» " if role == "user" else "  "
                            has_text = False
                            for part in content:
                                if isinstance(part, dict):
                                    text = part.get("text", "") or part.get(
                                        "input_text", ""
                                    )
                                    if text:
                                        # Skip system context for content
                                        if not text.strip().startswith(
                                            "<environment_context>"
                                        ):
                                            messages.append(f"{role_prefix}{text}")
                                            has_text = True
                            if has_text:
                                turn_count += 1

                    # Extract event messages (user prompts) - actual human inputs
                    if msg_type == "event_msg":
                        event_type = payload.get("type", "")
                        if event_type == "user_message":
                            msg = payload.get("message", "")
                            if msg:
                                messages.append(f"» {msg}")
                                user_prompts.append(msg)
                        elif event_type == "agent_reasoning":
                            text = payload.get("text", "")
                            if text:
                                messages.append(f"  {text}")

            if not session_id:
                # Extract from filename: rollout-2025-12-17T18-24-27-019b2d57-...
                session_id = (
                    session_file.stem.split("-", 1)[-1]
                    if "-" in session_file.stem
                    else session_file.stem
                )

            # Skip sessions with no actual user prompt
            if not user_prompts:
                return None

            # Generate title from first actual user prompt (80-char hard truncate)
            title = truncate_title(user_prompts[0], max_length=80, word_break=False)

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
        # Try to get ID from session_meta in file content first
        try:
            with open(session_file, "r", encoding="utf-8") as f:
                for line in f:
                    if not line.strip():
                        continue
                    try:
                        data = orjson.loads(line)
                        if data.get("type") == "session_meta":
                            session_id = data.get("payload", {}).get("id", "")
                            if session_id:
                                return session_id
                            break
                    except orjson.JSONDecodeError:
                        continue
        except Exception:
            pass

        # Fallback to filename extraction
        return (
            session_file.stem.split("-", 1)[-1]
            if "-" in session_file.stem
            else session_file.stem
        )

    def _scan_session_files(self) -> dict[str, tuple[Path, float]]:
        """Scan all Codex CLI session files."""
        current_files: dict[str, tuple[Path, float]] = {}

        for session_file in self._sessions_dir.rglob("*.jsonl"):
            session_id = self._get_session_id_from_file(session_file)
            mtime = session_file.stat().st_mtime
            current_files[session_id] = (session_file, mtime)

        return current_files

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to resume a Codex CLI session."""
        cmd = ["codex"]
        if yolo:
            cmd.append("--dangerously-bypass-approvals-and-sandbox")
        cmd.extend(["resume", session.id])
        return cmd
