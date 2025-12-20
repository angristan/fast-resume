"""Tantivy full-text search index for sessions."""

import shutil
from datetime import datetime
from pathlib import Path

import tantivy

from .adapters.base import Session
from .config import INDEX_DIR, MAX_PREVIEW_LENGTH, SCHEMA_VERSION

# Version file to detect schema changes
_VERSION_FILE = ".schema_version"


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
        # ID field - stored for retrieval
        schema_builder.add_text_field("id", stored=True)
        # Title - stored and indexed for search
        schema_builder.add_text_field("title", stored=True)
        # Directory - stored and indexed
        schema_builder.add_text_field("directory", stored=True)
        # Agent - stored for filtering
        schema_builder.add_text_field("agent", stored=True)
        # Content - stored and indexed for full-text search
        schema_builder.add_text_field("content", stored=True)
        # Timestamp - stored as float (Unix timestamp)
        schema_builder.add_float_field("timestamp", stored=True)
        # Message count - stored as integer
        schema_builder.add_integer_field("message_count", stored=True)
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
        """Get all session IDs with their timestamps and agents.

        Returns:
            Dict mapping session_id to (timestamp, agent) tuple.
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
            timestamp = doc.get_first("timestamp")
            agent = doc.get_first("agent")
            if session_id and timestamp is not None and agent:
                known[session_id] = (timestamp, agent)

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
                )
            )
        writer.commit()
        self._write_version()

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
