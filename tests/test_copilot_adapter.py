"""Tests for GitHub Copilot CLI session adapter."""

import json
from datetime import datetime

import pytest

from fast_resume.adapters.copilot import CopilotAdapter


@pytest.fixture
def adapter():
    """Create a CopilotAdapter instance."""
    return CopilotAdapter()


@pytest.fixture
def copilot_session_data():
    """Sample Copilot CLI session JSONL data."""
    return [
        {
            "type": "session.start",
            "data": {
                "sessionId": "abc123-def456",
                "version": 1,
                "producer": "copilot-agent",
            },
            "timestamp": "2025-12-20T10:00:00.000Z",
        },
        {
            "type": "session.info",
            "data": {
                "infoType": "folder_trust",
                "message": "Folder /home/user/project has been added to trusted folders.",
            },
            "timestamp": "2025-12-20T10:00:01.000Z",
        },
        {
            "type": "user.message",
            "data": {
                "content": "Help me implement a REST API endpoint",
                "attachments": [],
            },
            "timestamp": "2025-12-20T10:00:10.000Z",
        },
        {
            "type": "assistant.message",
            "data": {
                "content": "I'll help you create a REST API endpoint. Let me look at your project structure.",
            },
            "timestamp": "2025-12-20T10:00:15.000Z",
        },
        {
            "type": "user.message",
            "data": {"content": "It should handle POST requests"},
            "timestamp": "2025-12-20T10:00:20.000Z",
        },
        {
            "type": "assistant.message",
            "data": {"content": "I'll create a POST endpoint for you."},
            "timestamp": "2025-12-20T10:00:25.000Z",
        },
    ]


@pytest.fixture
def copilot_session_file(temp_dir, copilot_session_data):
    """Create a mock Copilot session file."""
    session_dir = temp_dir / "session-state"
    session_dir.mkdir(parents=True)
    session_file = session_dir / "session-abc123.jsonl"

    with open(session_file, "w") as f:
        for entry in copilot_session_data:
            f.write(json.dumps(entry) + "\n")

    return session_file


class TestCopilotAdapter:
    """Tests for CopilotAdapter."""

    def test_name_and_attributes(self, adapter):
        """Test adapter has correct name and attributes."""
        assert adapter.name == "copilot-cli"
        assert adapter.color is not None
        assert adapter.badge == "copilot"

    def test_parse_session_basic(self, adapter, copilot_session_file):
        """Test parsing a basic Copilot session file."""
        session = adapter._parse_session_file(copilot_session_file)

        assert session is not None
        assert session.agent == "copilot-cli"
        assert session.id == "abc123-def456"
        assert session.title == "Help me implement a REST API endpoint"
        assert session.directory == "/home/user/project"
        assert "Help me implement a REST API endpoint" in session.content
        assert "I'll help you create a REST API endpoint" in session.content

    def test_parse_session_extracts_session_id(self, adapter, temp_dir):
        """Test that session ID is extracted from session.start."""
        session_dir = temp_dir / "session-state"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "fallback-id.jsonl"

        data = [
            {
                "type": "session.start",
                "data": {"sessionId": "real-session-id-123"},
            },
            {
                "type": "user.message",
                "data": {"content": "Hello world test message"},
            },
            {
                "type": "assistant.message",
                "data": {"content": "Hi there!"},
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert session.id == "real-session-id-123"

    def test_parse_session_uses_filename_as_fallback_id(self, adapter, temp_dir):
        """Test that filename is used as ID when session.start is missing."""
        session_dir = temp_dir / "session-state"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "my-session-file.jsonl"

        data = [
            {
                "type": "user.message",
                "data": {"content": "Test message without session start"},
            },
            {
                "type": "assistant.message",
                "data": {"content": "Response"},
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert session.id == "my-session-file"

    def test_parse_session_extracts_directory_from_folder_trust(
        self, adapter, temp_dir
    ):
        """Test directory extraction from folder_trust message."""
        session_dir = temp_dir / "session-state"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "session-dir.jsonl"

        data = [
            {
                "type": "session.info",
                "data": {
                    "infoType": "folder_trust",
                    "message": "Folder /Users/dev/my-project has been added to trusted folders.",
                },
            },
            {
                "type": "user.message",
                "data": {"content": "Test directory extraction"},
            },
            {
                "type": "assistant.message",
                "data": {"content": "Response"},
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert session.directory == "/Users/dev/my-project"

    def test_parse_session_counts_human_turns(self, adapter, copilot_session_file):
        """Test that human turns are counted correctly."""
        session = adapter._parse_session_file(copilot_session_file)

        assert session is not None
        assert session.message_count == 4  # Two user + two assistant turns

    def test_parse_empty_session_returns_none(self, adapter, temp_dir):
        """Test that empty sessions return None."""
        session_dir = temp_dir / "session-state"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "session-empty.jsonl"
        session_file.touch()

        session = adapter._parse_session_file(session_file)

        assert session is None

    def test_parse_session_no_user_message_returns_none(self, adapter, temp_dir):
        """Test that sessions with no user messages return None."""
        session_dir = temp_dir / "session-state"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "session-no-user.jsonl"

        data = [
            {
                "type": "session.start",
                "data": {"sessionId": "test"},
            },
            {
                "type": "assistant.message",
                "data": {"content": "Just an assistant message"},
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is None

    def test_parse_malformed_json_lines(self, adapter, temp_dir):
        """Test that malformed JSON lines are skipped gracefully."""
        session_dir = temp_dir / "session-state"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "session-malformed.jsonl"

        with open(session_file, "w") as f:
            f.write("not valid json\n")
            f.write(
                json.dumps(
                    {
                        "type": "user.message",
                        "data": {"content": "Valid message here"},
                    }
                )
                + "\n"
            )
            f.write("{broken json\n")
            f.write(
                json.dumps(
                    {"type": "assistant.message", "data": {"content": "Response"}}
                )
                + "\n"
            )

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert "Valid message" in session.content

    def test_get_resume_command(self, adapter):
        """Test resume command generation."""
        from fast_resume.adapters.base import Session

        session = Session(
            id="abc123-def456-ghi789",
            agent="copilot",
            title="Test",
            directory="/test",
            timestamp=datetime.now(),
            preview="",
            content="",
        )

        cmd = adapter.get_resume_command(session)

        assert cmd == ["copilot", "--resume", "abc123-def456-ghi789"]

    def test_find_sessions(self, temp_dir):
        """Test finding sessions in the session directory."""
        session_dir = temp_dir / "session-state"
        session_dir.mkdir(parents=True)

        # Create two session files
        for i in range(2):
            session_file = session_dir / f"session-{i:03d}.jsonl"
            data = [
                {
                    "type": "user.message",
                    "data": {"content": f"Session {i} user message here"},
                },
                {
                    "type": "assistant.message",
                    "data": {"content": f"Session {i} response"},
                },
            ]
            with open(session_file, "w") as f:
                for entry in data:
                    f.write(json.dumps(entry) + "\n")

        adapter = CopilotAdapter(sessions_dir=session_dir)
        sessions = adapter.find_sessions()

        assert len(sessions) == 2

    def test_is_available_when_dir_exists(self, temp_dir):
        """Test is_available returns True when directory exists."""
        session_dir = temp_dir / "session-state"
        session_dir.mkdir(parents=True)

        adapter = CopilotAdapter(sessions_dir=session_dir)
        assert adapter.is_available() is True

    def test_is_available_when_dir_missing(self, temp_dir):
        """Test is_available returns False when directory doesn't exist."""
        missing_dir = temp_dir / "nonexistent"

        adapter = CopilotAdapter(sessions_dir=missing_dir)
        assert adapter.is_available() is False

    def test_parse_session_ignores_other_info_types(self, adapter, temp_dir):
        """Test that non-folder_trust info types don't set directory."""
        session_dir = temp_dir / "session-state"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "session-info.jsonl"

        data = [
            {
                "type": "session.info",
                "data": {
                    "infoType": "authentication",
                    "message": "Logged in as user",
                },
            },
            {
                "type": "session.info",
                "data": {
                    "infoType": "mcp",
                    "message": "Connected to MCP Server",
                },
            },
            {
                "type": "user.message",
                "data": {"content": "Test without folder trust"},
            },
            {
                "type": "assistant.message",
                "data": {"content": "Response"},
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert session.directory == ""  # No folder_trust info

    def test_parse_session_truncates_long_title(self, adapter, temp_dir):
        """Test that long titles are truncated properly."""
        session_dir = temp_dir / "session-state"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "session-long.jsonl"

        long_message = "This is a very long message that should be truncated " * 5

        data = [
            {
                "type": "user.message",
                "data": {"content": long_message},
            },
            {
                "type": "assistant.message",
                "data": {"content": "Response"},
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert len(session.title) <= 103  # 100 chars + "..."
        assert session.title.endswith("...")
