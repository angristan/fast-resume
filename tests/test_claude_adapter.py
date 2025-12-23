"""Tests for Claude Code session adapter."""

import json
from datetime import datetime

import pytest

from fast_resume.adapters.claude import ClaudeAdapter


@pytest.fixture
def adapter():
    """Create a ClaudeAdapter instance."""
    return ClaudeAdapter()


@pytest.fixture
def claude_session_data():
    """Sample Claude Code session JSONL data."""
    return [
        {
            "type": "user",
            "cwd": "/home/user/project",
            "message": {"content": "Help me fix this bug in the login system"},
        },
        {
            "type": "assistant",
            "message": {
                "content": "I'll help you fix the bug. Let me look at the code."
            },
        },
        {"type": "user", "message": {"content": "Thanks, it's in auth.py"}},
        {
            "type": "assistant",
            "message": {
                "content": "I see the issue. The token validation is incorrect."
            },
        },
        {"type": "summary", "summary": "Fix login bug in auth.py"},
    ]


@pytest.fixture
def claude_session_file(temp_dir, claude_session_data):
    """Create a mock Claude session file."""
    project_dir = temp_dir / "projects" / "project-abc123"
    project_dir.mkdir(parents=True)
    session_file = project_dir / "session-001.jsonl"

    with open(session_file, "w") as f:
        for entry in claude_session_data:
            f.write(json.dumps(entry) + "\n")

    return session_file


class TestClaudeAdapter:
    """Tests for ClaudeAdapter."""

    def test_name_and_attributes(self, adapter):
        """Test adapter has correct name and attributes."""
        assert adapter.name == "claude"
        assert adapter.color is not None
        assert adapter.badge == "claude"

    def test_parse_session_basic(self, adapter, claude_session_file):
        """Test parsing a basic Claude session file."""
        session = adapter._parse_session_file(claude_session_file)

        assert session is not None
        assert session.agent == "claude"
        assert session.title == "Fix login bug in auth.py"
        assert session.directory == "/home/user/project"
        assert "Help me fix this bug" in session.content
        assert "I'll help you fix the bug" in session.content

    def test_parse_session_without_summary(self, adapter, temp_dir):
        """Test parsing session without summary uses first user message as title."""
        project_dir = temp_dir / "projects" / "project-abc123"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "session-002.jsonl"

        data = [
            {
                "type": "user",
                "cwd": "/home/user/project",
                "message": {
                    "content": "Implement a new feature for user authentication"
                },
            },
            {
                "type": "assistant",
                "message": {"content": "I'll implement that feature."},
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert "Implement a new feature" in session.title

    def test_parse_session_with_list_content(self, adapter, temp_dir):
        """Test parsing session with list-style content (multi-part messages)."""
        project_dir = temp_dir / "projects" / "project-abc123"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "session-003.jsonl"

        data = [
            {
                "type": "user",
                "cwd": "/test",
                "message": {
                    "content": [{"type": "text", "text": "Hello from list content"}]
                },
            },
            {
                "type": "assistant",
                "message": {"content": [{"type": "text", "text": "Response here"}]},
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert "Hello from list content" in session.content

    def test_parse_session_skips_meta_messages(self, adapter, temp_dir):
        """Test that meta messages are skipped."""
        project_dir = temp_dir / "projects" / "project-abc123"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "session-004.jsonl"

        data = [
            {
                "type": "user",
                "cwd": "/test",
                "isMeta": True,
                "message": {"content": "Meta message"},
            },
            {
                "type": "user",
                "cwd": "/test",
                "message": {"content": "Real user message here"},
            },
            {"type": "assistant", "message": {"content": "Response"}},
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert "Meta message" not in session.content
        assert "Real user message" in session.content

    def test_parse_session_skips_command_messages(self, adapter, temp_dir):
        """Test that command messages are skipped."""
        project_dir = temp_dir / "projects" / "project-abc123"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "session-005.jsonl"

        data = [
            {
                "type": "user",
                "cwd": "/test",
                "message": {"content": "<command>some command</command>"},
            },
            {
                "type": "user",
                "cwd": "/test",
                "message": {"content": "Actual question from user"},
            },
            {"type": "assistant", "message": {"content": "Response"}},
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert "<command>" not in session.content

    def test_parse_empty_session_returns_none(self, adapter, temp_dir):
        """Test that empty sessions return None."""
        project_dir = temp_dir / "projects" / "project-abc123"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "session-empty.jsonl"
        session_file.touch()

        session = adapter._parse_session_file(session_file)

        assert session is None

    def test_parse_session_no_user_message_returns_none(self, adapter, temp_dir):
        """Test that sessions with no user messages return None."""
        project_dir = temp_dir / "projects" / "project-abc123"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "session-no-user.jsonl"

        data = [
            {"type": "assistant", "message": {"content": "Just an assistant message"}},
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is None

    def test_parse_malformed_json_lines(self, adapter, temp_dir):
        """Test that malformed JSON lines are skipped gracefully."""
        project_dir = temp_dir / "projects" / "project-abc123"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "session-malformed.jsonl"

        with open(session_file, "w") as f:
            f.write("not valid json\n")
            f.write(
                json.dumps(
                    {
                        "type": "user",
                        "cwd": "/test",
                        "message": {"content": "Valid message"},
                    }
                )
                + "\n"
            )
            f.write("{broken json\n")
            f.write(
                json.dumps({"type": "assistant", "message": {"content": "Response"}})
                + "\n"
            )

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert "Valid message" in session.content

    def test_get_resume_command(self, adapter):
        """Test resume command generation."""
        from fast_resume.adapters.base import Session

        session = Session(
            id="session-abc123",
            agent="claude",
            title="Test",
            directory="/test",
            timestamp=datetime.now(),
            preview="",
            content="",
        )

        cmd = adapter.get_resume_command(session)

        assert cmd == ["claude", "--resume", "session-abc123"]

    def test_find_sessions_skips_agent_files(self, temp_dir):
        """Test that agent subprocess files are skipped."""
        project_dir = temp_dir / "project-abc"
        project_dir.mkdir(parents=True)

        # Create a regular session
        regular = project_dir / "session-001.jsonl"
        with open(regular, "w") as f:
            f.write(
                json.dumps(
                    {
                        "type": "user",
                        "cwd": "/test",
                        "message": {"content": "Regular session"},
                    }
                )
                + "\n"
            )
            f.write(
                json.dumps({"type": "assistant", "message": {"content": "Response"}})
                + "\n"
            )

        # Create an agent subprocess file (should be skipped)
        agent_file = project_dir / "agent-subprocess.jsonl"
        with open(agent_file, "w") as f:
            f.write(
                json.dumps(
                    {
                        "type": "user",
                        "cwd": "/test",
                        "message": {"content": "Agent subprocess"},
                    }
                )
                + "\n"
            )

        adapter = ClaudeAdapter(sessions_dir=temp_dir)
        sessions = adapter.find_sessions()

        assert len(sessions) == 1
        assert "Regular session" in sessions[0].content
