"""OpenCode session adapter.

Supports both the new SQLite format (OpenCode 1.2+) and the legacy
split-JSON-files format. The SQLite database at opencode.db is preferred
when present.
"""

import orjson
import sqlite3
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, OPENCODE_DB, OPENCODE_DIR, OPENCODE_LEGACY_DIR
from ..logging_config import log_parse_error
from .base import ErrorCallback, ParseError, RawAdapterStats, Session, SessionCallback


class OpenCodeAdapter:
    """Adapter for OpenCode sessions.

    Reads from the SQLite database (opencode.db) when available,
    falling back to the legacy split-JSON storage directory.
    """

    name = "opencode"
    color = AGENTS["opencode"]["color"]
    badge = AGENTS["opencode"]["badge"]
    supports_yolo = False

    def __init__(
        self,
        *,
        data_dir: Path | None = None,
        db_path: Path | None = None,
        legacy_dir: Path | None = None,
    ) -> None:
        self._data_dir = data_dir if data_dir is not None else OPENCODE_DIR
        self._db_path = db_path if db_path is not None else OPENCODE_DB
        self._legacy_dir = legacy_dir if legacy_dir is not None else OPENCODE_LEGACY_DIR

    def _has_sqlite(self) -> bool:
        """Check if the SQLite database exists."""
        return self._db_path.exists()

    def _has_legacy(self) -> bool:
        """Check if the legacy JSON storage directory exists."""
        return self._legacy_dir.exists() and (self._legacy_dir / "session").exists()

    def is_available(self) -> bool:
        """Check if OpenCode data is available (SQLite or legacy)."""
        return self._has_sqlite() or self._has_legacy()

    @property
    def backend(self) -> str:
        """Return which storage backend is active: 'sqlite', 'json', or 'none'."""
        if self._has_sqlite():
            return "sqlite"
        if self._has_legacy():
            return "json"
        return "none"

    # ── SQLite methods ────────────────────────────────────────────────

    def _load_sessions_from_db(self, on_error: ErrorCallback = None) -> list[Session]:
        """Load all sessions from the SQLite database."""
        sessions = []
        try:
            conn = sqlite3.connect(str(self._db_path), timeout=5)
            conn.row_factory = sqlite3.Row
            cursor = conn.cursor()

            # Fetch all sessions with their project worktree for directory
            cursor.execute("""
                SELECT
                    s.id,
                    s.title,
                    s.directory,
                    s.time_created,
                    s.time_updated
                FROM session s
                ORDER BY s.time_updated DESC
            """)

            session_rows = cursor.fetchall()

            # Build a set of session IDs for batch queries
            session_ids = [row["id"] for row in session_rows]
            if not session_ids:
                conn.close()
                return []

            # Fetch message roles via json_extract (avoids Python-side JSON parsing)
            CHUNK = 900
            messages_by_session: dict[str, list[tuple[str, str]]] = defaultdict(list)
            for i in range(0, len(session_ids), CHUNK):
                chunk = session_ids[i : i + CHUNK]
                cursor.execute(
                    """
                    SELECT m.id, m.session_id, json_extract(m.data, '$.role')
                    FROM message m
                    WHERE m.session_id IN ({})
                    ORDER BY m.time_created ASC
                """.format(",".join("?" * len(chunk))),
                    chunk,
                )
                for row in cursor.fetchall():
                    messages_by_session[row[1]].append((row[0], row[2] or ""))

            # Fetch text parts via json_extract, filtered in SQL
            parts_by_message: dict[str, list[str]] = defaultdict(list)
            for i in range(0, len(session_ids), CHUNK):
                chunk = session_ids[i : i + CHUNK]
                cursor.execute(
                    """
                    SELECT p.message_id, json_extract(p.data, '$.text')
                    FROM part p
                    WHERE p.session_id IN ({})
                      AND json_extract(p.data, '$.type') = 'text'
                    ORDER BY p.time_created ASC
                """.format(",".join("?" * len(chunk))),
                    chunk,
                )
                for row in cursor.fetchall():
                    if row[1]:
                        parts_by_message[row[0]].append(row[1])

            conn.close()

            # Build Session objects
            for row in session_rows:
                session = self._build_session_from_row(
                    row, messages_by_session, parts_by_message, on_error=on_error
                )
                if session:
                    sessions.append(session)

        except sqlite3.Error as e:
            error = ParseError(
                agent=self.name,
                file_path=str(self._db_path),
                error_type="sqlite3.Error",
                message=str(e),
            )
            log_parse_error(
                error.agent, error.file_path, error.error_type, error.message
            )
            if on_error:
                on_error(error)

        return sessions

    def _build_session_from_row(
        self,
        row,
        messages_by_session: dict[str, list[tuple[str, str]]],
        parts_by_message: dict[str, list[str]],
        on_error: ErrorCallback = None,
    ) -> Session | None:
        """Build a Session from a SQLite row + pre-fetched messages/parts."""
        try:
            session_id = row["id"]
            title = row["title"] or "Untitled session"
            directory = row["directory"] or ""

            # Timestamps are integer milliseconds
            time_created = row["time_created"] or 0
            time_updated = row["time_updated"] or 0
            time_ms = max(time_created, time_updated)
            if time_ms:
                timestamp = datetime.fromtimestamp(time_ms / 1000)
            else:
                timestamp = datetime.now()

            # Build message content
            messages: list[str] = []
            session_msgs = messages_by_session.get(session_id, [])
            for msg_id, role in session_msgs:
                role_prefix = "» " if role == "user" else "  "
                for text in parts_by_message.get(msg_id, []):
                    messages.append(f"{role_prefix}{text}")

            full_content = "\n\n".join(messages)

            return Session(
                id=session_id,
                agent=self.name,
                title=title,
                directory=directory,
                timestamp=timestamp,
                content=full_content,
                message_count=len(session_msgs),
            )
        except (KeyError, TypeError, ValueError) as e:
            error = ParseError(
                agent=self.name,
                file_path=f"opencode_db:{row['id'] if row else 'unknown'}",
                error_type=type(e).__name__,
                message=str(e),
            )
            log_parse_error(
                error.agent, error.file_path, error.error_type, error.message
            )
            if on_error:
                on_error(error)
            return None

    def _load_sessions_incremental_sqlite(
        self,
        known: dict[str, tuple[float, str]],
        on_error: ErrorCallback = None,
        on_session: SessionCallback = None,
    ) -> tuple[list[Session], list[str]]:
        """Incremental session loading from SQLite database."""
        new_or_modified = []
        all_current_ids: set[str] = set()

        try:
            conn = sqlite3.connect(str(self._db_path), timeout=5)
            conn.row_factory = sqlite3.Row
            cursor = conn.cursor()

            # First pass: get all session IDs and timestamps to determine changes
            cursor.execute("""
                SELECT id, title, directory, time_created, time_updated
                FROM session
            """)

            sessions_to_fetch: list[dict] = []
            session_ids_to_fetch: set[str] = set()

            for row in cursor.fetchall():
                session_id = row["id"]
                all_current_ids.add(session_id)

                time_created = row["time_created"] or 0
                time_updated = row["time_updated"] or 0
                time_ms = max(time_created, time_updated)
                if time_ms:
                    mtime = datetime.fromtimestamp(time_ms / 1000).timestamp()
                else:
                    mtime = self._db_path.stat().st_mtime

                known_entry = known.get(session_id)
                if known_entry is None or mtime > known_entry[0] + 0.001:
                    sessions_to_fetch.append(
                        {
                            "id": session_id,
                            "title": row["title"],
                            "directory": row["directory"],
                            "time_created": time_created,
                            "time_updated": time_updated,
                            "mtime": mtime,
                        }
                    )
                    session_ids_to_fetch.add(session_id)

            # Find deleted sessions
            deleted_ids = [
                sid
                for sid, (_, agent) in known.items()
                if agent == self.name and sid not in all_current_ids
            ]

            if not sessions_to_fetch:
                conn.close()
                return [], deleted_ids

            # Fetch message roles via json_extract
            fetch_ids = list(session_ids_to_fetch)
            messages_by_session: dict[str, list[tuple[str, str]]] = defaultdict(list)

            CHUNK = 900
            for i in range(0, len(fetch_ids), CHUNK):
                chunk = fetch_ids[i : i + CHUNK]
                cursor.execute(
                    """
                    SELECT id, session_id, json_extract(data, '$.role')
                    FROM message
                    WHERE session_id IN ({})
                    ORDER BY time_created ASC
                """.format(",".join("?" * len(chunk))),
                    chunk,
                )
                for row in cursor.fetchall():
                    messages_by_session[row[1]].append((row[0], row[2] or ""))

            # Fetch text parts via json_extract, filtered in SQL
            parts_by_message: dict[str, list[str]] = defaultdict(list)
            for i in range(0, len(fetch_ids), CHUNK):
                chunk = fetch_ids[i : i + CHUNK]
                cursor.execute(
                    """
                    SELECT message_id, json_extract(data, '$.text')
                    FROM part
                    WHERE session_id IN ({})
                      AND json_extract(data, '$.type') = 'text'
                    ORDER BY time_created ASC
                """.format(",".join("?" * len(chunk))),
                    chunk,
                )
                for row in cursor.fetchall():
                    if row[1]:
                        parts_by_message[row[0]].append(row[1])

            conn.close()

            # Build sessions and invoke callbacks
            for sdata in sessions_to_fetch:
                session = self._build_session_from_dict(
                    sdata, messages_by_session, parts_by_message, on_error=on_error
                )
                if session:
                    session.mtime = sdata["mtime"]
                    new_or_modified.append(session)
                    if on_session:
                        on_session(session)

        except sqlite3.Error as e:
            error = ParseError(
                agent=self.name,
                file_path=str(self._db_path),
                error_type="sqlite3.Error",
                message=str(e),
            )
            log_parse_error(
                error.agent, error.file_path, error.error_type, error.message
            )
            if on_error:
                on_error(error)
            # If db fails, treat all known sessions as deleted
            deleted_ids = [
                sid for sid, (_, agent) in known.items() if agent == self.name
            ]
            return [], deleted_ids

        return new_or_modified, deleted_ids

    def _build_session_from_dict(
        self,
        sdata: dict,
        messages_by_session: dict[str, list[tuple[str, str]]],
        parts_by_message: dict[str, list[str]],
        on_error: ErrorCallback = None,
    ) -> Session | None:
        """Build a Session from a dict of session data + pre-fetched messages/parts."""
        try:
            session_id = sdata["id"]
            title = sdata["title"] or "Untitled session"
            directory = sdata["directory"] or ""

            time_created = sdata["time_created"] or 0
            time_updated = sdata["time_updated"] or 0
            time_ms = max(time_created, time_updated)
            if time_ms:
                timestamp = datetime.fromtimestamp(time_ms / 1000)
            else:
                timestamp = datetime.now()

            messages: list[str] = []
            session_msgs = messages_by_session.get(session_id, [])
            for msg_id, role in session_msgs:
                role_prefix = "» " if role == "user" else "  "
                for text in parts_by_message.get(msg_id, []):
                    messages.append(f"{role_prefix}{text}")

            full_content = "\n\n".join(messages)

            return Session(
                id=session_id,
                agent=self.name,
                title=title,
                directory=directory,
                timestamp=timestamp,
                content=full_content,
                message_count=len(session_msgs),
            )
        except (KeyError, TypeError, ValueError) as e:
            error = ParseError(
                agent=self.name,
                file_path=f"opencode_db:{sdata.get('id', 'unknown')}",
                error_type=type(e).__name__,
                message=str(e),
            )
            log_parse_error(
                error.agent, error.file_path, error.error_type, error.message
            )
            if on_error:
                on_error(error)
            return None

    # ── Legacy JSON methods ───────────────────────────────────────────

    def _find_sessions_legacy(self) -> list[Session]:
        """Find all sessions from legacy split-JSON storage."""
        sessions = []
        session_dir = self._legacy_dir / "session"
        message_dir = self._legacy_dir / "message"
        part_dir = self._legacy_dir / "part"

        if not session_dir.exists():
            return []

        # Pre-index all messages by session_id
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

        # Pre-index all parts by message_id
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

        for project_dir in session_dir.iterdir():
            if not project_dir.is_dir():
                continue
            for session_file in project_dir.glob("ses_*.json"):
                session = self._parse_legacy_session(
                    session_file, messages_by_session, parts_by_message
                )
                if session:
                    sessions.append(session)

        return sessions

    def _parse_legacy_session(
        self,
        session_file: Path,
        messages_by_session: dict[str, list[tuple[Path, str, str]]],
        parts_by_message: dict[str, list[str]],
        on_error: ErrorCallback = None,
    ) -> Session | None:
        """Parse a legacy JSON session file."""
        try:
            with open(session_file, "rb") as f:
                data = orjson.loads(f.read())

            session_id = data.get("id", "")
            title = data.get("title", "Untitled session")
            directory = data.get("directory", "")

            time_data = data.get("time", {})
            created = time_data.get("created", 0)
            updated = time_data.get("updated", 0)
            time_ms = max(created, updated) if (created or updated) else 0
            if time_ms:
                timestamp = datetime.fromtimestamp(time_ms / 1000)
            else:
                timestamp = datetime.fromtimestamp(session_file.stat().st_mtime)

            messages = self._get_legacy_messages(
                session_id, messages_by_session, parts_by_message
            )
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

    def _get_legacy_messages(
        self,
        session_id: str,
        messages_by_session: dict[str, list[tuple[Path, str, str]]],
        parts_by_message: dict[str, list[str]],
    ) -> list[str]:
        """Get all messages for a session from pre-indexed legacy parts."""
        messages: list[str] = []
        session_msgs = sorted(
            messages_by_session.get(session_id, []), key=lambda x: x[0].name
        )
        for _msg_file, msg_id, role in session_msgs:
            role_prefix = "» " if role == "user" else "  "
            for text in parts_by_message.get(msg_id, []):
                messages.append(f"{role_prefix}{text}")
        return messages

    def _find_sessions_incremental_legacy(
        self,
        known: dict[str, tuple[float, str]],
        on_error: ErrorCallback = None,
        on_session: SessionCallback = None,
    ) -> tuple[list[Session], list[str]]:
        """Incremental session loading from legacy JSON storage."""
        session_dir = self._legacy_dir / "session"
        if not session_dir.exists():
            deleted_ids = [
                sid for sid, (_, agent) in known.items() if agent == self.name
            ]
            return [], deleted_ids

        # Scan session files and get timestamps
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
                        time_data = data.get("time", {})
                        created = time_data.get("created", 0)
                        updated = time_data.get("updated", 0)
                        time_ms = max(created, updated) if (created or updated) else 0
                        if time_ms:
                            mtime = datetime.fromtimestamp(time_ms / 1000).timestamp()
                        else:
                            mtime = session_file.stat().st_mtime
                        current_sessions[session_id] = (session_file, mtime)
                except OSError, orjson.JSONDecodeError:
                    continue

        # Check which sessions need parsing
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
        message_dir = self._legacy_dir / "message"
        part_dir = self._legacy_dir / "part"

        def read_message_file(msg_file: Path) -> tuple[str, Path, str, str] | None:
            try:
                with open(msg_file, "rb") as f:
                    data = orjson.loads(f.read())
                msg_id = data.get("id", "")
                role = data.get("role", "")
                if msg_id:
                    return (msg_file.parent.name, msg_file, msg_id, role)
            except OSError, orjson.JSONDecodeError:
                pass
            return None

        def read_part_file(part_file: Path) -> tuple[str, str] | None:
            try:
                with open(part_file, "rb") as f:
                    data = orjson.loads(f.read())
                if data.get("type") == "text":
                    text = data.get("text", "")
                    if text:
                        return (part_file.parent.name, text)
            except OSError, orjson.JSONDecodeError:
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

            sorted_sessions = sorted(
                sessions_to_parse, key=lambda x: len(messages_by_session.get(x[0], []))
            )

            # Step 2: Process sessions in batches
            BATCH_SIZE = 5
            new_or_modified = []
            for i in range(0, len(sorted_sessions), BATCH_SIZE):
                batch = sorted_sessions[i : i + BATCH_SIZE]

                batch_part_files = []
                for session_id, _, _ in batch:
                    for _, msg_id, _ in messages_by_session.get(session_id, []):
                        msg_part_dir = part_dir / msg_id
                        if msg_part_dir.exists():
                            batch_part_files.extend(msg_part_dir.glob("*.json"))

                parts_by_message: dict[str, list[str]] = defaultdict(list)
                for result in executor.map(read_part_file, batch_part_files):
                    if result:
                        msg_id, text = result
                        parts_by_message[msg_id].append(text)

                for session_id, path, mtime in batch:
                    session = self._parse_legacy_session(
                        path, messages_by_session, parts_by_message, on_error=on_error
                    )
                    if session:
                        session.mtime = mtime
                        new_or_modified.append(session)
                        if on_session:
                            on_session(session)

        return new_or_modified, deleted_ids

    # ── Public interface ──────────────────────────────────────────────

    def find_sessions(self) -> list[Session]:
        """Find all OpenCode sessions."""
        if not self.is_available():
            return []

        if self._has_sqlite():
            return self._load_sessions_from_db()

        return self._find_sessions_legacy()

    def find_sessions_incremental(
        self,
        known: dict[str, tuple[float, str]],
        on_error: ErrorCallback = None,
        on_session: SessionCallback = None,
    ) -> tuple[list[Session], list[str]]:
        """Find sessions incrementally, comparing against known sessions."""
        if not self.is_available():
            deleted_ids = [
                sid for sid, (_, agent) in known.items() if agent == self.name
            ]
            return [], deleted_ids

        if self._has_sqlite():
            return self._load_sessions_incremental_sqlite(
                known, on_error=on_error, on_session=on_session
            )

        return self._find_sessions_incremental_legacy(
            known, on_error=on_error, on_session=on_session
        )

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to resume an OpenCode session."""
        return ["opencode", session.directory, "--session", session.id]

    def get_raw_stats(self) -> RawAdapterStats:
        """Get raw statistics from the OpenCode data."""
        if not self.is_available():
            return RawAdapterStats(
                agent=self.name,
                data_dir=str(self._data_dir),
                available=False,
                file_count=0,
                total_bytes=0,
            )

        file_count = 0
        total_bytes = 0

        # Count SQLite database
        if self._has_sqlite():
            try:
                file_count += 1
                total_bytes += self._db_path.stat().st_size
                # Also count WAL/SHM files
                for suffix in ["-wal", "-shm"]:
                    wal = self._db_path.with_name(self._db_path.name + suffix)
                    if wal.exists():
                        file_count += 1
                        total_bytes += wal.stat().st_size
            except OSError:
                pass

        # Only count legacy JSON files if SQLite is not available
        # (scanning 200k+ JSON files takes 25+ seconds)
        if not self._has_sqlite() and self._has_legacy():
            for subdir in ["session", "message", "part"]:
                dir_path = self._legacy_dir / subdir
                if dir_path.exists():
                    for json_file in dir_path.rglob("*.json"):
                        try:
                            file_count += 1
                            total_bytes += json_file.stat().st_size
                        except OSError:
                            pass

        return RawAdapterStats(
            agent=self.name,
            data_dir=f"{self._data_dir} ({self.backend})",
            available=True,
            file_count=file_count,
            total_bytes=total_bytes,
        )
