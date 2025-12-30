"""Tests for TUI utility functions."""

import os
from datetime import datetime, timedelta
from unittest.mock import MagicMock, patch

import pytest
from rich.text import Text

from fast_resume.adapters.base import Session
from fast_resume.tui import (
    FastResumeApp,
    KeywordSuggester,
    format_directory,
    format_time_ago,
    get_agent_icon,
    highlight_matches,
    _icon_cache,
)


class TestFormatDirectory:
    """Tests for format_directory function."""

    def test_replaces_home_with_tilde(self):
        """Test that home directory is replaced with ~."""
        home = os.path.expanduser("~")
        path = f"{home}/projects/myapp"
        result = format_directory(path)
        assert result == "~/projects/myapp"

    def test_leaves_non_home_paths_unchanged(self):
        """Test that paths outside home are unchanged."""
        path = "/var/log/myapp"
        result = format_directory(path)
        assert result == "/var/log/myapp"

    def test_handles_home_directory_exactly(self):
        """Test that home directory itself is replaced."""
        home = os.path.expanduser("~")
        result = format_directory(home)
        assert result == "~"

    def test_handles_empty_string(self):
        """Test that empty string returns n/a."""
        result = format_directory("")
        assert result == "n/a"

    def test_handles_root_path(self):
        """Test that root path is unchanged."""
        result = format_directory("/")
        assert result == "/"

    def test_handles_relative_path(self):
        """Test that relative paths are unchanged."""
        result = format_directory("relative/path")
        assert result == "relative/path"


class TestFormatTimeAgo:
    """Tests for format_time_ago function."""

    def test_formats_recent_time(self):
        """Test formatting of recent timestamps."""
        now = datetime.now()
        result = format_time_ago(now)
        # humanize.naturaltime returns something like "now" or "just now"
        assert "now" in result.lower() or "second" in result.lower()

    def test_formats_hours_ago(self):
        """Test formatting of timestamps hours ago."""
        dt = datetime.now() - timedelta(hours=2)
        result = format_time_ago(dt)
        assert "hour" in result.lower()

    def test_formats_days_ago(self):
        """Test formatting of timestamps days ago."""
        dt = datetime.now() - timedelta(days=3)
        result = format_time_ago(dt)
        assert "day" in result.lower()

    def test_formats_weeks_ago(self):
        """Test formatting of timestamps weeks ago."""
        dt = datetime.now() - timedelta(weeks=2)
        result = format_time_ago(dt)
        # Could be "2 weeks ago" or "14 days ago"
        assert "week" in result.lower() or "day" in result.lower()


class TestHighlightMatches:
    """Tests for highlight_matches function."""

    def test_single_term_highlighting(self):
        """Test highlighting a single term."""
        result = highlight_matches("Hello world", "world")
        assert isinstance(result, Text)
        assert str(result) == "Hello world"
        # Check that styling was applied
        spans = list(result._spans)
        assert len(spans) > 0

    def test_multiple_terms(self):
        """Test highlighting multiple search terms."""
        result = highlight_matches("Hello world test", "hello test")
        assert isinstance(result, Text)
        # Both terms should have spans
        spans = list(result._spans)
        assert len(spans) >= 2

    def test_case_insensitive_matching(self):
        """Test that matching is case-insensitive."""
        result = highlight_matches("Hello WORLD", "world")
        spans = list(result._spans)
        assert len(spans) > 0
        # Check span covers the right position (6-11 for "WORLD")
        span = spans[0]
        assert span.start == 6
        assert span.end == 11

    def test_truncation_with_max_len(self):
        """Test that text is truncated with max_len."""
        long_text = "This is a very long text that should be truncated"
        result = highlight_matches(long_text, "", max_len=20)
        assert str(result) == "This is a very lo..."
        assert len(str(result)) == 20

    def test_empty_query_returns_plain_text(self):
        """Test that empty query returns text without highlighting."""
        result = highlight_matches("Hello world", "")
        assert isinstance(result, Text)
        assert str(result) == "Hello world"
        # No spans should be added for empty query
        spans = list(result._spans)
        assert len(spans) == 0

    def test_multiple_occurrences_of_same_term(self):
        """Test highlighting multiple occurrences of the same term."""
        result = highlight_matches("test test test", "test")
        spans = list(result._spans)
        # Should have 3 spans for 3 occurrences
        assert len(spans) == 3

    def test_no_match_returns_plain_text(self):
        """Test that non-matching query returns plain text."""
        result = highlight_matches("Hello world", "xyz")
        assert str(result) == "Hello world"
        spans = list(result._spans)
        assert len(spans) == 0

    def test_custom_style(self):
        """Test that custom style is applied."""
        result = highlight_matches("Hello world", "world", style="bold red")
        spans = list(result._spans)
        assert len(spans) > 0
        assert spans[0].style == "bold red"

    def test_empty_text(self):
        """Test handling of empty text."""
        result = highlight_matches("", "query")
        assert str(result) == ""

    def test_whitespace_only_terms_are_skipped(self):
        """Test that whitespace-only terms are skipped."""
        result = highlight_matches("Hello world", "   ")
        spans = list(result._spans)
        assert len(spans) == 0


class TestGetAgentIcon:
    """Tests for get_agent_icon function."""

    def test_returns_renderable(self):
        """Test that function returns a renderable object."""
        # Clear cache to ensure fresh test
        _icon_cache.clear()

        result = get_agent_icon("claude")
        # Should return some renderable (could be Columns or Text)
        assert result is not None

    def test_falls_back_to_badge_when_no_icon(self):
        """Test that badge is returned when icon file doesn't exist."""
        _icon_cache.clear()

        # Use a fake agent name that won't have an icon
        result = get_agent_icon("nonexistent-agent")
        # Should return a Text object with the badge
        assert result is not None
        # The result should contain the agent name
        text_str = str(result)
        assert "nonexistent-agent" in text_str

    def test_uses_cache(self):
        """Test that icon cache is used."""
        _icon_cache.clear()

        # First call should populate cache
        get_agent_icon("claude")
        assert "claude" in _icon_cache

        # Cache should be used on second call
        cached_value = _icon_cache["claude"]
        get_agent_icon("claude")
        assert _icon_cache["claude"] is cached_value

    def test_handles_unknown_agent_config(self):
        """Test handling of agent with no config."""
        _icon_cache.clear()

        result = get_agent_icon("unknown-agent")
        assert result is not None


class TestHighlightMatchesEdgeCases:
    """Additional edge case tests for highlight_matches."""

    def test_overlapping_terms_handled(self):
        """Test that overlapping terms are handled correctly."""
        # "test" and "testing" overlap - should still work
        result = highlight_matches("testing framework", "test testing")
        assert str(result) == "testing framework"

    def test_special_regex_characters_in_query(self):
        """Test that special regex characters don't break matching."""
        # The function uses string.find(), not regex, so this should work
        result = highlight_matches("Hello (world)", "(world)")
        spans = list(result._spans)
        assert len(spans) > 0

    def test_unicode_text(self):
        """Test handling of unicode text."""
        result = highlight_matches("Hello 世界", "世界")
        spans = list(result._spans)
        assert len(spans) > 0

    def test_truncation_preserves_highlighting(self):
        """Test that truncation still applies highlighting to visible text."""
        result = highlight_matches("Hello world is great", "hello", max_len=15)
        # Should be "Hello world ..."
        assert str(result).endswith("...")
        spans = list(result._spans)
        # "Hello" should still be highlighted
        assert len(spans) > 0
        assert spans[0].start == 0
        assert spans[0].end == 5

    def test_truncation_removes_match_beyond_limit(self):
        """Test that matches beyond truncation limit are not highlighted."""
        result = highlight_matches("Short text with match at end", "end", max_len=15)
        # Text truncated before "end" appears
        assert "end" not in str(result)
        spans = list(result._spans)
        assert len(spans) == 0


class TestSessionPreviewContent:
    """Tests for SessionPreview content rendering."""

    def test_multiple_messages_rendered(self):
        """Test that multiple messages are rendered."""
        from fast_resume.tui.preview import SessionPreview

        preview = SessionPreview()

        session = Session(
            id="test-session",
            agent="claude",
            title="Test Session",
            directory="/test",
            timestamp=datetime.now(),
            preview="» First message\n\n  Response one\n\n» Second message",
            content="» First message\n\n  Response one\n\n» Second message",
            message_count=2,
        )

        result = preview.build_preview_text(session, "")
        text_str = str(result)

        # Should contain both user messages and response
        assert "First message" in text_str
        assert "Response one" in text_str
        assert "Second message" in text_str

    def test_user_prompt_styling(self):
        """Test that user prompts have the » prefix."""
        from fast_resume.tui.preview import SessionPreview

        preview = SessionPreview()

        session = Session(
            id="test-session",
            agent="claude",
            title="Test Session",
            directory="/test",
            timestamp=datetime.now(),
            preview="» Hello world",
            content="» Hello world",
            message_count=1,
        )

        result = preview.build_preview_text(session, "")
        text_str = str(result)

        assert "»" in text_str
        assert "Hello world" in text_str

    def test_agent_badge_on_assistant_message(self):
        """Test that assistant messages show agent badge."""
        from fast_resume.tui.preview import SessionPreview

        preview = SessionPreview()

        session = Session(
            id="test-session",
            agent="claude",
            title="Test Session",
            directory="/test",
            timestamp=datetime.now(),
            preview="  This is an assistant response",
            content="  This is an assistant response",
            message_count=1,
        )

        result = preview.build_preview_text(session, "")
        text_str = str(result)

        # Should contain agent badge
        assert "claude" in text_str
        assert "●" in text_str

    def test_code_block_rendering(self):
        """Test that code blocks are rendered."""
        from fast_resume.tui.preview import SessionPreview

        preview = SessionPreview()

        session = Session(
            id="test-session",
            agent="claude",
            title="Test Session",
            directory="/test",
            timestamp=datetime.now(),
            preview="» Question\n\n  Here's code:\n```python\nprint('hello')\n```",
            content="» Question\n\n  Here's code:\n```python\nprint('hello')\n```",
            message_count=2,
        )

        result = preview.build_preview_text(session, "")
        text_str = str(result)

        # Code content should be present (syntax highlighted or plain)
        assert "print" in text_str

    def test_query_highlighting(self):
        """Test that search terms are highlighted in preview."""
        from fast_resume.tui.preview import SessionPreview

        preview = SessionPreview()

        session = Session(
            id="test-session",
            agent="claude",
            title="Test Session",
            directory="/test",
            timestamp=datetime.now(),
            preview="» Find the special word here",
            content="» Find the special word here",
            message_count=1,
        )

        result = preview.build_preview_text(session, "special")
        text_str = str(result)
        assert "special" in text_str

    def test_truncation_indicator(self):
        """Test that truncated messages show indicator."""
        from fast_resume.tui.preview import SessionPreview

        preview = SessionPreview()

        # Create a message with many lines (more than MAX_ASSISTANT_LINES)
        long_response = "\n".join([f"  Line {i}" for i in range(10)])
        session = Session(
            id="test-session",
            agent="claude",
            title="Test Session",
            directory="/test",
            timestamp=datetime.now(),
            preview=f"» Question\n\n{long_response}",
            content=f"» Question\n\n{long_response}",
            message_count=2,
        )

        result = preview.build_preview_text(session, "")
        text_str = str(result)

        # Should show truncation indicator (⋯ or ...)
        assert "⋯" in text_str or "..." in text_str


class TestFormatDirectoryEdgeCases:
    """Additional edge case tests for format_directory."""

    def test_path_with_spaces(self):
        """Test handling of paths with spaces."""
        home = os.path.expanduser("~")
        path = f"{home}/my project/with spaces"
        result = format_directory(path)
        assert result == "~/my project/with spaces"

    def test_nested_home_path(self):
        """Test handling of deeply nested paths under home."""
        home = os.path.expanduser("~")
        path = f"{home}/deeply/nested/path/to/project"
        result = format_directory(path)
        assert result == "~/deeply/nested/path/to/project"


# =============================================================================
# TUI Integration Tests using Textual Pilot
# =============================================================================


@pytest.fixture
def sample_sessions():
    """Create sample sessions for TUI testing."""
    return [
        Session(
            id="session-1",
            agent="claude",
            title="Fix authentication bug",
            directory="/home/user/web-app",
            timestamp=datetime.now() - timedelta(hours=1),
            preview="Help me fix the auth bug",
            content="Help me fix the authentication bug in login.py",
            message_count=4,
            mtime=1705312200.0,
        ),
        Session(
            id="session-2",
            agent="vibe",
            title="Create REST API",
            directory="/home/user/api-project",
            timestamp=datetime.now() - timedelta(hours=2),
            preview="Build a REST endpoint",
            content="Create a REST API endpoint for users",
            message_count=6,
            mtime=1705312100.0,
        ),
        Session(
            id="session-3",
            agent="claude",
            title="Refactor database queries",
            directory="/home/user/backend",
            timestamp=datetime.now() - timedelta(days=1),
            preview="Optimize SQL queries",
            content="Refactor the database queries for better performance",
            message_count=8,
            mtime=1705225200.0,
        ),
    ]


@pytest.fixture
def mock_search_engine(sample_sessions):
    """Create a mock search engine that returns sample sessions."""
    mock = MagicMock()
    mock.search.return_value = sample_sessions
    mock.get_session_count.return_value = len(sample_sessions)
    mock.get_all_sessions.return_value = sample_sessions
    # Return sessions from index for sync loading (avoids async complexity in tests)
    mock._load_from_index.return_value = sample_sessions
    mock._sessions = sample_sessions
    mock._streaming_in_progress = False
    # Mock streaming to return immediately with no changes
    mock.get_sessions_streaming.return_value = (sample_sessions, 0, 0, 0)
    mock.get_resume_command.return_value = ["claude", "--resume", "session-1"]
    # Mock adapter with supports_yolo=False to skip modal in tests
    mock_adapter = MagicMock()
    mock_adapter.supports_yolo = False
    mock.get_adapter_for_session.return_value = mock_adapter
    return mock


class TestFastResumeAppBasic:
    """Basic TUI integration tests."""

    @pytest.mark.asyncio
    async def test_app_launches(self, mock_search_engine):
        """Test that the app launches without errors."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                # Wait for app to settle
                await pilot.pause()
                # App should be running
                assert app.is_running

    @pytest.mark.asyncio
    async def test_no_version_check_skips_update_check(self, mock_search_engine):
        """Test that no_version_check=True skips the version check."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp(no_version_check=True)
            with patch.object(app, "_check_for_updates") as mock_check:
                async with app.run_test(size=(120, 40)) as pilot:
                    await pilot.pause()
                    # Version check should not have been called
                    mock_check.assert_not_called()

    @pytest.mark.asyncio
    async def test_version_check_runs_by_default(self, mock_search_engine):
        """Test that version check runs when no_version_check=False."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp(no_version_check=False)
            with patch.object(app, "_check_for_updates") as mock_check:
                async with app.run_test(size=(120, 40)) as pilot:
                    await pilot.pause()
                    # Version check should have been called
                    mock_check.assert_called_once()

    @pytest.mark.asyncio
    async def test_app_displays_title(self, mock_search_engine):
        """Test that app title is displayed."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                # Check title label exists
                title = app.query_one("#app-title")
                # Title text could be in different formats
                assert title is not None

    @pytest.mark.asyncio
    async def test_app_has_search_input(self, mock_search_engine):
        """Test that search input is present."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                search_input = app.query_one("#search-input")
                assert search_input is not None

    @pytest.mark.asyncio
    async def test_app_has_results_table(self, mock_search_engine):
        """Test that results table is present."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                table = app.query_one("#results-table")
                assert table is not None

    @pytest.mark.asyncio
    async def test_table_displays_sessions(self, mock_search_engine, sample_sessions):
        """Test that sessions are actually displayed in the table."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                table = app.query_one("#results-table")
                # Table should have rows for the sessions
                assert table.row_count == len(sample_sessions)
                # First session should be selected
                assert app.selected_session is not None
                assert app.selected_session.id == sample_sessions[0].id


class TestFastResumeAppNavigation:
    """Tests for TUI navigation."""

    @pytest.mark.asyncio
    async def test_escape_quits_app(self, mock_search_engine):
        """Test that Escape key quits the app."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                await pilot.press("escape")
                # App should have exited
                assert not app.is_running

    @pytest.mark.asyncio
    async def test_q_quits_app(self, mock_search_engine):
        """Test that 'q' key quits the app when not in search input."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                # Focus the table first (away from search input)
                table = app.query_one("#results-table")
                table.focus()
                await pilot.pause()
                await pilot.press("q")
                assert not app.is_running

    @pytest.mark.asyncio
    async def test_slash_focuses_search(self, mock_search_engine):
        """Test that '/' focuses the search input."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                # Unfocus search first
                await pilot.press("tab")
                # Press / to focus search
                await pilot.press("/")
                search_input = app.query_one("#search-input")
                assert search_input.has_focus

    @pytest.mark.asyncio
    async def test_ctrl_backtick_toggles_preview(self, mock_search_engine):
        """Test that Ctrl+` toggles preview pane."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                # Preview should be visible by default
                assert app.show_preview is True

                # Ctrl+` should toggle it off
                await pilot.press("ctrl+grave_accent")
                assert app.show_preview is False

                # Ctrl+` again should toggle it back on
                await pilot.press("ctrl+grave_accent")
                assert app.show_preview is True


class TestFastResumeAppSearch:
    """Tests for search functionality."""

    @pytest.mark.asyncio
    async def test_typing_triggers_search(self, mock_search_engine):
        """Test that typing in search triggers a search."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                # Type in search
                await pilot.press("a", "u", "t", "h")
                # Wait for debounce
                await pilot.pause()

                # Search should have been called
                mock_search_engine.search.assert_called()

    @pytest.mark.asyncio
    async def test_initial_query_is_used(self, mock_search_engine):
        """Test that initial query is set in search input."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp(initial_query="test query")
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                search_input = app.query_one("#search-input")
                assert search_input.value == "test query"


class TestFastResumeAppFilters:
    """Tests for filter functionality."""

    @pytest.mark.asyncio
    async def test_initial_agent_filter(self, mock_search_engine):
        """Test that initial agent filter is applied."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp(agent_filter="claude")
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                assert app.active_filter == "claude"

    @pytest.mark.asyncio
    async def test_filter_buttons_exist(self, mock_search_engine):
        """Test that filter buttons are present."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                # Check for filter buttons
                all_filter = app.query_one("#filter-all")
                claude_filter = app.query_one("#filter-claude")
                vibe_filter = app.query_one("#filter-vibe")

                assert all_filter is not None
                assert claude_filter is not None
                assert vibe_filter is not None

    @pytest.mark.asyncio
    async def test_typing_agent_keyword_syncs_filter_button(self, mock_search_engine):
        """Test that typing agent: keyword syncs the filter button."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                # Initially no filter
                assert app.active_filter is None

                # Type agent:claude in search input
                search_input = app.query_one("#search-input")
                search_input.value = "agent:claude"
                await pilot.pause()

                # Filter button should sync to claude
                assert app.active_filter == "claude"

    @pytest.mark.asyncio
    async def test_filter_button_adds_agent_keyword(self, mock_search_engine):
        """Test that clicking filter button adds agent: keyword to query."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                search_input = app.query_one("#search-input")

                # Type some search text
                search_input.value = "test query"
                await pilot.pause()

                # Set filter to claude
                app._set_filter("claude")
                await pilot.pause()

                # Query should have agent:claude added
                assert "agent:claude" in search_input.value
                assert "test query" in search_input.value

    @pytest.mark.asyncio
    async def test_filter_all_removes_agent_keyword(self, mock_search_engine):
        """Test that setting filter to All removes agent: keyword."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                search_input = app.query_one("#search-input")

                # Start with agent keyword
                search_input.value = "agent:claude test query"
                await pilot.pause()
                assert app.active_filter == "claude"

                # Set filter to None (All)
                app._set_filter(None)
                await pilot.pause()

                # agent: keyword should be removed
                assert "agent:" not in search_input.value
                assert "test query" in search_input.value

    @pytest.mark.asyncio
    async def test_session_count_updates_with_filter(self, sample_sessions):
        """Test that session count label shows filtered count when filter is active.

        This is a regression test for the feature that shows x/total_for_agent
        instead of x/total_all when an agent filter is active.
        """
        # sample_sessions has: 2 claude, 1 vibe = 3 total
        mock = MagicMock()
        mock._streaming_in_progress = False
        mock._sessions = sample_sessions
        mock._load_from_index.return_value = sample_sessions
        mock.get_sessions_streaming.return_value = (sample_sessions, 0, 0, 0)
        mock.get_resume_command.return_value = ["claude", "--resume", "session-1"]
        mock_adapter = MagicMock()
        mock_adapter.supports_yolo = False
        mock.get_adapter_for_session.return_value = mock_adapter

        # Mock get_session_count to return filtered counts
        def mock_get_session_count(agent_filter=None):
            if agent_filter is None:
                return 3
            elif agent_filter == "claude":
                return 2
            elif agent_filter == "vibe":
                return 1
            return 0

        mock.get_session_count.side_effect = mock_get_session_count

        # Mock search to return filtered results
        def mock_search(query, agent_filter=None, limit=100):
            if agent_filter == "claude":
                return [s for s in sample_sessions if s.agent == "claude"]
            elif agent_filter == "vibe":
                return [s for s in sample_sessions if s.agent == "vibe"]
            return sample_sessions

        mock.search.side_effect = mock_search

        with patch("fast_resume.tui.app.SessionSearch", return_value=mock):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()

                # Initially no filter - should show total (3)
                from textual.widgets import Label

                count_label = app.query_one("#session-count", Label)
                assert "3" in str(count_label.render())

                # Filter by claude - should show 2
                app._set_filter("claude")
                await pilot.pause()
                assert "2" in str(count_label.render())

                # Filter by vibe - should show 1
                app._set_filter("vibe")
                await pilot.pause()
                assert "1" in str(count_label.render())

                # Back to all - should show 3
                app._set_filter(None)
                await pilot.pause()
                assert "3" in str(count_label.render())


class TestFastResumeAppPreview:
    """Tests for preview pane functionality."""

    @pytest.mark.asyncio
    async def test_preview_pane_visible_by_default(self, mock_search_engine):
        """Test that preview pane is visible by default."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                preview = app.query_one("#preview-container")
                assert "hidden" not in preview.classes

    @pytest.mark.asyncio
    async def test_preview_height_adjustable(self, mock_search_engine):
        """Test that preview height can be adjusted."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                # Focus table first (away from search input)
                table = app.query_one("#results-table")
                table.focus()
                await pilot.pause()

                initial_height = app.preview_height

                # Increase preview height (use 'equals' which maps to same action)
                await pilot.press("equals")
                await pilot.pause()
                assert app.preview_height > initial_height

                # Decrease preview height
                await pilot.press("minus")
                await pilot.press("minus")
                await pilot.pause()
                assert app.preview_height < initial_height


class TestFastResumeAppResumeCommand:
    """Tests for resume command functionality."""

    @pytest.mark.asyncio
    async def test_get_resume_command_initially_none(self, mock_search_engine):
        """Test that resume command is None before selection."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                assert app.get_resume_command() is None

    @pytest.mark.asyncio
    async def test_get_resume_directory_initially_none(self, mock_search_engine):
        """Test that resume directory is None before selection."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                assert app.get_resume_directory() is None

    @pytest.mark.asyncio
    async def test_enter_in_search_triggers_resume(
        self, mock_search_engine, sample_sessions
    ):
        """Test that pressing Enter in search input resumes the selected session."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()

                # Search input is focused by default, press Enter to resume
                await pilot.press("enter")
                await pilot.pause()

                # App should have exited
                assert not app.is_running

                # Resume command should be set
                cmd = app.get_resume_command()
                assert cmd is not None
                assert cmd == ["claude", "--resume", "session-1"]

    @pytest.mark.asyncio
    async def test_resume_sets_directory(self, mock_search_engine, sample_sessions):
        """Test that resuming a session sets the directory."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()

                # Press Enter in search input to resume
                await pilot.press("enter")
                await pilot.pause()

                # Resume directory should be set
                directory = app.get_resume_directory()
                assert directory == "/home/user/web-app"

    @pytest.mark.asyncio
    async def test_navigate_and_resume_different_session(
        self, mock_search_engine, sample_sessions
    ):
        """Test navigating to a different session and resuming it."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            # Set up mock to return different command for each session
            def mock_resume_cmd(session, yolo=False):
                if session.id == "session-2":
                    return ["vibe", "resume", "session-2"]
                return ["claude", "--resume", session.id]

            mock_search_engine.get_resume_command.side_effect = mock_resume_cmd

            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()

                # Navigate to table with Tab (or move to second row)
                table = app.query_one("#results-table")
                table.focus()
                await pilot.pause()

                # Move down to second session
                await pilot.press("down")
                await pilot.pause()

                # Go back to search and press Enter
                search_input = app.query_one("#search-input")
                search_input.focus()
                await pilot.pause()
                await pilot.press("enter")
                await pilot.pause()

                # Should have selected session-2 (cursor was on row 1)
                directory = app.get_resume_directory()
                assert directory == "/home/user/api-project"

    @pytest.mark.asyncio
    async def test_vim_navigation_j_k(self, mock_search_engine, sample_sessions):
        """Test vim-style navigation with j/k keys."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()

                # Focus the table
                table = app.query_one("#results-table")
                table.focus()
                await pilot.pause()

                # Move down with j
                await pilot.press("j")
                await pilot.pause()

                # Move back up with k
                await pilot.press("k")
                await pilot.pause()

                # Focus search input and resume (cursor should be on first row)
                search_input = app.query_one("#search-input")
                search_input.focus()
                await pilot.pause()
                await pilot.press("enter")
                await pilot.pause()

                # Should be first session
                directory = app.get_resume_directory()
                assert directory == "/home/user/web-app"


class TestFastResumeAppYoloModal:
    """Tests for yolo mode modal functionality."""

    @pytest.mark.asyncio
    async def test_modal_shown_when_adapter_supports_yolo(self, sample_sessions):
        """Test that modal is shown when adapter supports yolo but session doesn't store it."""
        mock = MagicMock()
        mock.search.return_value = sample_sessions
        mock.get_session_count.return_value = len(sample_sessions)
        mock._load_from_index.return_value = sample_sessions
        mock._sessions = sample_sessions
        mock._streaming_in_progress = False
        mock.get_sessions_streaming.return_value = (sample_sessions, 0, 0, 0)
        mock.get_resume_command.return_value = ["claude", "--resume", "session-1"]
        # Adapter supports yolo
        mock_adapter = MagicMock()
        mock_adapter.supports_yolo = True
        mock.get_adapter_for_session.return_value = mock_adapter

        with patch("fast_resume.tui.app.SessionSearch", return_value=mock):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()

                # Press Enter to resume - should show modal
                await pilot.press("enter")
                await pilot.pause()

                # App should still be running (modal is shown)
                assert app.is_running

                # Press 'n' to select normal mode
                await pilot.press("n")
                await pilot.pause()

                # Now app should have exited
                assert not app.is_running

    @pytest.mark.asyncio
    async def test_modal_skipped_when_session_has_yolo(self, sample_sessions):
        """Test that modal is skipped when session has stored yolo=True."""
        # Create a session with yolo=True
        yolo_session = Session(
            id="session-yolo",
            agent="codex",
            title="Yolo session",
            directory="/home/user/yolo-project",
            timestamp=sample_sessions[0].timestamp,
            preview="Test preview",
            content="Test content",
            message_count=5,
            mtime=1705225200.0,
            yolo=True,  # Session was started in yolo mode
        )
        sessions_with_yolo = [yolo_session] + sample_sessions

        mock = MagicMock()
        mock.search.return_value = sessions_with_yolo
        mock.get_session_count.return_value = len(sessions_with_yolo)
        mock._load_from_index.return_value = sessions_with_yolo
        mock._sessions = sessions_with_yolo
        mock._streaming_in_progress = False
        mock.get_sessions_streaming.return_value = (sessions_with_yolo, 0, 0, 0)
        mock.get_resume_command.return_value = [
            "codex",
            "--dangerously-bypass-approvals-and-sandbox",
            "resume",
            "session-yolo",
        ]
        # Adapter supports yolo
        mock_adapter = MagicMock()
        mock_adapter.supports_yolo = True
        mock.get_adapter_for_session.return_value = mock_adapter

        with patch("fast_resume.tui.app.SessionSearch", return_value=mock):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()

                # Press Enter to resume - should exit immediately (no modal)
                await pilot.press("enter")
                await pilot.pause()

                # App should have exited without showing modal
                assert not app.is_running

                # Resume command should include yolo flag
                cmd = app.get_resume_command()
                assert cmd is not None
                assert "--dangerously-bypass-approvals-and-sandbox" in cmd

    @pytest.mark.asyncio
    async def test_yolo_selected_in_modal(self, sample_sessions):
        """Test that selecting yolo in modal adds yolo flag to command."""
        mock = MagicMock()
        mock.search.return_value = sample_sessions
        mock.get_session_count.return_value = len(sample_sessions)
        mock._load_from_index.return_value = sample_sessions
        mock._sessions = sample_sessions
        mock._streaming_in_progress = False
        mock.get_sessions_streaming.return_value = (sample_sessions, 0, 0, 0)

        def mock_resume_cmd(session, yolo=False):
            if yolo:
                return [
                    "claude",
                    "--dangerously-skip-permissions",
                    "--resume",
                    session.id,
                ]
            return ["claude", "--resume", session.id]

        mock.get_resume_command.side_effect = mock_resume_cmd
        mock_adapter = MagicMock()
        mock_adapter.supports_yolo = True
        mock.get_adapter_for_session.return_value = mock_adapter

        with patch("fast_resume.tui.app.SessionSearch", return_value=mock):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()

                # Press Enter to resume - should show modal
                await pilot.press("enter")
                await pilot.pause()

                # Press 'y' to select yolo mode
                await pilot.press("y")
                await pilot.pause()

                # App should have exited
                assert not app.is_running

                # Resume command should include yolo flag
                cmd = app.get_resume_command()
                assert cmd is not None
                assert "--dangerously-skip-permissions" in cmd


class TestFastResumeAppRunTui:
    """Tests for run_tui function integration."""

    @pytest.mark.asyncio
    async def test_run_tui_returns_none_on_escape(self, mock_search_engine):
        """Test that run_tui returns (None, None) when user presses Escape."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                await pilot.press("escape")
                await pilot.pause()

                # Both should be None when escaped
                assert app.get_resume_command() is None
                assert app.get_resume_directory() is None

    @pytest.mark.asyncio
    async def test_selected_session_accessible(
        self, mock_search_engine, sample_sessions
    ):
        """Test that selected session is accessible after selection."""
        with patch(
            "fast_resume.tui.app.SessionSearch", return_value=mock_search_engine
        ):
            app = FastResumeApp()
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()

                # Focus and select
                table = app.query_one("#results-table")
                table.focus()
                await pilot.pause()

                await pilot.press("enter")
                await pilot.pause()

                # Check that selected_session was set
                assert app.selected_session is not None
                assert app.selected_session.id == "session-1"
                assert app.selected_session.agent == "claude"


class TestKeywordSuggester:
    """Tests for KeywordSuggester autocomplete."""

    @pytest.fixture
    def suggester(self):
        """Create a KeywordSuggester instance."""
        return KeywordSuggester()

    @pytest.mark.asyncio
    async def test_suggests_agent_completion(self, suggester):
        """Test that agent:cl suggests claude."""
        result = await suggester.get_suggestion("agent:cl")
        assert result == "agent:claude"

    @pytest.mark.asyncio
    async def test_suggests_agent_codex(self, suggester):
        """Test that agent:cod suggests codex."""
        result = await suggester.get_suggestion("agent:cod")
        assert result == "agent:codex"

    @pytest.mark.asyncio
    async def test_suggests_date_today(self, suggester):
        """Test that date:to suggests today."""
        result = await suggester.get_suggestion("date:to")
        assert result == "date:today"

    @pytest.mark.asyncio
    async def test_suggests_date_yesterday(self, suggester):
        """Test that date:ye suggests yesterday."""
        result = await suggester.get_suggestion("date:ye")
        assert result == "date:yesterday"

    @pytest.mark.asyncio
    async def test_no_suggestion_for_dir(self, suggester):
        """Test that dir: has no suggestions (user-specific)."""
        result = await suggester.get_suggestion("dir:pro")
        assert result is None

    @pytest.mark.asyncio
    async def test_no_suggestion_for_empty_value(self, suggester):
        """Test no suggestion when value is empty."""
        result = await suggester.get_suggestion("agent:")
        assert result is None

    @pytest.mark.asyncio
    async def test_no_suggestion_for_no_match(self, suggester):
        """Test no suggestion when no values match."""
        result = await suggester.get_suggestion("agent:xyz")
        assert result is None

    @pytest.mark.asyncio
    async def test_suggests_with_prefix_text(self, suggester):
        """Test suggestion works with text before keyword."""
        result = await suggester.get_suggestion("my search agent:cl")
        assert result == "my search agent:claude"

    @pytest.mark.asyncio
    async def test_suggests_negated_agent(self, suggester):
        """Test that agent:!cl suggests !claude."""
        result = await suggester.get_suggestion("agent:!cl")
        assert result == "agent:!claude"

    @pytest.mark.asyncio
    async def test_suggests_with_dash_negation(self, suggester):
        """Test that -agent:cl suggests claude."""
        result = await suggester.get_suggestion("-agent:cl")
        assert result == "-agent:claude"

    @pytest.mark.asyncio
    async def test_no_suggestion_for_plain_text(self, suggester):
        """Test no suggestion for plain text without keywords."""
        result = await suggester.get_suggestion("some search query")
        assert result is None

    @pytest.mark.asyncio
    async def test_case_insensitive(self, suggester):
        """Test that suggestions are case insensitive."""
        result = await suggester.get_suggestion("agent:CL")
        assert result == "agent:claude"
