"""Tantivy full-text search index for sessions."""

import shutil
from collections import Counter
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path

import tantivy

from .adapters.base import Session
from .config import INDEX_DIR, MAX_PREVIEW_LENGTH, SCHEMA_VERSION

# Version file to detect schema changes
_VERSION_FILE = ".schema_version"


@dataclass
class IndexStats:
    """Statistics about the index contents."""

    total_sessions: int
    sessions_by_agent: dict[str, int]
    total_messages: int
    oldest_session: datetime | None
    newest_session: datetime | None
    top_directories: list[
        tuple[str, int, int]
    ]  # (directory, sessions, messages) tuples
    index_size_bytes: int
    # Time breakdown
    sessions_today: int
    sessions_this_week: int
    sessions_this_month: int
    sessions_older: int
    # Content metrics
    total_content_chars: int
    avg_content_chars: int
    avg_messages_per_session: float
    # Activity patterns
    sessions_by_weekday: dict[str, int]  # Mon, Tue, etc.
    sessions_by_hour: dict[int, int]  # 0-23
    # Daily activity (date string -> (sessions, messages))
    daily_activity: list[tuple[str, int, int]]  # (date, sessions, messages)
    # Per-agent raw data
    messages_by_agent: dict[str, int] | None = None
    content_chars_by_agent: dict[str, int] | None = None


class TantivyIndex:
    """Manages a Tantivy full-text search index for sessions.

    This is the single source of truth for session data.
    """

    def __init__(self, index_path: Path = INDEX_DIR) -> None:
        self.index_path = index_path
        self._index: tantivy.Index | None = None
        self._schema: tantivy.Schema | None = None
        self._version_file = index_path / _VERSION_FILE

    def _build_schema(self) -> tantivy.Schema:
        """Build the Tantivy schema for sessions."""
        schema_builder = tantivy.SchemaBuilder()
        # ID field - stored and indexed with raw tokenizer for exact term matching
        schema_builder.add_text_field("id", stored=True, tokenizer_name="raw")
        # Title - stored and indexed for search
        schema_builder.add_text_field("title", stored=True)
        # Directory - stored and indexed
        schema_builder.add_text_field("directory", stored=True)
        # Agent - stored for filtering (raw tokenizer to preserve hyphens)
        schema_builder.add_text_field("agent", stored=True, tokenizer_name="raw")
        # Content - stored and indexed for full-text search
        schema_builder.add_text_field("content", stored=True)
        # Timestamp - stored as float (Unix timestamp)
        schema_builder.add_float_field("timestamp", stored=True)
        # Message count - stored as integer
        schema_builder.add_integer_field("message_count", stored=True)
        # File modification time - for incremental updates
        schema_builder.add_float_field("mtime", stored=True)
        # Yolo mode - session was started with auto-approve/skip-permissions
        schema_builder.add_boolean_field("yolo", stored=True)
        return schema_builder.build()

    def _check_version(self) -> bool:
        """Check if index version matches current schema version."""
        if not self._version_file.exists():
            return False
        try:
            stored_version = int(self._version_file.read_text().strip())
            return stored_version == SCHEMA_VERSION
        except (ValueError, OSError):
            return False

    def _write_version(self) -> None:
        """Write current schema version to version file."""
        self._version_file.parent.mkdir(parents=True, exist_ok=True)
        self._version_file.write_text(str(SCHEMA_VERSION))

    def _clear(self) -> None:
        """Clear the index directory."""
        self._index = None
        self._schema = None
        if self.index_path.exists():
            shutil.rmtree(self.index_path)

    def _ensure_index(self) -> tantivy.Index:
        """Ensure the index is loaded or created."""
        if self._index is not None:
            return self._index

        # Check version - rebuild if schema changed
        if self.index_path.exists() and not self._check_version():
            self._clear()

        self._schema = self._build_schema()

        if self.index_path.exists():
            # Open existing index
            self._index = tantivy.Index(self._schema, path=str(self.index_path))
        else:
            # Create new index
            self.index_path.mkdir(parents=True, exist_ok=True)
            self._index = tantivy.Index(self._schema, path=str(self.index_path))
            self._write_version()

        return self._index

    def get_known_sessions(self) -> dict[str, tuple[float, str]]:
        """Get all session IDs with their mtimes and agents.

        Returns:
            Dict mapping session_id to (mtime, agent) tuple.
        """
        if not self.index_path.exists() or not self._check_version():
            return {}

        index = self._ensure_index()
        index.reload()
        searcher = index.searcher()

        if searcher.num_docs == 0:
            return {}

        known: dict[str, tuple[float, str]] = {}

        # Match all documents
        all_query = tantivy.Query.all_query()
        results = searcher.search(all_query, limit=searcher.num_docs).hits

        for _score, doc_address in results:
            doc = searcher.doc(doc_address)
            session_id = doc.get_first("id")
            mtime = doc.get_first("mtime")
            agent = doc.get_first("agent")
            if session_id and mtime is not None and agent:
                known[session_id] = (mtime, agent)

        return known

    def get_all_sessions(self) -> list[Session]:
        """Retrieve all sessions from the index.

        Returns:
            List of Session objects, unsorted.
        """
        if not self.index_path.exists() or not self._check_version():
            return []

        index = self._ensure_index()
        index.reload()
        searcher = index.searcher()

        if searcher.num_docs == 0:
            return []

        sessions: list[Session] = []

        # Match all documents
        all_query = tantivy.Query.all_query()
        results = searcher.search(all_query, limit=searcher.num_docs).hits

        for _score, doc_address in results:
            doc = searcher.doc(doc_address)
            session = self._doc_to_session(doc)
            if session:
                sessions.append(session)

        return sessions

    def get_session_count(self, agent_filter: str | None = None) -> int:
        """Get the total number of sessions in the index.

        Args:
            agent_filter: If provided, only count sessions for this agent.
        """
        if not self.index_path.exists() or not self._check_version():
            return 0

        index = self._ensure_index()
        index.reload()
        searcher = index.searcher()

        if agent_filter is None:
            return searcher.num_docs

        # Count sessions for specific agent using term query
        schema = index.schema
        query = tantivy.Query.term_query(schema, "agent", agent_filter)
        # Tantivy requires limit > 0, use count property for total matches
        return searcher.search(query, limit=1).count  # type: ignore[attr-defined]

    def get_stats(self) -> IndexStats:
        """Get statistics about the index contents."""
        empty_stats = IndexStats(
            total_sessions=0,
            sessions_by_agent={},
            total_messages=0,
            oldest_session=None,
            newest_session=None,
            top_directories=[],
            index_size_bytes=0,
            sessions_today=0,
            sessions_this_week=0,
            sessions_this_month=0,
            sessions_older=0,
            total_content_chars=0,
            avg_content_chars=0,
            avg_messages_per_session=0.0,
            sessions_by_weekday={},
            sessions_by_hour={},
            daily_activity=[],
        )

        if not self.index_path.exists() or not self._check_version():
            return empty_stats

        index = self._ensure_index()
        index.reload()
        searcher = index.searcher()

        if searcher.num_docs == 0:
            empty_stats.index_size_bytes = self._get_index_size()
            return empty_stats

        # Collect stats from all documents
        agent_counts: Counter[str] = Counter()
        agent_messages: Counter[str] = Counter()
        agent_content_chars: Counter[str] = Counter()
        dir_counts: Counter[str] = Counter()
        dir_messages: Counter[str] = Counter()
        weekday_counts: Counter[str] = Counter()
        hour_counts: Counter[int] = Counter()
        daily_sessions: Counter[str] = Counter()
        daily_messages: Counter[str] = Counter()
        total_messages = 0
        total_content_chars = 0
        oldest_ts: float | None = None
        newest_ts: float | None = None

        # Time boundaries
        now = datetime.now()
        today_start = now.replace(hour=0, minute=0, second=0, microsecond=0)
        week_start = today_start.replace(day=today_start.day - today_start.weekday())
        month_start = today_start.replace(day=1)

        sessions_today = 0
        sessions_this_week = 0
        sessions_this_month = 0
        sessions_older = 0

        weekday_names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]

        all_query = tantivy.Query.all_query()
        results = searcher.search(all_query, limit=searcher.num_docs).hits

        for _score, doc_address in results:
            doc = searcher.doc(doc_address)

            agent = doc.get_first("agent")
            if agent:
                agent_counts[agent] += 1

            directory = doc.get_first("directory")
            if directory:
                dir_counts[directory] += 1

            msg_count = doc.get_first("message_count")
            if msg_count:
                total_messages += msg_count
                if directory:
                    dir_messages[directory] += msg_count
                if agent:
                    agent_messages[agent] += msg_count

            content = doc.get_first("content")
            if content:
                content_len = len(content)
                total_content_chars += content_len
                if agent:
                    agent_content_chars[agent] += content_len

            timestamp = doc.get_first("timestamp")
            if timestamp is not None:
                if oldest_ts is None or timestamp < oldest_ts:
                    oldest_ts = timestamp
                if newest_ts is None or timestamp > newest_ts:
                    newest_ts = timestamp

                # Time breakdown
                dt = datetime.fromtimestamp(timestamp)
                if dt >= today_start:
                    sessions_today += 1
                if dt >= week_start:
                    sessions_this_week += 1
                if dt >= month_start:
                    sessions_this_month += 1
                else:
                    sessions_older += 1

                # Activity patterns
                weekday_counts[weekday_names[dt.weekday()]] += 1
                hour_counts[dt.hour] += 1

                # Daily activity
                date_str = dt.strftime("%Y-%m-%d")
                daily_sessions[date_str] += 1
                if msg_count:
                    daily_messages[date_str] += msg_count

        num_docs = searcher.num_docs

        # Build daily activity list sorted by date
        all_dates = sorted(set(daily_sessions.keys()) | set(daily_messages.keys()))
        daily_activity = [
            (d, daily_sessions.get(d, 0), daily_messages.get(d, 0)) for d in all_dates
        ]

        return IndexStats(
            total_sessions=num_docs,
            sessions_by_agent=dict(agent_counts),
            total_messages=total_messages,
            oldest_session=datetime.fromtimestamp(oldest_ts) if oldest_ts else None,
            newest_session=datetime.fromtimestamp(newest_ts) if newest_ts else None,
            top_directories=[
                (d, count, dir_messages[d]) for d, count in dir_counts.most_common(10)
            ],
            index_size_bytes=self._get_index_size(),
            sessions_today=sessions_today,
            sessions_this_week=sessions_this_week,
            sessions_this_month=sessions_this_month,
            sessions_older=sessions_older,
            total_content_chars=total_content_chars,
            avg_content_chars=total_content_chars // num_docs if num_docs else 0,
            avg_messages_per_session=total_messages / num_docs if num_docs else 0.0,
            sessions_by_weekday={d: weekday_counts.get(d, 0) for d in weekday_names},
            sessions_by_hour=dict(hour_counts),
            daily_activity=daily_activity,
            messages_by_agent=dict(agent_messages),
            content_chars_by_agent=dict(agent_content_chars),
        )

    def _get_index_size(self) -> int:
        """Get total size of the index directory in bytes."""
        if not self.index_path.exists():
            return 0
        total = 0
        for f in self.index_path.rglob("*"):
            if f.is_file():
                total += f.stat().st_size
        return total

    def _doc_to_session(self, doc: tantivy.Document) -> Session | None:
        """Convert a Tantivy document to a Session object."""
        try:
            session_id = doc.get_first("id")
            if not session_id:
                return None

            timestamp_float = doc.get_first("timestamp")
            if timestamp_float is None:
                return None

            content = doc.get_first("content") or ""

            return Session(
                id=session_id,
                agent=doc.get_first("agent") or "",
                title=doc.get_first("title") or "",
                directory=doc.get_first("directory") or "",
                timestamp=datetime.fromtimestamp(timestamp_float),
                preview=content[:MAX_PREVIEW_LENGTH],
                content=content,
                message_count=doc.get_first("message_count") or 0,
                mtime=doc.get_first("mtime") or 0.0,
                yolo=doc.get_first("yolo") or False,
            )
        except Exception:
            return None

    def delete_sessions(self, session_ids: list[str]) -> None:
        """Remove sessions from the index by ID."""
        if not session_ids:
            return

        index = self._ensure_index()
        writer = index.writer()
        for sid in session_ids:
            writer.delete_documents_by_term("id", sid)
        writer.commit()

    def add_sessions(self, sessions: list[Session]) -> None:
        """Add sessions to the index."""
        if not sessions:
            return

        index = self._ensure_index()
        writer = index.writer()
        for session in sessions:
            writer.add_document(
                tantivy.Document(
                    id=session.id,
                    title=session.title,
                    directory=session.directory,
                    agent=session.agent,
                    content=session.content,
                    timestamp=session.timestamp.timestamp(),
                    message_count=session.message_count,
                    mtime=session.mtime,
                    yolo=session.yolo,
                )
            )
        writer.commit()

    def update_sessions(self, sessions: list[Session]) -> None:
        """Update sessions in the index (delete then add in a single transaction)."""
        if not sessions:
            return

        index = self._ensure_index()
        writer = index.writer()
        # Delete existing documents first
        for session in sessions:
            writer.delete_documents_by_term("id", session.id)
        # Add new versions
        for session in sessions:
            writer.add_document(
                tantivy.Document(
                    id=session.id,
                    title=session.title,
                    directory=session.directory,
                    agent=session.agent,
                    content=session.content,
                    timestamp=session.timestamp.timestamp(),
                    message_count=session.message_count,
                    mtime=session.mtime,
                    yolo=session.yolo,
                )
            )
        writer.commit()

    def search(
        self,
        query: str,
        agent_filter: str | None = None,
        limit: int = 100,
    ) -> list[tuple[str, float]]:
        """Search the index and return (session_id, score) pairs.

        Uses fuzzy matching with edit distance 1 for typo tolerance.
        """
        index = self._ensure_index()
        index.reload()
        searcher = index.searcher()
        schema = index.schema

        try:
            # Build fuzzy query for each term
            query_parts = self._build_fuzzy_query(query, schema)

            # Add agent filter if specified
            if agent_filter:
                agent_query = tantivy.Query.term_query(schema, "agent", agent_filter)
                query_parts.append((tantivy.Occur.Must, agent_query))

            # Combine all query parts
            if not query_parts:
                return []

            combined_query = tantivy.Query.boolean_query(query_parts)
            results = searcher.search(combined_query, limit).hits

            # Extract session IDs and scores
            output = []
            for score, doc_address in results:
                doc = searcher.doc(doc_address)
                session_id = doc.get_first("id")
                if session_id:
                    output.append((session_id, score))

            return output
        except Exception:
            # If query fails, return empty results
            return []

    def _build_fuzzy_query(
        self, query: str, schema: tantivy.Schema
    ) -> list[tuple[tantivy.Occur, tantivy.Query]]:
        """Build fuzzy query parts for all terms in the query.

        Each term is searched with edit distance 1 in both title and content fields.
        All terms must match (AND logic).
        """
        if not query.strip():
            return []

        terms = query.split()
        query_parts = []

        for term in terms:
            if not term:
                continue

            # Create fuzzy queries for title and content fields
            # Using prefix=True allows matching longer words that start with the term
            fuzzy_title = tantivy.Query.fuzzy_term_query(
                schema, "title", term, distance=1, prefix=True
            )
            fuzzy_content = tantivy.Query.fuzzy_term_query(
                schema, "content", term, distance=1, prefix=True
            )

            # Combine with OR - term can match in either field
            term_query = tantivy.Query.boolean_query(
                [
                    (tantivy.Occur.Should, fuzzy_title),
                    (tantivy.Occur.Should, fuzzy_content),
                ]
            )

            # Each term must match (AND logic between terms)
            query_parts.append((tantivy.Occur.Must, term_query))

        return query_parts
