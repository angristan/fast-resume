"""Tests for Codex CLI session adapter."""

import json
from datetime import datetime

import pytest

from fast_resume.adapters.codex import CodexAdapter


@pytest.fixture
def adapter():
    """Create a CodexAdapter instance."""
    return CodexAdapter()


@pytest.fixture
def codex_session_data():
    """Sample Codex CLI session JSONL data."""
    return [
        {
            "type": "session_meta",
            "payload": {"id": "abc123", "cwd": "/home/user/project"},
        },
        {
            "type": "event_msg",
            "payload": {
                "type": "user_message",
                "message": "Help me refactor this function",
            },
        },
        {
            "type": "event_msg",
            "payload": {
                "type": "agent_reasoning",
                "text": "I'll analyze the function structure.",
            },
        },
        {
            "type": "response_item",
            "payload": {
                "role": "assistant",
                "content": [{"text": "Here's the refactored code."}],
            },
        },
    ]


@pytest.fixture
def codex_session_file(temp_dir, codex_session_data):
    """Create a mock Codex session file."""
    session_dir = temp_dir / "2025" / "12" / "20"
    session_dir.mkdir(parents=True)
    session_file = session_dir / "rollout-2025-12-20T10-00-00-abc123.jsonl"

    with open(session_file, "w") as f:
        for entry in codex_session_data:
            f.write(json.dumps(entry) + "\n")

    return session_file


class TestCodexAdapter:
    """Tests for CodexAdapter."""

    def test_name_and_attributes(self, adapter):
        """Test adapter has correct name and attributes."""
        assert adapter.name == "codex"
        assert adapter.color is not None
        assert adapter.badge == "codex"

    def test_parse_session_basic(self, adapter, codex_session_file):
        """Test parsing a basic Codex session file."""
        session = adapter._parse_session_file(codex_session_file)

        assert session is not None
        assert session.agent == "codex"
        assert session.id == "abc123"
        assert session.directory == "/home/user/project"
        assert "Help me refactor" in session.title
        assert "Help me refactor" in session.content

    def test_parse_session_extracts_user_prompts(self, adapter, temp_dir):
        """Test that user prompts from event_msg are extracted."""
        session_file = temp_dir / "session.jsonl"

        data = [
            {"type": "session_meta", "payload": {"id": "test123", "cwd": "/test"}},
            {
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "First prompt"},
            },
            {
                "type": "event_msg",
                "payload": {"type": "agent_reasoning", "text": "Thinking..."},
            },
            {
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "Second prompt"},
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert "First prompt" in session.title
        assert "First prompt" in session.content
        assert "Second prompt" in session.content

    def test_parse_session_skips_environment_context(self, adapter, temp_dir):
        """Test that environment context messages are skipped in content."""
        session_file = temp_dir / "session.jsonl"

        data = [
            {"type": "session_meta", "payload": {"id": "test123", "cwd": "/test"}},
            {
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "Real prompt"},
            },
            {
                "type": "response_item",
                "payload": {
                    "role": "user",
                    "content": [
                        {"text": "<environment_context>...</environment_context>"}
                    ],
                },
            },
            {
                "type": "response_item",
                "payload": {"role": "assistant", "content": [{"text": "Response"}]},
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert "<environment_context>" not in session.content

    def test_parse_session_extracts_id_from_filename(self, adapter, temp_dir):
        """Test session ID extraction from filename when not in metadata."""
        session_file = temp_dir / "rollout-2025-12-20T10-00-00-fallback123.jsonl"

        data = [
            {
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "Test prompt"},
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert "2025-12-20T10-00-00-fallback123" in session.id

    def test_parse_session_no_user_prompts_returns_none(self, adapter, temp_dir):
        """Test that sessions without user prompts return None."""
        session_file = temp_dir / "session.jsonl"

        data = [
            {"type": "session_meta", "payload": {"id": "test123", "cwd": "/test"}},
            {
                "type": "response_item",
                "payload": {
                    "role": "assistant",
                    "content": [{"text": "Just assistant"}],
                },
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is None

    def test_parse_session_truncates_long_title(self, adapter, temp_dir):
        """Test that long titles are truncated."""
        session_file = temp_dir / "session.jsonl"

        long_message = "A" * 200
        data = [
            {"type": "session_meta", "payload": {"id": "test123", "cwd": "/test"}},
            {
                "type": "event_msg",
                "payload": {"type": "user_message", "message": long_message},
            },
        ]

        with open(session_file, "w") as f:
            for entry in data:
                f.write(json.dumps(entry) + "\n")

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert len(session.title) <= 83  # 80 + "..."
        assert session.title.endswith("...")

    def test_parse_malformed_json(self, adapter, temp_dir):
        """Test handling of malformed JSON."""
        session_file = temp_dir / "session.jsonl"

        with open(session_file, "w") as f:
            f.write("not json\n")
            f.write(
                json.dumps(
                    {
                        "type": "session_meta",
                        "payload": {"id": "test123", "cwd": "/test"},
                    }
                )
                + "\n"
            )
            f.write(
                json.dumps(
                    {
                        "type": "event_msg",
                        "payload": {"type": "user_message", "message": "Valid"},
                    }
                )
                + "\n"
            )

        session = adapter._parse_session_file(session_file)

        assert session is not None
        assert "Valid" in session.content

    def test_get_resume_command(self, adapter):
        """Test resume command generation."""
        from fast_resume.adapters.base import Session

        session = Session(
            id="session-abc123",
            agent="codex",
            title="Test",
            directory="/test",
            timestamp=datetime.now(),
            preview="",
            content="",
        )

        cmd = adapter.get_resume_command(session)

        assert cmd == ["codex", "resume", "session-abc123"]

    def test_find_sessions_recursive(self, temp_dir):
        """Test that find_sessions searches recursively in date directories."""
        # Create sessions in different date directories
        dir1 = temp_dir / "2025" / "12" / "19"
        dir1.mkdir(parents=True)
        dir2 = temp_dir / "2025" / "12" / "20"
        dir2.mkdir(parents=True)

        for i, d in enumerate([dir1, dir2]):
            session_file = d / f"session-{i}.jsonl"
            with open(session_file, "w") as f:
                f.write(
                    json.dumps(
                        {
                            "type": "session_meta",
                            "payload": {"id": f"id{i}", "cwd": "/test"},
                        }
                    )
                    + "\n"
                )
                f.write(
                    json.dumps(
                        {
                            "type": "event_msg",
                            "payload": {
                                "type": "user_message",
                                "message": f"Prompt {i}",
                            },
                        }
                    )
                    + "\n"
                )

        adapter = CodexAdapter(sessions_dir=temp_dir)
        sessions = adapter.find_sessions()

        assert len(sessions) == 2
