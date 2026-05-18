"""Kiro CLI session adapter.

Kiro stores each session as a pair of files in `~/.kiro/sessions/cli/`:

    <uuid>.json    Metadata: session_id, cwd, created_at, updated_at, title.
    <uuid>.jsonl   Event stream with kinds: Prompt, AssistantMessage, ToolResults.

The metadata file already carries enough to render a row; the JSONL is parsed
for full message content so search can match conversation text.
"""

import orjson
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, KIRO_DIR
from ..logging_config import log_parse_error
from .base import BaseSessionAdapter, ErrorCallback, ParseError, Session, truncate_title


class KiroAdapter(BaseSessionAdapter):
    """Adapter for Kiro CLI sessions."""

    name = "kiro"
    color = AGENTS["kiro"]["color"]
    badge = AGENTS["kiro"]["badge"]
    supports_yolo = True

    def __init__(self, sessions_dir: Path | None = None) -> None:
        self._sessions_dir = sessions_dir if sessions_dir is not None else KIRO_DIR

    def find_sessions(self) -> list[Session]:
        """Find all Kiro CLI sessions.

        Routes through `_scan_session_files` so iteration shares a single
        source of truth with the incremental scanner and we don't duplicate
        the `glob("*.json")` logic in two places.
        """
        if not self.is_available():
            return []
        sessions: list[Session] = []
        for path, _mtime in self._scan_session_files().values():
            session = self._parse_session_file(path)
            if session:
                sessions.append(session)
        return sessions

    def _parse_session_file(
        self, session_file: Path, on_error: ErrorCallback = None
    ) -> Session | None:
        """Parse a Kiro session given its `<uuid>.json` metadata path.

        The matching `<uuid>.jsonl` (if present) is read for message content.
        """
        meta_file = session_file
        try:
            with open(meta_file, "rb") as f:
                meta = orjson.loads(f.read())
        except orjson.JSONDecodeError as e:
            self._report(on_error, str(meta_file), "JSONDecodeError", str(e))
            return None
        except OSError as e:
            self._report(on_error, str(meta_file), "OSError", str(e))
            return None

        try:
            session_id = meta.get("session_id") or meta_file.stem
            directory = meta.get("cwd", "")
            title_from_meta = (meta.get("title") or "").strip()

            timestamp = self._parse_timestamp(
                meta.get("updated_at") or meta.get("created_at"), meta_file
            )

            events_file = meta_file.with_suffix(".jsonl")
            messages, first_user_prompt, turn_count = self._parse_events(
                events_file, on_error=on_error
            )

            title = title_from_meta or first_user_prompt
            if title:
                title = truncate_title(title, max_length=80, word_break=False)
            else:
                title = "Kiro session"

            full_content = "\n\n".join(messages)

            return Session(
                id=session_id,
                agent=self.name,
                title=title,
                directory=directory,
                timestamp=timestamp,
                content=full_content,
                message_count=turn_count,
            )
        except (KeyError, TypeError, AttributeError) as e:
            self._report(on_error, str(meta_file), type(e).__name__, str(e))
            return None

    def _parse_events(
        self, events_file: Path, on_error: ErrorCallback = None
    ) -> tuple[list[str], str, int]:
        """Parse the `.jsonl` event stream alongside the metadata file.

        Returns:
            (messages, first_user_prompt, turn_count)
        """
        messages: list[str] = []
        first_user_prompt = ""
        turn_count = 0

        if not events_file.exists():
            return messages, first_user_prompt, turn_count

        try:
            with open(events_file, "rb") as f:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        entry = orjson.loads(line)
                    except orjson.JSONDecodeError:
                        continue

                    kind = entry.get("kind", "")
                    data = entry.get("data", {})
                    if kind == "Prompt":
                        text = self._extract_text(data.get("content", []))
                        if text:
                            messages.append(f"» {text}")
                            turn_count += 1
                            if not first_user_prompt:
                                first_user_prompt = text
                    elif kind == "AssistantMessage":
                        text = self._extract_text(data.get("content", []))
                        if text:
                            messages.append(f"  {text}")
                            turn_count += 1
                    # ToolResults are intentionally excluded from the index.
        except OSError as e:
            # Permission-denied / IO failure on the event stream produces an
            # empty session content; surface it so the caller knows parsing
            # was partial rather than the session actually being empty.
            self._report(on_error, str(events_file), "OSError", str(e))

        return messages, first_user_prompt, turn_count

    @staticmethod
    def _extract_text(content: list) -> str:
        """Collect plain-text segments from a Kiro content list, ignoring tool parts."""
        parts: list[str] = []
        for item in content:
            if not isinstance(item, dict):
                continue
            if item.get("kind") != "text":
                continue
            text = item.get("data", "")
            if isinstance(text, str) and text.strip():
                parts.append(text)
        return "\n".join(parts).strip()

    @staticmethod
    def _parse_timestamp(value: str | None, fallback_file: Path) -> datetime:
        if value:
            try:
                # Kiro emits RFC3339 with trailing 'Z'.
                ts = value.replace("Z", "+00:00") if value.endswith("Z") else value
                dt = datetime.fromisoformat(ts)
                if dt.tzinfo is not None:
                    dt = dt.astimezone().replace(tzinfo=None)
                return dt
            except ValueError:
                pass
        return datetime.fromtimestamp(fallback_file.stat().st_mtime)

    def _report(
        self,
        on_error: ErrorCallback,
        file_path: str,
        error_type: str,
        message: str,
    ) -> None:
        error = ParseError(
            agent=self.name,
            file_path=file_path,
            error_type=error_type,
            message=message,
        )
        log_parse_error(error.agent, error.file_path, error.error_type, error.message)
        if on_error:
            on_error(error)

    def _scan_session_files(self) -> dict[str, tuple[Path, float]]:
        """Scan Kiro session metadata files.

        Uses the newer of the meta + events mtimes so that index updates fire
        when only the JSONL grows (a still-running session).
        """
        current: dict[str, tuple[Path, float]] = {}
        for meta_file in self._sessions_dir.glob("*.json"):
            try:
                mtime = meta_file.stat().st_mtime
            except OSError:
                continue
            events_file = meta_file.with_suffix(".jsonl")
            try:
                events_mtime = events_file.stat().st_mtime
                if events_mtime > mtime:
                    mtime = events_mtime
            except OSError:
                pass
            current[meta_file.stem] = (meta_file, mtime)
        return current

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to resume a Kiro CLI session."""
        cmd = ["kiro-cli", "chat"]
        if yolo:
            cmd.append("--trust-all-tools")
        cmd.extend(["--resume-id", session.id])
        return cmd
