"""Integration tests for SessionSearch.

These tests use real adapters and a real TantivyIndex to test
actual data flow through the search system.
"""

import json
from datetime import datetime, timedelta

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
        # Title uses first user message (matches Claude Code's Resume Session UI)
        assert claude_session.title == "Help me fix the authentication bug"
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
            content="",
            message_count=0,
            mtime=0,
        )

        adapter = configured_search.get_adapter_for_session(fake_session)
        assert adapter is None

        cmd = configured_search.get_resume_command(fake_session)
        assert cmd == []


class TestTantivyFiltering:
    """Tests for Tantivy-based filtering (multi-value, negation, date ranges).

    These tests verify that all filtering is correctly pushed to Tantivy
    instead of being done via Python post-filtering.
    """

    @pytest.fixture
    def multi_session_index(self, temp_dir):
        """Create an index with multiple sessions for comprehensive filter testing."""
        index = TantivyIndex(index_path=temp_dir / "filter_test_index")
        now = datetime.now()

        sessions = [
            # Claude sessions
            Session(
                id="claude-1",
                agent="claude",
                title="Fix auth bug",
                directory="/home/user/web-app",
                timestamp=now - timedelta(hours=1),  # 1 hour ago
                content="authentication token validation",
                message_count=5,
                mtime=1000.0,
            ),
            Session(
                id="claude-2",
                agent="claude",
                title="Add tests",
                directory="/home/user/api-server",
                timestamp=now - timedelta(days=2),  # 2 days ago
                content="unit tests for api endpoints",
                message_count=3,
                mtime=1001.0,
            ),
            # Codex sessions
            Session(
                id="codex-1",
                agent="codex",
                title="Refactor code",
                directory="/home/user/web-app",
                timestamp=now - timedelta(hours=2),  # 2 hours ago
                content="refactoring the database layer",
                message_count=4,
                mtime=1002.0,
            ),
            Session(
                id="codex-2",
                agent="codex",
                title="Deploy script",
                directory="/home/user/devops",
                timestamp=now - timedelta(days=5),  # 5 days ago
                content="deployment automation script",
                message_count=2,
                mtime=1003.0,
            ),
            # Vibe session
            Session(
                id="vibe-1",
                agent="vibe",
                title="Create API",
                directory="/home/user/api-server",
                timestamp=now - timedelta(minutes=30),  # 30 min ago
                content="REST API endpoint creation",
                message_count=6,
                mtime=1004.0,
            ),
            # Copilot-vscode session (hyphenated agent name)
            Session(
                id="copilot-1",
                agent="copilot-vscode",
                title="Code completion",
                directory="/home/user/frontend",
                timestamp=now - timedelta(days=1),  # 1 day ago (yesterday)
                content="autocomplete suggestions",
                message_count=10,
                mtime=1005.0,
            ),
        ]
        index.add_sessions(sessions)
        return index

    # --- Multi-value Agent Filter Tests ---

    def test_multi_value_agent_filter_or(self, multi_session_index):
        """Test that agent:claude,codex matches both agents (OR logic)."""
        from fast_resume.query import Filter

        results = multi_session_index.search(
            "", agent_filter=Filter(include=["claude", "codex"])
        )
        assert len(results) == 4
        ids = {r[0] for r in results}
        assert ids == {"claude-1", "claude-2", "codex-1", "codex-2"}

    def test_single_agent_filter(self, multi_session_index):
        """Test single agent filter works correctly."""
        from fast_resume.query import Filter

        results = multi_session_index.search("", agent_filter=Filter(include=["vibe"]))
        assert len(results) == 1
        assert results[0][0] == "vibe-1"

    def test_hyphenated_agent_filter(self, multi_session_index):
        """Test filtering by hyphenated agent name like copilot-vscode."""
        from fast_resume.query import Filter

        results = multi_session_index.search(
            "", agent_filter=Filter(include=["copilot-vscode"])
        )
        assert len(results) == 1
        assert results[0][0] == "copilot-1"

    # --- Negated Agent Filter Tests ---

    def test_negated_agent_filter(self, multi_session_index):
        """Test that -agent:claude excludes claude sessions."""
        from fast_resume.query import Filter

        results = multi_session_index.search(
            "", agent_filter=Filter(exclude=["claude"])
        )
        assert len(results) == 4
        ids = {r[0] for r in results}
        assert "claude-1" not in ids
        assert "claude-2" not in ids

    def test_negated_multi_agent_filter(self, multi_session_index):
        """Test that -agent:claude,codex excludes both agents."""
        from fast_resume.query import Filter

        results = multi_session_index.search(
            "", agent_filter=Filter(exclude=["claude", "codex"])
        )
        assert len(results) == 2
        ids = {r[0] for r in results}
        assert ids == {"vibe-1", "copilot-1"}

    # --- Mixed Include/Exclude Agent Filter Tests ---

    def test_mixed_agent_filter(self, multi_session_index):
        """Test mixed include/exclude: agent:claude,codex,!codex -> only claude."""
        from fast_resume.query import Filter

        # Include claude and codex, but exclude codex -> only claude
        results = multi_session_index.search(
            "", agent_filter=Filter(include=["claude", "codex"], exclude=["codex"])
        )
        assert len(results) == 2
        ids = {r[0] for r in results}
        assert ids == {"claude-1", "claude-2"}

    # --- Directory Filter Tests ---

    def test_directory_substring_filter(self, multi_session_index):
        """Test directory substring matching."""
        from fast_resume.query import Filter

        results = multi_session_index.search(
            "", directory_filter=Filter(include=["web-app"])
        )
        assert len(results) == 2
        ids = {r[0] for r in results}
        assert ids == {"claude-1", "codex-1"}

    def test_directory_partial_match(self, multi_session_index):
        """Test that partial directory names match (substring)."""
        from fast_resume.query import Filter

        # "api" should match "api-server"
        results = multi_session_index.search(
            "", directory_filter=Filter(include=["api"])
        )
        assert len(results) == 2
        ids = {r[0] for r in results}
        assert ids == {"claude-2", "vibe-1"}

    def test_directory_case_insensitive(self, multi_session_index):
        """Test that directory filter is case-insensitive."""
        from fast_resume.query import Filter

        results = multi_session_index.search(
            "", directory_filter=Filter(include=["WEB-APP"])
        )
        assert len(results) == 2

    def test_negated_directory_filter(self, multi_session_index):
        """Test that negated directory filter excludes matching directories."""
        from fast_resume.query import Filter

        results = multi_session_index.search(
            "", directory_filter=Filter(exclude=["web-app"])
        )
        assert len(results) == 4
        ids = {r[0] for r in results}
        assert "claude-1" not in ids
        assert "codex-1" not in ids

    # --- Date Filter Tests ---

    def test_date_less_than_filter(self, multi_session_index):
        """Test date:<3h returns sessions within last 3 hours."""
        from fast_resume.query import DateFilter, DateOp

        now = datetime.now()
        cutoff = now - timedelta(hours=3)

        results = multi_session_index.search(
            "",
            date_filter=DateFilter(
                op=DateOp.LESS_THAN, value="<3h", cutoff=cutoff, negated=False
            ),
        )
        # Should match: claude-1 (1h), codex-1 (2h), vibe-1 (30m)
        assert len(results) == 3
        ids = {r[0] for r in results}
        assert ids == {"claude-1", "codex-1", "vibe-1"}

    def test_date_greater_than_filter(self, multi_session_index):
        """Test date:>3d returns sessions older than 3 days."""
        from fast_resume.query import DateFilter, DateOp

        now = datetime.now()
        cutoff = now - timedelta(days=3)

        results = multi_session_index.search(
            "",
            date_filter=DateFilter(
                op=DateOp.GREATER_THAN, value=">3d", cutoff=cutoff, negated=False
            ),
        )
        # Should match: codex-2 (5 days ago)
        assert len(results) == 1
        assert results[0][0] == "codex-2"

    def test_date_today_filter(self, multi_session_index):
        """Test date:today returns sessions from today."""
        from fast_resume.query import DateFilter, DateOp

        today_start = datetime.now().replace(hour=0, minute=0, second=0, microsecond=0)

        results = multi_session_index.search(
            "",
            date_filter=DateFilter(
                op=DateOp.EXACT, value="today", cutoff=today_start, negated=False
            ),
        )
        # Should match sessions from today: claude-1, codex-1, vibe-1
        assert len(results) >= 1  # At least the recent ones

    def test_negated_date_filter(self, multi_session_index):
        """Test date:!today excludes today's sessions."""
        from fast_resume.query import DateFilter, DateOp

        today_start = datetime.now().replace(hour=0, minute=0, second=0, microsecond=0)

        results = multi_session_index.search(
            "",
            date_filter=DateFilter(
                op=DateOp.EXACT, value="today", cutoff=today_start, negated=True
            ),
        )
        # Should exclude today's sessions
        ids = {r[0] for r in results}
        # Sessions from before today: claude-2 (2d), codex-2 (5d), copilot-1 (1d)
        assert "claude-2" in ids
        assert "codex-2" in ids

    # --- Combined Filter Tests ---

    def test_combined_agent_and_directory(self, multi_session_index):
        """Test combining agent and directory filters."""
        from fast_resume.query import Filter

        results = multi_session_index.search(
            "",
            agent_filter=Filter(include=["claude"]),
            directory_filter=Filter(include=["web-app"]),
        )
        assert len(results) == 1
        assert results[0][0] == "claude-1"

    def test_combined_agent_directory_date(self, multi_session_index):
        """Test combining agent, directory, and date filters."""
        from fast_resume.query import DateFilter, DateOp, Filter

        now = datetime.now()
        cutoff = now - timedelta(hours=5)

        results = multi_session_index.search(
            "",
            agent_filter=Filter(include=["claude", "codex"]),
            directory_filter=Filter(include=["web"]),
            date_filter=DateFilter(
                op=DateOp.LESS_THAN, value="<5h", cutoff=cutoff, negated=False
            ),
        )
        # claude-1 and codex-1 both match web-app and are within 5h
        assert len(results) == 2
        ids = {r[0] for r in results}
        assert ids == {"claude-1", "codex-1"}

    def test_text_search_with_filters(self, multi_session_index):
        """Test combining text search with filters."""
        from fast_resume.query import Filter

        results = multi_session_index.search(
            "authentication",
            agent_filter=Filter(include=["claude"]),
        )
        assert len(results) == 1
        assert results[0][0] == "claude-1"

    def test_no_results_with_conflicting_filters(self, multi_session_index):
        """Test that conflicting filters return no results."""
        from fast_resume.query import Filter

        # vibe agent but web-app directory (no vibe session in web-app)
        results = multi_session_index.search(
            "",
            agent_filter=Filter(include=["vibe"]),
            directory_filter=Filter(include=["web-app"]),
        )
        assert len(results) == 0

    # --- Edge Cases ---

    def test_empty_query_returns_all(self, multi_session_index):
        """Test that empty query with no filters returns all sessions."""
        results = multi_session_index.search("")
        assert len(results) == 6
