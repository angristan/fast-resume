"""VS Code Copilot (copilot-vscode) session adapter."""

import orjson
import sys
from datetime import datetime
from pathlib import Path
from urllib.parse import unquote, urlparse

from ..config import AGENTS, MAX_PREVIEW_LENGTH
from ..logging_config import log_parse_error
from .base import ErrorCallback, ParseError, RawAdapterStats, Session, truncate_title

# VS Code storage paths vary by platform
if sys.platform == "darwin":
    VSCODE_STORAGE = Path.home() / "Library" / "Application Support" / "Code"
elif sys.platform == "win32":
    VSCODE_STORAGE = Path.home() / "AppData" / "Roaming" / "Code"
else:  # Linux
    VSCODE_STORAGE = Path.home() / ".config" / "Code"

CHAT_SESSIONS_DIR = (
    VSCODE_STORAGE / "User" / "globalStorage" / "emptyWindowChatSessions"
)
WORKSPACE_STORAGE_DIR = VSCODE_STORAGE / "User" / "workspaceStorage"


class CopilotVSCodeAdapter:
    """Adapter for VS Code Copilot Chat sessions."""

    name = "copilot-vscode"
    color = AGENTS["copilot-vscode"]["color"]
    badge = AGENTS["copilot-vscode"]["badge"]

    def __init__(
        self,
        chat_sessions_dir: Path | None = None,
        workspace_storage_dir: Path | None = None,
    ) -> None:
        self._chat_sessions_dir = (
            chat_sessions_dir if chat_sessions_dir is not None else CHAT_SESSIONS_DIR
        )
        self._workspace_storage_dir = (
            workspace_storage_dir
            if workspace_storage_dir is not None
            else WORKSPACE_STORAGE_DIR
        )

    def is_available(self) -> bool:
        """Check if VS Code Copilot Chat data exists."""
        # Check empty window sessions
        if self._chat_sessions_dir.exists() and any(
            self._chat_sessions_dir.glob("*.json")
        ):
            return True
        # Check workspace sessions
        if self._workspace_storage_dir.exists():
            for ws_dir in self._workspace_storage_dir.iterdir():
                chat_dir = ws_dir / "chatSessions"
                if chat_dir.exists() and any(chat_dir.glob("*.json")):
                    return True
        return False

    def _get_session_id_from_file(self, session_file: Path) -> str | None:
        """Extract session ID from session file, returns None on error."""
        try:
            with open(session_file, "rb") as f:
                data = orjson.loads(f.read())
            return data.get("sessionId", session_file.stem)
        except Exception:
            return None

    def _get_workspace_directory(self, workspace_dir: Path) -> str:
        """Get the workspace folder path from workspace.json."""
        workspace_json = workspace_dir / "workspace.json"
        if workspace_json.exists():
            try:
                with open(workspace_json, "rb") as f:
                    data = orjson.loads(f.read())
                folder = data.get("folder", "")
                if folder.startswith("file://"):
                    # Parse and decode the file URI
                    parsed = urlparse(folder)
                    return unquote(parsed.path)
            except Exception:
                pass
        return ""

    def _get_all_session_files(self) -> list[tuple[Path, str]]:
        """Get all session files with their associated workspace directories.

        Returns list of (session_file_path, workspace_directory).
        """
        session_files: list[tuple[Path, str]] = []

        # Empty window sessions (no workspace directory)
        if self._chat_sessions_dir.exists():
            for session_file in self._chat_sessions_dir.glob("*.json"):
                session_files.append((session_file, ""))

        # Workspace-specific sessions
        if self._workspace_storage_dir.exists():
            for ws_dir in self._workspace_storage_dir.iterdir():
                if not ws_dir.is_dir():
                    continue
                chat_dir = ws_dir / "chatSessions"
                if chat_dir.exists():
                    ws_directory = self._get_workspace_directory(ws_dir)
                    for session_file in chat_dir.glob("*.json"):
                        session_files.append((session_file, ws_directory))

        return session_files

    def find_sessions(self) -> list[Session]:
        """Find all VS Code Copilot Chat sessions."""
        if not self.is_available():
            return []

        sessions = []
        for session_file, ws_directory in self._get_all_session_files():
            session = self._parse_session(session_file, ws_directory)
            if session:
                sessions.append(session)

        return sessions

    def _parse_session(
        self,
        session_file: Path,
        workspace_directory: str = "",
        on_error: ErrorCallback = None,
    ) -> Session | None:
        """Parse a VS Code Copilot Chat session file."""
        try:
            with open(session_file, "rb") as f:
                data = orjson.loads(f.read())

            session_id = data.get("sessionId", session_file.stem)
            title = data.get("customTitle", "")
            requests = data.get("requests", [])

            if not requests:
                return None

            # Extract messages
            messages: list[str] = []
            directory = workspace_directory  # Use workspace directory as default
            turn_count = 0

            for req in requests:
                # User message
                msg = req.get("message", {})
                user_text = msg.get("text", "")
                if user_text:
                    messages.append(f"» {user_text}")
                    turn_count += 1

                # Try to extract directory from content references if not already set
                if not directory:
                    for ref in req.get("contentReferences", []):
                        ref_data = ref.get("reference", {})
                        uri = ref_data.get("uri", {})
                        fs_path = uri.get("fsPath", "")
                        if fs_path:
                            # Get parent directory
                            directory = str(Path(fs_path).parent)
                            break

                # Assistant response
                response = req.get("response", [])
                has_response = False
                for resp_part in response:
                    if isinstance(resp_part, dict):
                        value = resp_part.get("value", "")
                        if value:
                            messages.append(f"  {value}")
                            has_response = True
                if has_response:
                    turn_count += 1

            if not messages:
                return None

            # Use first user message as title if no custom title
            if not title and messages:
                first_msg = messages[0].lstrip("» ").strip()
                title = truncate_title(first_msg)

            # Get timestamp from file or data
            creation_date = data.get("creationDate")
            last_message_date = data.get("lastMessageDate")
            if last_message_date:
                timestamp = datetime.fromtimestamp(last_message_date / 1000)
            elif creation_date:
                timestamp = datetime.fromtimestamp(creation_date / 1000)
            else:
                timestamp = datetime.fromtimestamp(session_file.stat().st_mtime)

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
                mtime=session_file.stat().st_mtime,
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

    def find_sessions_incremental(
        self,
        known: dict[str, tuple[float, str]],
        on_error: ErrorCallback = None,
    ) -> tuple[list[Session], list[str]]:
        """Find sessions incrementally, comparing against known sessions."""
        if not self.is_available():
            deleted_ids = [
                sid for sid, (_, agent) in known.items() if agent == self.name
            ]
            return [], deleted_ids

        # Scan all session files and build current state
        current_files: dict[str, tuple[Path, float, str]] = {}

        for session_file, ws_directory in self._get_all_session_files():
            session_id = self._get_session_id_from_file(session_file)
            if session_id is None:
                continue
            mtime = session_file.stat().st_mtime
            current_files[session_id] = (session_file, mtime, ws_directory)

        # Find new and modified sessions
        new_or_modified = []
        for session_id, (path, mtime, ws_directory) in current_files.items():
            known_entry = known.get(session_id)
            if known_entry is None or mtime > known_entry[0] + 0.001:
                session = self._parse_session(path, ws_directory, on_error=on_error)
                if session:
                    new_or_modified.append(session)

        # Find deleted sessions
        current_ids = set(current_files.keys())
        deleted_ids = [
            sid
            for sid, (_, agent) in known.items()
            if agent == self.name and sid not in current_ids
        ]

        return new_or_modified, deleted_ids

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to open VS Code.

        Note: VS Code Copilot Chat doesn't support resuming specific sessions
        via command line. We open VS Code in the session's directory instead.
        """
        if session.directory:
            return ["code", session.directory]
        return ["code"]

    def get_raw_stats(self) -> RawAdapterStats:
        """Get raw statistics from VS Code Copilot data folders."""
        if not self.is_available():
            return RawAdapterStats(
                agent=self.name,
                data_dir=str(self._chat_sessions_dir),
                available=False,
                file_count=0,
                total_bytes=0,
            )

        session_files = self._get_all_session_files()
        total_bytes = 0
        for path, _ in session_files:
            try:
                total_bytes += path.stat().st_size
            except OSError:
                pass

        return RawAdapterStats(
            agent=self.name,
            data_dir=str(self._chat_sessions_dir),
            available=True,
            file_count=len(session_files),
            total_bytes=total_bytes,
        )
