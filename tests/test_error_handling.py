"""Tests for error handling and logging."""

import os


from fast_resume.adapters.base import ParseError
from fast_resume.adapters.claude import ClaudeAdapter
from fast_resume.adapters.codex import CodexAdapter
from fast_resume.adapters.vibe import VibeAdapter
from fast_resume.logging_config import log_parse_error, setup_logging


class TestLogging:
    """Tests for logging functionality."""

    def test_setup_logging_creates_directory(self, temp_dir, monkeypatch):
        """Test that setup_logging creates the cache directory."""
        log_file = temp_dir / "test-errors.log"
        monkeypatch.setattr("fast_resume.logging_config.LOG_FILE", log_file)
        monkeypatch.setattr("fast_resume.logging_config.CACHE_DIR", temp_dir)

        # Clear any existing handlers
        import logging

        logger = logging.getLogger("fast_resume.parse_errors")
        logger.handlers.clear()

        setup_logging()

        assert temp_dir.exists()

    def test_log_parse_error_writes_to_file(self, temp_dir, monkeypatch):
        """Test that log_parse_error writes to the log file."""
        log_file = temp_dir / "test-errors.log"
        monkeypatch.setattr("fast_resume.logging_config.LOG_FILE", log_file)
        monkeypatch.setattr("fast_resume.logging_config.CACHE_DIR", temp_dir)

        # Clear any existing handlers and set up fresh
        import logging

        logger = logging.getLogger("fast_resume.parse_errors")
        logger.handlers.clear()

        setup_logging()
        log_parse_error("claude", "/test/path.jsonl", "TestError", "Test message")

        # Force flush
        for handler in logger.handlers:
            handler.flush()

        assert log_file.exists()
        content = log_file.read_text()
        assert "[claude]" in content
        assert "TestError" in content
        assert "/test/path.jsonl" in content
        assert "Test message" in content


class TestClaudeAdapterErrorHandling:
    """Tests for Claude adapter error handling."""

    def test_oserror_calls_on_error_callback(self, temp_dir):
        """Test that OSError during parsing calls the on_error callback."""
        project_dir = temp_dir / "projects" / "test-project"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "unreadable.jsonl"

        # Create file then make it unreadable
        session_file.write_text('{"type": "user", "message": {"content": "test"}}')
        os.chmod(session_file, 0o000)

        errors: list[ParseError] = []

        def on_error(error: ParseError):
            errors.append(error)

        adapter = ClaudeAdapter(sessions_dir=temp_dir / "projects")
        result = adapter._parse_session_file(session_file, on_error=on_error)

        # Restore permissions for cleanup
        os.chmod(session_file, 0o644)

        assert result is None
        assert len(errors) == 1
        assert errors[0].agent == "claude"
        assert errors[0].error_type == "OSError"
        assert "unreadable.jsonl" in errors[0].file_path

    def test_no_callback_still_returns_none_on_error(self, temp_dir):
        """Test that parsing errors return None even without callback."""
        project_dir = temp_dir / "projects" / "test-project"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "unreadable.jsonl"

        session_file.write_text('{"type": "user", "message": {"content": "test"}}')
        os.chmod(session_file, 0o000)

        adapter = ClaudeAdapter(sessions_dir=temp_dir / "projects")
        result = adapter._parse_session_file(session_file, on_error=None)

        os.chmod(session_file, 0o644)

        assert result is None

    def test_incremental_passes_on_error_to_parse(self, temp_dir):
        """Test that find_sessions_incremental passes on_error to parsing."""
        project_dir = temp_dir / "projects" / "test-project"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "test-session.jsonl"

        # Create valid file structure but make content cause parsing to work
        # then make file unreadable
        session_file.write_text('{"type": "user", "message": {"content": "test"}}')
        os.chmod(session_file, 0o000)

        errors: list[ParseError] = []

        def on_error(error: ParseError):
            errors.append(error)

        adapter = ClaudeAdapter(sessions_dir=temp_dir / "projects")
        # Note: On macOS, stat() works on chmod 000 files, so scanning succeeds
        # but reading fails - this is what we want to test
        sessions, deleted = adapter.find_sessions_incremental({}, on_error=on_error)

        os.chmod(session_file, 0o644)

        # Should have collected the error
        assert len(errors) == 1
        assert errors[0].error_type == "OSError"


class TestVibeAdapterErrorHandling:
    """Tests for Vibe adapter error handling."""

    def test_json_decode_error_calls_callback(self, temp_dir):
        """Test that JSONDecodeError calls on_error callback."""
        session_file = temp_dir / "session_test.json"
        session_file.write_text("not valid json at all")

        errors: list[ParseError] = []

        def on_error(error: ParseError):
            errors.append(error)

        adapter = VibeAdapter(sessions_dir=temp_dir)
        result = adapter._parse_session_file(session_file, on_error=on_error)

        assert result is None
        assert len(errors) == 1
        assert errors[0].agent == "vibe"
        assert errors[0].error_type == "JSONDecodeError"

    def test_oserror_calls_callback(self, temp_dir):
        """Test that OSError calls on_error callback for Vibe."""
        session_file = temp_dir / "session_unreadable.json"
        session_file.write_text('{"metadata": {}}')
        os.chmod(session_file, 0o000)

        errors: list[ParseError] = []

        def on_error(error: ParseError):
            errors.append(error)

        adapter = VibeAdapter(sessions_dir=temp_dir)
        result = adapter._parse_session_file(session_file, on_error=on_error)

        os.chmod(session_file, 0o644)

        assert result is None
        assert len(errors) == 1
        assert errors[0].error_type == "OSError"


class TestCodexAdapterErrorHandling:
    """Tests for Codex adapter error handling."""

    def test_oserror_calls_callback(self, temp_dir):
        """Test that OSError calls on_error callback for Codex."""
        sessions_dir = temp_dir / "sessions"
        sessions_dir.mkdir(parents=True)
        session_file = sessions_dir / "rollout-test.jsonl"
        session_file.write_text('{"type": "session_meta", "payload": {"id": "test"}}')
        os.chmod(session_file, 0o000)

        errors: list[ParseError] = []

        def on_error(error: ParseError):
            errors.append(error)

        adapter = CodexAdapter(sessions_dir=sessions_dir)
        result = adapter._parse_session_file(session_file, on_error=on_error)

        os.chmod(session_file, 0o644)

        assert result is None
        assert len(errors) == 1
        assert errors[0].agent == "codex"
        assert errors[0].error_type == "OSError"


class TestSearchEngineErrorPropagation:
    """Tests for error propagation through SessionSearch."""

    def test_streaming_collects_errors(self, temp_dir, monkeypatch):
        """Test that get_sessions_streaming collects errors from adapters."""
        from fast_resume.search import SessionSearch

        # Create a search engine with a temp index
        index_dir = temp_dir / "index"
        monkeypatch.setattr("fast_resume.index.INDEX_DIR", index_dir)

        # Create unreadable Claude session
        claude_dir = temp_dir / "claude" / "project"
        claude_dir.mkdir(parents=True)
        session_file = claude_dir / "test.jsonl"
        session_file.write_text(
            '{"type": "user", "cwd": "/tmp", "message": {"content": "test"}}'
        )
        os.chmod(session_file, 0o000)

        # Create search engine with custom adapter
        search = SessionSearch()
        # Replace adapters with just our test adapter
        search.adapters = [ClaudeAdapter(sessions_dir=temp_dir / "claude")]

        errors: list[ParseError] = []

        def on_error(error: ParseError):
            errors.append(error)

        def on_progress():
            pass

        search.get_sessions_streaming(on_progress, on_error=on_error)

        os.chmod(session_file, 0o644)

        # Should have collected the error
        assert len(errors) == 1
        assert errors[0].agent == "claude"
