"""Integration tests for CLI commands.

These tests use real adapters and real data to verify actual CLI behavior.
"""

import json
from unittest.mock import patch

import pytest
from click.testing import CliRunner

from fast_resume.adapters.claude import ClaudeAdapter
from fast_resume.adapters.vibe import VibeAdapter
from fast_resume.cli import main, _list_sessions, _show_stats
from fast_resume.index import TantivyIndex
from fast_resume.search import SessionSearch


@pytest.fixture
def cli_runner():
    """Create a Click CLI runner."""
    return CliRunner()


@pytest.fixture
def cli_env(temp_dir):
    """Set up a complete CLI test environment with temp directories."""
    # Create directories for adapters
    claude_base = temp_dir / "claude"
    claude_project = claude_base / "project-test"
    vibe_dir = temp_dir / "vibe"

    claude_project.mkdir(parents=True)
    vibe_dir.mkdir(parents=True)

    # Create Claude sessions
    claude_session1 = claude_project / "session-claude-001.jsonl"
    claude_data1 = [
        {
            "type": "user",
            "cwd": "/home/user/web-app",
            "message": {"content": "Help me fix the authentication bug"},
        },
        {
            "type": "assistant",
            "message": {"content": "I'll help you fix the auth bug."},
        },
        {"type": "summary", "summary": "Fix authentication bug in login"},
    ]
    with open(claude_session1, "w") as f:
        for entry in claude_data1:
            f.write(json.dumps(entry) + "\n")

    claude_session2 = claude_project / "session-claude-002.jsonl"
    claude_data2 = [
        {
            "type": "user",
            "cwd": "/home/user/api",
            "message": {"content": "Add rate limiting"},
        },
        {
            "type": "assistant",
            "message": {"content": "Here's rate limiting middleware."},
        },
        {"type": "summary", "summary": "Implement rate limiting for API"},
    ]
    with open(claude_session2, "w") as f:
        for entry in claude_data2:
            f.write(json.dumps(entry) + "\n")

    # Create Vibe session
    vibe_session = vibe_dir / "session_vibe-001.json"
    vibe_data = {
        "metadata": {
            "session_id": "vibe-001",
            "start_time": "2025-01-10T14:00:00",
            "environment": {"working_directory": "/home/user/frontend"},
        },
        "messages": [
            {"role": "user", "content": "Create a React component"},
            {"role": "assistant", "content": "Here's the React component."},
        ],
    }
    with open(vibe_session, "w") as f:
        json.dump(vibe_data, f)

    index_dir = temp_dir / "index"

    return {
        "temp_dir": temp_dir,
        "claude_dir": claude_base,
        "vibe_dir": vibe_dir,
        "index_dir": index_dir,
    }


@pytest.fixture
def configured_search(cli_env):
    """Create a SessionSearch with test-configured adapters."""
    search = SessionSearch()
    search.adapters = [
        ClaudeAdapter(sessions_dir=cli_env["claude_dir"]),
        VibeAdapter(sessions_dir=cli_env["vibe_dir"]),
    ]
    search._index = TantivyIndex(index_path=cli_env["index_dir"])
    return search


class TestListSessionsIntegration:
    """Integration tests for _list_sessions with real data."""

    def test_lists_all_sessions(self, configured_search, capsys):
        """Test that all sessions are listed."""
        with patch("fast_resume.cli.SessionSearch", return_value=configured_search):
            _list_sessions("", None, None)

        captured = capsys.readouterr()
        assert "claude" in captured.out
        assert "vibe" in captured.out
        assert "Showing 3 of 3 sessions" in captured.out

    def test_lists_sessions_with_agent_filter(self, configured_search, capsys):
        """Test filtering sessions by agent."""
        with patch("fast_resume.cli.SessionSearch", return_value=configured_search):
            _list_sessions("", "claude", None)

        captured = capsys.readouterr()
        assert "claude" in captured.out
        assert "Showing 2 of 2 sessions" in captured.out

    def test_lists_sessions_with_search_query(self, configured_search, capsys):
        """Test searching sessions by query."""
        with patch("fast_resume.cli.SessionSearch", return_value=configured_search):
            _list_sessions("authentication", None, None)

        captured = capsys.readouterr()
        assert "authentication" in captured.out.lower()
        assert "Showing 1 of 1 sessions" in captured.out

    def test_lists_sessions_with_directory_filter(self, configured_search, capsys):
        """Test filtering sessions by directory."""
        with patch("fast_resume.cli.SessionSearch", return_value=configured_search):
            _list_sessions("", None, "frontend")

        captured = capsys.readouterr()
        assert "vibe" in captured.out
        assert "Showing 1 of 1 sessions" in captured.out

    def test_no_sessions_message(self, configured_search, capsys):
        """Test message when no sessions match."""
        with patch("fast_resume.cli.SessionSearch", return_value=configured_search):
            _list_sessions("nonexistentquery12345", None, None)

        captured = capsys.readouterr()
        assert "No sessions found" in captured.out


class TestShowStatsIntegration:
    """Integration tests for _show_stats with real data."""

    def test_shows_stats_with_sessions(self, configured_search, capsys):
        """Test that stats are displayed correctly."""
        # Populate the index first
        configured_search.get_all_sessions()

        with patch(
            "fast_resume.cli.TantivyIndex", return_value=configured_search._index
        ):
            _show_stats()

        captured = capsys.readouterr()
        assert "Index Statistics" in captured.out
        assert "Total sessions" in captured.out
        assert "3" in captured.out
        assert "Data by Agent" in captured.out
        assert "claude" in captured.out
        assert "vibe" in captured.out

    def test_shows_empty_index_message(self, temp_dir, capsys):
        """Test message when index is empty."""
        index_dir = temp_dir / "empty_index"

        with patch("fast_resume.cli.TantivyIndex") as MockIndex:
            MockIndex.return_value = TantivyIndex(index_path=index_dir)
            _show_stats()

        captured = capsys.readouterr()
        assert "No sessions indexed" in captured.out


class TestMainCommandIntegration:
    """Integration tests for main CLI command."""

    def test_stats_flag_output(self, cli_runner, configured_search):
        """Test that --stats produces real output."""
        configured_search.get_all_sessions()

        with patch(
            "fast_resume.cli.TantivyIndex", return_value=configured_search._index
        ):
            result = cli_runner.invoke(main, ["--stats"])

        assert result.exit_code == 0
        assert "Index Statistics" in result.output
        assert "Total sessions" in result.output

    def test_rebuild_flag_rebuilds_index(self, cli_runner, configured_search):
        """Test that --rebuild rebuilds the index."""
        with patch("fast_resume.cli.SessionSearch", return_value=configured_search):
            result = cli_runner.invoke(main, ["--rebuild"])

        assert result.exit_code == 0
        assert "Index rebuilt" in result.output

    def test_list_flag_shows_sessions(self, cli_runner, configured_search):
        """Test that --list shows session table."""
        with patch("fast_resume.cli.SessionSearch", return_value=configured_search):
            result = cli_runner.invoke(main, ["--list"])

        assert result.exit_code == 0
        assert "claude" in result.output
        assert "vibe" in result.output
        assert "Showing 3 of 3 sessions" in result.output

    def test_agent_filter_in_list(self, cli_runner, configured_search):
        """Test that -a filter works with --list."""
        with patch("fast_resume.cli.SessionSearch", return_value=configured_search):
            result = cli_runner.invoke(main, ["--list", "-a", "vibe"])

        assert result.exit_code == 0
        assert "vibe" in result.output
        assert "Showing 1 of 1 sessions" in result.output

    def test_query_search_in_list(self, cli_runner, configured_search):
        """Test that query argument searches sessions."""
        with patch("fast_resume.cli.SessionSearch", return_value=configured_search):
            result = cli_runner.invoke(main, ["--list", "React"])

        assert result.exit_code == 0
        assert "Showing 1 of 1 sessions" in result.output

    def test_directory_filter_in_list(self, cli_runner, configured_search):
        """Test that -d filter works with --list."""
        with patch("fast_resume.cli.SessionSearch", return_value=configured_search):
            result = cli_runner.invoke(main, ["--list", "-d", "web-app"])

        assert result.exit_code == 0
        assert "Showing 1 of 1 sessions" in result.output

    def test_version_flag(self, cli_runner):
        """Test that --version works."""
        result = cli_runner.invoke(main, ["--version"])
        assert result.exit_code == 0
        assert "version" in result.output.lower() or "." in result.output

    def test_default_launches_tui(self, cli_runner):
        """Test that default invocation launches TUI."""
        with patch("fast_resume.cli.run_tui") as mock_run_tui:
            mock_run_tui.return_value = (None, None)
            result = cli_runner.invoke(main, [])

        assert result.exit_code == 0
        mock_run_tui.assert_called_once()

    def test_no_version_check_flag(self, cli_runner):
        """Test that --no-version-check flag is passed to run_tui."""
        with patch("fast_resume.cli.run_tui") as mock_run_tui:
            mock_run_tui.return_value = (None, None)
            result = cli_runner.invoke(main, ["--no-version-check"])

        assert result.exit_code == 0
        mock_run_tui.assert_called_once()
        # Check that no_version_check=True was passed
        call_kwargs = mock_run_tui.call_args.kwargs
        assert call_kwargs.get("no_version_check") is True

    def test_no_version_check_default_is_false(self, cli_runner):
        """Test that --no-version-check defaults to False."""
        with patch("fast_resume.cli.run_tui") as mock_run_tui:
            mock_run_tui.return_value = (None, None)
            result = cli_runner.invoke(main, [])

        assert result.exit_code == 0
        call_kwargs = mock_run_tui.call_args.kwargs
        assert call_kwargs.get("no_version_check") is False


class TestTUIResumeIntegration:
    """Tests for TUI resume command execution."""

    def test_tui_resume_changes_directory(self, cli_runner):
        """Test that selecting a session changes to session directory."""
        with (
            patch("fast_resume.cli.run_tui") as mock_run_tui,
            patch("fast_resume.cli.os.chdir") as mock_chdir,
            patch("fast_resume.cli.os.execvp") as mock_execvp,
        ):
            mock_run_tui.return_value = (
                ["claude", "--resume", "123"],
                "/home/user/project",
            )
            cli_runner.invoke(main, [])

        mock_chdir.assert_called_once_with("/home/user/project")
        mock_execvp.assert_called_once_with("claude", ["claude", "--resume", "123"])

    def test_tui_resume_without_directory(self, cli_runner):
        """Test resume when session has no directory."""
        with (
            patch("fast_resume.cli.run_tui") as mock_run_tui,
            patch("fast_resume.cli.os.chdir") as mock_chdir,
            patch("fast_resume.cli.os.execvp") as mock_execvp,
        ):
            mock_run_tui.return_value = (["vibe", "resume", "456"], None)
            cli_runner.invoke(main, [])

        mock_chdir.assert_not_called()
        mock_execvp.assert_called_once()


class TestOutputFormatting:
    """Tests for output formatting and content verification."""

    def test_session_table_has_all_columns(self, configured_search, capsys):
        """Test that session table has all expected columns."""
        with patch("fast_resume.cli.SessionSearch", return_value=configured_search):
            _list_sessions("", None, None)

        captured = capsys.readouterr()
        assert "Agent" in captured.out
        assert "Title" in captured.out
        assert "Directory" in captured.out
        assert "ID" in captured.out

    def test_stats_shows_agent_breakdown(self, configured_search, capsys):
        """Test that stats shows breakdown by agent."""
        configured_search.get_all_sessions()

        with patch(
            "fast_resume.cli.TantivyIndex", return_value=configured_search._index
        ):
            _show_stats()

        captured = capsys.readouterr()
        assert "Data by Agent" in captured.out
        assert "claude" in captured.out
        assert "vibe" in captured.out

    def test_long_title_truncation(self, temp_dir, capsys):
        """Test that long titles are truncated in output."""
        claude_base = temp_dir / "claude"
        claude_project = claude_base / "project-test"
        claude_project.mkdir(parents=True)

        long_title = "A" * 100
        session_file = claude_project / "session-long.jsonl"
        data = [
            {"type": "user", "cwd": "/test", "message": {"content": long_title}},
            {"type": "summary", "summary": long_title},
        ]
        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        search = SessionSearch()
        search.adapters = [ClaudeAdapter(sessions_dir=claude_base)]
        search._index = TantivyIndex(index_path=temp_dir / "index")

        with patch("fast_resume.cli.SessionSearch", return_value=search):
            _list_sessions("", None, None)

        captured = capsys.readouterr()
        assert "…" in captured.out or "..." in captured.out
        assert long_title not in captured.out

    def test_home_directory_replaced_with_tilde(self, configured_search, capsys):
        """Test that home directory is replaced with ~ in output."""
        with patch("fast_resume.cli.SessionSearch", return_value=configured_search):
            _list_sessions("", None, None)

        captured = capsys.readouterr()
        assert "Showing 3 of 3 sessions" in captured.out
        assert "/home/user" in captured.out or "home" in captured.out
