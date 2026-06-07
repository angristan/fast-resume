"""Cursor session adapter."""

import orjson
import sqlite3
import sys
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from urllib.parse import unquote, urlparse

from ..config import AGENTS
from ..logging_config import log_parse_error
from .base import (
    ErrorCallback,
    ParseError,
    RawAdapterStats,
    Session,
    SessionCallback,
    truncate_title,
)

# Cursor storage paths vary by platform
if sys.platform == "darwin":
    CURSOR_USER_DIR = Path.home() / "Library" / "Application Support" / "Cursor" / "User"
elif sys.platform == "win32":
    CURSOR_USER_DIR = Path.home() / "AppData" / "Roaming" / "Cursor" / "User"
else:  # Linux
    CURSOR_USER_DIR = Path.home() / ".config" / "Cursor" / "User"


class CursorAdapter:
    """Adapter for Cursor editor sessions stored in SQLite."""

    name = "cursor"
    color = AGENTS["cursor"]["color"]
    badge = AGENTS["cursor"]["badge"]
    supports_yolo = False

    def __init__(
        self,
        user_dir: Path | None = None,
        global_db_path: Path | None = None,
        workspace_storage_dir: Path | None = None,
    ) -> None:
        self._user_dir = user_dir if user_dir is not None else CURSOR_USER_DIR
        self._global_db_path = (
            global_db_path
            if global_db_path is not None
            else self._user_dir / "globalStorage" / "state.vscdb"
        )
        self._workspace_storage_dir = (
            workspace_storage_dir
            if workspace_storage_dir is not None
            else self._user_dir / "workspaceStorage"
        )

    def _emit_error(
        self,
        file_path: str | Path,
        error_type: str,
        message: str,
        on_error: ErrorCallback = None,
    ) -> None:
        """Emit and log a parse error."""
        error = ParseError(
            agent=self.name,
            file_path=str(file_path),
            error_type=error_type,
            message=message,
        )
        log_parse_error(error.agent, error.file_path, error.error_type, error.message)
        if on_error:
            on_error(error)

    def _session_id(self, composer_id: str) -> str:
        """Create a namespaced session ID to avoid cross-agent collisions."""
        return f"cursor:{composer_id}"

    def _composer_id_from_session_id(self, session_id: str) -> str:
        """Extract raw composer ID from a namespaced session ID."""
        if session_id.startswith("cursor:"):
            return session_id.split(":", 1)[1]
        return session_id

    def is_available(self) -> bool:
        """Check if Cursor data is available."""
        return self._global_db_path.exists() or self._workspace_storage_dir.exists()

    def _iter_workspace_dirs(self) -> list[Path]:
        """List Cursor workspace storage directories."""
        if not self._workspace_storage_dir.exists():
            return []

        dirs: list[Path] = []
        try:
            for workspace_dir in self._workspace_storage_dir.iterdir():
                if workspace_dir.is_dir():
                    dirs.append(workspace_dir)
        except OSError:
            return []

        return dirs

    def _sanitize_json_text(self, raw: str) -> str:
        """Sanitize Cursor JSON payloads that may include control chars."""
        return "".join(
            c if (c.isprintable() or c in "\n\r\t") else " " for c in raw
        )

    def _parse_cursor_json(
        self,
        raw: str,
        source: str | Path,
        on_error: ErrorCallback = None,
    ) -> dict | list | None:
        """Parse Cursor JSON safely after sanitization."""
        try:
            return orjson.loads(self._sanitize_json_text(raw))
        except orjson.JSONDecodeError as e:
            self._emit_error(
                source,
                "JSONDecodeError",
                str(e),
                on_error=on_error,
            )
            return None

    def _coerce_millis(self, value: object) -> int:
        """Coerce a Cursor millisecond timestamp into int milliseconds."""
        if isinstance(value, int):
            return value
        if isinstance(value, float):
            return int(value)
        if isinstance(value, str):
            try:
                return int(float(value))
            except ValueError:
                return 0
        return 0

    def _read_workspace_folder(self, workspace_json: Path) -> str:
        """Read and decode a workspace folder path from workspace.json."""
        if not workspace_json.exists():
            return ""

        try:
            with open(workspace_json, "rb") as f:
                data = orjson.loads(f.read())
        except OSError, orjson.JSONDecodeError:
            return ""

        folder = data.get("folder", "")
        if not isinstance(folder, str) or not folder:
            return ""

        if folder.startswith("file://"):
            parsed = urlparse(folder)
            path = unquote(parsed.path)
            # file:///C:/Users/... -> C:/Users/... on Windows
            if (
                sys.platform == "win32"
                and len(path) > 2
                and path.startswith("/")
                and path[2] == ":"
            ):
                path = path[1:]
            return path

        return folder

    def _read_workspace_composers(
        self, workspace_db: Path, on_error: ErrorCallback = None
    ) -> list[dict]:
        """Read composer list from a workspace state.vscdb database."""
        if not workspace_db.exists():
            return []

        conn: sqlite3.Connection | None = None
        try:
            conn = sqlite3.connect(str(workspace_db), timeout=5)
            conn.row_factory = sqlite3.Row
            cursor = conn.cursor()

            row = cursor.execute(
                "SELECT value FROM ItemTable WHERE key = 'composer.composerData'"
            ).fetchone()
            if not row:
                return []

            parsed = self._parse_cursor_json(
                row["value"],
                f"{workspace_db}:composer.composerData",
                on_error=on_error,
            )
            if not isinstance(parsed, dict):
                return []

            composers = parsed.get("allComposers", [])
            if not isinstance(composers, list):
                return []

            return [c for c in composers if isinstance(c, dict)]
        except sqlite3.Error as e:
            self._emit_error(
                workspace_db,
                "sqlite3.Error",
                str(e),
                on_error=on_error,
            )
            return []
        finally:
            if conn is not None:
                conn.close()

    @dataclass
    class _SessionMetadata:
        id: str
        composer_id: str
        title: str
        directory: str
        timestamp: datetime
        mtime: float

    def _scan_sessions_metadata(
        self, on_error: ErrorCallback = None
    ) -> dict[str, _SessionMetadata]:
        """Scan all workspaces and return active Cursor session metadata."""
        sessions: dict[str, CursorAdapter._SessionMetadata] = {}

        for workspace_dir in self._iter_workspace_dirs():
            ws_db = workspace_dir / "state.vscdb"
            if not ws_db.exists():
                continue

            try:
                ws_mtime = ws_db.stat().st_mtime
            except OSError:
                continue

            workspace_folder = self._read_workspace_folder(workspace_dir / "workspace.json")
            composers = self._read_workspace_composers(ws_db, on_error=on_error)

            for composer in composers:
                if composer.get("isArchived") is True:
                    continue

                composer_id = composer.get("composerId", "")
                if not isinstance(composer_id, str) or not composer_id:
                    continue

                title = composer.get("name", "Untitled session")
                if not isinstance(title, str) or not title.strip():
                    title = "Untitled session"

                created_ms = self._coerce_millis(composer.get("createdAt"))
                updated_ms = self._coerce_millis(composer.get("lastUpdatedAt"))
                time_ms = max(created_ms, updated_ms)

                timestamp = (
                    datetime.fromtimestamp(time_ms / 1000)
                    if time_ms > 0
                    else datetime.fromtimestamp(ws_mtime)
                )
                mtime = max(ws_mtime, (time_ms / 1000) if time_ms > 0 else 0.0)

                session_id = self._session_id(composer_id)
                existing = sessions.get(session_id)
                if existing is not None:
                    if mtime <= existing.mtime:
                        continue

                sessions[session_id] = self._SessionMetadata(
                    id=session_id,
                    composer_id=composer_id,
                    title=title.strip(),
                    directory=workspace_folder,
                    timestamp=timestamp,
                    mtime=mtime,
                )

        return sessions

    def _connect_global_db(self, on_error: ErrorCallback = None) -> sqlite3.Connection | None:
        """Open Cursor global state database."""
        if not self._global_db_path.exists():
            return None

        try:
            conn = sqlite3.connect(str(self._global_db_path), timeout=5)
            conn.row_factory = sqlite3.Row
            return conn
        except sqlite3.Error as e:
            self._emit_error(
                self._global_db_path,
                "sqlite3.Error",
                str(e),
                on_error=on_error,
            )
            return None

    def _clean_text(self, text: object) -> str:
        """Normalize text payloads from bubbles."""
        if not isinstance(text, str):
            return ""
        return text.strip()

    def _append_bubble_content(
        self, lines: list[str], bubble: dict, bubble_type: int
    ) -> bool:
        """Append bubble content and return whether a turn was added."""
        text = self._clean_text(bubble.get("text"))

        if bubble_type == 1:
            if text:
                lines.append(f"» {text}")
                return True
            return False

        added = False

        tool_data = bubble.get("toolFormerData")
        if isinstance(tool_data, dict):
            tool_name = tool_data.get("name", "")
            if isinstance(tool_name, str) and tool_name:
                lines.append(f"  [tool {tool_name}]")
                added = True

        if text:
            lines.append(f"  {text}")
            added = True

        return added

    def _load_messages_for_composer(
        self,
        conn: sqlite3.Connection,
        composer_id: str,
        on_error: ErrorCallback = None,
    ) -> tuple[list[str], int]:
        """Load ordered bubble content for a Cursor composer."""
        try:
            cursor = conn.cursor()
            composer_key = f"composerData:{composer_id}"
            row = cursor.execute(
                "SELECT value FROM cursorDiskKV WHERE key = ?",
                (composer_key,),
            ).fetchone()
            if not row:
                return [], 0

            parsed = self._parse_cursor_json(
                row["value"],
                f"{self._global_db_path}:{composer_key}",
                on_error=on_error,
            )
            if not isinstance(parsed, dict):
                return [], 0

            headers = parsed.get("fullConversationHeadersOnly", [])
            if not isinstance(headers, list):
                return [], 0

            bubble_prefix = f"bubbleId:{composer_id}:"
            bubble_rows = cursor.execute(
                "SELECT key, value FROM cursorDiskKV WHERE key LIKE ?",
                (f"{bubble_prefix}%",),
            ).fetchall()

            bubble_map: dict[str, str] = {}
            for bubble_row in bubble_rows:
                key = bubble_row["key"]
                value = bubble_row["value"]
                if (
                    isinstance(key, str)
                    and key.startswith(bubble_prefix)
                    and isinstance(value, str)
                ):
                    bubble_map[key[len(bubble_prefix) :]] = value

            lines: list[str] = []
            turn_count = 0

            for header in headers:
                if not isinstance(header, dict):
                    continue

                bubble_id = header.get("bubbleId", "")
                if not isinstance(bubble_id, str) or not bubble_id:
                    continue

                bubble_raw = bubble_map.get(bubble_id)
                if not bubble_raw:
                    continue

                bubble = self._parse_cursor_json(
                    bubble_raw,
                    f"{self._global_db_path}:{bubble_prefix}{bubble_id}",
                    on_error=on_error,
                )
                if not isinstance(bubble, dict):
                    continue

                bubble_type = header.get("type")
                if isinstance(bubble_type, float):
                    bubble_type = int(bubble_type)
                elif not isinstance(bubble_type, int):
                    raw_type = bubble.get("type")
                    if isinstance(raw_type, float):
                        bubble_type = int(raw_type)
                    elif isinstance(raw_type, int):
                        bubble_type = raw_type
                    else:
                        bubble_type = 2

                if self._append_bubble_content(lines, bubble, bubble_type):
                    turn_count += 1

            return lines, turn_count
        except sqlite3.Error as e:
            self._emit_error(
                self._global_db_path,
                "sqlite3.Error",
                str(e),
                on_error=on_error,
            )
            return [], 0

    def _build_session(
        self,
        metadata: _SessionMetadata,
        global_conn: sqlite3.Connection | None,
        on_error: ErrorCallback = None,
    ) -> Session:
        """Build a Session from scanned metadata and global DB messages."""
        title = metadata.title
        lines: list[str] = []
        turn_count = 0

        if global_conn is not None:
            lines, turn_count = self._load_messages_for_composer(
                global_conn,
                metadata.composer_id,
                on_error=on_error,
            )

        content = "\n\n".join(lines) if lines else title

        return Session(
            id=metadata.id,
            agent=self.name,
            title=truncate_title(title),
            directory=metadata.directory,
            timestamp=metadata.timestamp,
            content=content,
            message_count=turn_count,
        )

    def find_sessions(self) -> list[Session]:
        """Find all Cursor sessions."""
        if not self.is_available():
            return []

        metadata = self._scan_sessions_metadata()
        if not metadata:
            return []

        sessions: list[Session] = []
        conn = self._connect_global_db()
        try:
            for data in metadata.values():
                session = self._build_session(data, conn)
                session.mtime = data.mtime
                sessions.append(session)
        finally:
            if conn is not None:
                conn.close()

        return sessions

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

        metadata = self._scan_sessions_metadata(on_error=on_error)
        current_ids = set(metadata.keys())

        deleted_ids = [
            sid
            for sid, (_, agent) in known.items()
            if agent == self.name and sid not in current_ids
        ]

        conn = self._connect_global_db(on_error=on_error)
        new_or_modified: list[Session] = []

        try:
            for session_id, data in metadata.items():
                mtime = data.mtime
                known_entry = known.get(session_id)
                if known_entry is None or mtime > known_entry[0] + 0.001:
                    session = self._build_session(data, conn, on_error=on_error)
                    session.mtime = mtime
                    new_or_modified.append(session)
                    if on_session:
                        on_session(session)
        finally:
            if conn is not None:
                conn.close()

        return new_or_modified, deleted_ids

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to open Cursor in the session workspace."""
        if session.directory:
            return ["cursor", session.directory]
        return ["cursor"]

    def get_raw_stats(self) -> RawAdapterStats:
        """Get raw statistics from Cursor state databases."""
        if not self.is_available():
            return RawAdapterStats(
                agent=self.name,
                data_dir=str(self._user_dir),
                available=False,
                file_count=0,
                total_bytes=0,
            )

        file_count = 0
        total_bytes = 0

        db_files = []
        if self._global_db_path.exists():
            db_files.append(self._global_db_path)

        for workspace_dir in self._iter_workspace_dirs():
            ws_db = workspace_dir / "state.vscdb"
            if ws_db.exists():
                db_files.append(ws_db)

        for db_file in db_files:
            try:
                file_count += 1
                total_bytes += db_file.stat().st_size
            except OSError:
                pass

        return RawAdapterStats(
            agent=self.name,
            data_dir=str(self._user_dir),
            available=True,
            file_count=file_count,
            total_bytes=total_bytes,
        )
