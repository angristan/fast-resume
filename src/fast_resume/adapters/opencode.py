"""OpenCode session adapter."""

import orjson
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, OPENCODE_DIR
from ..logging_config import log_parse_error
from .base import ErrorCallback, ParseError, RawAdapterStats, Session, SessionCallback


class OpenCodeAdapter:
    """Adapter for OpenCode sessions."""

    name = "opencode"
    color = AGENTS["opencode"]["color"]
    badge = AGENTS["opencode"]["badge"]
    supports_yolo = False

    def __init__(self, sessions_dir: Path | None = None) -> None:
        self._sessions_dir = sessions_dir if sessions_dir is not None else OPENCODE_DIR

    def is_available(self) -> bool:
        """Check if OpenCode data directory exists."""
        return self._sessions_dir.exists()

    def find_sessions(self) -> list[Session]:
        """Find all OpenCode sessions."""
        if not self.is_available():
            return []

        sessions = []
        session_dir = self._sessions_dir / "session"
        message_dir = self._sessions_dir / "message"
        part_dir = self._sessions_dir / "part"

        if not session_dir.exists():
            return []

        # Pre-index all messages by session_id: {session_id: [(msg_file, msg_id, role), ...]}
        messages_by_session: dict[str, list[tuple[Path, str, str]]] = defaultdict(list)
        if message_dir.exists():
            for msg_file in message_dir.glob("*/msg_*.json"):
                try:
                    with open(msg_file, "rb") as f:
                        msg_data = orjson.loads(f.read())
                    session_id = msg_file.parent.name
                    msg_id = msg_data.get("id", "")
                    role = msg_data.get("role", "")
                    if msg_id:
                        messages_by_session[session_id].append((msg_file, msg_id, role))
                except Exception:
                    continue

        # Pre-index all parts by message_id: {msg_id: [text, ...]}
        parts_by_message: dict[str, list[str]] = defaultdict(list)
        if part_dir.exists():
            for part_file in sorted(part_dir.glob("*/*.json")):
                try:
                    with open(part_file, "rb") as f:
                        part_data = orjson.loads(f.read())
                    msg_id = part_file.parent.name
                    if part_data.get("type") == "text":
                        text = part_data.get("text", "")
                        if text:
                            parts_by_message[msg_id].append(text)
                except Exception:
                    continue

        # OpenCode stores sessions in project-hash subdirectories
        for project_dir in session_dir.iterdir():
            if not project_dir.is_dir():
                continue

            for session_file in project_dir.glob("ses_*.json"):
                session = self._parse_session(
                    session_file, messages_by_session, parts_by_message
                )
                if session:
                    sessions.append(session)

        return sessions

    def _parse_session(
        self,
        session_file: Path,
        messages_by_session: dict[str, list[tuple[Path, str, str]]],
        parts_by_message: dict[str, list[str]],
        on_error: ErrorCallback = None,
    ) -> Session | None:
        """Parse an OpenCode session file."""
        try:
            with open(session_file, "rb") as f:
                data = orjson.loads(f.read())

            session_id = data.get("id", "")
            title = data.get("title", "Untitled session")
            directory = data.get("directory", "")

            # Parse timestamp from milliseconds
            time_data = data.get("time", {})
            created = time_data.get("created", 0)
            if created:
                timestamp = datetime.fromtimestamp(created / 1000)
            else:
                timestamp = datetime.fromtimestamp(session_file.stat().st_mtime)

            # Get message content from pre-indexed data
            messages = self._get_session_messages(
                session_id, messages_by_session, parts_by_message
            )

            # Count actual message turns (not text parts)
            turn_count = len(messages_by_session.get(session_id, []))

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

    def _get_session_messages(
        self,
        session_id: str,
        messages_by_session: dict[str, list[tuple[Path, str, str]]],
        parts_by_message: dict[str, list[str]],
    ) -> list[str]:
        """Get all messages for a session from pre-indexed parts."""
        messages: list[str] = []

        # Sort by filename to maintain order
        session_msgs = sorted(
            messages_by_session.get(session_id, []), key=lambda x: x[0].name
        )

        for _msg_file, msg_id, role in session_msgs:
            role_prefix = "» " if role == "user" else "  "
            for text in parts_by_message.get(msg_id, []):
                messages.append(f"{role_prefix}{text}")

        return messages

    def find_sessions_incremental(
        self,
        known: dict[str, tuple[float, str]],
        on_error: ErrorCallback = None,
        on_session: SessionCallback = None,
    ) -> tuple[list[Session], list[str]]:
        """Find sessions incrementally, comparing against known sessions.

        Uses bulk loading of messages/parts for efficiency, then calls
        on_session progressively as each session is parsed.
        """
        if not self.is_available():
            deleted_ids = [
                sid for sid, (_, agent) in known.items() if agent == self.name
            ]
            return [], deleted_ids

        session_dir = self._sessions_dir / "session"
        if not session_dir.exists():
            deleted_ids = [
                sid for sid, (_, agent) in known.items() if agent == self.name
            ]
            return [], deleted_ids

        # Scan session files and get timestamps (fast - just file listing + metadata)
        current_sessions: dict[str, tuple[Path, float]] = {}

        for project_dir in session_dir.iterdir():
            if not project_dir.is_dir():
                continue

            for session_file in project_dir.glob("ses_*.json"):
                try:
                    with open(session_file, "rb") as f:
                        data = orjson.loads(f.read())
                    session_id = data.get("id", "")
                    if session_id:
                        # Use created timestamp to match what _parse_session stores
                        created = data.get("time", {}).get("created", 0)
                        if created:
                            mtime = datetime.fromtimestamp(created / 1000).timestamp()
                        else:
                            mtime = session_file.stat().st_mtime
                        current_sessions[session_id] = (session_file, mtime)
                except (OSError, orjson.JSONDecodeError):
                    # Skip files that can't be read during scanning
                    continue

        # Check which sessions need parsing
        # Use 1ms tolerance for mtime comparison due to datetime precision loss
        sessions_to_parse: list[tuple[str, Path, float]] = []
        session_ids_to_parse: set[str] = set()
        for session_id, (path, mtime) in current_sessions.items():
            known_entry = known.get(session_id)
            if known_entry is None or mtime > known_entry[0] + 0.001:
                sessions_to_parse.append((session_id, path, mtime))
                session_ids_to_parse.add(session_id)

        # Find deleted sessions
        current_ids = set(current_sessions.keys())
        deleted_ids = [
            sid
            for sid, (_, agent) in known.items()
            if agent == self.name and sid not in current_ids
        ]

        if not sessions_to_parse:
            return [], deleted_ids

        # Parallel file I/O with ThreadPoolExecutor
        message_dir = self._sessions_dir / "message"
        part_dir = self._sessions_dir / "part"

        def read_message_file(msg_file: Path) -> tuple[str, Path, str, str] | None:
            """Read a message file and return (session_id, path, msg_id, role)."""
            try:
                with open(msg_file, "rb") as f:
                    data = orjson.loads(f.read())
                msg_id = data.get("id", "")
                role = data.get("role", "")
                if msg_id:
                    return (msg_file.parent.name, msg_file, msg_id, role)
            except (OSError, orjson.JSONDecodeError):
                pass
            return None

        def read_part_file(part_file: Path) -> tuple[str, str] | None:
            """Read a part file and return (msg_id, text) if it's a text part."""
            try:
                with open(part_file, "rb") as f:
                    data = orjson.loads(f.read())
                if data.get("type") == "text":
                    text = data.get("text", "")
                    if text:
                        return (part_file.parent.name, text)
            except (OSError, orjson.JSONDecodeError):
                pass
            return None

        # Step 1: Bulk read all message files in parallel
        all_msg_files = []
        for session_id in session_ids_to_parse:
            session_msg_dir = message_dir / session_id
            if session_msg_dir.exists():
                all_msg_files.extend(session_msg_dir.glob("msg_*.json"))

        messages_by_session: dict[str, list[tuple[Path, str, str]]] = defaultdict(list)
        with ThreadPoolExecutor(max_workers=16) as executor:
            for result in executor.map(read_message_file, all_msg_files):
                if result:
                    session_id, path, msg_id, role = result
                    messages_by_session[session_id].append((path, msg_id, role))

            # Sort sessions by message count (smallest first for faster initial results)
            sorted_sessions = sorted(
                sessions_to_parse, key=lambda x: len(messages_by_session.get(x[0], []))
            )

            # Step 2: Process sessions in batches (reuse executor)
            BATCH_SIZE = 5
            new_or_modified = []
            for i in range(0, len(sorted_sessions), BATCH_SIZE):
                batch = sorted_sessions[i : i + BATCH_SIZE]

                # Collect part files for this batch
                batch_part_files = []
                for session_id, _, _ in batch:
                    for _, msg_id, _ in messages_by_session.get(session_id, []):
                        msg_part_dir = part_dir / msg_id
                        if msg_part_dir.exists():
                            batch_part_files.extend(msg_part_dir.glob("*.json"))

                # Read parts in parallel
                parts_by_message: dict[str, list[str]] = defaultdict(list)
                for result in executor.map(read_part_file, batch_part_files):
                    if result:
                        msg_id, text = result
                        parts_by_message[msg_id].append(text)

                # Parse and callback
                for session_id, path, mtime in batch:
                    session = self._parse_session(
                        path, messages_by_session, parts_by_message, on_error=on_error
                    )
                    if session:
                        session.mtime = mtime
                        new_or_modified.append(session)
                        if on_session:
                            on_session(session)

        return new_or_modified, deleted_ids

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to resume an OpenCode session."""
        return ["opencode", session.directory, "--session", session.id]

    def get_raw_stats(self) -> RawAdapterStats:
        """Get raw statistics from the OpenCode data folder."""
        if not self.is_available():
            return RawAdapterStats(
                agent=self.name,
                data_dir=str(self._sessions_dir),
                available=False,
                file_count=0,
                total_bytes=0,
            )

        # Count all files: session, message, and part directories
        file_count = 0
        total_bytes = 0

        for subdir in ["session", "message", "part"]:
            dir_path = self._sessions_dir / subdir
            if dir_path.exists():
                for json_file in dir_path.rglob("*.json"):
                    try:
                        file_count += 1
                        total_bytes += json_file.stat().st_size
                    except OSError:
                        pass

        return RawAdapterStats(
            agent=self.name,
            data_dir=str(self._sessions_dir),
            available=True,
            file_count=file_count,
            total_bytes=total_bytes,
        )
