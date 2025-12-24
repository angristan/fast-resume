"""Tests for VS Code Copilot (copilot-vscode) session adapter."""

import json
from datetime import datetime

import pytest

from fast_resume.adapters.copilot_vscode import CopilotVSCodeAdapter


@pytest.fixture
def copilot_vscode_session_data():
    """Sample VS Code Copilot session JSON data."""
    return {
        "sessionId": "vscode-session-abc123",
        "customTitle": "Help with Python code",
        "creationDate": 1703059200000,  # 2023-12-20T10:00:00Z
        "lastMessageDate": 1703059260000,  # 2023-12-20T10:01:00Z
        "requests": [
            {
                "message": {"text": "How do I read a file in Python?"},
                "response": [{"value": "You can use the open() function."}],
            },
            {
                "message": {"text": "Can you show me an example?"},
                "response": [
                    {
                        "value": "Here's an example:\n```python\nwith open('file.txt') as f:\n    content = f.read()\n```"
                    }
                ],
            },
        ],
    }


@pytest.fixture
def copilot_vscode_session_file(temp_dir, copilot_vscode_session_data):
    """Create a mock VS Code Copilot session file."""
    session_dir = temp_dir / "emptyWindowChatSessions"
    session_dir.mkdir(parents=True)
    # Note: filename differs from sessionId to test the fix
    session_file = session_dir / "different-filename-uuid.json"

    with open(session_file, "w") as f:
        json.dump(copilot_vscode_session_data, f)

    return session_file


class TestCopilotVSCodeAdapter:
    """Tests for CopilotVSCodeAdapter."""

    def test_name_and_attributes(self):
        """Test adapter has correct name and attributes."""
        adapter = CopilotVSCodeAdapter()
        assert adapter.name == "copilot-vscode"
        assert adapter.color is not None
        assert adapter.badge == "vscode"

    def test_parse_session_basic(self, temp_dir, copilot_vscode_session_data):
        """Test parsing a basic VS Code Copilot session file."""
        session_dir = temp_dir / "emptyWindowChatSessions"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "test-session.json"

        with open(session_file, "w") as f:
            json.dump(copilot_vscode_session_data, f)

        adapter = CopilotVSCodeAdapter(chat_sessions_dir=session_dir)
        session = adapter._parse_session(session_file)

        assert session is not None
        assert session.agent == "copilot-vscode"
        assert session.id == "vscode-session-abc123"
        assert session.title == "Help with Python code"
        assert "How do I read a file in Python?" in session.content
        assert "You can use the open() function." in session.content

    def test_parse_session_extracts_session_id_from_json(self, temp_dir):
        """Test that session ID is extracted from sessionId field, not filename."""
        session_dir = temp_dir / "emptyWindowChatSessions"
        session_dir.mkdir(parents=True)
        # Filename is different from the sessionId in JSON
        session_file = session_dir / "random-uuid-filename.json"

        data = {
            "sessionId": "actual-session-id-from-json",
            "requests": [
                {
                    "message": {"text": "Test message"},
                    "response": [{"value": "Test response"}],
                },
            ],
        }

        with open(session_file, "w") as f:
            json.dump(data, f)

        adapter = CopilotVSCodeAdapter(chat_sessions_dir=session_dir)
        session = adapter._parse_session(session_file)

        assert session is not None
        assert session.id == "actual-session-id-from-json"
        assert session.id != "random-uuid-filename"

    def test_parse_session_uses_filename_as_fallback_id(self, temp_dir):
        """Test that filename is used as ID when sessionId field is missing."""
        session_dir = temp_dir / "emptyWindowChatSessions"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "fallback-id-test.json"

        data = {
            # No sessionId field
            "requests": [
                {
                    "message": {"text": "Test message without session ID"},
                    "response": [{"value": "Response"}],
                },
            ],
        }

        with open(session_file, "w") as f:
            json.dump(data, f)

        adapter = CopilotVSCodeAdapter(chat_sessions_dir=session_dir)
        session = adapter._parse_session(session_file)

        assert session is not None
        assert session.id == "fallback-id-test"

    def test_get_session_id_from_file(self, temp_dir):
        """Test _get_session_id_from_file helper extracts correct ID."""
        session_dir = temp_dir / "emptyWindowChatSessions"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "some-filename.json"

        data = {
            "sessionId": "the-real-session-id",
            "requests": [],
        }

        with open(session_file, "w") as f:
            json.dump(data, f)

        adapter = CopilotVSCodeAdapter(chat_sessions_dir=session_dir)
        session_id = adapter._get_session_id_from_file(session_file)

        assert session_id == "the-real-session-id"

    def test_get_session_id_from_file_returns_none_on_error(self, temp_dir):
        """Test _get_session_id_from_file returns None for invalid files."""
        session_dir = temp_dir / "emptyWindowChatSessions"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "invalid.json"

        with open(session_file, "w") as f:
            f.write("not valid json")

        adapter = CopilotVSCodeAdapter(chat_sessions_dir=session_dir)
        session_id = adapter._get_session_id_from_file(session_file)

        assert session_id is None

    def test_find_sessions_incremental_uses_correct_session_id(self, temp_dir):
        """Test that incremental scanning uses the same session ID as parsing.

        This is a regression test for a bug where find_sessions_incremental
        used the filename as session ID, but _parse_session used the sessionId
        from the JSON. This caused cache lookups to always fail.
        """
        session_dir = temp_dir / "emptyWindowChatSessions"
        session_dir.mkdir(parents=True)

        # Create session with sessionId different from filename
        session_file = session_dir / "uuid-filename-12345.json"
        data = {
            "sessionId": "json-session-id-abc",
            "requests": [
                {
                    "message": {"text": "Test for incremental"},
                    "response": [{"value": "Response"}],
                },
            ],
        }

        with open(session_file, "w") as f:
            json.dump(data, f)

        adapter = CopilotVSCodeAdapter(
            chat_sessions_dir=session_dir,
            workspace_storage_dir=temp_dir / "nonexistent",
        )

        # First call - no known sessions, should return session as new
        sessions, deleted = adapter.find_sessions_incremental({})
        assert len(sessions) == 1
        assert sessions[0].id == "json-session-id-abc"

        # Build known dict using the session ID from the parsed session
        mtime = session_file.stat().st_mtime
        known = {"json-session-id-abc": (mtime, "copilot-vscode")}

        # Second call - session should NOT be returned as new
        sessions, deleted = adapter.find_sessions_incremental(known)
        assert len(sessions) == 0, (
            "Session was incorrectly marked as new - cache lookup likely using wrong ID"
        )
        assert len(deleted) == 0

    def test_find_sessions_incremental_detects_modified(self, temp_dir):
        """Test that modified sessions are detected by mtime change."""
        session_dir = temp_dir / "emptyWindowChatSessions"
        session_dir.mkdir(parents=True)

        session_file = session_dir / "test-session.json"
        data = {
            "sessionId": "test-session-id",
            "requests": [
                {
                    "message": {"text": "Original message"},
                    "response": [{"value": "Original response"}],
                },
            ],
        }

        with open(session_file, "w") as f:
            json.dump(data, f)

        adapter = CopilotVSCodeAdapter(
            chat_sessions_dir=session_dir,
            workspace_storage_dir=temp_dir / "nonexistent",
        )

        # Simulate known with old mtime
        known = {"test-session-id": (0.0, "copilot-vscode")}

        sessions, deleted = adapter.find_sessions_incremental(known)
        assert len(sessions) == 1
        assert sessions[0].id == "test-session-id"

    def test_find_sessions_incremental_detects_deleted(self, temp_dir):
        """Test that deleted sessions are detected."""
        session_dir = temp_dir / "emptyWindowChatSessions"
        session_dir.mkdir(parents=True)

        adapter = CopilotVSCodeAdapter(
            chat_sessions_dir=session_dir,
            workspace_storage_dir=temp_dir / "nonexistent",
        )

        # Known session that no longer exists
        known = {"deleted-session-id": (12345.0, "copilot-vscode")}

        sessions, deleted = adapter.find_sessions_incremental(known)
        assert len(sessions) == 0
        assert "deleted-session-id" in deleted

    def test_parse_session_generates_title_from_first_message(self, temp_dir):
        """Test that title is generated from first user message if no customTitle."""
        session_dir = temp_dir / "emptyWindowChatSessions"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "no-title.json"

        data = {
            "sessionId": "test-id",
            # No customTitle
            "requests": [
                {
                    "message": {"text": "This should become the title"},
                    "response": [{"value": "Response"}],
                },
            ],
        }

        with open(session_file, "w") as f:
            json.dump(data, f)

        adapter = CopilotVSCodeAdapter(chat_sessions_dir=session_dir)
        session = adapter._parse_session(session_file)

        assert session is not None
        assert session.title == "This should become the title"

    def test_parse_session_empty_requests_returns_none(self, temp_dir):
        """Test that sessions with no requests return None."""
        session_dir = temp_dir / "emptyWindowChatSessions"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "empty-session.json"

        data = {
            "sessionId": "empty",
            "requests": [],
        }

        with open(session_file, "w") as f:
            json.dump(data, f)

        adapter = CopilotVSCodeAdapter(chat_sessions_dir=session_dir)
        session = adapter._parse_session(session_file)

        assert session is None

    def test_parse_session_counts_turns(self, temp_dir):
        """Test that message turns are counted correctly."""
        session_dir = temp_dir / "emptyWindowChatSessions"
        session_dir.mkdir(parents=True)
        session_file = session_dir / "turns.json"

        data = {
            "sessionId": "turns-test",
            "requests": [
                {
                    "message": {"text": "First question"},
                    "response": [{"value": "First answer"}],
                },
                {
                    "message": {"text": "Second question"},
                    "response": [{"value": "Second answer"}],
                },
                {
                    "message": {"text": "Third question"},
                    "response": [{"value": "Third answer"}],
                },
            ],
        }

        with open(session_file, "w") as f:
            json.dump(data, f)

        adapter = CopilotVSCodeAdapter(chat_sessions_dir=session_dir)
        session = adapter._parse_session(session_file)

        assert session is not None
        assert session.message_count == 6  # 3 user + 3 assistant

    def test_find_sessions(self, temp_dir):
        """Test finding sessions in the session directory."""
        session_dir = temp_dir / "emptyWindowChatSessions"
        session_dir.mkdir(parents=True)

        # Create two session files
        for i in range(2):
            session_file = session_dir / f"session-{i}.json"
            data = {
                "sessionId": f"session-id-{i}",
                "requests": [
                    {
                        "message": {"text": f"Session {i} message"},
                        "response": [{"value": f"Session {i} response"}],
                    },
                ],
            }
            with open(session_file, "w") as f:
                json.dump(data, f)

        adapter = CopilotVSCodeAdapter(
            chat_sessions_dir=session_dir,
            workspace_storage_dir=temp_dir / "nonexistent",
        )
        sessions = adapter.find_sessions()

        assert len(sessions) == 2

    def test_is_available_when_dir_exists_with_sessions(self, temp_dir):
        """Test is_available returns True when directory has session files."""
        session_dir = temp_dir / "emptyWindowChatSessions"
        session_dir.mkdir(parents=True)
        (session_dir / "session.json").write_text("{}")

        adapter = CopilotVSCodeAdapter(
            chat_sessions_dir=session_dir,
            workspace_storage_dir=temp_dir / "nonexistent",
        )
        assert adapter.is_available() is True

    def test_is_available_when_dir_missing(self, temp_dir):
        """Test is_available returns False when directories don't exist."""
        adapter = CopilotVSCodeAdapter(
            chat_sessions_dir=temp_dir / "nonexistent1",
            workspace_storage_dir=temp_dir / "nonexistent2",
        )
        assert adapter.is_available() is False

    def test_get_resume_command(self):
        """Test resume command generation."""
        from fast_resume.adapters.base import Session

        adapter = CopilotVSCodeAdapter()

        session = Session(
            id="test-id",
            agent="copilot-vscode",
            title="Test",
            directory="/test/project",
            timestamp=datetime.now(),
            preview="",
            content="",
        )

        cmd = adapter.get_resume_command(session)
        assert cmd == ["code", "/test/project"]

    def test_get_resume_command_no_directory(self):
        """Test resume command when session has no directory."""
        from fast_resume.adapters.base import Session

        adapter = CopilotVSCodeAdapter()

        session = Session(
            id="test-id",
            agent="copilot-vscode",
            title="Test",
            directory="",
            timestamp=datetime.now(),
            preview="",
            content="",
        )

        cmd = adapter.get_resume_command(session)
        assert cmd == ["code"]
