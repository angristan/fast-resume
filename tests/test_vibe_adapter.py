"""Tests for Vibe (Mistral) session adapter."""

import json
from datetime import datetime
from unittest.mock import patch

import pytest

from fast_resume.adapters.vibe import VibeAdapter


@pytest.fixture
def adapter():
    """Create a VibeAdapter instance."""
    return VibeAdapter()


@pytest.fixture
def vibe_session_data():
    """Sample Vibe session JSON data."""
    return {
        "metadata": {
            "session_id": "vibe-session-001",
            "start_time": "2025-12-20T10:00:00",
            "environment": {"working_directory": "/home/user/project"},
        },
        "messages": [
            {"role": "system", "content": "You are a helpful assistant."},
            {"role": "user", "content": "Help me write a REST API"},
            {
                "role": "assistant",
                "content": "I'll help you create a REST API. Let's start with the endpoints.",
            },
            {"role": "user", "content": "Start with user authentication"},
            {"role": "assistant", "content": "Here's the authentication endpoint..."},
        ],
    }


@pytest.fixture
def vibe_session_file(temp_dir, vibe_session_data):
    """Create a mock Vibe session file."""
    session_file = temp_dir / "session_vibe-session-001.json"
    with open(session_file, "w") as f:
        json.dump(vibe_session_data, f)
    return session_file


class TestVibeAdapter:
    """Tests for VibeAdapter."""

    def test_name_and_attributes(self, adapter):
        """Test adapter has correct name and attributes."""
        assert adapter.name == "vibe"
        assert adapter.color is not None
        assert adapter.badge == "vibe"

    def test_parse_session_basic(self, adapter, vibe_session_file):
        """Test parsing a basic Vibe session file."""
        session = adapter._parse_session_file(vibe_session_file)

        assert session is not None
        assert session.agent == "vibe"
        assert session.id == "vibe-session-001"
        assert session.directory == "/home/user/project"
        assert "Help me write a REST API" in session.title
        assert "Help me write a REST API" in session.content
        assert "I'll help you create" in session.content

    def test_parse_session_skips_system_messages(self, adapter, vibe_session_file):
        """Test that system messages are skipped in content."""
        session = adapter._parse_session_file(vibe_session_file)

        assert session is not None
        assert "You are a helpful assistant" not in session.content

    def test_parse_session_with_list_content(self, adapter, temp_dir):
        """Test parsing session with list-style content."""
        session_file = temp_dir / "session_test.json"
        data = {
            "metadata": {
                "session_id": "test-001",
                "start_time": "2025-12-20T10:00:00",
                "environment": {"working_directory": "/test"},
            },
            "messages": [
                {
                    "role": "user",
                    "content": [{"type": "text", "text": "Multi-part message"}],
                },
                {
                    "role": "assistant",
                    "content": [{"type": "text", "text": "Response here"}],
                },
            ],
        }
        with open(session_file, "w") as f:
            json.dump(data, f)

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert "Multi-part message" in session.content
        assert "Response here" in session.content

    def test_parse_session_extracts_id_from_metadata(self, adapter, temp_dir):
        """Test session ID extraction from metadata."""
        session_file = temp_dir / "session_different_name.json"
        data = {
            "metadata": {
                "session_id": "actual-session-id",
                "environment": {"working_directory": "/test"},
            },
            "messages": [
                {"role": "user", "content": "Test message"},
            ],
        }
        with open(session_file, "w") as f:
            json.dump(data, f)

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert session.id == "actual-session-id"

    def test_parse_session_uses_filename_as_fallback_id(self, adapter, temp_dir):
        """Test that filename is used as fallback ID."""
        session_file = temp_dir / "session_fallback123.json"
        data = {
            "metadata": {"environment": {"working_directory": "/test"}},
            "messages": [
                {"role": "user", "content": "Test message"},
            ],
        }
        with open(session_file, "w") as f:
            json.dump(data, f)

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert session.id == "session_fallback123"

    def test_parse_session_uses_file_mtime_without_timestamp(self, adapter, temp_dir):
        """Test that file mtime is used when start_time is missing."""
        session_file = temp_dir / "session_no_time.json"
        data = {
            "metadata": {
                "session_id": "test",
                "environment": {"working_directory": "/test"},
            },
            "messages": [
                {"role": "user", "content": "Test"},
            ],
        }
        with open(session_file, "w") as f:
            json.dump(data, f)

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert session.timestamp.year >= 2024

    def test_parse_session_handles_invalid_timestamp(self, adapter, temp_dir):
        """Test handling of invalid timestamp format."""
        session_file = temp_dir / "session_bad_time.json"
        data = {
            "metadata": {
                "session_id": "test",
                "start_time": "not a valid timestamp",
                "environment": {"working_directory": "/test"},
            },
            "messages": [
                {"role": "user", "content": "Test"},
            ],
        }
        with open(session_file, "w") as f:
            json.dump(data, f)

        session = adapter._parse_session_file(session_file)

        assert session is not None
        # Should fall back to file mtime
        assert session.timestamp.year >= 2024

    def test_parse_session_generates_title_from_first_user_message(
        self, adapter, temp_dir
    ):
        """Test title generation from first user message."""
        session_file = temp_dir / "session_title.json"
        data = {
            "metadata": {
                "session_id": "test",
                "environment": {"working_directory": "/test"},
            },
            "messages": [
                {
                    "role": "user",
                    "content": "Implement OAuth2 authentication for the API",
                },
                {"role": "assistant", "content": "I'll implement OAuth2 for you."},
            ],
        }
        with open(session_file, "w") as f:
            json.dump(data, f)

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert "Implement OAuth2 authentication" in session.title

    def test_parse_session_truncates_long_title(self, adapter, temp_dir):
        """Test that long titles are truncated."""
        session_file = temp_dir / "session_long.json"
        long_message = "A" * 200
        data = {
            "metadata": {
                "session_id": "test",
                "environment": {"working_directory": "/test"},
            },
            "messages": [
                {"role": "user", "content": long_message},
            ],
        }
        with open(session_file, "w") as f:
            json.dump(data, f)

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert len(session.title) <= 83  # 80 + "..."
        assert session.title.endswith("...")

    def test_parse_session_default_title_when_no_user_message(self, adapter, temp_dir):
        """Test default title when no user messages exist."""
        session_file = temp_dir / "session_no_user.json"
        data = {
            "metadata": {
                "session_id": "test",
                "environment": {"working_directory": "/test"},
            },
            "messages": [
                {"role": "assistant", "content": "How can I help?"},
            ],
        }
        with open(session_file, "w") as f:
            json.dump(data, f)

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert session.title == "Vibe session"

    def test_parse_session_handles_malformed_file(self, adapter, temp_dir):
        """Test handling of malformed JSON."""
        session_file = temp_dir / "session_bad.json"
        with open(session_file, "w") as f:
            f.write("not valid json")

        session = adapter._parse_session_file(session_file)

        assert session is None

    def test_get_resume_command(self, adapter):
        """Test resume command generation."""
        from fast_resume.adapters.base import Session

        session = Session(
            id="vibe-abc123",
            agent="vibe",
            title="Test",
            directory="/test",
            timestamp=datetime.now(),
            preview="",
            content="",
        )

        cmd = adapter.get_resume_command(session)

        assert cmd == ["vibe", "--resume", "vibe-abc123"]

    def test_find_sessions(self, temp_dir):
        """Test finding all Vibe sessions."""
        # Create multiple session files
        for i in range(3):
            session_file = temp_dir / f"session_test{i}.json"
            data = {
                "metadata": {
                    "session_id": f"session-{i}",
                    "environment": {"working_directory": f"/project{i}"},
                },
                "messages": [
                    {"role": "user", "content": f"Message {i}"},
                ],
            }
            with open(session_file, "w") as f:
                json.dump(data, f)

        adapter = VibeAdapter(sessions_dir=temp_dir)
        sessions = adapter.find_sessions()

        assert len(sessions) == 3

    def test_find_sessions_only_matches_session_files(self, temp_dir):
        """Test that only session_*.json files are matched."""
        # Create a valid session file
        session_file = temp_dir / "session_valid.json"
        data = {
            "metadata": {
                "session_id": "valid",
                "environment": {"working_directory": "/test"},
            },
            "messages": [{"role": "user", "content": "Test"}],
        }
        with open(session_file, "w") as f:
            json.dump(data, f)

        # Create other JSON files that should be ignored
        other_file = temp_dir / "config.json"
        with open(other_file, "w") as f:
            json.dump({"not": "a session"}, f)

        adapter = VibeAdapter(sessions_dir=temp_dir)
        sessions = adapter.find_sessions()

        assert len(sessions) == 1
        assert sessions[0].id == "valid"

    def test_find_sessions_returns_empty_when_unavailable(self, adapter):
        """Test that find_sessions returns empty list when unavailable."""
        with patch.object(adapter, "is_available", return_value=False):
            sessions = adapter.find_sessions()
            assert sessions == []
