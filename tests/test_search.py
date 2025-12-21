"""Integration tests for SessionSearch.

These tests use real adapters and a real TantivyIndex to test
actual data flow through the search system.
"""

import json
from datetime import datetime

import pytest

from fast_resume.adapters.base import Session
from fast_resume.adapters.claude import ClaudeAdapter
from fast_resume.adapters.vibe import VibeAdapter
from fast_resume.index import TantivyIndex
from fast_resume.search import SessionSearch


@pytest.fixture
def search_env(temp_dir):
    """Set up a complete search environment with temp directories for all adapters."""
    # Create directories for each adapter
    # Claude expects CLAUDE_DIR/project-*/session.jsonl
    claude_base = temp_dir / "claude"
    claude_project = claude_base / "project-abc"
    vibe_dir = temp_dir / "vibe"

    claude_project.mkdir(parents=True)
    vibe_dir.mkdir(parents=True)

    # Create a Claude session file (JSONL format)
    claude_session = claude_project / "session-claude-001.jsonl"
    claude_data = [
        {
            "type": "user",
            "cwd": "/home/user/web-app",
            "message": {"content": "Help me fix the authentication bug"},
        },
        {
            "type": "assistant",
            "message": {
                "content": "I'll help you fix the auth bug. Let me look at the code."
            },
        },
        {"type": "user", "message": {"content": "It's in the login handler"}},
        {
            "type": "assistant",
            "message": {"content": "Found it - the token validation is wrong."},
        },
        {"type": "summary", "summary": "Fix authentication bug in login"},
    ]
    with open(claude_session, "w") as f:
        for entry in claude_data:
            f.write(json.dumps(entry) + "\n")

    # Create a Vibe session file (JSON format)
    vibe_session = vibe_dir / "session_vibe-001.json"
    vibe_data = {
        "metadata": {
            "session_id": "vibe-001",
            "start_time": "2025-01-10T14:00:00",
            "environment": {"working_directory": "/home/user/api-project"},
        },
        "messages": [
            {"role": "user", "content": "Create a REST API endpoint"},
            {"role": "assistant", "content": "I'll create the REST endpoint for you."},
            {"role": "user", "content": "Add rate limiting"},
            {"role": "assistant", "content": "Here's the rate limiting middleware."},
        ],
    }
    with open(vibe_session, "w") as f:
        json.dump(vibe_data, f)

    # Create index in temp dir
    index_dir = temp_dir / "index"

    return {
        "temp_dir": temp_dir,
        "claude_dir": claude_base,
        "vibe_dir": vibe_dir,
        "index_dir": index_dir,
        "claude_session": claude_session,
        "vibe_session": vibe_session,
    }


@pytest.fixture
def configured_search(search_env):
    """Create a SessionSearch with test-configured adapters."""
    search = SessionSearch()
    search.adapters = [
        ClaudeAdapter(sessions_dir=search_env["claude_dir"]),
        VibeAdapter(sessions_dir=search_env["vibe_dir"]),
    ]
    search._index = TantivyIndex(index_path=search_env["index_dir"])
    return search


class TestSessionDiscovery:
    """Tests for session discovery across adapters."""

    def test_discovers_sessions_from_multiple_adapters(self, configured_search):
        """Test that sessions are discovered from different adapter types."""
        sessions = configured_search.get_all_sessions()

        assert len(sessions) == 2

        agents = {s.agent for s in sessions}
        assert "claude" in agents
        assert "vibe" in agents

    def test_sessions_have_correct_metadata(self, configured_search):
        """Test that discovered sessions have correct metadata."""
        sessions = configured_search.get_all_sessions()

        claude_session = next(s for s in sessions if s.agent == "claude")
        assert claude_session.title == "Fix authentication bug in login"
        assert claude_session.directory == "/home/user/web-app"
        assert "authentication bug" in claude_session.content

        vibe_session = next(s for s in sessions if s.agent == "vibe")
        assert "REST API" in vibe_session.title
        assert vibe_session.directory == "/home/user/api-project"

    def test_sessions_sorted_by_timestamp_newest_first(self, configured_search):
        """Test that sessions are sorted by timestamp, newest first."""
        sessions = configured_search.get_all_sessions()

        timestamps = [s.timestamp for s in sessions]
        assert timestamps == sorted(timestamps, reverse=True)


class TestSearchFunctionality:
    """Tests for full-text search."""

    def test_search_finds_content_in_messages(self, configured_search):
        """Test that search finds content within session messages."""
        # First load sessions
        configured_search.get_all_sessions()

        # Search for term in Claude session
        results = configured_search.search("authentication")
        assert len(results) >= 1
        assert any(s.agent == "claude" for s in results)

    def test_search_finds_content_across_adapters(self, configured_search):
        """Test that search works across different adapter types."""
        configured_search.get_all_sessions()

        # Search for term in Vibe session
        results = configured_search.search("endpoint")
        assert len(results) >= 1
        assert any(s.agent == "vibe" for s in results)

    def test_empty_query_returns_all_sessions(self, configured_search):
        """Test that empty query returns all sessions."""
        configured_search.get_all_sessions()

        results = configured_search.search("")
        assert len(results) == 2

    def test_no_match_returns_empty(self, configured_search):
        """Test that non-matching query returns empty list."""
        configured_search.get_all_sessions()

        results = configured_search.search("xyznonexistent123")
        assert len(results) == 0


class TestFiltering:
    """Tests for session filtering."""

    def test_filter_by_agent(self, configured_search):
        """Test filtering sessions by agent type."""
        configured_search.get_all_sessions()

        claude_only = configured_search.search("", agent_filter="claude")
        assert len(claude_only) == 1
        assert claude_only[0].agent == "claude"

        vibe_only = configured_search.search("", agent_filter="vibe")
        assert len(vibe_only) == 1
        assert vibe_only[0].agent == "vibe"

    def test_filter_by_directory(self, configured_search):
        """Test filtering sessions by directory substring."""
        configured_search.get_all_sessions()

        results = configured_search.search("", directory_filter="web-app")
        assert len(results) == 1
        assert results[0].agent == "claude"

    def test_filter_by_directory_case_insensitive(self, configured_search):
        """Test that directory filter is case-insensitive."""
        configured_search.get_all_sessions()

        results = configured_search.search("", directory_filter="WEB-APP")
        assert len(results) == 1

    def test_combine_filters(self, configured_search):
        """Test combining agent and directory filters."""
        configured_search.get_all_sessions()

        # Filter that matches
        results = configured_search.search(
            "", agent_filter="claude", directory_filter="web"
        )
        assert len(results) == 1

        # Filter that doesn't match (wrong agent for directory)
        results = configured_search.search(
            "", agent_filter="vibe", directory_filter="web-app"
        )
        assert len(results) == 0

    def test_limit_parameter(self, configured_search):
        """Test that limit parameter restricts results."""
        configured_search.get_all_sessions()

        results = configured_search.search("", limit=1)
        assert len(results) == 1


class TestCaching:
    """Tests for session caching behavior."""

    def test_second_call_uses_cache(self, configured_search):
        """Test that second call returns cached sessions."""
        sessions1 = configured_search.get_all_sessions()
        sessions2 = configured_search.get_all_sessions()

        # Should be the same list object (cached)
        assert sessions1 is sessions2

    def test_force_refresh_bypasses_cache(self, configured_search):
        """Test that force_refresh=True reloads sessions."""
        sessions1 = configured_search.get_all_sessions()
        sessions2 = configured_search.get_all_sessions(force_refresh=True)

        # Should be different list objects
        assert sessions1 is not sessions2
        # But same content
        assert len(sessions1) == len(sessions2)


class TestResumeCommand:
    """Tests for resume command generation."""

    def test_get_resume_command_for_claude(self, configured_search):
        """Test that correct resume command is generated for Claude."""
        sessions = configured_search.get_all_sessions()
        claude_session = next(s for s in sessions if s.agent == "claude")

        cmd = configured_search.get_resume_command(claude_session)
        assert cmd[0] == "claude"
        assert "--resume" in cmd or "-c" in cmd

    def test_get_resume_command_for_vibe(self, configured_search):
        """Test that correct resume command is generated for Vibe."""
        sessions = configured_search.get_all_sessions()
        vibe_session = next(s for s in sessions if s.agent == "vibe")

        cmd = configured_search.get_resume_command(vibe_session)
        assert cmd[0] == "vibe"

    def test_get_adapter_for_session(self, configured_search):
        """Test that correct adapter is returned for session."""
        sessions = configured_search.get_all_sessions()

        for session in sessions:
            adapter = configured_search.get_adapter_for_session(session)
            assert adapter is not None
            assert adapter.name == session.agent


class TestIncrementalUpdates:
    """Tests for incremental update detection."""

    def test_detects_new_session(self, search_env, configured_search):
        """Test that new sessions are detected on refresh."""
        # Initial load
        sessions1 = configured_search.get_all_sessions()
        assert len(sessions1) == 2

        # Add a new Vibe session
        new_session = search_env["vibe_dir"] / "session_vibe-002.json"
        new_data = {
            "metadata": {
                "session_id": "vibe-002",
                "start_time": "2025-01-15T10:00:00",
                "environment": {"working_directory": "/home/user/new-project"},
            },
            "messages": [
                {"role": "user", "content": "New session content"},
                {"role": "assistant", "content": "Response here"},
            ],
        }
        with open(new_session, "w") as f:
            json.dump(new_data, f)

        # Force refresh should find new session
        sessions2 = configured_search.get_all_sessions(force_refresh=True)
        assert len(sessions2) == 3

    def test_session_count_from_index(self, configured_search):
        """Test that session count reflects indexed sessions."""
        configured_search.get_all_sessions()

        count = configured_search.get_session_count()
        assert count == 2


class TestEdgeCases:
    """Tests for edge cases."""

    def test_empty_index_returns_empty_list(self, temp_dir):
        """Test that empty directories return no sessions."""
        empty_dir = temp_dir / "empty"
        empty_dir.mkdir()

        search = SessionSearch()
        search.adapters = [
            ClaudeAdapter(sessions_dir=empty_dir),
            VibeAdapter(sessions_dir=empty_dir),
        ]
        search._index = TantivyIndex(index_path=temp_dir / "index")

        sessions = search.get_all_sessions()
        assert sessions == []

    def test_unknown_agent_returns_none(self, configured_search):
        """Test that unknown agent returns no adapter."""
        fake_session = Session(
            id="fake",
            agent="unknown-agent",
            title="Test",
            directory="/tmp",
            timestamp=datetime.now(),
            preview="",
            content="",
            message_count=0,
            mtime=0,
        )

        adapter = configured_search.get_adapter_for_session(fake_session)
        assert adapter is None

        cmd = configured_search.get_resume_command(fake_session)
        assert cmd == []
