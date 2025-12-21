"""Tests for OpenCode session adapter."""

import json
from collections import defaultdict
from datetime import datetime
from pathlib import Path
from unittest.mock import patch

import pytest

from fast_resume.adapters.opencode import OpenCodeAdapter


@pytest.fixture
def adapter():
    """Create an OpenCodeAdapter instance."""
    return OpenCodeAdapter()


def build_indexes(
    message_dir: Path, part_dir: Path
) -> tuple[dict[str, list[tuple[Path, str, str]]], dict[str, list[str]]]:
    """Build pre-indexed message and part dicts from directories."""
    messages_by_session: dict[str, list[tuple[Path, str, str]]] = defaultdict(list)
    if message_dir.exists():
        for msg_file in message_dir.glob("*/msg_*.json"):
            try:
                with open(msg_file, "r", encoding="utf-8") as f:
                    msg_data = json.load(f)
                session_id = msg_file.parent.name
                msg_id = msg_data.get("id", "")
                role = msg_data.get("role", "")
                if msg_id:
                    messages_by_session[session_id].append((msg_file, msg_id, role))
            except Exception:
                continue

    parts_by_message: dict[str, list[str]] = defaultdict(list)
    if part_dir.exists():
        for part_file in sorted(part_dir.glob("*/*.json")):
            try:
                with open(part_file, "r", encoding="utf-8") as f:
                    part_data = json.load(f)
                msg_id = part_file.parent.name
                if part_data.get("type") == "text":
                    text = part_data.get("text", "")
                    if text:
                        parts_by_message[msg_id].append(text)
            except Exception:
                continue

    return messages_by_session, parts_by_message


def create_opencode_structure(base_dir, sessions):
    """Create OpenCode directory structure with sessions."""
    session_dir = base_dir / "session"
    message_dir = base_dir / "message"
    part_dir = base_dir / "part"

    for session in sessions:
        # Create session file
        project_hash = session.get("project_hash", "proj_abc123")
        sess_dir = session_dir / project_hash
        sess_dir.mkdir(parents=True, exist_ok=True)

        session_file = sess_dir / f"ses_{session['id']}.json"
        session_data = {
            "id": session["id"],
            "title": session.get("title", "Untitled session"),
            "directory": session.get("directory", "/test"),
            "time": {
                "created": session.get(
                    "created_ms", int(datetime.now().timestamp() * 1000)
                )
            },
        }
        with open(session_file, "w") as f:
            json.dump(session_data, f)

        # Create messages
        for msg in session.get("messages", []):
            msg_session_dir = message_dir / session["id"]
            msg_session_dir.mkdir(parents=True, exist_ok=True)

            msg_file = msg_session_dir / f"msg_{msg['id']}.json"
            msg_data = {"id": msg["id"], "role": msg["role"]}
            with open(msg_file, "w") as f:
                json.dump(msg_data, f)

            # Create parts
            for part in msg.get("parts", []):
                parts_dir = part_dir / msg["id"]
                parts_dir.mkdir(parents=True, exist_ok=True)

                part_file = parts_dir / f"{part['id']}.json"
                with open(part_file, "w") as f:
                    json.dump(part, f)


class TestOpenCodeAdapter:
    """Tests for OpenCodeAdapter."""

    def test_name_and_attributes(self, adapter):
        """Test adapter has correct name and attributes."""
        assert adapter.name == "opencode"
        assert adapter.color is not None
        assert adapter.badge == "opencode"

    def test_parse_session_basic(self, adapter, temp_dir):
        """Test parsing a basic OpenCode session."""
        sessions = [
            {
                "id": "ses_001",
                "title": "Fix database query",
                "directory": "/home/user/project",
                "project_hash": "proj_abc",
                "messages": [
                    {
                        "id": "msg_001",
                        "role": "user",
                        "parts": [
                            {
                                "id": "part_001",
                                "type": "text",
                                "text": "Help me optimize this query",
                            }
                        ],
                    },
                    {
                        "id": "msg_002",
                        "role": "assistant",
                        "parts": [
                            {
                                "id": "part_002",
                                "type": "text",
                                "text": "I'll help you optimize it.",
                            }
                        ],
                    },
                ],
            }
        ]

        create_opencode_structure(temp_dir, sessions)

        session_file = temp_dir / "session" / "proj_abc" / "ses_ses_001.json"
        messages_by_session, parts_by_message = build_indexes(
            temp_dir / "message", temp_dir / "part"
        )
        session = adapter._parse_session(
            session_file, messages_by_session, parts_by_message
        )

        assert session is not None
        assert session.agent == "opencode"
        assert session.id == "ses_001"
        assert session.title == "Fix database query"
        assert session.directory == "/home/user/project"
        assert "Help me optimize" in session.content
        assert "I'll help you optimize" in session.content

    def test_parse_session_uses_file_mtime_without_timestamp(self, adapter, temp_dir):
        """Test that file mtime is used when timestamp is missing."""
        session_dir = temp_dir / "session" / "proj_abc"
        session_dir.mkdir(parents=True)

        session_file = session_dir / "ses_test.json"
        # Session without time data
        with open(session_file, "w") as f:
            json.dump({"id": "test", "title": "Test", "directory": "/test"}, f)

        messages_by_session, parts_by_message = build_indexes(
            temp_dir / "message", temp_dir / "part"
        )
        session = adapter._parse_session(
            session_file, messages_by_session, parts_by_message
        )

        assert session is not None
        # Should use file mtime, which will be recent
        assert session.timestamp.year >= 2024

    def test_get_session_messages(self, adapter, temp_dir):
        """Test message retrieval from parts."""
        message_dir = temp_dir / "message"
        part_dir = temp_dir / "part"

        # Create message
        msg_dir = message_dir / "ses_001"
        msg_dir.mkdir(parents=True)
        with open(msg_dir / "msg_001.json", "w") as f:
            json.dump({"id": "msg_001", "role": "user"}, f)

        # Create parts
        parts_dir = part_dir / "msg_001"
        parts_dir.mkdir(parents=True)
        with open(parts_dir / "part_001.json", "w") as f:
            json.dump({"type": "text", "text": "User message here"}, f)

        messages_by_session, parts_by_message = build_indexes(message_dir, part_dir)
        messages = adapter._get_session_messages(
            "ses_001", messages_by_session, parts_by_message
        )

        assert len(messages) == 1
        assert "» User message here" in messages[0]

    def test_get_session_messages_skips_non_text_parts(self, adapter, temp_dir):
        """Test that non-text parts are skipped."""
        message_dir = temp_dir / "message"
        part_dir = temp_dir / "part"

        msg_dir = message_dir / "ses_001"
        msg_dir.mkdir(parents=True)
        with open(msg_dir / "msg_001.json", "w") as f:
            json.dump({"id": "msg_001", "role": "assistant"}, f)

        parts_dir = part_dir / "msg_001"
        parts_dir.mkdir(parents=True)
        with open(parts_dir / "part_001.json", "w") as f:
            json.dump({"type": "tool_use", "name": "read_file"}, f)
        with open(parts_dir / "part_002.json", "w") as f:
            json.dump({"type": "text", "text": "Here's the file content"}, f)

        messages_by_session, parts_by_message = build_indexes(message_dir, part_dir)
        messages = adapter._get_session_messages(
            "ses_001", messages_by_session, parts_by_message
        )

        assert len(messages) == 1
        assert "Here's the file content" in messages[0]
        assert "tool_use" not in str(messages)

    def test_get_session_messages_handles_missing_dir(self, adapter, temp_dir):
        """Test handling of missing message directory."""
        messages_by_session, parts_by_message = build_indexes(
            temp_dir / "message", temp_dir / "part"
        )
        messages = adapter._get_session_messages(
            "nonexistent", messages_by_session, parts_by_message
        )

        assert messages == []

    def test_get_resume_command(self, adapter):
        """Test resume command generation."""
        from fast_resume.adapters.base import Session

        session = Session(
            id="ses_abc123",
            agent="opencode",
            title="Test",
            directory="/home/user/project",
            timestamp=datetime.now(),
            preview="",
            content="",
        )

        cmd = adapter.get_resume_command(session)

        assert cmd == ["opencode", "/home/user/project", "--session", "ses_abc123"]

    def test_find_sessions(self, temp_dir):
        """Test finding all sessions."""
        sessions = [
            {
                "id": "ses_001",
                "title": "Session 1",
                "directory": "/project1",
                "project_hash": "proj_aaa",
                "messages": [
                    {
                        "id": "msg_001",
                        "role": "user",
                        "parts": [{"id": "p1", "type": "text", "text": "Hello"}],
                    }
                ],
            },
            {
                "id": "ses_002",
                "title": "Session 2",
                "directory": "/project2",
                "project_hash": "proj_bbb",
                "messages": [
                    {
                        "id": "msg_002",
                        "role": "user",
                        "parts": [{"id": "p2", "type": "text", "text": "World"}],
                    }
                ],
            },
        ]
        create_opencode_structure(temp_dir, sessions)

        adapter = OpenCodeAdapter(sessions_dir=temp_dir)
        found = adapter.find_sessions()

        assert len(found) == 2
        titles = {s.title for s in found}
        assert "Session 1" in titles
        assert "Session 2" in titles

    def test_find_sessions_returns_empty_when_unavailable(self, adapter):
        """Test that find_sessions returns empty list when unavailable."""
        with patch.object(adapter, "is_available", return_value=False):
            sessions = adapter.find_sessions()
            assert sessions == []
