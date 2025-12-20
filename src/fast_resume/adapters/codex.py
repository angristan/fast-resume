"""Codex CLI session adapter."""

import orjson
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, CODEX_DIR, MAX_CONTENT_LENGTH, MAX_PREVIEW_LENGTH
from .base import Session


class CodexAdapter:
    """Adapter for Codex CLI sessions."""

    name = "codex"
    color = AGENTS["codex"]["color"]
    badge = AGENTS["codex"]["badge"]

    def is_available(self) -> bool:
        """Check if Codex CLI data directory exists."""
        return CODEX_DIR.exists()

    def find_sessions(self) -> list[Session]:
        """Find all Codex CLI sessions."""
        if not self.is_available():
            return []

        sessions = []
        # Codex stores sessions in YYYY/MM/DD subdirectories
        for session_file in CODEX_DIR.rglob("*.jsonl"):
            session = self._parse_session(session_file)
            if session:
                sessions.append(session)

        return sessions

    def _parse_session(self, session_file: Path) -> Session | None:
        """Parse a Codex CLI session file."""
        try:
            session_id = ""
            directory = ""
            timestamp = datetime.fromtimestamp(session_file.stat().st_mtime)
            messages: list[str] = []
            user_prompts: list[str] = []  # Actual human inputs for title and count

            with open(session_file, "r", encoding="utf-8") as f:
                for line in f:
                    if not line.strip():
                        continue
                    try:
                        data = orjson.loads(line)
                    except orjson.JSONDecodeError:
                        continue

                    msg_type = data.get("type", "")
                    payload = data.get("payload", {})

                    # Get session metadata
                    if msg_type == "session_meta":
                        session_id = payload.get("id", "")
                        directory = payload.get("cwd", "")

                    # Extract response items for preview content
                    if msg_type == "response_item":
                        role = payload.get("role", "")
                        content = payload.get("content", [])
                        if role in ("user", "assistant"):
                            role_prefix = "» " if role == "user" else "  "
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

            # Generate title from first actual user prompt
            title = user_prompts[0][:80]
            if len(user_prompts[0]) > 80:
                title += "..."

            full_content = "\n\n".join(messages)[:MAX_CONTENT_LENGTH]
            preview = full_content[:MAX_PREVIEW_LENGTH]

            return Session(
                id=session_id,
                agent=self.name,
                title=title,
                directory=directory,
                timestamp=timestamp,
                preview=preview,
                content=full_content,
                message_count=len(user_prompts),
            )
        except Exception:
            return None

    def get_resume_command(self, session: Session) -> list[str]:
        """Get command to resume a Codex CLI session."""
        return ["codex", "resume", session.id]
