"""Tests for Crush (charmbracelet) session adapter."""

import json
import sqlite3
from datetime import datetime

import pytest

from fast_resume.adapters.crush import CrushAdapter


@pytest.fixture
def adapter():
    """Create a CrushAdapter instance."""
    return CrushAdapter()


def create_crush_db(db_path, sessions_data):
    """Create a mock Crush SQLite database."""
    conn = sqlite3.connect(str(db_path))
    cursor = conn.cursor()

    # Create tables matching Crush schema
    cursor.execute("""
        CREATE TABLE sessions (
            id TEXT PRIMARY KEY,
            title TEXT,
            message_count INTEGER,
            updated_at INTEGER,
            created_at INTEGER
        )
    """)

    cursor.execute("""
        CREATE TABLE messages (
            id TEXT PRIMARY KEY,
            session_id TEXT,
            role TEXT,
            parts TEXT,
            model TEXT,
            created_at INTEGER,
            FOREIGN KEY (session_id) REFERENCES sessions(id)
        )
    """)

    for session in sessions_data:
        cursor.execute(
            "INSERT INTO sessions VALUES (?, ?, ?, ?, ?)",
            (
                session["id"],
                session["title"],
                session["message_count"],
                session["updated_at"],
                session["created_at"],
            ),
        )
        for msg in session.get("messages", []):
            cursor.execute(
                "INSERT INTO messages VALUES (?, ?, ?, ?, ?, ?)",
                (
                    msg["id"],
                    session["id"],
                    msg["role"],
                    msg["parts"],
                    msg.get("model", ""),
                    msg["created_at"],
                ),
            )

    conn.commit()
    conn.close()


class TestCrushAdapter:
    """Tests for CrushAdapter."""

    def test_name_and_attributes(self, adapter):
        """Test adapter has correct name and attributes."""
        assert adapter.name == "crush"
        assert adapter.color is not None
        assert adapter.badge == "crush"

    def test_parse_session_basic(self, adapter, temp_dir):
        """Test parsing a basic Crush session from database."""
        db_path = temp_dir / "crush.db"

        # Timestamps in milliseconds (current style)
        now_ms = int(datetime.now().timestamp() * 1000)

        sessions_data = [
            {
                "id": "session-001",
                "title": "Fix authentication bug",
                "message_count": 2,
                "updated_at": now_ms,
                "created_at": now_ms - 60000,
                "messages": [
                    {
                        "id": "msg-001",
                        "role": "user",
                        "parts": json.dumps(
                            [
                                {
                                    "type": "text",
                                    "data": {"text": "Help me fix the auth bug"},
                                }
                            ]
                        ),
                        "created_at": now_ms - 60000,
                    },
                    {
                        "id": "msg-002",
                        "role": "assistant",
                        "parts": json.dumps(
                            [
                                {
                                    "type": "text",
                                    "data": {"text": "I'll help you fix it."},
                                }
                            ]
                        ),
                        "created_at": now_ms - 30000,
                    },
                ],
            }
        ]

        create_crush_db(db_path, sessions_data)

        sessions = adapter._load_sessions_from_db(db_path, "/home/user/project")

        assert len(sessions) == 1
        session = sessions[0]
        assert session.agent == "crush"
        assert session.title == "Fix authentication bug"
        assert session.directory == "/home/user/project"
        assert "Help me fix the auth bug" in session.content
        assert "I'll help you fix it" in session.content

    def test_parse_session_with_unix_seconds_timestamp(self, adapter, temp_dir):
        """Test that Unix seconds timestamps are handled correctly."""
        db_path = temp_dir / "crush.db"

        # Timestamps in seconds (old style)
        now_sec = int(datetime.now().timestamp())

        sessions_data = [
            {
                "id": "session-002",
                "title": "Old session",
                "message_count": 1,
                "updated_at": now_sec,
                "created_at": now_sec,
                "messages": [
                    {
                        "id": "msg-001",
                        "role": "user",
                        "parts": json.dumps(
                            [{"type": "text", "data": {"text": "Test message"}}]
                        ),
                        "created_at": now_sec,
                    },
                ],
            }
        ]

        create_crush_db(db_path, sessions_data)

        sessions = adapter._load_sessions_from_db(db_path, "/test")

        assert len(sessions) == 1
        # Should not crash and timestamp should be reasonable
        assert sessions[0].timestamp.year >= 2024

    def test_parse_session_without_title_uses_first_message(self, adapter, temp_dir):
        """Test that sessions without title use first user message."""
        db_path = temp_dir / "crush.db"
        now_ms = int(datetime.now().timestamp() * 1000)

        sessions_data = [
            {
                "id": "session-003",
                "title": "",  # No title
                "message_count": 1,
                "updated_at": now_ms,
                "created_at": now_ms,
                "messages": [
                    {
                        "id": "msg-001",
                        "role": "user",
                        "parts": json.dumps(
                            [
                                {
                                    "type": "text",
                                    "data": {"text": "This should become the title"},
                                }
                            ]
                        ),
                        "created_at": now_ms,
                    },
                ],
            }
        ]

        create_crush_db(db_path, sessions_data)

        sessions = adapter._load_sessions_from_db(db_path, "/test")

        assert len(sessions) == 1
        assert "This should become the title" in sessions[0].title

    def test_extract_text_from_parts_with_tool_calls(self, adapter):
        """Test extraction of text from parts with tool calls."""
        parts_json = json.dumps(
            [
                {"type": "text", "data": {"text": "Let me check that file."}},
                {
                    "type": "tool_call",
                    "data": {"name": "read_file", "args": {"path": "/test.py"}},
                },
                {
                    "type": "tool_result",
                    "data": {"name": "read_file", "content": "print('hello')"},
                },
            ]
        )

        text = adapter._extract_text_from_parts(parts_json)

        assert "Let me check that file" in text
        assert "[calling read_file]" in text
        assert "[read_file]:" in text

    def test_extract_text_from_parts_skips_long_tool_results(self, adapter):
        """Test that long tool results are skipped."""
        parts_json = json.dumps(
            [
                {"type": "text", "data": {"text": "Analyzing..."}},
                {
                    "type": "tool_result",
                    "data": {"name": "read_file", "content": "x" * 600},
                },
            ]
        )

        text = adapter._extract_text_from_parts(parts_json)

        assert "Analyzing" in text
        assert "x" * 500 not in text  # Long content should be skipped

    def test_extract_text_from_invalid_json(self, adapter):
        """Test handling of invalid parts JSON."""
        text = adapter._extract_text_from_parts("not valid json")

        assert text == ""

    def test_parse_session_skips_empty_sessions(self, adapter, temp_dir):
        """Test that sessions with no messages are skipped."""
        db_path = temp_dir / "crush.db"
        now_ms = int(datetime.now().timestamp() * 1000)

        sessions_data = [
            {
                "id": "session-empty",
                "title": "Empty session",
                "message_count": 0,  # Will be filtered by SQL
                "updated_at": now_ms,
                "created_at": now_ms,
                "messages": [],
            }
        ]

        create_crush_db(db_path, sessions_data)

        sessions = adapter._load_sessions_from_db(db_path, "/test")

        assert len(sessions) == 0

    def test_get_resume_command(self, adapter):
        """Test resume command generation."""
        from fast_resume.adapters.base import Session

        session = Session(
            id="session-abc123",
            agent="crush",
            title="Test",
            directory="/test/project",
            timestamp=datetime.now(),
            preview="",
            content="",
        )

        cmd = adapter.get_resume_command(session)

        # Crush just opens in the project directory
        assert cmd == ["crush"]

    def test_find_sessions_from_projects_file(self, temp_dir):
        """Test finding sessions from projects.json file."""
        # Create project data directory with database
        data_dir = temp_dir / "project1_data"
        data_dir.mkdir()
        db_path = data_dir / "crush.db"

        now_ms = int(datetime.now().timestamp() * 1000)
        sessions_data = [
            {
                "id": "session-001",
                "title": "Test session",
                "message_count": 1,
                "updated_at": now_ms,
                "created_at": now_ms,
                "messages": [
                    {
                        "id": "msg-001",
                        "role": "user",
                        "parts": json.dumps(
                            [
                                {
                                    "type": "text",
                                    "data": {
                                        "text": "Hello world, this is a test message"
                                    },
                                }
                            ]
                        ),
                        "created_at": now_ms,
                    },
                ],
            }
        ]
        create_crush_db(db_path, sessions_data)

        # Create projects.json
        projects_file = temp_dir / "projects.json"
        projects = {
            "projects": [{"path": "/home/user/project1", "data_dir": str(data_dir)}]
        }
        with open(projects_file, "w") as f:
            json.dump(projects, f)

        adapter = CrushAdapter(projects_file=projects_file)
        sessions = adapter.find_sessions()

        assert len(sessions) == 1
        assert sessions[0].directory == "/home/user/project1"

    def test_find_sessions_handles_missing_db(self, temp_dir):
        """Test that missing database files are handled gracefully."""
        # Create projects.json pointing to non-existent db
        projects_file = temp_dir / "projects.json"
        projects = {
            "projects": [
                {
                    "path": "/home/user/project1",
                    "data_dir": str(temp_dir / "nonexistent"),
                }
            ]
        }
        with open(projects_file, "w") as f:
            json.dump(projects, f)

        adapter = CrushAdapter(projects_file=projects_file)
        sessions = adapter.find_sessions()

        assert len(sessions) == 0
