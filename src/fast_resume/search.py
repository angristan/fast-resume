"""Search engine for aggregating and searching sessions."""

import hashlib
import orjson
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Callable
from datetime import datetime
from pathlib import Path

from .adapters import (
    ClaudeAdapter,
    CodexAdapter,
    CopilotAdapter,
    CrushAdapter,
    OpenCodeAdapter,
    Session,
    VibeAdapter,
)
from .config import (
    CACHE_DIR,
    CACHE_VERSION,
    CLAUDE_DIR,
    CODEX_DIR,
    COPILOT_DIR,
    CRUSH_PROJECTS_FILE,
    OPENCODE_DIR,
    VIBE_DIR,
)
from .index import TantivyIndex


def _get_dir_mtime(path: Path) -> float:
    """Get the latest mtime of a directory tree (shallow check)."""
    if not path.exists():
        return 0
    try:
        # Just check the directory itself and immediate children
        mtimes = [path.stat().st_mtime]
        for child in path.iterdir():
            try:
                mtimes.append(child.stat().st_mtime)
            except OSError:
                pass
        return max(mtimes)
    except OSError:
        return 0


def _get_cache_key() -> str:
    """Generate a cache key based on directory mtimes and cache version."""
    mtimes = [
        _get_dir_mtime(CLAUDE_DIR),
        _get_dir_mtime(CODEX_DIR),
        _get_dir_mtime(COPILOT_DIR),
        _get_dir_mtime(OPENCODE_DIR),
        _get_dir_mtime(VIBE_DIR),
        _get_dir_mtime(CRUSH_PROJECTS_FILE.parent),
    ]
    key = f"v{CACHE_VERSION}:" + ":".join(str(m) for m in mtimes)
    return hashlib.md5(key.encode()).hexdigest()[:16]


class SessionSearch:
    """Aggregates sessions from all adapters and provides search."""

    def __init__(self) -> None:
        self.adapters = [
            ClaudeAdapter(),
            CodexAdapter(),
            CopilotAdapter(),
            CrushAdapter(),
            OpenCodeAdapter(),
            VibeAdapter(),
        ]
        self._sessions: list[Session] | None = None
        self._sessions_by_id: dict[str, Session] = {}
        self._streaming_in_progress: bool = False
        self._cache_file = CACHE_DIR / "sessions.json"
        self._cache_key: str | None = (
            None  # Cache the key to avoid repeated mtime checks
        )
        self._index = TantivyIndex()

    def _get_cache_key(self) -> str:
        """Get cache key, computing it only once per instance."""
        if self._cache_key is None:
            self._cache_key = _get_cache_key()
        return self._cache_key

    def _load_from_cache(self) -> list[Session] | None:
        """Try to load sessions from cache."""
        if not self._cache_file.exists():
            return None

        try:
            with open(self._cache_file, "rb") as f:
                data = orjson.loads(f.read())

            # Check if cache is valid
            cache_key = self._get_cache_key()
            if data.get("key") != cache_key:
                return None

            # Also check if Tantivy index needs rebuild
            if self._index.needs_rebuild(cache_key):
                return None

            # Reconstruct sessions
            sessions = []
            for s in data.get("sessions", []):
                session = Session(
                    id=s["id"],
                    agent=s["agent"],
                    title=s["title"],
                    directory=s["directory"],
                    timestamp=datetime.fromisoformat(s["timestamp"]),
                    preview=s["preview"],
                    content=s["content"],
                    message_count=s.get("message_count", 0),
                )
                sessions.append(session)
                self._sessions_by_id[session.id] = session
            return sessions
        except Exception:
            return None

    def _save_to_cache(self, sessions: list[Session]) -> None:
        """Save sessions to cache and build Tantivy index."""
        cache_key = self._get_cache_key()
        try:
            CACHE_DIR.mkdir(parents=True, exist_ok=True)
            data = {
                "key": cache_key,
                "sessions": [
                    {
                        "id": s.id,
                        "agent": s.agent,
                        "title": s.title,
                        "directory": s.directory,
                        "timestamp": s.timestamp.isoformat(),
                        "preview": s.preview,
                        "content": s.content,
                        "message_count": s.message_count,
                    }
                    for s in sessions
                ],
            }
            with open(self._cache_file, "wb") as f:
                f.write(orjson.dumps(data))

            # Build Tantivy index
            self._index.build_index(sessions, cache_key)
        except Exception:
            pass  # Cache write failure is not critical

    def get_all_sessions(self, force_refresh: bool = False) -> list[Session]:
        """Get all sessions from all adapters."""
        if self._sessions is not None and not force_refresh:
            return self._sessions

        # If streaming is in progress, return current partial results
        # to avoid starting a second concurrent load
        if self._streaming_in_progress:
            return self._sessions if self._sessions is not None else []

        # Try cache first
        if not force_refresh:
            cached = self._load_from_cache()
            if cached is not None:
                self._sessions = cached
                return cached

        # Load from adapters in parallel
        sessions: list[Session] = []

        def load_adapter(adapter):
            if adapter.is_available():
                return adapter.find_sessions()
            return []

        with ThreadPoolExecutor(max_workers=len(self.adapters)) as executor:
            results = executor.map(load_adapter, self.adapters)
            for result in results:
                sessions.extend(result)

        # Sort by timestamp, newest first
        sessions.sort(key=lambda s: s.timestamp, reverse=True)
        self._sessions = sessions

        # Build sessions_by_id lookup
        self._sessions_by_id = {s.id: s for s in sessions}

        # Save to cache and build Tantivy index
        self._save_to_cache(sessions)

        return sessions

    def get_sessions_streaming(
        self, on_progress: Callable[[list[Session]], None]
    ) -> list[Session]:
        """Load sessions with progress callback for each adapter that completes."""
        # Check cache first
        cached = self._load_from_cache()
        if cached is not None:
            self._sessions = cached
            on_progress(cached)
            return cached

        # Mark streaming as in progress and initialize _sessions
        # so concurrent searches can use partial results
        self._streaming_in_progress = True
        self._sessions = []

        def load_adapter(adapter):
            if adapter.is_available():
                return adapter.find_sessions()
            return []

        try:
            with ThreadPoolExecutor(max_workers=len(self.adapters)) as executor:
                futures = {executor.submit(load_adapter, a): a for a in self.adapters}
                for future in as_completed(futures):
                    result = future.result()
                    if result:
                        self._sessions.extend(result)
                        # Update sessions_by_id lookup
                        for s in result:
                            self._sessions_by_id[s.id] = s
                        # Sort and report progress
                        self._sessions.sort(key=lambda s: s.timestamp, reverse=True)
                        on_progress(self._sessions.copy())
        finally:
            self._streaming_in_progress = False

        self._save_to_cache(self._sessions)
        return self._sessions

    def search(
        self,
        query: str,
        agent_filter: str | None = None,
        directory_filter: str | None = None,
        limit: int = 100,
    ) -> list[Session]:
        """Search sessions using Tantivy full-text search with fuzzy matching."""
        # Ensure sessions are loaded (populates _sessions_by_id)
        sessions = self.get_all_sessions()

        # If no query, return filtered sessions by timestamp
        if not query:
            if agent_filter:
                sessions = [s for s in sessions if s.agent == agent_filter]
            if directory_filter:
                sessions = [
                    s
                    for s in sessions
                    if directory_filter.lower() in s.directory.lower()
                ]
            return sessions[:limit]

        # Use Tantivy for fuzzy search
        results = self._index.search(query, agent_filter=agent_filter, limit=limit)

        # Lookup full session objects from results
        matched_sessions = []
        for session_id, _score in results:
            session = self._sessions_by_id.get(session_id)
            if session:
                # Apply directory filter (substring match not supported in FTS)
                if directory_filter:
                    if directory_filter.lower() not in session.directory.lower():
                        continue
                matched_sessions.append(session)

        return matched_sessions[:limit]

    def get_adapter_for_session(self, session: Session):
        """Get the adapter for a session."""
        for adapter in self.adapters:
            if adapter.name == session.agent:
                return adapter
        return None

    def get_resume_command(self, session: Session) -> list[str]:
        """Get the resume command for a session."""
        adapter = self.get_adapter_for_session(session)
        if adapter:
            return adapter.get_resume_command(session)
        return []
