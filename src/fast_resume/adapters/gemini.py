"""Gemini CLI session adapter.

Gemini stores chats per project under `~/.gemini/tmp/<project-slug>/chats/`.
Two formats coexist:

    session-<ts>-<short>.json   Single JSON object with `sessionId` + `messages[]`.
    session-<ts>-<short>.jsonl  Streaming JSONL: first line is the session
                                metadata, subsequent lines are message objects
                                interleaved with `{"$set": {...}}` updates.

Working directories are recovered from `~/.gemini/projects.json`, which maps
directory paths to project slugs.
"""

import orjson
from datetime import datetime
from pathlib import Path

from ..config import AGENTS, GEMINI_DIR
from ..logging_config import log_parse_error
from .base import BaseSessionAdapter, ErrorCallback, ParseError, Session, truncate_title


class GeminiAdapter(BaseSessionAdapter):
    """Adapter for Gemini CLI sessions."""

    name = "gemini"
    color = AGENTS["gemini"]["color"]
    badge = AGENTS["gemini"]["badge"]
    supports_yolo = True

    def __init__(self, sessions_dir: Path | None = None) -> None:
        # `_sessions_dir` is the Gemini config root (`~/.gemini`); chats live
        # under `tmp/<slug>/chats/`. Treating the config root as the adapter
        # root keeps `is_available()` honest for fresh installs.
        self._sessions_dir = sessions_dir if sessions_dir is not None else GEMINI_DIR

    @property
    def _chats_root(self) -> Path:
        return self._sessions_dir / "tmp"

    @property
    def _projects_file(self) -> Path:
        return self._sessions_dir / "projects.json"

    def is_available(self) -> bool:
        return self._chats_root.exists()

    def _load_project_dirs(self) -> dict[str, str]:
        """Return `{slug: directory}` derived from `projects.json`."""
        if not self._projects_file.exists():
            return {}
        try:
            with open(self._projects_file, "rb") as f:
                data = orjson.loads(f.read())
        except OSError as e:
            log_parse_error(self.name, str(self._projects_file), "OSError", str(e))
            return {}
        except orjson.JSONDecodeError as e:
            log_parse_error(
                self.name, str(self._projects_file), "JSONDecodeError", str(e)
            )
            return {}
        projects = data.get("projects") or {}
        # projects.json stores directory -> slug; invert for our lookup.
        result: dict[str, str] = {}
        if isinstance(projects, dict):
            for directory, slug in projects.items():
                if isinstance(directory, str) and isinstance(slug, str):
                    result[slug] = directory
        return result

    def find_sessions(self) -> list[Session]:
        if not self.is_available():
            return []

        slug_to_dir = self._load_project_dirs()
        sessions: list[Session] = []
        # Route through `_scan_session_files` so the same session id is never
        # yielded twice when both the legacy `.json` and newer `.jsonl` exist
        # for it. `_scan_session_files` picks the file with the newer mtime.
        for path, _mtime in self._scan_session_files().values():
            session = self._parse_session_file(path, slug_to_dir=slug_to_dir)
            if session:
                sessions.append(session)
        return sessions

    def _iter_session_files(self):
        """Yield every `session-*.json` and `session-*.jsonl` chat file."""
        for chats_dir in self._chats_root.glob("*/chats"):
            if not chats_dir.is_dir():
                continue
            yield from chats_dir.glob("session-*.json")
            yield from chats_dir.glob("session-*.jsonl")

    def _slug_for(self, session_file: Path) -> str:
        # session_file is .../tmp/<slug>/chats/<file>
        try:
            return session_file.parent.parent.name
        except IndexError:
            return ""

    def _parse_session_file(
        self,
        session_file: Path,
        on_error: ErrorCallback = None,
        slug_to_dir: dict[str, str] | None = None,
    ) -> Session | None:
        try:
            if session_file.suffix == ".jsonl":
                meta, messages = self._parse_jsonl(session_file)
            else:
                meta, messages = self._parse_json(session_file)
        except orjson.JSONDecodeError as e:
            self._report(on_error, str(session_file), "JSONDecodeError", str(e))
            return None
        except OSError as e:
            self._report(on_error, str(session_file), "OSError", str(e))
            return None

        try:
            session_id = meta.get("sessionId") or self._id_from_filename(session_file)
            if not session_id:
                return None

            slug_to_dir = (
                slug_to_dir if slug_to_dir is not None else self._load_project_dirs()
            )
            directory = slug_to_dir.get(self._slug_for(session_file), "")

            timestamp = self._parse_timestamp(
                meta.get("lastUpdated") or meta.get("startTime"), session_file
            )

            display_messages: list[str] = []
            first_user_prompt = ""
            turn_count = 0
            for role, text in messages:
                if not text:
                    continue
                if role == "user":
                    display_messages.append(f"» {text}")
                    turn_count += 1
                    if not first_user_prompt:
                        first_user_prompt = text
                else:  # assistant ("gemini")
                    display_messages.append(f"  {text}")
                    turn_count += 1

            if not first_user_prompt:
                # Skip sessions that never had a real user message (e.g. only
                # info/error rows). They aren't useful to resume.
                return None

            title = truncate_title(first_user_prompt, max_length=80, word_break=False)

            return Session(
                id=session_id,
                agent=self.name,
                title=title,
                directory=directory,
                timestamp=timestamp,
                content="\n\n".join(display_messages),
                message_count=turn_count,
            )
        except (KeyError, TypeError, AttributeError) as e:
            self._report(on_error, str(session_file), type(e).__name__, str(e))
            return None

    def _parse_json(self, session_file: Path) -> tuple[dict, list[tuple[str, str]]]:
        """Parse the legacy single-JSON session format."""
        with open(session_file, "rb") as f:
            payload = orjson.loads(f.read())
        meta = {
            "sessionId": payload.get("sessionId"),
            "startTime": payload.get("startTime"),
            "lastUpdated": payload.get("lastUpdated"),
        }
        messages = [self._classify(msg) for msg in payload.get("messages", []) or []]
        return meta, [m for m in messages if m is not None]

    def _parse_jsonl(self, session_file: Path) -> tuple[dict, list[tuple[str, str]]]:
        """Parse the streaming JSONL session format."""
        meta: dict = {}
        messages: list[tuple[str, str]] = []
        seen_ids: set[str] = set()

        with open(session_file, "rb") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    entry = orjson.loads(line)
                except orjson.JSONDecodeError:
                    continue

                if not isinstance(entry, dict):
                    continue

                if "$set" in entry:
                    # Gemini emits incremental metadata patches; only merge
                    # known timestamp keys so a malicious file can't swap
                    # `sessionId` (and silently overwrite another session's
                    # entry in the index).
                    patch = entry.get("$set") or {}
                    if isinstance(patch, dict):
                        for key in ("startTime", "lastUpdated"):
                            if key in patch:
                                meta[key] = patch[key]
                    continue

                if "sessionId" in entry and "messages" not in entry:
                    # First line of a JSONL file is the session header.
                    meta.update(
                        {
                            k: entry.get(k)
                            for k in ("sessionId", "startTime", "lastUpdated")
                            if k in entry
                        }
                    )
                    continue

                # Deduplicate Gemini's repeated message rows (it re-emits the
                # whole message every time fields like `toolCalls` update).
                msg_id = entry.get("id")
                if msg_id and msg_id in seen_ids:
                    continue
                if msg_id:
                    seen_ids.add(msg_id)

                classified = self._classify(entry)
                if classified is not None:
                    messages.append(classified)

        return meta, messages

    @staticmethod
    def _classify(msg: dict) -> tuple[str, str] | None:
        """Map a Gemini message dict to (role, text) or None if not indexable."""
        if not isinstance(msg, dict):
            return None
        msg_type = msg.get("type", "")
        if msg_type not in ("user", "gemini"):
            return None  # Skip info/error/system entries.
        content = msg.get("content")
        if isinstance(content, str):
            text = content
        elif isinstance(content, list):
            parts = []
            for part in content:
                if isinstance(part, dict):
                    t = part.get("text", "")
                    if isinstance(t, str) and t:
                        parts.append(t)
            text = "\n".join(parts)
        else:
            text = ""
        text = text.strip()
        if not text:
            return None
        role = "user" if msg_type == "user" else "assistant"
        return role, text

    @staticmethod
    def _id_from_filename(session_file: Path) -> str:
        # Filenames look like `session-2026-05-09T06-54-7c6dccb5.jsonl`; the
        # final hyphen-separated chunk is the short id, but lacks the rest of
        # the UUID. Fall back to the full stem so it stays unique.
        return session_file.stem

    @staticmethod
    def _parse_timestamp(value: str | None, fallback_file: Path) -> datetime:
        if value:
            try:
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
        current: dict[str, tuple[Path, float]] = {}
        for session_file in self._iter_session_files():
            try:
                mtime = session_file.stat().st_mtime
            except OSError:
                continue
            session_id = self._session_id_quick(session_file)
            if not session_id:
                continue
            # If both .json and .jsonl exist for the same id, keep whichever
            # has the latest mtime so incremental updates always re-parse the
            # active file.
            existing = current.get(session_id)
            if existing is None or mtime > existing[1]:
                current[session_id] = (session_file, mtime)
        return current

    # Cap how far into a JSONL we'll scan looking for the session header.
    # Real-world Gemini files put the header on line 1; this guard prevents
    # an adversarial / corrupted file from forcing a full-file scan.
    _MAX_HEADER_SCAN_LINES = 32

    def _session_id_quick(self, session_file: Path) -> str:
        """Extract the session's `sessionId` from disk.

        Must match what `_parse_session_file` will set on `Session.id`, or
        `find_sessions_incremental` can't join scan results against the
        stored index (which keys by parsed `sessionId`, not filename stem).
        For JSONL files we scan a few leading lines past any `$set` patches
        until we find the session header; for legacy single-JSON files we
        decode the whole payload.
        """
        try:
            with open(session_file, "rb") as f:
                if session_file.suffix == ".jsonl":
                    for _ in range(self._MAX_HEADER_SCAN_LINES):
                        raw = f.readline()
                        if not raw:
                            break
                        line = raw.strip()
                        if not line:
                            continue
                        try:
                            entry = orjson.loads(line)
                        except orjson.JSONDecodeError:
                            continue
                        if not isinstance(entry, dict):
                            continue
                        # Skip metadata-patch rows: Gemini emits these
                        # interleaved and they don't carry sessionId.
                        if "$set" in entry:
                            continue
                        sid = entry.get("sessionId")
                        if isinstance(sid, str) and sid:
                            return sid
                    return session_file.stem
                else:
                    payload = orjson.loads(f.read())
                    if isinstance(payload, dict):
                        sid = payload.get("sessionId")
                        if isinstance(sid, str) and sid:
                            return sid
                    return session_file.stem
        except OSError:
            return session_file.stem
        except orjson.JSONDecodeError:
            return session_file.stem

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get command to resume a Gemini CLI session."""
        cmd = ["gemini"]
        if yolo:
            cmd.append("--yolo")
        cmd.extend(["--resume", session.id])
        return cmd

    def get_raw_stats(self):
        # Override so stats reflect chat files under tmp/<slug>/chats,
        # not the empty top-level Gemini directory.
        from .base import RawAdapterStats

        if not self.is_available():
            return RawAdapterStats(
                agent=self.name,
                data_dir=str(self._chats_root),
                available=False,
                file_count=0,
                total_bytes=0,
            )

        files = self._scan_session_files()
        total_bytes = 0
        for path, _ in files.values():
            try:
                total_bytes += path.stat().st_size
            except OSError:
                pass
        return RawAdapterStats(
            agent=self.name,
            data_dir=str(self._chats_root),
            available=True,
            file_count=len(files),
            total_bytes=total_bytes,
        )
