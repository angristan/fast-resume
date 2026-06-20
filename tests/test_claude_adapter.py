"""Tests for Claude Code session adapter."""

import json
import os
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
        assert adapter.supports_yolo is True

    def test_parse_session_basic(self, adapter, claude_session_file):
        """Test parsing a basic Claude session file."""
        session = adapter._parse_session_file(claude_session_file)

        assert session is not None
        assert session.agent == "claude"
        # Title uses first user message (matches Claude Code's Resume Session UI)
        assert session.title == "Help me fix this bug in the login system"
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

    def test_parse_session_prefers_sessions_index_summary(self, temp_dir):
        """Test that Claude /rename titles from sessions-index are used."""
        project_dir = temp_dir / "projects" / "project-abc123"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "session-rename.jsonl"

        data = [
            {
                "type": "user",
                "cwd": "/home/user/project",
                "message": {"content": "Original first prompt for this session"},
            },
            {
                "type": "assistant",
                "message": {"content": "Response"},
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        with open(project_dir / "sessions-index.json", "w") as f:
            json.dump(
                {
                    "version": 1,
                    "entries": [
                        {
                            "sessionId": "session-rename",
                            "summary": "Renamed Claude thread",
                            "modified": "2026-06-03T16:11:02.882Z",
                            "fileMtime": 1780503062882,
                        }
                    ],
                },
                f,
            )

        adapter = ClaudeAdapter(sessions_dir=temp_dir / "projects")
        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert session.title == "Renamed Claude thread"
        assert "Original first prompt" in session.content

    def test_parse_session_prefers_ai_title(self, adapter, temp_dir):
        """Test that Claude's auto-generated ai-title is used as the title."""
        project_dir = temp_dir / "projects" / "project-abc123"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "session-ai-title.jsonl"

        data = [
            {
                "type": "user",
                "cwd": "/home/user/project",
                "message": {"content": "Help me fix this bug in the login system"},
            },
            {
                "type": "ai-title",
                "aiTitle": "Fix login token validation",
                "sessionId": "session-ai-title",
            },
            {
                "type": "assistant",
                "message": {"content": "On it."},
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert session.title == "Fix login token validation"
        assert "Help me fix this bug" in session.content

    def test_parse_session_uses_latest_ai_title(self, adapter, temp_dir):
        """Test that the most recent ai-title wins when it is rewritten."""
        project_dir = temp_dir / "projects" / "project-abc123"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "session-ai-latest.jsonl"

        data = [
            {
                "type": "user",
                "cwd": "/home/user/project",
                "message": {"content": "Start working on something"},
            },
            {
                "type": "ai-title",
                "aiTitle": "First guess at the topic",
                "sessionId": "session-ai-latest",
            },
            {
                "type": "ai-title",
                "aiTitle": "What the session became",
                "sessionId": "session-ai-latest",
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert session.title == "What the session became"

    def test_parse_session_rename_overrides_ai_title(self, temp_dir):
        """Test that a /rename title takes precedence over the auto ai-title."""
        project_dir = temp_dir / "projects" / "project-abc123"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "session-rename-ai.jsonl"

        data = [
            {
                "type": "user",
                "cwd": "/home/user/project",
                "message": {"content": "Original first prompt for this session"},
            },
            {
                "type": "ai-title",
                "aiTitle": "Auto generated title",
                "sessionId": "session-rename-ai",
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        with open(project_dir / "sessions-index.json", "w") as f:
            json.dump(
                {
                    "version": 1,
                    "entries": [
                        {
                            "sessionId": "session-rename-ai",
                            "summary": "Renamed Claude thread",
                            "modified": "2026-06-03T16:11:02.882Z",
                            "fileMtime": 1780503062882,
                        }
                    ],
                },
                f,
            )

        adapter = ClaudeAdapter(sessions_dir=temp_dir / "projects")
        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert session.title == "Renamed Claude thread"

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

    def test_scan_skips_dangling_symlinks(self, temp_dir):
        """Test that dangling symlinks don't crash _scan_session_files."""
        project_dir = temp_dir / "project-abc"
        project_dir.mkdir(parents=True)

        # Create a valid session file
        valid_file = project_dir / "session-001.jsonl"
        with open(valid_file, "w") as f:
            f.write(
                json.dumps(
                    {
                        "type": "user",
                        "cwd": "/test",
                        "message": {"content": "Hello"},
                    }
                )
                + "\n"
            )

        # Create a dangling symlink
        dangling = project_dir / ".#session-002.jsonl"
        dangling.symlink_to(temp_dir / "nonexistent.jsonl")

        adapter = ClaudeAdapter(sessions_dir=temp_dir)
        files = adapter._scan_session_files()

        assert len(files) == 1
        assert "session-001" in files

    def test_incremental_detects_sessions_index_title_update(self, temp_dir):
        """Test that sessions-index mtime triggers Claude title refreshes."""
        project_dir = temp_dir / "project-abc"
        project_dir.mkdir(parents=True)
        session_file = project_dir / "session-rename.jsonl"
        sessions_index_file = project_dir / "sessions-index.json"

        data = [
            {
                "type": "user",
                "cwd": "/test",
                "message": {"content": "Original first prompt for this session"},
            },
            {"type": "assistant", "message": {"content": "Response"}},
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session_file_mtime = 1700000000.0
        os.utime(session_file, (session_file_mtime, session_file_mtime))

        with open(sessions_index_file, "w") as f:
            json.dump(
                {
                    "version": 1,
                    "entries": [
                        {
                            "sessionId": "session-rename",
                            "summary": "Updated Claude title",
                            "modified": "2023-01-01T00:00:00.000Z",
                            "fileMtime": int(session_file_mtime * 1000),
                        }
                    ],
                },
                f,
            )
        index_mtime = session_file_mtime + 10
        os.utime(sessions_index_file, (index_mtime, index_mtime))

        adapter = ClaudeAdapter(sessions_dir=temp_dir)
        new_or_modified, deleted_ids = adapter.find_sessions_incremental(
            {"session-rename": (session_file_mtime, "claude")}
        )

        assert deleted_ids == []
        assert len(new_or_modified) == 1
        assert new_or_modified[0].title == "Updated Claude title"
        assert new_or_modified[0].mtime == index_mtime
