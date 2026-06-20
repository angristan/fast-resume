"""Claude Code session adapter."""

import orjson
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, CLAUDE_DIR
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
        self._project_indexes: dict[
            Path, tuple[float, dict[str, tuple[str, float]]]
        ] = {}

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
            first_user_message = ""
            ai_title = ""
            directory = ""
            timestamp = datetime.fromtimestamp(session_file.stat().st_mtime)
            messages: list[str] = []
            # Count conversation turns (user + assistant, not tool results)
            turn_count = 0

            with open(session_file, "rb") as f:
                for line in f:
                    if not line.strip():
                        continue
                    try:
                        data = orjson.loads(line)
                    except orjson.JSONDecodeError:
                        # Skip malformed lines within the file
                        continue

                    msg_type = data.get("type", "")

                    # Claude Code's auto-generated session title (shown in its
                    # own Resume UI). It gets rewritten as the session evolves,
                    # so keep the latest non-empty one.
                    if msg_type == "ai-title" and data.get("aiTitle"):
                        ai_title = data["aiTitle"]

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

            # Prefer Claude's own list title when available: a /rename display
            # name (sessions-index.json), then Claude's auto-generated aiTitle
            # (the title shown in its Resume UI), then the first user message.
            title_source = (
                self._get_index_title(session_file) or ai_title or first_user_message
            )
            title = truncate_title(title_source)

            # Skip sessions with no actual conversation content
            if not messages:
                return None

            full_content = "\n\n".join(messages)

            return Session(
                id=session_file.stem,
                agent=self.name,
                title=title,
                directory=directory,
                timestamp=timestamp,
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

    def _get_project_index_mtime(self, project_dir: Path) -> float:
        """Return the mtime of a Claude project sessions-index file."""
        try:
            return (project_dir / "sessions-index.json").stat().st_mtime
        except OSError:
            return 0.0

    def _parse_index_timestamp(self, value: str) -> float:
        """Parse a Claude sessions-index ISO timestamp to a unix timestamp."""
        if not value:
            return 0.0

        try:
            return datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()
        except ValueError:
            return 0.0

    def _load_project_index(self, project_dir: Path) -> dict[str, tuple[str, float]]:
        """Load Claude session titles from a project's sessions-index.json."""
        index_mtime = self._get_project_index_mtime(project_dir)
        cached = self._project_indexes.get(project_dir)
        if cached and cached[0] == index_mtime:
            return cached[1]

        session_titles: dict[str, tuple[str, float]] = {}
        if index_mtime:
            try:
                with open(project_dir / "sessions-index.json", "rb") as f:
                    data = orjson.loads(f.read())

                entries = data.get("entries", [])
                if isinstance(entries, list):
                    for entry in entries:
                        if not isinstance(entry, dict):
                            continue

                        session_id = entry.get("sessionId", "")
                        summary = entry.get("summary", "")
                        if not isinstance(session_id, str) or not session_id:
                            continue
                        if not isinstance(summary, str) or not summary.strip():
                            continue

                        entry_mtime = index_mtime
                        modified = entry.get("modified", "")
                        if isinstance(modified, str):
                            entry_mtime = max(
                                entry_mtime, self._parse_index_timestamp(modified)
                            )

                        file_mtime = entry.get("fileMtime")
                        if isinstance(file_mtime, int | float):
                            entry_mtime = max(entry_mtime, file_mtime / 1000)

                        session_titles[session_id] = (
                            summary.strip(),
                            entry_mtime,
                        )
            except OSError, orjson.JSONDecodeError:
                session_titles = {}

        self._project_indexes[project_dir] = (index_mtime, session_titles)
        return session_titles

    def _get_index_title(self, session_file: Path) -> str:
        """Return Claude's indexed session title for a session file."""
        entry = self._load_project_index(session_file.parent).get(session_file.stem)
        return entry[0] if entry else ""

    def _scan_session_files(self) -> dict[str, tuple[Path, float]]:
        """Scan all Claude Code session files."""
        current_files: dict[str, tuple[Path, float]] = {}

        for project_dir in self._sessions_dir.iterdir():
            if not project_dir.is_dir():
                continue

            project_index = self._load_project_index(project_dir)

            for session_file in project_dir.glob("*.jsonl"):
                if session_file.name.startswith("agent-"):
                    continue

                try:
                    mtime = session_file.stat().st_mtime
                except OSError:
                    continue
                session_id = session_file.stem
                index_entry = project_index.get(session_id)
                if index_entry:
                    mtime = max(mtime, index_entry[1])
                current_files[session_id] = (session_file, mtime)

        return current_files

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to resume a Claude Code session."""
        cmd = ["claude"]
        if yolo:
            cmd.append("--dangerously-skip-permissions")
        cmd.extend(["--resume", session.id])
        return cmd
