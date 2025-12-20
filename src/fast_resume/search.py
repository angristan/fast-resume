"""Search engine for aggregating and searching sessions."""

from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Callable

from .adapters import (
    ClaudeAdapter,
    CodexAdapter,
    CopilotAdapter,
    CrushAdapter,
    OpenCodeAdapter,
    Session,
    VibeAdapter,
)
from .index import TantivyIndex


class SessionSearch:
    """Aggregates sessions from all adapters and provides search.

    Uses Tantivy as the single source of truth for session data.
    """

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
        self._index = TantivyIndex()

    def _load_from_cache(self) -> list[Session] | None:
        """Try to load sessions from index if no changes detected (fast path for TUI)."""
        # Get known sessions from Tantivy
        known = self._index.get_known_sessions()
        if not known:
            return None

        # Check if any adapter has changes
        for adapter in self.adapters:
            new_or_modified, deleted_ids = adapter.find_sessions_incremental(known)
            if new_or_modified or deleted_ids:
                # Changes detected - need full update
                return None

        # No changes - load from index
        sessions = self._index.get_all_sessions()
        if not sessions:
            return None

        # Populate sessions_by_id
        for session in sessions:
            self._sessions_by_id[session.id] = session

        return sessions

    def get_all_sessions(self, force_refresh: bool = False) -> list[Session]:
        """Get all sessions from all adapters with incremental updates."""
        if self._sessions is not None and not force_refresh:
            return self._sessions

        # If streaming is in progress, return current partial results
        if self._streaming_in_progress:
            return self._sessions if self._sessions is not None else []

        # Get known sessions from Tantivy for incremental comparison
        known = self._index.get_known_sessions() if not force_refresh else {}

        # Ask each adapter for changes
        all_new_or_modified: list[Session] = []
        all_deleted_ids: list[str] = []

        def get_incremental(adapter):
            return adapter.find_sessions_incremental(known)

        with ThreadPoolExecutor(max_workers=len(self.adapters)) as executor:
            results = executor.map(get_incremental, self.adapters)
            for new_or_modified, deleted_ids in results:
                all_new_or_modified.extend(new_or_modified)
                all_deleted_ids.extend(deleted_ids)

        # If no changes and we have data in index, load from index
        if not all_new_or_modified and not all_deleted_ids and known:
            self._sessions = self._index.get_all_sessions()
            for session in self._sessions:
                self._sessions_by_id[session.id] = session
            self._sessions.sort(key=lambda s: s.timestamp, reverse=True)
            return self._sessions

        # Apply deletions to index
        self._index.delete_sessions(all_deleted_ids)

        # Delete modified sessions before re-adding (avoid duplicates)
        modified_ids = [s.id for s in all_new_or_modified]
        self._index.delete_sessions(modified_ids)

        # Apply additions/updates to index
        self._index.add_sessions(all_new_or_modified)

        # Load all sessions from index
        self._sessions = self._index.get_all_sessions()
        for session in self._sessions:
            self._sessions_by_id[session.id] = session

        # Sort by timestamp, newest first
        self._sessions.sort(key=lambda s: s.timestamp, reverse=True)

        return self._sessions

    def get_sessions_streaming(
        self, on_progress: Callable[[list[Session]], None]
    ) -> list[Session]:
        """Load sessions with progress callback for each adapter that completes."""
        # Get known sessions from Tantivy
        known = self._index.get_known_sessions()

        # Mark streaming as in progress
        self._streaming_in_progress = True
        all_new_or_modified: list[Session] = []
        all_deleted_ids: list[str] = []

        def get_incremental(adapter):
            return adapter.find_sessions_incremental(known)

        try:
            with ThreadPoolExecutor(max_workers=len(self.adapters)) as executor:
                futures = {
                    executor.submit(get_incremental, a): a for a in self.adapters
                }
                for future in as_completed(futures):
                    new_or_modified, deleted_ids = future.result()
                    all_new_or_modified.extend(new_or_modified)
                    all_deleted_ids.extend(deleted_ids)

                    # Report progress with current sessions from index + new ones
                    # For streaming, we build up partial results
                    current_sessions = self._index.get_all_sessions()
                    # Add newly found sessions (not yet in index)
                    existing_ids = {s.id for s in current_sessions}
                    for session in all_new_or_modified:
                        if session.id not in existing_ids:
                            current_sessions.append(session)
                    # Remove deleted
                    deleted_set = set(all_deleted_ids)
                    current_sessions = [
                        s for s in current_sessions if s.id not in deleted_set
                    ]
                    current_sessions.sort(key=lambda s: s.timestamp, reverse=True)
                    on_progress(current_sessions)
        finally:
            self._streaming_in_progress = False

        # Apply all changes to index
        self._index.delete_sessions(all_deleted_ids)
        # Delete modified sessions before re-adding (avoid duplicates)
        modified_ids = [s.id for s in all_new_or_modified]
        self._index.delete_sessions(modified_ids)
        self._index.add_sessions(all_new_or_modified)

        # Load final state from index
        self._sessions = self._index.get_all_sessions()
        for session in self._sessions:
            self._sessions_by_id[session.id] = session
        self._sessions.sort(key=lambda s: s.timestamp, reverse=True)

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
