"""Tantivy full-text search index for sessions."""

import shutil
from pathlib import Path

import tantivy

from .adapters.base import Session
from .config import TANTIVY_INDEX_DIR


class TantivyIndex:
    """Manages a Tantivy full-text search index for sessions."""

    def __init__(self, index_path: Path = TANTIVY_INDEX_DIR) -> None:
        self.index_path = index_path
        self._index: tantivy.Index | None = None
        self._schema: tantivy.Schema | None = None
        self._cache_key_file = index_path / ".cache_key"

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
        # Content - indexed but not stored (save space)
        schema_builder.add_text_field("content", stored=False)
        return schema_builder.build()

    def _ensure_index(self) -> tantivy.Index:
        """Ensure the index is loaded or created."""
        if self._index is not None:
            return self._index

        self._schema = self._build_schema()

        if self.index_path.exists():
            # Open existing index
            self._index = tantivy.Index(self._schema, path=str(self.index_path))
        else:
            # Create new index
            self.index_path.mkdir(parents=True, exist_ok=True)
            self._index = tantivy.Index(self._schema, path=str(self.index_path))

        return self._index

    def get_stored_cache_key(self) -> str | None:
        """Get the cache key stored with the index."""
        if not self._cache_key_file.exists():
            return None
        try:
            return self._cache_key_file.read_text().strip()
        except Exception:
            return None

    def needs_rebuild(self, cache_key: str) -> bool:
        """Check if the index needs to be rebuilt."""
        stored_key = self.get_stored_cache_key()
        return stored_key != cache_key

    def clear(self) -> None:
        """Clear the index directory."""
        self._index = None
        if self.index_path.exists():
            shutil.rmtree(self.index_path)

    def build_index(self, sessions: list[Session], cache_key: str) -> None:
        """Build the index from sessions."""
        # Clear existing index
        self.clear()

        # Create fresh index
        self.index_path.mkdir(parents=True, exist_ok=True)
        self._schema = self._build_schema()
        self._index = tantivy.Index(self._schema, path=str(self.index_path))

        # Index all sessions
        writer = self._index.writer()
        for session in sessions:
            writer.add_document(
                tantivy.Document(
                    id=session.id,
                    title=session.title,
                    directory=session.directory,
                    agent=session.agent,
                    content=session.content,
                )
            )
        writer.commit()

        # Store cache key
        self._cache_key_file.write_text(cache_key)

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
        index.reload()  # Ensure we have latest data
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
