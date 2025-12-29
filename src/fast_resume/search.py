"""Search engine for aggregating and searching sessions."""

from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import timedelta
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
from .query import DateFilter, DateOp, Filter, parse_query


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

    def get_sessions_streaming(
        self,
        on_progress: Callable[[], None],
        on_error: ErrorCallback = None,
    ) -> tuple[list[Session], int, int, int]:
        """Load sessions with progress callback for each adapter that completes.

        Sessions are indexed incrementally as each adapter completes, allowing
        Tantivy search to work during streaming.

        Args:
            on_progress: Callback for progress updates
            on_error: Optional callback for parse errors

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

        def get_incremental(adapter):
            return adapter.find_sessions_incremental(known, on_error=on_error)

        try:
            with ThreadPoolExecutor(max_workers=len(self.adapters)) as executor:
                futures = {
                    executor.submit(get_incremental, a): a for a in self.adapters
                }
                for future in as_completed(futures):
                    new_or_modified, deleted_ids = future.result()

                    # Skip if no changes from this adapter
                    if not new_or_modified and not deleted_ids:
                        continue

                    # Handle deletions first
                    if deleted_ids:
                        self._index.delete_sessions(deleted_ids)
                        for sid in deleted_ids:
                            self._sessions_by_id.pop(sid, None)
                        total_deleted += len(deleted_ids)

                    # Index incrementally + update _sessions_by_id for search lookup
                    if new_or_modified:
                        # Update atomically (delete + add in single transaction)
                        self._index.update_sessions(new_or_modified)
                        for session in new_or_modified:
                            self._sessions_by_id[session.id] = session
                            # Count new vs updated
                            if session.id in known:
                                total_updated += 1
                            else:
                                total_new += 1

                    # Notify progress - TUI will query the index
                    on_progress()
        finally:
            self._streaming_in_progress = False

        # Load final state from index
        self._sessions = self._index.get_all_sessions()
        for session in self._sessions:
            self._sessions_by_id[session.id] = session
        self._sessions.sort(key=lambda s: s.timestamp, reverse=True)

        return self._sessions, total_new, total_updated, total_deleted

    def _matches_date_filter(self, session: Session, date_filter: DateFilter) -> bool:
        """Check if a session matches the date filter."""
        session_ts = session.timestamp
        match = False

        if date_filter.op == DateOp.LESS_THAN:
            # Session must be newer than cutoff (within the time range)
            match = session_ts >= date_filter.cutoff
        elif date_filter.op == DateOp.GREATER_THAN:
            # Session must be older than cutoff
            match = session_ts < date_filter.cutoff
        elif date_filter.op == DateOp.EXACT:
            # For "today" or "yesterday", session must be on that day
            if date_filter.value.lower() == "today":
                match = session_ts >= date_filter.cutoff
            elif date_filter.value.lower() == "yesterday":
                next_day = date_filter.cutoff + timedelta(days=1)
                match = date_filter.cutoff <= session_ts < next_day
            else:
                match = True
        else:
            match = True

        return not match if date_filter.negated else match

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

        # If no query, return filtered sessions by timestamp
        if not search_text:
            sessions = sorted(
                self._sessions_by_id.values(),
                key=lambda s: s.timestamp,
                reverse=True,
            )
            if effective_agent:
                sessions = [s for s in sessions if effective_agent.matches(s.agent)]
            if effective_dir:
                sessions = [
                    s
                    for s in sessions
                    if effective_dir.matches(s.directory, substring=True)
                ]
            if date_filter:
                sessions = [
                    s for s in sessions if self._matches_date_filter(s, date_filter)
                ]
            return sessions[:limit]

        # For Tantivy search, extract first agent value for index query (if not negated)
        tantivy_agent = None
        if (
            effective_agent
            and not effective_agent.negated
            and len(effective_agent.values) == 1
        ):
            tantivy_agent = effective_agent.values[0]

        # Use Tantivy for fuzzy search
        results = self._index.search(
            search_text,
            agent_filter=tantivy_agent,
            limit=limit * 2,  # Fetch extra for post-filtering
        )

        # Lookup full session objects from results
        matched_sessions = []
        for session_id, _score in results:
            session = self._sessions_by_id.get(session_id)
            if session:
                # Apply agent filter (for multi-value or negated filters)
                if effective_agent and tantivy_agent is None:
                    if not effective_agent.matches(session.agent):
                        continue
                # Apply directory filter (substring match not supported in FTS)
                if effective_dir:
                    if not effective_dir.matches(session.directory, substring=True):
                        continue
                # Apply date filter
                if date_filter:
                    if not self._matches_date_filter(session, date_filter):
                        continue
                matched_sessions.append(session)
                if len(matched_sessions) >= limit:
                    break

        return matched_sessions[:limit]

    def get_session_count(self, agent_filter: str | None = None) -> int:
        """Get the total number of sessions in the index.

        Args:
            agent_filter: If provided, only count sessions for this agent.
        """
        return self._index.get_session_count(agent_filter)

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
