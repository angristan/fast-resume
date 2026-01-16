"""Tantivy full-text search index for sessions."""

import re
import shutil
from collections import Counter
from dataclasses import dataclass
from datetime import datetime, timedelta
from pathlib import Path

import tantivy

from .adapters.base import Session
from .config import INDEX_DIR, SCHEMA_VERSION
from .query import DateFilter, DateOp, Filter

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
        # Directory - stored with raw tokenizer for regex substring matching
        schema_builder.add_text_field("directory", stored=True, tokenizer_name="raw")
        # Agent - stored for filtering (raw tokenizer to preserve hyphens)
        schema_builder.add_text_field("agent", stored=True, tokenizer_name="raw")
        # Content - stored and indexed for full-text search
        schema_builder.add_text_field("content", stored=True)
        # Timestamp - stored and indexed for range queries
        schema_builder.add_float_field("timestamp", stored=True, indexed=True)
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
        week_start = today_start - timedelta(days=today_start.weekday())
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
        agent_filter: Filter | None = None,
        directory_filter: Filter | None = None,
        date_filter: DateFilter | None = None,
        limit: int = 100,
    ) -> list[tuple[str, float]]:
        """Search the index and return (session_id, score) pairs.

        Uses a hybrid approach:
        - Exact matches (via parsed query) are boosted 5x for better ranking
        - Fuzzy matches (edit distance 1) provide typo tolerance

        All filters are applied at the Tantivy level for efficiency:
        - agent_filter: term_set_query for includes, MustNot for excludes
        - directory_filter: regex_query for substring matching
        - date_filter: range_query on timestamp field
        """
        index = self._ensure_index()
        index.reload()
        searcher = index.searcher()
        schema = index.schema

        try:
            query_parts: list[tuple[tantivy.Occur, tantivy.Query]] = []

            if query.strip():
                # Build hybrid query: exact (boosted) + fuzzy (for typo tolerance)
                text_query = self._build_hybrid_query(query, index, schema)
                query_parts.append((tantivy.Occur.Must, text_query))

            # Add agent filter if specified
            agent_query = self._build_agent_filter_query(agent_filter, schema)
            if agent_query:
                query_parts.append((tantivy.Occur.Must, agent_query))

            # Add directory filter if specified
            dir_query = self._build_directory_filter_query(directory_filter, schema)
            if dir_query:
                query_parts.append((tantivy.Occur.Must, dir_query))

            # Add date filter if specified
            date_query = self._build_date_filter_query(date_filter, schema)
            if date_query:
                query_parts.append((tantivy.Occur.Must, date_query))

            # Combine all query parts
            if not query_parts:
                # No text query and no filters - match all documents
                combined_query = tantivy.Query.all_query()
            else:
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

    def _build_agent_filter_query(
        self,
        agent_filter: Filter | None,
        schema: tantivy.Schema,
    ) -> tantivy.Query | None:
        """Build a Tantivy query for agent filtering.

        Supports:
        - Multiple include values (OR logic via term_set_query)
        - Multiple exclude values (AND logic via MustNot)
        - Mixed include/exclude
        """
        if not agent_filter:
            return None

        parts: list[tuple[tantivy.Occur, tantivy.Query]] = []

        # Include filter: match any of the included agents
        if agent_filter.include:
            if len(agent_filter.include) == 1:
                # Single value: use term_query
                include_query = tantivy.Query.term_query(
                    schema, "agent", agent_filter.include[0]
                )
            else:
                # Multiple values: use term_set_query (OR)
                include_query = tantivy.Query.term_set_query(
                    schema, "agent", agent_filter.include
                )
            parts.append((tantivy.Occur.Must, include_query))

        # Exclude filter: reject any of the excluded agents
        for excluded in agent_filter.exclude:
            exclude_query = tantivy.Query.term_query(schema, "agent", excluded)
            parts.append((tantivy.Occur.MustNot, exclude_query))

        if not parts:
            return None

        # If only excludes, we need a base query to exclude from
        if not agent_filter.include and agent_filter.exclude:
            # Match all, then exclude
            parts.insert(0, (tantivy.Occur.Must, tantivy.Query.all_query()))

        return tantivy.Query.boolean_query(parts)

    def _build_directory_filter_query(
        self,
        directory_filter: Filter | None,
        schema: tantivy.Schema,
    ) -> tantivy.Query | None:
        """Build a Tantivy query for directory filtering using regex.

        Uses regex_query for substring matching (case-insensitive).
        """
        if not directory_filter:
            return None

        parts: list[tuple[tantivy.Occur, tantivy.Query]] = []

        # Include filter: match any directory containing the substring
        if directory_filter.include:
            include_parts: list[tuple[tantivy.Occur, tantivy.Query]] = []
            for dir_pattern in directory_filter.include:
                # Escape regex special characters and build case-insensitive pattern
                escaped = re.escape(dir_pattern)
                regex_pattern = f"(?i).*{escaped}.*"
                include_query = tantivy.Query.regex_query(
                    schema, "directory", regex_pattern
                )
                include_parts.append((tantivy.Occur.Should, include_query))

            if len(include_parts) == 1:
                parts.append((tantivy.Occur.Must, include_parts[0][1]))
            else:
                parts.append(
                    (tantivy.Occur.Must, tantivy.Query.boolean_query(include_parts))
                )

        # Exclude filter: reject directories containing the substring
        for dir_pattern in directory_filter.exclude:
            escaped = re.escape(dir_pattern)
            regex_pattern = f"(?i).*{escaped}.*"
            exclude_query = tantivy.Query.regex_query(
                schema, "directory", regex_pattern
            )
            parts.append((tantivy.Occur.MustNot, exclude_query))

        if not parts:
            return None

        # If only excludes, we need a base query to exclude from
        if not directory_filter.include and directory_filter.exclude:
            parts.insert(0, (tantivy.Occur.Must, tantivy.Query.all_query()))

        return tantivy.Query.boolean_query(parts)

    def _build_date_filter_query(
        self,
        date_filter: DateFilter | None,
        schema: tantivy.Schema,
    ) -> tantivy.Query | None:
        """Build a Tantivy query for date filtering using range queries.

        Supports:
        - date:<1h (sessions newer than 1 hour)
        - date:>1d (sessions older than 1 day)
        - date:today (sessions from today)
        - date:yesterday (sessions from yesterday only)
        - Negation via date:!today or -date:today
        """
        if not date_filter:
            return None

        cutoff_ts = date_filter.cutoff.timestamp()

        # Build the range query based on operator
        # Use float('inf') and float('-inf') for unbounded ranges
        if date_filter.op == DateOp.LESS_THAN:
            # Sessions newer than cutoff (timestamp >= cutoff)
            range_query = tantivy.Query.range_query(
                schema,
                "timestamp",
                tantivy.FieldType.Float,
                lower_bound=cutoff_ts,
                upper_bound=float("inf"),
                include_lower=True,
                include_upper=True,
            )
        elif date_filter.op == DateOp.GREATER_THAN:
            # Sessions older than cutoff (timestamp < cutoff)
            range_query = tantivy.Query.range_query(
                schema,
                "timestamp",
                tantivy.FieldType.Float,
                lower_bound=float("-inf"),
                upper_bound=cutoff_ts,
                include_lower=True,
                include_upper=False,
            )
        elif date_filter.op == DateOp.EXACT:
            if date_filter.value.lower() == "today":
                # Sessions from today (timestamp >= today_start)
                range_query = tantivy.Query.range_query(
                    schema,
                    "timestamp",
                    tantivy.FieldType.Float,
                    lower_bound=cutoff_ts,
                    upper_bound=float("inf"),
                    include_lower=True,
                    include_upper=True,
                )
            elif date_filter.value.lower() == "yesterday":
                # Sessions from yesterday only (cutoff <= timestamp < cutoff + 1 day)
                next_day_ts = (date_filter.cutoff + timedelta(days=1)).timestamp()
                range_query = tantivy.Query.range_query(
                    schema,
                    "timestamp",
                    tantivy.FieldType.Float,
                    lower_bound=cutoff_ts,
                    upper_bound=next_day_ts,
                    include_lower=True,
                    include_upper=False,
                )
            else:
                # Unknown exact date, match all
                return None
        else:
            return None

        # Handle negation
        if date_filter.negated:
            return tantivy.Query.boolean_query(
                [
                    (tantivy.Occur.Must, tantivy.Query.all_query()),
                    (tantivy.Occur.MustNot, range_query),
                ]
            )

        return range_query

    def _build_hybrid_query(
        self,
        query: str,
        index: tantivy.Index,
        schema: tantivy.Schema,
    ) -> tantivy.Query:
        """Build a hybrid query combining exact and fuzzy matching.

        Exact matches are boosted 5x to rank higher than fuzzy matches.
        This provides typo tolerance while favoring exact matches.
        """
        # Exact match query (boosted) - uses BM25 scoring
        exact_query = index.parse_query(query, ["title", "content"])
        boosted_exact = tantivy.Query.boost_query(exact_query, 5.0)

        # Fuzzy match queries for typo tolerance
        fuzzy_parts: list[tuple[tantivy.Occur, tantivy.Query]] = []
        for term in query.split():
            if not term:
                continue
            # Fuzzy query for title and content
            fuzzy_title = tantivy.Query.fuzzy_term_query(
                schema, "title", term, distance=1, prefix=True
            )
            fuzzy_content = tantivy.Query.fuzzy_term_query(
                schema, "content", term, distance=1, prefix=True
            )
            # Either field can match
            term_query = tantivy.Query.boolean_query(
                [
                    (tantivy.Occur.Should, fuzzy_title),
                    (tantivy.Occur.Should, fuzzy_content),
                ]
            )
            fuzzy_parts.append((tantivy.Occur.Must, term_query))

        # Combine: exact OR fuzzy (either can match, but exact scores higher)
        if fuzzy_parts:
            fuzzy_query = tantivy.Query.boolean_query(fuzzy_parts)
            return tantivy.Query.boolean_query(
                [
                    (tantivy.Occur.Should, boosted_exact),
                    (tantivy.Occur.Should, fuzzy_query),
                ]
            )
        else:
            return boosted_exact
