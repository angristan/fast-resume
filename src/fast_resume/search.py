"""Search engine for aggregating and searching sessions."""

import threading
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Callable

from .adapters import (
    ClaudeAdapter,
    CodexAdapter,
    CopilotAdapter,
    CopilotVSCodeAdapter,
    CrushAdapter,
    ErrorCallback,
    OpenCodeAdapter,
    Session,
    VibeAdapter,
)
from .index import TantivyIndex
from .query import Filter, parse_query


class SessionSearch:
    """Aggregates sessions from all adapters and provides search.

    Uses Tantivy as the single source of truth for session data.
    """

    def __init__(self) -> None:
        self.adapters = [
            ClaudeAdapter(),
            CodexAdapter(),
            CopilotAdapter(),
            CopilotVSCodeAdapter(),
            CrushAdapter(),
            OpenCodeAdapter(),
            VibeAdapter(),
        ]
        self._sessions: list[Session] | None = None
        self._sessions_by_id: dict[str, Session] = {}
        self._streaming_in_progress: bool = False
        self._index = TantivyIndex()

    def _load_from_index(self) -> list[Session] | None:
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

        # Update modified sessions atomically (delete + add in single transaction)
        self._index.update_sessions(all_new_or_modified)

        # Load all sessions from index
        self._sessions = self._index.get_all_sessions()
        for session in self._sessions:
            self._sessions_by_id[session.id] = session

        # Sort by timestamp, newest first
        self._sessions.sort(key=lambda s: s.timestamp, reverse=True)

        return self._sessions

    def index_sessions_parallel(
        self,
        on_progress: Callable[[], None],
        on_error: ErrorCallback = None,
        batch_size: int = 100,
    ) -> tuple[list[Session], int, int, int]:
        """Load sessions with progress callback for each adapter that completes.

        Sessions are indexed progressively in batches as they are parsed,
        allowing Tantivy search to work during streaming while avoiding
        the overhead of committing each session individually.

        Args:
            on_progress: Callback for progress updates
            on_error: Optional callback for parse errors
            batch_size: Number of sessions to batch before committing and updating UI

        Returns:
            Tuple of (sessions, new_count, updated_count, deleted_count)
        """
        # Get known sessions from Tantivy
        known = self._index.get_known_sessions()

        # Pre-populate _sessions_by_id with existing sessions from index
        # so search() can find them during streaming
        existing_sessions = self._index.get_all_sessions()
        for session in existing_sessions:
            self._sessions_by_id[session.id] = session

        # Mark streaming as in progress
        self._streaming_in_progress = True
        total_new = 0
        total_updated = 0
        total_deleted = 0

        # Thread-safe batch buffer for progressive indexing
        lock = threading.Lock()
        pending_sessions: list[Session] = []
        all_deleted_ids: list[str] = []
        sessions_since_progress = 0

        def flush_pending() -> None:
            """Flush pending sessions to index (must be called with lock held)."""
            nonlocal pending_sessions
            if pending_sessions:
                self._index.update_sessions(pending_sessions)
                pending_sessions = []

        def handle_session(session: Session) -> None:
            """Buffer session for batched indexing (thread-safe)."""
            nonlocal total_new, total_updated, sessions_since_progress
            with lock:
                # Add to lookup immediately for search during streaming
                self._sessions_by_id[session.id] = session
                pending_sessions.append(session)
                # Count new vs updated
                if session.id in known:
                    total_updated += 1
                else:
                    total_new += 1
                # Update UI periodically with committed sessions
                sessions_since_progress += 1
                if sessions_since_progress >= batch_size:
                    sessions_since_progress = 0
                    flush_pending()  # Commit before UI update so search sees new sessions
                    on_progress()

        def get_incremental(adapter):
            return adapter.find_sessions_incremental(
                known, on_error=on_error, on_session=handle_session
            )

        try:
            with ThreadPoolExecutor(max_workers=len(self.adapters)) as executor:
                futures = {
                    executor.submit(get_incremental, a): a for a in self.adapters
                }
                for future in as_completed(futures):
                    _new_or_modified, deleted_ids = future.result()

                    with lock:
                        # Flush pending sessions when adapter completes
                        flush_pending()

                        # Accumulate deletions (will be processed at the end)
                        if deleted_ids:
                            all_deleted_ids.extend(deleted_ids)
                            for sid in deleted_ids:
                                self._sessions_by_id.pop(sid, None)
                            total_deleted += len(deleted_ids)
        finally:
            # Final flush of any remaining sessions
            with lock:
                flush_pending()
            # Process all deletions in a single operation
            if all_deleted_ids:
                self._index.delete_sessions(all_deleted_ids)
            self._streaming_in_progress = False

        # Load final state from index
        self._sessions = self._index.get_all_sessions()
        for session in self._sessions:
            self._sessions_by_id[session.id] = session
        self._sessions.sort(key=lambda s: s.timestamp, reverse=True)

        return self._sessions, total_new, total_updated, total_deleted

    def search(
        self,
        query: str,
        agent_filter: str | None = None,
        directory_filter: str | None = None,
        limit: int = 100,
    ) -> list[Session]:
        """Search sessions using Tantivy full-text search with fuzzy matching.

        Supports keyword syntax in the query:
        - agent:value,value2 - Filter by agent (comma for OR, ! or - for NOT)
        - dir:value - Filter by directory (substring match)
        - date:value - Filter by date (today, yesterday, <1h, >1d, etc.)

        Explicit filter parameters take precedence over keywords in the query.
        All filtering is done at the Tantivy level for efficiency.
        """
        # Parse keyword syntax from query
        parsed = parse_query(query)
        search_text = parsed.text

        # Merge filters: explicit params take precedence over parsed keywords
        # Convert string params to Filter objects for consistency
        if agent_filter is not None:
            effective_agent: Filter | None = Filter(include=[agent_filter])
        else:
            effective_agent = parsed.agent

        if directory_filter is not None:
            effective_dir: Filter | None = Filter(include=[directory_filter])
        else:
            effective_dir = parsed.directory

        date_filter = parsed.date

        # During streaming, _sessions_by_id is updated incrementally
        # Only call get_all_sessions() if not streaming and no sessions loaded yet
        if not self._streaming_in_progress and self._sessions is None:
            self.get_all_sessions()

        # Use Tantivy for all searching and filtering
        results = self._index.search(
            search_text,
            agent_filter=effective_agent,
            directory_filter=effective_dir,
            date_filter=date_filter,
            limit=limit,
        )

        # Lookup full session objects from results
        matched_sessions = []
        for session_id, _score in results:
            session = self._sessions_by_id.get(session_id)
            if session:
                matched_sessions.append(session)

        return matched_sessions

    def get_session_count(self, agent_filter: str | None = None) -> int:
        """Get the total number of sessions in the index.

        Args:
            agent_filter: If provided, only count sessions for this agent.
        """
        return self._index.get_session_count(agent_filter)

    def get_agents_with_sessions(self) -> set[str]:
        """Get the set of agent names that have at least one session."""
        agents = set()
        for adapter in self.adapters:
            if self._index.get_session_count(adapter.name) > 0:
                agents.add(adapter.name)
        return agents

    def get_adapter_for_session(self, session: Session):
        """Get the adapter for a session."""
        for adapter in self.adapters:
            if adapter.name == session.agent:
                return adapter
        return None

    def get_resume_command(self, session: Session, yolo: bool = False) -> list[str]:
        """Get the resume command for a session."""
        adapter = self.get_adapter_for_session(session)
        if adapter:
            return adapter.get_resume_command(session, yolo=yolo)
        return []
