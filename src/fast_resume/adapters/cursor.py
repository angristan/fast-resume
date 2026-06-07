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

CURSOR_PROJECTS_DIR = Path.home() / ".cursor" / "projects"


@dataclass
class _GlobalComposerData:
    title: str
    created_ms: int
    updated_ms: int
    mtime: float
    directory: str


@dataclass
class _WorkspaceComposerData:
    title: str
    created_ms: int
    updated_ms: int
    mtime: float
    directory: str


@dataclass
class _TranscriptData:
    file_path: Path
    mtime: float
    directory: str
    title_hint: str


@dataclass
class _SessionMetadata:
    id: str
    composer_id: str
    title: str
    directory: str
    timestamp: datetime
    mtime: float
    transcript_file: Path | None


class CursorAdapter:
    """Adapter for Cursor sessions stored in SQLite and local transcripts."""

    name = "cursor"
    color = AGENTS["cursor"]["color"]
    badge = AGENTS["cursor"]["badge"]
    supports_yolo = False

    def __init__(
        self,
        user_dir: Path | None = None,
        global_db_path: Path | None = None,
        workspace_storage_dir: Path | None = None,
        projects_dir: Path | None = None,
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
        self._projects_dir = projects_dir if projects_dir is not None else CURSOR_PROJECTS_DIR

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

    def is_available(self) -> bool:
        """Check if Cursor data is available."""
        return (
            self._global_db_path.exists()
            or self._workspace_storage_dir.exists()
            or self._projects_dir.exists()
        )

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

    def _iter_transcript_files(self) -> list[Path]:
        """List top-level Cursor agent transcript files."""
        if not self._projects_dir.exists():
            return []

        try:
            # Match: <project>/agent-transcripts/<session>/<session>.jsonl
            return sorted(self._projects_dir.glob("*/agent-transcripts/*/*.jsonl"))
        except OSError:
            return []

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

    def _decode_file_uri(self, value: str) -> str:
        """Decode file URI into a local path when possible."""
        if not value.startswith("file://"):
            return value

        parsed = urlparse(value)
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

        return self._decode_file_uri(folder)

    def _extract_directory_from_global(self, composer: dict) -> str:
        """Extract workspace directory from a global composer payload."""
        context = composer.get("context")
        if not isinstance(context, dict):
            return ""

        folder_selections = context.get("folderSelections")
        if not isinstance(folder_selections, list):
            return ""

        for entry in folder_selections:
            if not isinstance(entry, dict):
                continue

            for key in ("path", "fsPath", "uri", "folder"):
                raw = entry.get(key)
                if isinstance(raw, str) and raw:
                    return self._decode_file_uri(raw)

        return ""

    def _decode_transcript_project_key(self, project_key: str) -> str:
        """Decode ~/.cursor/projects key into a likely absolute path."""
        if project_key == "empty-window":
            return ""

        decoded = unquote(project_key).replace("-", "/")
        if decoded and not decoded.startswith("/"):
            decoded = "/" + decoded
        return decoded

    def _extract_transcript_title(
        self,
        transcript_file: Path,
        on_error: ErrorCallback = None,
    ) -> str:
        """Extract a best-effort title from the first user text in transcript."""
        try:
            with open(transcript_file, "rb") as f:
                for line in f:
                    if not line.strip():
                        continue
                    try:
                        record = orjson.loads(line)
                    except orjson.JSONDecodeError:
                        continue

                    if record.get("role") != "user":
                        continue

                    message = record.get("message", {})
                    content = message.get("content", "") if isinstance(message, dict) else ""

                    if isinstance(content, str):
                        text = content.strip()
                        if text:
                            return truncate_title(text)
                    elif isinstance(content, list):
                        for part in content:
                            if isinstance(part, dict) and part.get("type") == "text":
                                text = str(part.get("text", "")).strip()
                                if text:
                                    return truncate_title(text)
                            elif isinstance(part, str):
                                text = part.strip()
                                if text:
                                    return truncate_title(text)
        except OSError as e:
            self._emit_error(
                transcript_file,
                "OSError",
                str(e),
                on_error=on_error,
            )

        return ""

    def _load_workspace_composers(
        self,
        workspace_db: Path,
        workspace_dir: str,
        workspace_mtime: float,
        on_error: ErrorCallback = None,
    ) -> dict[str, _WorkspaceComposerData]:
        """Load composer hints from a workspace DB (old and new schema)."""
        results: dict[str, _WorkspaceComposerData] = {}
        conn: sqlite3.Connection | None = None

        try:
            conn = sqlite3.connect(str(workspace_db), timeout=5)
            conn.row_factory = sqlite3.Row
            cursor = conn.cursor()

            row = cursor.execute(
                "SELECT value FROM ItemTable WHERE key = 'composer.composerData'"
            ).fetchone()
            if not row:
                return {}

            parsed = self._parse_cursor_json(
                row["value"],
                f"{workspace_db}:composer.composerData",
                on_error=on_error,
            )
            if not isinstance(parsed, dict):
                return {}

            # Old shape: allComposers with metadata
            composers = parsed.get("allComposers", [])
            if isinstance(composers, list):
                for composer in composers:
                    if not isinstance(composer, dict):
                        continue
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
                    mtime = max(workspace_mtime, (time_ms / 1000) if time_ms else 0.0)

                    current = results.get(composer_id)
                    if current is not None and mtime <= current.mtime:
                        continue

                    results[composer_id] = _WorkspaceComposerData(
                        title=title.strip(),
                        created_ms=created_ms,
                        updated_ms=updated_ms,
                        mtime=mtime,
                        directory=workspace_dir,
                    )

            # New shape: selected/last focused IDs only
            for key in ("selectedComposerIds", "lastFocusedComposerIds"):
                ids = parsed.get(key, [])
                if not isinstance(ids, list):
                    continue
                for composer_id in ids:
                    if not isinstance(composer_id, str) or not composer_id:
                        continue

                    current = results.get(composer_id)
                    if current is not None and workspace_mtime <= current.mtime:
                        continue

                    results[composer_id] = _WorkspaceComposerData(
                        title="Untitled session",
                        created_ms=0,
                        updated_ms=0,
                        mtime=workspace_mtime,
                        directory=workspace_dir,
                    )

            return results
        except sqlite3.Error as e:
            self._emit_error(
                workspace_db,
                "sqlite3.Error",
                str(e),
                on_error=on_error,
            )
            return {}
        finally:
            if conn is not None:
                conn.close()

    def _scan_workspace_data(
        self, on_error: ErrorCallback = None
    ) -> dict[str, _WorkspaceComposerData]:
        """Scan workspace DBs and return composer-level hints."""
        results: dict[str, _WorkspaceComposerData] = {}

        for workspace_path in self._iter_workspace_dirs():
            workspace_db = workspace_path / "state.vscdb"
            if not workspace_db.exists():
                continue

            try:
                workspace_mtime = workspace_db.stat().st_mtime
            except OSError:
                continue

            workspace_dir = self._read_workspace_folder(workspace_path / "workspace.json")
            workspace_entries = self._load_workspace_composers(
                workspace_db,
                workspace_dir,
                workspace_mtime,
                on_error=on_error,
            )

            for composer_id, entry in workspace_entries.items():
                current = results.get(composer_id)
                if current is not None and entry.mtime <= current.mtime:
                    continue
                results[composer_id] = entry

        return results

    def _scan_global_data(
        self, on_error: ErrorCallback = None
    ) -> dict[str, _GlobalComposerData]:
        """Scan global composerData records from cursorDiskKV."""
        results: dict[str, _GlobalComposerData] = {}
        if not self._global_db_path.exists():
            return results

        try:
            db_mtime = self._global_db_path.stat().st_mtime
        except OSError:
            db_mtime = 0.0

        conn: sqlite3.Connection | None = None
        try:
            conn = sqlite3.connect(str(self._global_db_path), timeout=5)
            conn.row_factory = sqlite3.Row
            cursor = conn.cursor()

            rows = cursor.execute(
                "SELECT key, value FROM cursorDiskKV WHERE key LIKE 'composerData:%'"
            ).fetchall()

            for row in rows:
                key = row["key"]
                value = row["value"]
                if not isinstance(key, str) or not key.startswith("composerData:"):
                    continue
                if not isinstance(value, str):
                    continue

                parsed = self._parse_cursor_json(
                    value,
                    f"{self._global_db_path}:{key}",
                    on_error=on_error,
                )
                if not isinstance(parsed, dict):
                    continue
                if parsed.get("isArchived") is True:
                    continue

                composer_id = parsed.get("composerId")
                if not isinstance(composer_id, str) or not composer_id:
                    composer_id = key.split(":", 1)[1]

                title = parsed.get("name", "Untitled session")
                if not isinstance(title, str) or not title.strip():
                    title = "Untitled session"

                created_ms = self._coerce_millis(parsed.get("createdAt"))
                updated_ms = self._coerce_millis(parsed.get("lastUpdatedAt"))
                time_ms = max(created_ms, updated_ms)
                mtime = max(db_mtime, (time_ms / 1000) if time_ms else 0.0)

                directory = self._extract_directory_from_global(parsed)

                current = results.get(composer_id)
                if current is not None and mtime <= current.mtime:
                    continue

                results[composer_id] = _GlobalComposerData(
                    title=title.strip(),
                    created_ms=created_ms,
                    updated_ms=updated_ms,
                    mtime=mtime,
                    directory=directory,
                )

            return results
        except sqlite3.Error as e:
            self._emit_error(
                self._global_db_path,
                "sqlite3.Error",
                str(e),
                on_error=on_error,
            )
            return {}
        finally:
            if conn is not None:
                conn.close()

    def _scan_transcript_data(
        self, on_error: ErrorCallback = None
    ) -> dict[str, _TranscriptData]:
        """Scan ~/.cursor/projects transcript files for session content."""
        results: dict[str, _TranscriptData] = {}

        for transcript_file in self._iter_transcript_files():
            composer_id = transcript_file.stem
            if not composer_id:
                continue

            # Ensure path shape: <id>/<id>.jsonl
            if transcript_file.parent.name != composer_id:
                continue

            project_dir = transcript_file.parent.parent.parent
            if project_dir.name == "agent-transcripts":
                continue

            try:
                mtime = transcript_file.stat().st_mtime
            except OSError:
                continue

            directory = self._decode_transcript_project_key(project_dir.name)
            title_hint = self._extract_transcript_title(transcript_file, on_error=on_error)

            current = results.get(composer_id)
            if current is not None and mtime <= current.mtime:
                continue

            results[composer_id] = _TranscriptData(
                file_path=transcript_file,
                mtime=mtime,
                directory=directory,
                title_hint=title_hint,
            )

        return results

    def _scan_sessions_metadata(
        self, on_error: ErrorCallback = None
    ) -> dict[str, _SessionMetadata]:
        """Scan all Cursor sources and build merged session metadata."""
        workspace_data = self._scan_workspace_data(on_error=on_error)
        global_data = self._scan_global_data(on_error=on_error)
        transcript_data = self._scan_transcript_data(on_error=on_error)

        all_composer_ids = (
            set(workspace_data.keys())
            | set(global_data.keys())
            | set(transcript_data.keys())
        )
        sessions: dict[str, _SessionMetadata] = {}

        for composer_id in all_composer_ids:
            ws = workspace_data.get(composer_id)
            gl = global_data.get(composer_id)
            ts = transcript_data.get(composer_id)

            title = "Untitled session"
            if gl is not None and gl.title and gl.title != "Untitled session":
                title = gl.title
            elif ws is not None and ws.title and ws.title != "Untitled session":
                title = ws.title
            elif ts is not None and ts.title_hint:
                title = ts.title_hint

            directory = ""
            if gl is not None and gl.directory:
                directory = gl.directory
            elif ws is not None and ws.directory:
                directory = ws.directory
            elif ts is not None and ts.directory:
                directory = ts.directory

            created_ms = 0
            updated_ms = 0
            if gl is not None:
                created_ms = max(created_ms, gl.created_ms)
                updated_ms = max(updated_ms, gl.updated_ms)
            if ws is not None:
                created_ms = max(created_ms, ws.created_ms)
                updated_ms = max(updated_ms, ws.updated_ms)

            max_mtime = 0.0
            if gl is not None:
                max_mtime = max(max_mtime, gl.mtime)
            if ws is not None:
                max_mtime = max(max_mtime, ws.mtime)
            if ts is not None:
                max_mtime = max(max_mtime, ts.mtime)

            time_ms = max(created_ms, updated_ms)
            if time_ms > 0:
                timestamp = datetime.fromtimestamp(time_ms / 1000)
                max_mtime = max(max_mtime, time_ms / 1000)
            elif max_mtime > 0:
                timestamp = datetime.fromtimestamp(max_mtime)
            else:
                timestamp = datetime.now()
                max_mtime = timestamp.timestamp()

            session_id = self._session_id(composer_id)
            sessions[session_id] = _SessionMetadata(
                id=session_id,
                composer_id=composer_id,
                title=title,
                directory=directory,
                timestamp=timestamp,
                mtime=max_mtime,
                transcript_file=ts.file_path if ts is not None else None,
            )

        return sessions

    def _append_transcript_part(
        self,
        lines: list[str],
        role: str,
        part: object,
    ) -> bool:
        """Append text from a transcript content part."""
        role_prefix = "» " if role == "user" else "  "

        if isinstance(part, str):
            text = part.strip()
            if text:
                lines.append(f"{role_prefix}{text}")
                return True
            return False

        if not isinstance(part, dict):
            return False

        part_type = part.get("type", "")
        if part_type == "text":
            text = str(part.get("text", "")).strip()
            if text:
                lines.append(f"{role_prefix}{text}")
                return True
            return False

        if part_type == "tool_use" and role != "user":
            name = part.get("name", "")
            if isinstance(name, str) and name:
                lines.append(f"  [tool {name}]")
                return True

        if part_type == "tool_result":
            content = part.get("content", "")
            if isinstance(content, str) and content.strip():
                lines.append(f"{role_prefix}{content.strip()}")
                return True

        return False

    def _load_transcript_messages(
        self,
        transcript_file: Path,
        on_error: ErrorCallback = None,
    ) -> tuple[list[str], int, str]:
        """Load session content from Cursor transcript JSONL."""
        lines: list[str] = []
        turn_count = 0
        first_user_text = ""

        try:
            with open(transcript_file, "rb") as f:
                for line in f:
                    if not line.strip():
                        continue

                    try:
                        record = orjson.loads(line)
                    except orjson.JSONDecodeError:
                        continue

                    role = record.get("role", "")
                    if role not in ("user", "assistant"):
                        continue

                    message = record.get("message", {})
                    content = message.get("content", "") if isinstance(message, dict) else ""

                    had_content = False
                    if isinstance(content, str):
                        text = content.strip()
                        if text:
                            prefix = "» " if role == "user" else "  "
                            lines.append(f"{prefix}{text}")
                            had_content = True
                            if role == "user" and not first_user_text:
                                first_user_text = text
                    elif isinstance(content, list):
                        for part in content:
                            if self._append_transcript_part(lines, role, part):
                                had_content = True
                                if (
                                    role == "user"
                                    and not first_user_text
                                    and isinstance(part, dict)
                                    and part.get("type") == "text"
                                ):
                                    first_user_text = str(part.get("text", "")).strip()

                    if had_content:
                        turn_count += 1

            return lines, turn_count, first_user_text
        except OSError as e:
            self._emit_error(
                transcript_file,
                "OSError",
                str(e),
                on_error=on_error,
            )
            return [], 0, ""

    def _load_messages_from_global(
        self,
        conn: sqlite3.Connection,
        composer_id: str,
        on_error: ErrorCallback = None,
    ) -> tuple[list[str], int]:
        """Load ordered bubble content for a Cursor composer from global DB."""
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

                text = bubble.get("text", "")
                if isinstance(text, str) and text.strip():
                    prefix = "» " if bubble_type == 1 else "  "
                    lines.append(f"{prefix}{text.strip()}")
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

    def _build_session(
        self,
        metadata: _SessionMetadata,
        global_conn: sqlite3.Connection | None,
        on_error: ErrorCallback = None,
    ) -> Session:
        """Build a Session from merged metadata and available content sources."""
        title = metadata.title
        lines: list[str] = []
        turn_count = 0
        first_user_text = ""

        if metadata.transcript_file is not None:
            lines, turn_count, first_user_text = self._load_transcript_messages(
                metadata.transcript_file,
                on_error=on_error,
            )

        if not lines and global_conn is not None:
            lines, turn_count = self._load_messages_from_global(
                global_conn,
                metadata.composer_id,
                on_error=on_error,
            )

        if title == "Untitled session" and first_user_text:
            title = first_user_text

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
                known_entry = known.get(session_id)
                if known_entry is None or data.mtime > known_entry[0] + 0.001:
                    session = self._build_session(data, conn, on_error=on_error)
                    session.mtime = data.mtime
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
        """Get raw statistics from Cursor data sources."""
        if not self.is_available():
            return RawAdapterStats(
                agent=self.name,
                data_dir=f"{self._user_dir} + {self._projects_dir}",
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

        for transcript_file in self._iter_transcript_files():
            try:
                file_count += 1
                total_bytes += transcript_file.stat().st_size
            except OSError:
                pass

        return RawAdapterStats(
            agent=self.name,
            data_dir=f"{self._user_dir} + {self._projects_dir}",
            available=True,
            file_count=file_count,
            total_bytes=total_bytes,
        )
