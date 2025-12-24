"""Tests for Tantivy index."""

from datetime import datetime

import pytest

from fast_resume.adapters.base import Session
from fast_resume.index import TantivyIndex


@pytest.fixture
def index(temp_dir):
    """Create a TantivyIndex instance with temp directory."""
    return TantivyIndex(index_path=temp_dir / "tantivy_index")


@pytest.fixture
def sample_session():
    """Create a sample session for testing."""
    return Session(
        id="session-123",
        agent="claude",
        title="Test session",
        directory="/home/user/project",
        timestamp=datetime(2024, 1, 15, 10, 30, 0),
        preview="Help me fix this bug",
        content="Help me fix this bug in the login system",
        message_count=4,
        mtime=1705312200.0,
    )


@pytest.fixture
def sample_session_updated():
    """Create an updated version of the sample session."""
    return Session(
        id="session-123",  # Same ID
        agent="claude",
        title="Test session - updated",
        directory="/home/user/project",
        timestamp=datetime(2024, 1, 15, 10, 30, 0),
        preview="Help me fix this bug - with more context",
        content="Help me fix this bug in the login system - with more context added",
        message_count=6,
        mtime=1705312300.0,
    )


class TestTantivyIndex:
    """Tests for TantivyIndex."""

    def test_add_and_retrieve_sessions(self, index, sample_session):
        """Test adding and retrieving sessions."""
        index.add_sessions([sample_session])

        sessions = index.get_all_sessions()
        assert len(sessions) == 1
        assert sessions[0].id == "session-123"
        assert sessions[0].title == "Test session"
        assert sessions[0].agent == "claude"

    def test_add_sessions_empty_list(self, index):
        """Test adding empty list does nothing."""
        index.add_sessions([])
        assert index.get_session_count() == 0

    def test_delete_sessions(self, index, sample_session):
        """Test deleting sessions by ID."""
        index.add_sessions([sample_session])
        assert index.get_session_count() == 1

        index.delete_sessions(["session-123"])
        assert index.get_session_count() == 0

    def test_delete_sessions_empty_list(self, index, sample_session):
        """Test deleting empty list does nothing."""
        index.add_sessions([sample_session])
        index.delete_sessions([])
        assert index.get_session_count() == 1

    def test_update_sessions_replaces_existing(
        self, index, sample_session, sample_session_updated
    ):
        """Test that update_sessions replaces existing session without duplicates."""
        # Add initial session
        index.add_sessions([sample_session])
        assert index.get_session_count() == 1

        # Update the session
        index.update_sessions([sample_session_updated])

        # Should still have only 1 session
        sessions = index.get_all_sessions()
        assert len(sessions) == 1
        assert sessions[0].id == "session-123"
        assert sessions[0].title == "Test session - updated"
        assert sessions[0].message_count == 6

    def test_update_sessions_no_duplicates_after_multiple_updates(
        self, index, sample_session
    ):
        """Test that multiple updates don't create duplicates."""
        index.add_sessions([sample_session])

        # Update multiple times
        for i in range(5):
            updated = Session(
                id="session-123",
                agent="claude",
                title=f"Update {i}",
                directory="/home/user/project",
                timestamp=datetime(2024, 1, 15, 10, 30, 0),
                preview="Updated content",
                content=f"Content version {i}",
                message_count=i + 1,
                mtime=1705312200.0 + i,
            )
            index.update_sessions([updated])

        # Should still have exactly 1 session
        sessions = index.get_all_sessions()
        assert len(sessions) == 1
        assert sessions[0].title == "Update 4"
        assert sessions[0].message_count == 5

    def test_update_sessions_adds_new_if_not_exists(self, index, sample_session):
        """Test that update_sessions adds session if it doesn't exist."""
        assert index.get_session_count() == 0

        index.update_sessions([sample_session])

        sessions = index.get_all_sessions()
        assert len(sessions) == 1
        assert sessions[0].id == "session-123"

    def test_update_sessions_empty_list(self, index, sample_session):
        """Test updating empty list does nothing."""
        index.add_sessions([sample_session])
        index.update_sessions([])
        assert index.get_session_count() == 1

    def test_update_multiple_sessions_atomically(self, index):
        """Test updating multiple sessions in a single call."""
        sessions = [
            Session(
                id=f"session-{i}",
                agent="claude",
                title=f"Session {i}",
                directory="/home/user/project",
                timestamp=datetime(2024, 1, 15, 10, 30, 0),
                preview=f"Content {i}",
                content=f"Full content {i}",
                message_count=i,
                mtime=1705312200.0,
            )
            for i in range(3)
        ]

        # Add initial sessions
        index.add_sessions(sessions)
        assert index.get_session_count() == 3

        # Update all sessions at once
        updated_sessions = [
            Session(
                id=f"session-{i}",
                agent="claude",
                title=f"Updated Session {i}",
                directory="/home/user/project",
                timestamp=datetime(2024, 1, 15, 10, 30, 0),
                preview=f"Updated content {i}",
                content=f"Updated full content {i}",
                message_count=i + 10,
                mtime=1705312300.0,
            )
            for i in range(3)
        ]
        index.update_sessions(updated_sessions)

        # Should still have exactly 3 sessions
        result = index.get_all_sessions()
        assert len(result) == 3

        # All should be updated
        for session in result:
            assert session.title.startswith("Updated Session")
            assert session.message_count >= 10

    def test_search_after_update(self, index, sample_session, sample_session_updated):
        """Test that search works correctly after updates."""
        index.add_sessions([sample_session])

        # Search for original content
        results = index.search("login")
        assert len(results) == 1

        # Update with new content
        index.update_sessions([sample_session_updated])

        # Search should still work and find the updated session
        results = index.search("context")
        assert len(results) == 1
        assert results[0][0] == "session-123"

    def test_search_with_hyphenated_agent_filter(self, index):
        """Test that agent filter works with hyphenated agent names like copilot-vscode.

        This is a regression test for a bug where hyphenated agent names
        were tokenized by the default tokenizer, splitting "copilot-vscode"
        into ["copilot", "vscode"] tokens. This caused term_query to fail
        when filtering by agent name during search.
        """
        sessions = [
            Session(
                id="session-vscode-1",
                agent="copilot-vscode",
                title="Code review feedback",
                directory="/project",
                timestamp=datetime(2024, 1, 15, 10, 30, 0),
                preview="Review my code",
                content="Please review my code for bugs",
                message_count=2,
                mtime=1705312200.0,
            ),
            Session(
                id="session-claude-1",
                agent="claude",
                title="Code review session",
                directory="/project",
                timestamp=datetime(2024, 1, 15, 11, 30, 0),
                preview="Another review",
                content="Code review for authentication module",
                message_count=3,
                mtime=1705315800.0,
            ),
        ]
        index.add_sessions(sessions)

        # Search with query + hyphenated agent filter should work
        results = index.search("review", agent_filter="copilot-vscode")
        assert len(results) == 1
        assert results[0][0] == "session-vscode-1"

        # Verify other agent still works too
        results = index.search("review", agent_filter="claude")
        assert len(results) == 1
        assert results[0][0] == "session-claude-1"

    def test_get_session_count(self, index):
        """Test getting session count."""
        assert index.get_session_count() == 0

        sessions = [
            Session(
                id=f"session-{i}",
                agent="claude",
                title=f"Session {i}",
                directory="/project",
                timestamp=datetime(2024, 1, 15, 10, 30, 0),
                preview="Content",
                content="Full content",
                message_count=1,
                mtime=1705312200.0,
            )
            for i in range(5)
        ]
        index.add_sessions(sessions)

        assert index.get_session_count() == 5

    def test_get_session_count_with_agent_filter(self, index):
        """Test getting session count filtered by agent.

        This is a regression test for a bug where limit=0 was passed to
        Tantivy search, which panics because limit must be > 0.
        """
        sessions = [
            Session(
                id="session-claude-1",
                agent="claude",
                title="Claude session 1",
                directory="/project",
                timestamp=datetime(2024, 1, 15, 10, 30, 0),
                preview="Content",
                content="Full content",
                message_count=1,
                mtime=1705312200.0,
            ),
            Session(
                id="session-claude-2",
                agent="claude",
                title="Claude session 2",
                directory="/project",
                timestamp=datetime(2024, 1, 15, 11, 30, 0),
                preview="Content",
                content="Full content",
                message_count=1,
                mtime=1705315800.0,
            ),
            Session(
                id="session-vscode-1",
                agent="copilot-vscode",
                title="VSCode session",
                directory="/project",
                timestamp=datetime(2024, 1, 15, 12, 30, 0),
                preview="Content",
                content="Full content",
                message_count=1,
                mtime=1705319400.0,
            ),
        ]
        index.add_sessions(sessions)

        # Total count
        assert index.get_session_count() == 3

        # Filtered counts
        assert index.get_session_count(agent_filter="claude") == 2
        assert index.get_session_count(agent_filter="copilot-vscode") == 1
        assert index.get_session_count(agent_filter="nonexistent") == 0
