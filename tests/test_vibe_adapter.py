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
def vibe_session_folder(temp_dir):
    """Create a mock Vibe session folder with meta.json and messages.jsonl."""
    session_dir = temp_dir / "session_20251220_100000_abc12345"
    session_dir.mkdir()

    # Write meta.json
    metadata = {
        "session_id": "abc12345-full-session-id",
        "start_time": "2025-12-20T10:00:00",
        "end_time": "2025-12-20T10:30:00",
        "environment": {"working_directory": "/home/user/project"},
        "title": "Build REST API",
        "total_messages": 4,
        "config": {"auto_approve": False},
    }
    with open(session_dir / "meta.json", "w") as f:
        json.dump(metadata, f)

    # Write messages.jsonl
    messages = [
        {"role": "system", "content": "You are a helpful assistant."},
        {"role": "user", "content": "Help me write a REST API"},
        {
            "role": "assistant",
            "content": "I'll help you create a REST API. Let's start.",
        },
        {"role": "user", "content": "Start with authentication"},
        {"role": "assistant", "content": "Here's the auth endpoint..."},
    ]
    with open(session_dir / "messages.jsonl", "w") as f:
        for msg in messages:
            f.write(json.dumps(msg) + "\n")

    return session_dir


class TestVibeAdapter:
    """Tests for VibeAdapter."""

    def test_name_and_attributes(self, adapter):
        """Test adapter has correct name and attributes."""
        assert adapter.name == "vibe"
        assert adapter.color is not None
        assert adapter.badge == "vibe"
        assert adapter.supports_yolo is True

    def test_parse_session_basic(self, temp_dir, vibe_session_folder):
        """Test parsing a Vibe session folder."""
        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(vibe_session_folder)

        assert session is not None
        assert session.agent == "vibe"
        assert session.id == "abc12345-full-session-id"
        assert session.directory == "/home/user/project"
        assert session.title == "Build REST API"
        assert "Help me write a REST API" in session.content
        assert "I'll help you create" in session.content

    def test_parse_session_skips_system_messages(self, temp_dir, vibe_session_folder):
        """Test that system messages are skipped in content."""
        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(vibe_session_folder)

        assert session is not None
        assert "You are a helpful assistant" not in session.content

    def test_parse_session_with_list_content(self, temp_dir):
        """Test parsing session with list-style content."""
        session_dir = temp_dir / "session_20251220_100000_list1234"
        session_dir.mkdir()

        metadata = {
            "session_id": "list1234",
            "environment": {"working_directory": "/test"},
        }
        with open(session_dir / "meta.json", "w") as f:
            json.dump(metadata, f)

        messages = [
            {
                "role": "user",
                "content": [{"type": "text", "text": "Multi-part message"}],
            },
            {
                "role": "assistant",
                "content": [{"type": "text", "text": "Response here"}],
            },
        ]
        with open(session_dir / "messages.jsonl", "w") as f:
            for msg in messages:
                f.write(json.dumps(msg) + "\n")

        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(session_dir)

        assert session is not None
        assert "Multi-part message" in session.content
        assert "Response here" in session.content

    def test_parse_session_extracts_id_from_metadata(self, temp_dir):
        """Test session ID extraction from metadata."""
        session_dir = temp_dir / "session_20251220_100000_different"
        session_dir.mkdir()

        metadata = {
            "session_id": "actual-session-id",
            "environment": {"working_directory": "/test"},
        }
        with open(session_dir / "meta.json", "w") as f:
            json.dump(metadata, f)

        with open(session_dir / "messages.jsonl", "w") as f:
            f.write('{"role": "user", "content": "Test message"}\n')

        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(session_dir)

        assert session is not None
        assert session.id == "actual-session-id"

    def test_parse_session_uses_dirname_as_fallback_id(self, temp_dir):
        """Test that dirname is used as fallback ID."""
        session_dir = temp_dir / "session_20251220_100000_fallback"
        session_dir.mkdir()

        metadata = {"environment": {"working_directory": "/test"}}
        with open(session_dir / "meta.json", "w") as f:
            json.dump(metadata, f)

        with open(session_dir / "messages.jsonl", "w") as f:
            f.write('{"role": "user", "content": "Test message"}\n')

        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(session_dir)

        assert session is not None
        assert session.id == "session_20251220_100000_fallback"

    def test_parse_session_uses_file_mtime_without_timestamp(self, temp_dir):
        """Test that file mtime is used when start_time is missing."""
        session_dir = temp_dir / "session_20251220_100000_notime"
        session_dir.mkdir()

        metadata = {
            "session_id": "test",
            "environment": {"working_directory": "/test"},
        }
        with open(session_dir / "meta.json", "w") as f:
            json.dump(metadata, f)

        with open(session_dir / "messages.jsonl", "w") as f:
            f.write('{"role": "user", "content": "Test"}\n')

        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(session_dir)

        assert session is not None
        assert session.timestamp.year >= 2024

    def test_parse_session_handles_invalid_timestamp(self, temp_dir):
        """Test handling of invalid timestamp format."""
        session_dir = temp_dir / "session_20251220_100000_badtime"
        session_dir.mkdir()

        metadata = {
            "session_id": "test",
            "start_time": "not a valid timestamp",
            "environment": {"working_directory": "/test"},
        }
        with open(session_dir / "meta.json", "w") as f:
            json.dump(metadata, f)

        with open(session_dir / "messages.jsonl", "w") as f:
            f.write('{"role": "user", "content": "Test"}\n')

        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(session_dir)

        assert session is not None
        # Should fall back to file mtime
        assert session.timestamp.year >= 2024

    def test_parse_session_uses_title_from_metadata(
        self, temp_dir, vibe_session_folder
    ):
        """Test that title is taken from metadata when available."""
        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(vibe_session_folder)

        assert session is not None
        assert session.title == "Build REST API"

    def test_parse_session_generates_title_when_missing(self, temp_dir):
        """Test title generation when not in metadata."""
        session_dir = temp_dir / "session_20251220_100000_notitle"
        session_dir.mkdir()

        metadata = {
            "session_id": "test",
            "environment": {"working_directory": "/test"},
        }
        with open(session_dir / "meta.json", "w") as f:
            json.dump(metadata, f)

        messages = [
            {"role": "user", "content": "Implement OAuth2 authentication for the API"},
            {"role": "assistant", "content": "I'll implement OAuth2 for you."},
        ]
        with open(session_dir / "messages.jsonl", "w") as f:
            for msg in messages:
                f.write(json.dumps(msg) + "\n")

        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(session_dir)

        assert session is not None
        assert "Implement OAuth2 authentication" in session.title

    def test_parse_session_truncates_long_title(self, temp_dir):
        """Test that long titles are truncated."""
        session_dir = temp_dir / "session_20251220_100000_long"
        session_dir.mkdir()

        long_message = "A" * 200
        metadata = {
            "session_id": "test",
            "environment": {"working_directory": "/test"},
        }
        with open(session_dir / "meta.json", "w") as f:
            json.dump(metadata, f)

        with open(session_dir / "messages.jsonl", "w") as f:
            f.write(json.dumps({"role": "user", "content": long_message}) + "\n")

        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(session_dir)

        assert session is not None
        assert len(session.title) <= 83  # 80 + "..."
        assert session.title.endswith("...")

    def test_parse_session_default_title_when_no_user_message(self, temp_dir):
        """Test default title when no user messages exist."""
        session_dir = temp_dir / "session_20251220_100000_nouser"
        session_dir.mkdir()

        metadata = {
            "session_id": "test",
            "environment": {"working_directory": "/test"},
        }
        with open(session_dir / "meta.json", "w") as f:
            json.dump(metadata, f)

        with open(session_dir / "messages.jsonl", "w") as f:
            f.write('{"role": "assistant", "content": "How can I help?"}\n')

        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(session_dir)

        assert session is not None
        assert session.title == "Vibe session"

    def test_parse_session_handles_malformed_jsonl_lines(self, temp_dir):
        """Test that malformed lines in messages.jsonl are skipped."""
        session_dir = temp_dir / "session_20251220_100000_badjsonl"
        session_dir.mkdir()

        metadata = {
            "session_id": "bad12345",
            "environment": {"working_directory": "/test"},
        }
        with open(session_dir / "meta.json", "w") as f:
            json.dump(metadata, f)

        with open(session_dir / "messages.jsonl", "w") as f:
            f.write('{"role": "user", "content": "Good message"}\n')
            f.write("not valid json\n")
            f.write('{"role": "assistant", "content": "Another good message"}\n')

        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(session_dir)

        assert session is not None
        assert "Good message" in session.content
        assert "Another good message" in session.content

    def test_parse_session_without_messages_file(self, temp_dir):
        """Test parsing folder without messages.jsonl file."""
        session_dir = temp_dir / "session_20251220_100000_nomsg"
        session_dir.mkdir()

        metadata = {
            "session_id": "nomsg123",
            "environment": {"working_directory": "/test"},
            "title": "Empty session",
        }
        with open(session_dir / "meta.json", "w") as f:
            json.dump(metadata, f)

        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(session_dir)

        assert session is not None
        assert session.id == "nomsg123"
        assert session.title == "Empty session"
        assert session.content == ""

    def test_parse_session_without_meta_returns_none(self, temp_dir):
        """Test that folder without meta.json returns None."""
        session_dir = temp_dir / "session_20251220_100000_nometa"
        session_dir.mkdir()

        # Only messages file, no meta.json
        with open(session_dir / "messages.jsonl", "w") as f:
            f.write('{"role": "user", "content": "Test"}\n')

        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(session_dir)

        assert session is None

    def test_parse_session_with_auto_approve(self, temp_dir):
        """Test parsing session with auto_approve enabled."""
        session_dir = temp_dir / "session_20251220_100000_yolo"
        session_dir.mkdir()

        metadata = {
            "session_id": "yolo1234",
            "start_time": "2025-12-20T12:00:00",
            "environment": {"working_directory": "/test"},
            "config": {"auto_approve": True},
        }
        with open(session_dir / "meta.json", "w") as f:
            json.dump(metadata, f)

        with open(session_dir / "messages.jsonl", "w") as f:
            f.write('{"role": "user", "content": "Test"}\n')

        adapter = VibeAdapter(sessions_dir=temp_dir)
        session = adapter._parse_session_file(session_dir)

        assert session is not None
        assert session.yolo is True

    def test_get_resume_command(self, adapter):
        """Test resume command generation."""
        from fast_resume.adapters.base import Session

        session = Session(
            id="vibe-abc123",
            agent="vibe",
            title="Test",
            directory="/test",
            timestamp=datetime.now(),
            content="",
        )

        cmd = adapter.get_resume_command(session)

        assert cmd == ["vibe", "--resume", "vibe-abc123"]

    def test_get_resume_command_with_yolo(self, adapter):
        """Test resume command generation with yolo flag."""
        from fast_resume.adapters.base import Session

        session = Session(
            id="vibe-abc123",
            agent="vibe",
            title="Test",
            directory="/test",
            timestamp=datetime.now(),
            content="",
        )

        cmd = adapter.get_resume_command(session, yolo=True)

        assert cmd == ["vibe", "--auto-approve", "--resume", "vibe-abc123"]

    def test_find_sessions(self, temp_dir):
        """Test finding all Vibe sessions."""
        # Create multiple session folders
        for i in range(3):
            session_dir = temp_dir / f"session_20251220_10000{i}_test{i}"
            session_dir.mkdir()

            metadata = {
                "session_id": f"session-{i}",
                "environment": {"working_directory": f"/project{i}"},
            }
            with open(session_dir / "meta.json", "w") as f:
                json.dump(metadata, f)

            with open(session_dir / "messages.jsonl", "w") as f:
                f.write(json.dumps({"role": "user", "content": f"Message {i}"}) + "\n")

        adapter = VibeAdapter(sessions_dir=temp_dir)
        sessions = adapter.find_sessions()

        assert len(sessions) == 3

    def test_find_sessions_only_matches_session_folders(self, temp_dir):
        """Test that only session_* folders are matched."""
        # Create a valid session folder
        session_dir = temp_dir / "session_20251220_100000_valid"
        session_dir.mkdir()

        metadata = {
            "session_id": "valid",
            "environment": {"working_directory": "/test"},
        }
        with open(session_dir / "meta.json", "w") as f:
            json.dump(metadata, f)

        with open(session_dir / "messages.jsonl", "w") as f:
            f.write('{"role": "user", "content": "Test"}\n')

        # Create other files/folders that should be ignored
        other_dir = temp_dir / "config"
        other_dir.mkdir()

        other_file = temp_dir / "session_legacy.json"
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

    def test_scan_session_files(self, temp_dir):
        """Test that _scan_session_files discovers session folders."""
        session_dir = temp_dir / "session_20251220_100000_scan1234"
        session_dir.mkdir()
        with open(session_dir / "meta.json", "w") as f:
            json.dump(
                {
                    "session_id": "scan-session",
                    "start_time": "2025-12-20T10:00:00",
                    "environment": {"working_directory": "/test"},
                },
                f,
            )

        adapter = VibeAdapter(sessions_dir=temp_dir)
        files = adapter._scan_session_files()

        assert "scan-session" in files
        path, mtime = files["scan-session"]
        assert path == session_dir
