"""Search engine for aggregating and searching sessions."""

import hashlib
import json
import re
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Callable
from datetime import datetime
from pathlib import Path

from rapidfuzz import fuzz, process

from .adapters import (
    ClaudeAdapter,
    CodexAdapter,
    OpenCodeAdapter,
    Session,
    VibeAdapter,
)
from .config import CACHE_DIR, CLAUDE_DIR, CODEX_DIR, OPENCODE_DIR, VIBE_DIR


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
    """Generate a cache key based on directory mtimes."""
    mtimes = [
        _get_dir_mtime(CLAUDE_DIR),
        _get_dir_mtime(CODEX_DIR),
        _get_dir_mtime(OPENCODE_DIR),
        _get_dir_mtime(VIBE_DIR),
    ]
    key = ":".join(str(m) for m in mtimes)
    return hashlib.md5(key.encode()).hexdigest()[:16]


class SessionSearch:
    """Aggregates sessions from all adapters and provides search."""

    def __init__(self) -> None:
        self.adapters = [
            ClaudeAdapter(),
            CodexAdapter(),
            OpenCodeAdapter(),
            VibeAdapter(),
        ]
        self._sessions: list[Session] | None = None
        self._cache_file = CACHE_DIR / "sessions.json"
        self._cache_key: str | None = None  # Cache the key to avoid repeated mtime checks

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
            with open(self._cache_file, "r") as f:
                data = json.load(f)

            # Check if cache is valid
            if data.get("key") != self._get_cache_key():
                return None

            # Reconstruct sessions
            sessions = []
            for s in data.get("sessions", []):
                sessions.append(
                    Session(
                        id=s["id"],
                        agent=s["agent"],
                        title=s["title"],
                        directory=s["directory"],
                        timestamp=datetime.fromisoformat(s["timestamp"]),
                        preview=s["preview"],
                        content=s["content"],
                    )
                )
            return sessions
        except Exception:
            return None

    def _save_to_cache(self, sessions: list[Session]) -> None:
        """Save sessions to cache."""
        try:
            CACHE_DIR.mkdir(parents=True, exist_ok=True)
            data = {
                "key": self._get_cache_key(),
                "sessions": [
                    {
                        "id": s.id,
                        "agent": s.agent,
                        "title": s.title,
                        "directory": s.directory,
                        "timestamp": s.timestamp.isoformat(),
                        "preview": s.preview,
                        "content": s.content,
                    }
                    for s in sessions
                ],
            }
            with open(self._cache_file, "w") as f:
                json.dump(data, f)
        except Exception:
            pass  # Cache write failure is not critical

    def get_all_sessions(self, force_refresh: bool = False) -> list[Session]:
        """Get all sessions from all adapters."""
        if self._sessions is not None and not force_refresh:
            return self._sessions

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

        with ThreadPoolExecutor(max_workers=4) as executor:
            results = executor.map(load_adapter, self.adapters)
            for result in results:
                sessions.extend(result)

        # Sort by timestamp, newest first
        sessions.sort(key=lambda s: s.timestamp, reverse=True)
        self._sessions = sessions

        # Save to cache in background
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

        # Load from adapters, reporting progress as each completes
        all_sessions: list[Session] = []

        def load_adapter(adapter):
            if adapter.is_available():
                return adapter.find_sessions()
            return []

        with ThreadPoolExecutor(max_workers=4) as executor:
            futures = {executor.submit(load_adapter, a): a for a in self.adapters}
            for future in as_completed(futures):
                result = future.result()
                if result:
                    all_sessions.extend(result)
                    # Sort and report progress
                    all_sessions.sort(key=lambda s: s.timestamp, reverse=True)
                    on_progress(all_sessions.copy())

        self._sessions = all_sessions
        self._save_to_cache(all_sessions)
        return all_sessions

    def _compute_hybrid_score(
        self, query: str, searchable: str, fuzzy_score: float
    ) -> float:
        """Compute hybrid score combining fuzzy matching with exact match bonuses."""
        query_lower = query.lower()
        searchable_lower = searchable.lower()

        # Exact substring bonus - query appears verbatim
        exact_bonus = 25 if query_lower in searchable_lower else 0

        # Token match bonus - all query words present
        tokens = query_lower.split()
        token_bonus = 15 if all(t in searchable_lower for t in tokens) else 0

        # Phrase/consecutive words bonus - words appear together in order
        phrase_bonus = 0
        if len(tokens) > 1:
            # Check if tokens appear consecutively (with some flexibility for punctuation)
            # Build pattern: word1.*?word2 with small gap allowed
            pattern = r"\b" + r"\W{0,3}".join(re.escape(t) for t in tokens) + r"\b"
            if re.search(pattern, searchable_lower):
                phrase_bonus = 30

        return fuzzy_score + exact_bonus + token_bonus + phrase_bonus

    def search(
        self,
        query: str,
        agent_filter: str | None = None,
        directory_filter: str | None = None,
        limit: int = 100,
    ) -> list[Session]:
        """Search sessions with hybrid fuzzy + exact matching."""
        sessions = self.get_all_sessions()

        # Apply agent filter
        if agent_filter:
            sessions = [s for s in sessions if s.agent == agent_filter]

        # Apply directory filter
        if directory_filter:
            sessions = [
                s for s in sessions if directory_filter.lower() in s.directory.lower()
            ]

        if not query:
            return sessions[:limit]

        # Build searchable strings: combine title, directory, and content
        choices = []
        for session in sessions:
            # Weight title and directory higher by including them multiple times
            searchable = f"{session.title} {session.title} {session.directory} {session.content[:2000]}"
            choices.append((session, searchable))

        # Use rapidfuzz for fuzzy matching
        results = process.extract(
            query,
            {i: c[1] for i, c in enumerate(choices)},
            scorer=fuzz.WRatio,
            limit=min(limit * 2, len(choices)),  # Get more results for re-ranking
        )

        # Re-rank with hybrid scoring
        scored_results = []
        for match in results:
            idx = match[2]  # Index in choices
            fuzzy_score = match[1]
            if fuzzy_score > 40:  # Lower threshold since we'll re-rank
                searchable = choices[idx][1]
                hybrid_score = self._compute_hybrid_score(query, searchable, fuzzy_score)
                scored_results.append((choices[idx][0], hybrid_score))

        # Sort by hybrid score descending
        scored_results.sort(key=lambda x: x[1], reverse=True)

        # Return sessions above threshold
        matched_sessions = []
        for session, score in scored_results[:limit]:
            if score > 50:
                matched_sessions.append(session)

        return matched_sessions

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
