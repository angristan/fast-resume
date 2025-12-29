"""End-to-end integration tests: TUI + Search + Index with real adapters.

These tests verify the complete data flow from session files on disk
through the Tantivy index and search engine to the TUI display.
Unlike the unit tests, these do NOT mock the search engine.
"""

import json
from datetime import datetime, timedelta

import pytest

from fast_resume.adapters.claude import ClaudeAdapter
from fast_resume.adapters.vibe import VibeAdapter
from fast_resume.index import TantivyIndex
from fast_resume.search import SessionSearch
from fast_resume.tui import FastResumeApp


async def wait_for_search_complete(app, pilot, timeout: float = 1.0):
    """Wait for the search to complete by checking is_loading flag."""
    elapsed = 0.0
    while app.is_loading and elapsed < timeout:
        await pilot.pause()
        elapsed += 0.05
    # Extra pause to ensure UI updates are processed
    await pilot.pause()


@pytest.fixture
def integration_env(temp_dir):
    """Set up a complete integration test environment with real session files.

    Creates realistic session data for multiple adapters that will be
    parsed, indexed, and displayed in the TUI.
    """
    # Create directories for each adapter
    claude_base = temp_dir / "claude"
    claude_project = claude_base / "project-webapp"
    vibe_dir = temp_dir / "vibe"
    index_dir = temp_dir / "index"

    claude_project.mkdir(parents=True)
    vibe_dir.mkdir(parents=True)

    # Claude session 1: Authentication bug fix
    claude_session_1 = claude_project / "session-auth-fix.jsonl"
    claude_data_1 = [
        {
            "type": "user",
            "cwd": "/home/user/webapp",
            "message": {"content": "Help me fix the authentication bug in login.py"},
        },
        {
            "type": "assistant",
            "message": {
                "content": "I'll analyze the authentication flow and find the bug."
            },
        },
        {"type": "user", "message": {"content": "The JWT token expires too quickly"}},
        {
            "type": "assistant",
            "message": {
                "content": "Found it - the expiry is set to 60 seconds instead of 3600."
            },
        },
        {"type": "summary", "summary": "Fix JWT token expiration bug"},
    ]
    with open(claude_session_1, "w") as f:
        for entry in claude_data_1:
            f.write(json.dumps(entry) + "\n")

    # Claude session 2: Database optimization
    claude_session_2 = claude_project / "session-db-optimize.jsonl"
    claude_data_2 = [
        {
            "type": "user",
            "cwd": "/home/user/webapp",
            "message": {"content": "The database queries are slow"},
        },
        {
            "type": "assistant",
            "message": {"content": "Let me check the query execution plans."},
        },
        {"type": "user", "message": {"content": "Especially the user search"}},
        {
            "type": "assistant",
            "message": {
                "content": "Adding an index on email will speed this up significantly."
            },
        },
        {"type": "summary", "summary": "Optimize database queries with indexes"},
    ]
    with open(claude_session_2, "w") as f:
        for entry in claude_data_2:
            f.write(json.dumps(entry) + "\n")

    # Vibe session: REST API development
    vibe_session = vibe_dir / "session_api-dev.json"
    vibe_data = {
        "metadata": {
            "session_id": "api-dev-001",
            "start_time": (datetime.now() - timedelta(hours=3)).isoformat(),
            "environment": {"working_directory": "/home/user/api-project"},
        },
        "messages": [
            {
                "role": "user",
                "content": "Create a REST API endpoint for user registration",
            },
            {"role": "assistant", "content": "I'll create a POST /api/users endpoint."},
            {"role": "user", "content": "Add input validation"},
            {
                "role": "assistant",
                "content": "Added email and password validation with proper error responses.",
            },
        ],
    }
    with open(vibe_session, "w") as f:
        json.dump(vibe_data, f)

    return {
        "temp_dir": temp_dir,
        "claude_dir": claude_base,
        "vibe_dir": vibe_dir,
        "index_dir": index_dir,
    }


@pytest.fixture
def real_search_engine(integration_env):
    """Create a real SessionSearch with test-configured adapters.

    This is NOT mocked - it uses real adapters and a real Tantivy index.
    The index is pre-populated so TUI loads synchronously (fast path).
    """
    search = SessionSearch()
    search.adapters = [
        ClaudeAdapter(sessions_dir=integration_env["claude_dir"]),
        VibeAdapter(sessions_dir=integration_env["vibe_dir"]),
    ]
    search._index = TantivyIndex(index_path=integration_env["index_dir"])

    # Pre-populate the index so TUI uses sync loading path
    # This triggers adapter scanning and index population
    sessions = search.get_all_sessions()
    assert len(sessions) == 3, f"Expected 3 sessions, got {len(sessions)}"

    return search


@pytest.fixture
def integration_app(real_search_engine):
    """Create a TUI app with a real (non-mocked) search engine."""
    app = FastResumeApp()
    # Replace the search engine with our test-configured one
    app.search_engine = real_search_engine
    return app


class TestEndToEndDataFlow:
    """Tests verifying complete data flow from files to TUI display."""

    @pytest.mark.asyncio
    async def test_real_sessions_appear_in_table(self, integration_app):
        """Sessions from disk are parsed, indexed, and displayed in TUI table."""
        async with integration_app.run_test(size=(120, 40)) as pilot:
            await pilot.pause()

            table = integration_app.query_one("#results-table")
            # Should have 3 sessions: 2 Claude + 1 Vibe
            assert table.row_count == 3

            # Verify sessions are loaded
            assert len(integration_app.sessions) == 3

            # Verify different agents are present
            agents = {s.agent for s in integration_app.sessions}
            assert "claude" in agents
            assert "vibe" in agents

    @pytest.mark.asyncio
    async def test_session_content_is_searchable(self, integration_app):
        """Content from session files is indexed and searchable."""
        async with integration_app.run_test(size=(120, 40)) as pilot:
            await pilot.pause()

            # Search for "JWT" which appears in the auth session
            search_input = integration_app.query_one("#search-input")
            search_input.value = "JWT"
            await pilot.pause()

            # Allow search debounce
            await pilot.pause()
            await pilot.pause()

            table = integration_app.query_one("#results-table")
            # Should find the auth session
            assert table.row_count >= 1

            # The result should be from the JWT session
            if integration_app._displayed_sessions:
                assert any(
                    "JWT" in s.content or "authentication" in s.content.lower()
                    for s in integration_app._displayed_sessions
                )

    @pytest.mark.asyncio
    async def test_search_query_filters_real_data(self, integration_app):
        """Typing a search query correctly filters indexed sessions."""
        async with integration_app.run_test(size=(120, 40)) as pilot:
            await wait_for_search_complete(integration_app, pilot)

            # Initially should have all 3 sessions
            table = integration_app.query_one("#results-table")
            assert table.row_count == 3

            # Search for "database" - should only match the db optimization session
            # Set search_query directly to bypass debounce
            integration_app.search_query = "database"
            await wait_for_search_complete(integration_app, pilot)

            # Should have fewer results - only the db optimization session
            assert table.row_count >= 1
            # All results should contain the search term
            for session in integration_app._displayed_sessions:
                assert (
                    "database" in session.content.lower()
                    or "database" in session.title.lower()
                )

    @pytest.mark.asyncio
    async def test_agent_filter_with_real_data(self, integration_app):
        """Agent filter correctly filters real adapter data."""
        async with integration_app.run_test(size=(120, 40)) as pilot:
            await pilot.pause()

            table = integration_app.query_one("#results-table")
            initial_count = table.row_count
            assert initial_count == 3

            # Filter by Claude only
            integration_app._set_filter("claude")
            await pilot.pause()

            # Should show only Claude sessions (2)
            assert table.row_count == 2
            for session in integration_app._displayed_sessions:
                assert session.agent == "claude"

            # Filter by Vibe only
            integration_app._set_filter("vibe")
            await pilot.pause()

            # Should show only Vibe session (1)
            assert table.row_count == 1
            assert integration_app._displayed_sessions[0].agent == "vibe"

            # Clear filter - should show all again
            integration_app._set_filter(None)
            await pilot.pause()
            assert table.row_count == 3

    @pytest.mark.asyncio
    async def test_combined_search_and_filter(self, integration_app):
        """Search query and agent filter work together correctly."""
        async with integration_app.run_test(size=(120, 40)) as pilot:
            await pilot.pause()

            # Set agent filter to Claude
            integration_app._set_filter("claude")
            await pilot.pause()

            # Search within Claude sessions for a term unique to one session
            search_input = integration_app.query_one("#search-input")
            search_input.value = "JWT token"
            # Wait for search debounce
            await pilot.pause()
            await pilot.pause()
            await pilot.pause()

            table = integration_app.query_one("#results-table")
            # Should find the auth session (Claude only, contains "JWT token")
            assert table.row_count >= 1
            for session in integration_app._displayed_sessions:
                assert session.agent == "claude"
                # The JWT token session should be about authentication
                assert (
                    "jwt" in session.content.lower()
                    or "authentication" in session.content.lower()
                )

    @pytest.mark.asyncio
    async def test_resume_command_from_real_session(self, integration_app):
        """Resume generates correct command from real session data."""
        async with integration_app.run_test(size=(120, 40)) as pilot:
            await pilot.pause()

            # Get the first selected session before resuming
            first_session = integration_app.selected_session
            assert first_session is not None

            # Press Enter to resume (triggers the yolo modal since real adapters support it)
            await pilot.press("enter")
            await pilot.pause()

            # App should still be running (modal is shown)
            assert integration_app.is_running

            # Press 'n' to select normal mode (no yolo)
            await pilot.press("n")
            await pilot.pause()

            # App should have exited after modal selection
            assert not integration_app.is_running

            # Check that resume command was generated
            cmd = integration_app.get_resume_command()
            assert cmd is not None
            assert len(cmd) > 0

            # Command should start with the correct agent binary
            assert cmd[0] == first_session.agent

    @pytest.mark.asyncio
    async def test_session_metadata_displayed_correctly(self, integration_app):
        """Session metadata (title, directory, time) is displayed correctly."""
        async with integration_app.run_test(size=(120, 40)) as pilot:
            await pilot.pause()

            # Verify sessions have correct metadata
            for session in integration_app.sessions:
                assert session.title  # Has a title
                assert session.directory  # Has a directory
                assert session.timestamp  # Has a timestamp
                assert session.message_count > 0  # Has messages

            # Verify a Claude session has expected content
            claude_sessions = [
                s for s in integration_app.sessions if s.agent == "claude"
            ]
            assert len(claude_sessions) == 2

            # One should be about JWT/auth
            titles = [s.title for s in claude_sessions]
            assert any(
                "JWT" in t or "expiration" in t or "bug" in t.lower() for t in titles
            )


class TestEmptyAndEdgeCases:
    """Tests for empty results and edge cases."""

    @pytest.mark.asyncio
    async def test_no_match_shows_empty_table(self, integration_app):
        """Search with no matches shows empty state gracefully."""
        async with integration_app.run_test(size=(120, 40)) as pilot:
            await wait_for_search_complete(integration_app, pilot)

            # Set search query directly (bypasses debounce timer)
            # First set is_loading to True to indicate search is starting
            integration_app.is_loading = True
            integration_app.search_query = "xyznonexistentterm123"
            await wait_for_search_complete(integration_app, pilot)

            # No actual sessions should be displayed
            assert len(integration_app._displayed_sessions) == 0

            # Table shows 1 row with "No sessions found" message
            table = integration_app.query_one("#results-table")
            assert table.row_count == 1  # Empty state row

            # App should still be running and responsive
            assert integration_app.is_running

    @pytest.mark.asyncio
    async def test_empty_search_returns_all(self, integration_app):
        """Clearing search input returns all sessions."""
        async with integration_app.run_test(size=(120, 40)) as pilot:
            await pilot.pause()

            search_input = integration_app.query_one("#search-input")
            table = integration_app.query_one("#results-table")

            # Should start with all 3 sessions
            initial_count = table.row_count
            assert initial_count == 3

            # Search for something specific that only matches one session
            search_input.value = "REST API endpoint"
            # Wait longer for search debounce and background worker
            await pilot.pause()
            await pilot.pause()
            await pilot.pause()
            await pilot.pause()

            filtered_count = table.row_count
            # Should have fewer results (only Vibe session has "REST API endpoint")
            assert filtered_count < initial_count

            # Clear search
            search_input.value = ""
            # Wait longer for search debounce and background worker
            await pilot.pause()
            await pilot.pause()
            await pilot.pause()
            await pilot.pause()

            # Should show all sessions again
            assert table.row_count == 3


class TestNavigationWithRealData:
    """Tests for navigation and selection with real data."""

    @pytest.mark.asyncio
    async def test_navigate_between_sessions(self, integration_app):
        """Keyboard navigation works with real session data."""
        async with integration_app.run_test(size=(120, 40)) as pilot:
            await pilot.pause()

            # Focus table
            table = integration_app.query_one("#results-table")
            table.focus()
            await pilot.pause()

            # Get first selected session
            first_session = integration_app.selected_session
            assert first_session is not None

            # Navigate down
            await pilot.press("j")
            await pilot.pause()

            # Should have different session selected
            second_session = integration_app.selected_session
            assert second_session is not None
            assert second_session.id != first_session.id

            # Navigate back up
            await pilot.press("k")
            await pilot.pause()

            # Should be back to first session
            assert integration_app.selected_session.id == first_session.id

    @pytest.mark.asyncio
    async def test_preview_shows_real_content(self, integration_app):
        """Preview pane shows actual session content."""
        from fast_resume.tui.preview import SessionPreview

        async with integration_app.run_test(size=(120, 40)) as pilot:
            await pilot.pause()

            # Ensure preview is visible
            assert integration_app.show_preview is True

            # Get the preview widget
            preview = integration_app.query_one(SessionPreview)
            assert preview is not None

            # Selected session should have content
            selected = integration_app.selected_session
            assert selected is not None
            assert len(selected.content) > 0


class TestSessionCountAccuracy:
    """Tests for session count display accuracy."""

    @pytest.mark.asyncio
    async def test_session_count_matches_actual(self, integration_app):
        """Session count label reflects actual indexed sessions."""
        async with integration_app.run_test(size=(120, 40)) as pilot:
            await pilot.pause()

            from textual.widgets import Label

            count_label = integration_app.query_one("#session-count", Label)
            label_text = str(count_label.render())

            # Should show 3 (total sessions)
            assert "3" in label_text

    @pytest.mark.asyncio
    async def test_session_count_updates_with_filter(self, integration_app):
        """Session count updates when filter is applied."""
        async with integration_app.run_test(size=(120, 40)) as pilot:
            await pilot.pause()

            from textual.widgets import Label

            count_label = integration_app.query_one("#session-count", Label)

            # Filter to Claude only (2 sessions)
            integration_app._set_filter("claude")
            await pilot.pause()

            label_text = str(count_label.render())
            assert "2" in label_text

            # Filter to Vibe only (1 session)
            integration_app._set_filter("vibe")
            await pilot.pause()

            label_text = str(count_label.render())
            assert "1" in label_text
