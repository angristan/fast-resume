"""Tests for OpenCode session adapter.

Tests cover both the new SQLite format (OpenCode 1.2+) and
the legacy split-JSON-files format.
"""

import json
import sqlite3
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


# ── SQLite test helpers ───────────────────────────────────────────────


def create_opencode_db(db_path, sessions_data, projects=None):
    """Create a mock OpenCode SQLite database matching the 1.2 schema.

    Args:
        db_path: Path to create the database at.
        sessions_data: List of session dicts, each with:
            - id, title, directory, time_created, time_updated
            - messages: list of {id, role, parts: [{id, type, text?, ...}]}
        projects: Optional list of project dicts with id, worktree.
    """
    conn = sqlite3.connect(str(db_path))
    cursor = conn.cursor()

    # Create tables matching OpenCode 1.2 Drizzle schema
    cursor.execute("""
        CREATE TABLE project (
            id TEXT PRIMARY KEY,
            worktree TEXT NOT NULL,
            vcs TEXT,
            name TEXT,
            icon_url TEXT,
            icon_color TEXT,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            time_initialized INTEGER,
            sandboxes TEXT NOT NULL DEFAULT '[]',
            commands TEXT
        )
    """)

    cursor.execute("""
        CREATE TABLE session (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            parent_id TEXT,
            slug TEXT NOT NULL,
            directory TEXT NOT NULL,
            title TEXT NOT NULL,
            version TEXT NOT NULL,
            share_url TEXT,
            summary_additions INTEGER,
            summary_deletions INTEGER,
            summary_files INTEGER,
            summary_diffs TEXT,
            revert TEXT,
            permission TEXT,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            time_compacting INTEGER,
            time_archived INTEGER,
            FOREIGN KEY (project_id) REFERENCES project(id) ON DELETE CASCADE
        )
    """)
    cursor.execute("CREATE INDEX session_project_idx ON session (project_id)")

    cursor.execute("""
        CREATE TABLE message (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            data TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
        )
    """)
    cursor.execute("CREATE INDEX message_session_idx ON message (session_id)")

    cursor.execute("""
        CREATE TABLE part (
            id TEXT PRIMARY KEY,
            message_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            data TEXT NOT NULL,
            FOREIGN KEY (message_id) REFERENCES message(id) ON DELETE CASCADE
        )
    """)
    cursor.execute("CREATE INDEX part_message_idx ON part (message_id)")
    cursor.execute("CREATE INDEX part_session_idx ON part (session_id)")

    # Insert default project if not specified
    if projects is None:
        projects = [{"id": "proj_default", "worktree": "/test"}]
    for proj in projects:
        cursor.execute(
            "INSERT INTO project (id, worktree, time_created, time_updated, sandboxes) "
            "VALUES (?, ?, ?, ?, ?)",
            (
                proj["id"],
                proj.get("worktree", "/test"),
                proj.get("time_created", 1704067200000),
                proj.get("time_updated", 1704067200000),
                "[]",
            ),
        )

    # Insert sessions, messages, and parts
    for session in sessions_data:
        project_id = session.get("project_id", "proj_default")
        time_created = session.get("time_created", 1704067200000)
        time_updated = session.get("time_updated", time_created)

        cursor.execute(
            "INSERT INTO session (id, project_id, slug, directory, title, version, "
            "time_created, time_updated) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            (
                session["id"],
                project_id,
                session.get("slug", session["id"]),
                session.get("directory", "/test"),
                session.get("title", "Untitled session"),
                session.get("version", "1.2.0"),
                time_created,
                time_updated,
            ),
        )

        msg_time = time_created
        for msg in session.get("messages", []):
            msg_time += 1000  # 1 second apart
            role = msg.get("role", "user")
            msg_data = {"role": role}
            if role == "user":
                msg_data["time"] = {"created": msg_time}
                msg_data["agent"] = "build"
                msg_data["model"] = {
                    "providerID": "anthropic",
                    "modelID": "claude-opus-4-5",
                }
            else:
                msg_data["time"] = {"created": msg_time, "completed": msg_time + 500}
                msg_data["parentID"] = "msg_parent"
                msg_data["modelID"] = "claude-opus-4-5"
                msg_data["providerID"] = "anthropic"
                msg_data["agent"] = "build"
                msg_data["mode"] = "build"
                msg_data["cost"] = 0.01
                msg_data["tokens"] = {
                    "input": 100,
                    "output": 50,
                    "reasoning": 0,
                    "cache": {"read": 0, "write": 0},
                }
                msg_data["path"] = {"cwd": "/test", "root": "/test"}

            cursor.execute(
                "INSERT INTO message (id, session_id, time_created, time_updated, data) "
                "VALUES (?, ?, ?, ?, ?)",
                (
                    msg["id"],
                    session["id"],
                    msg_time,
                    msg_time,
                    json.dumps(msg_data),
                ),
            )

            part_time = msg_time
            for part in msg.get("parts", []):
                part_time += 100
                part_data = {k: v for k, v in part.items() if k != "id"}
                cursor.execute(
                    "INSERT INTO part (id, message_id, session_id, time_created, "
                    "time_updated, data) VALUES (?, ?, ?, ?, ?, ?)",
                    (
                        part["id"],
                        msg["id"],
                        session["id"],
                        part_time,
                        part_time,
                        json.dumps(part_data),
                    ),
                )

    conn.commit()
    conn.close()


# ── Legacy JSON test helpers ──────────────────────────────────────────


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


def create_legacy_opencode_structure(base_dir, sessions):
    """Create legacy OpenCode directory structure with split JSON files."""
    session_dir = base_dir / "session"
    message_dir = base_dir / "message"
    part_dir = base_dir / "part"

    for session in sessions:
        project_hash = session.get("project_hash", "proj_abc123")
        sess_dir = session_dir / project_hash
        sess_dir.mkdir(parents=True, exist_ok=True)

        session_file = sess_dir / f"ses_{session['id']}.json"
        time_data = {
            "created": session.get("created_ms", int(datetime.now().timestamp() * 1000))
        }
        if "updated_ms" in session:
            time_data["updated"] = session["updated_ms"]
        session_data = {
            "id": session["id"],
            "title": session.get("title", "Untitled session"),
            "directory": session.get("directory", "/test"),
            "time": time_data,
        }
        with open(session_file, "w") as f:
            json.dump(session_data, f)

        for msg in session.get("messages", []):
            msg_session_dir = message_dir / session["id"]
            msg_session_dir.mkdir(parents=True, exist_ok=True)

            msg_file = msg_session_dir / f"msg_{msg['id']}.json"
            msg_data = {"id": msg["id"], "role": msg["role"]}
            with open(msg_file, "w") as f:
                json.dump(msg_data, f)

            for part in msg.get("parts", []):
                parts_dir = part_dir / msg["id"]
                parts_dir.mkdir(parents=True, exist_ok=True)

                part_file = parts_dir / f"{part['id']}.json"
                with open(part_file, "w") as f:
                    json.dump(part, f)


# ── SQLite format tests ──────────────────────────────────────────────


class TestOpenCodeSQLite:
    """Tests for OpenCode SQLite format (1.2+)."""

    def test_name_and_attributes(self, adapter):
        """Test adapter has correct name and attributes."""
        assert adapter.name == "opencode"
        assert adapter.color is not None
        assert adapter.badge == "opencode"
        assert adapter.supports_yolo is False

    def test_find_sessions_from_sqlite(self, temp_dir):
        """Test finding sessions from SQLite database."""
        db_path = temp_dir / "opencode.db"
        now_ms = int(datetime.now().timestamp() * 1000)

        sessions_data = [
            {
                "id": "ses_001",
                "title": "Fix database query",
                "directory": "/home/user/project",
                "time_created": now_ms - 60000,
                "time_updated": now_ms,
                "messages": [
                    {
                        "id": "msg_001",
                        "role": "user",
                        "parts": [
                            {
                                "id": "prt_001",
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
                                "id": "prt_002",
                                "type": "text",
                                "text": "I'll help you optimize it.",
                            }
                        ],
                    },
                ],
            }
        ]

        create_opencode_db(db_path, sessions_data)

        adapter = OpenCodeAdapter(db_path=db_path, legacy_dir=temp_dir / "nonexistent")
        found = adapter.find_sessions()

        assert len(found) == 1
        session = found[0]
        assert session.agent == "opencode"
        assert session.id == "ses_001"
        assert session.title == "Fix database query"
        assert session.directory == "/home/user/project"
        assert "Help me optimize" in session.content
        assert "I'll help you optimize" in session.content
        assert session.message_count == 2

    def test_find_sessions_multiple(self, temp_dir):
        """Test finding multiple sessions from SQLite."""
        db_path = temp_dir / "opencode.db"
        now_ms = int(datetime.now().timestamp() * 1000)

        sessions_data = [
            {
                "id": "ses_001",
                "title": "Session 1",
                "directory": "/project1",
                "time_created": now_ms - 120000,
                "time_updated": now_ms - 60000,
                "messages": [
                    {
                        "id": "msg_001",
                        "role": "user",
                        "parts": [{"id": "prt_001", "type": "text", "text": "Hello"}],
                    }
                ],
            },
            {
                "id": "ses_002",
                "title": "Session 2",
                "directory": "/project2",
                "time_created": now_ms - 60000,
                "time_updated": now_ms,
                "messages": [
                    {
                        "id": "msg_002",
                        "role": "user",
                        "parts": [{"id": "prt_002", "type": "text", "text": "World"}],
                    }
                ],
            },
        ]

        create_opencode_db(db_path, sessions_data)

        adapter = OpenCodeAdapter(db_path=db_path, legacy_dir=temp_dir / "nonexistent")
        found = adapter.find_sessions()

        assert len(found) == 2
        titles = {s.title for s in found}
        assert "Session 1" in titles
        assert "Session 2" in titles

    def test_skips_non_text_parts(self, temp_dir):
        """Test that non-text parts (tool, snapshot, etc.) are skipped."""
        db_path = temp_dir / "opencode.db"
        now_ms = int(datetime.now().timestamp() * 1000)

        sessions_data = [
            {
                "id": "ses_001",
                "title": "Tool session",
                "directory": "/test",
                "time_created": now_ms,
                "time_updated": now_ms,
                "messages": [
                    {
                        "id": "msg_001",
                        "role": "assistant",
                        "parts": [
                            {
                                "id": "prt_001",
                                "type": "tool",
                                "callID": "toolu_123",
                                "tool": "bash",
                                "state": {
                                    "status": "completed",
                                    "input": {"command": "ls"},
                                    "output": "file.txt",
                                },
                            },
                            {
                                "id": "prt_002",
                                "type": "text",
                                "text": "Here is the result",
                            },
                        ],
                    }
                ],
            }
        ]

        create_opencode_db(db_path, sessions_data)

        adapter = OpenCodeAdapter(db_path=db_path, legacy_dir=temp_dir / "nonexistent")
        found = adapter.find_sessions()

        assert len(found) == 1
        assert "Here is the result" in found[0].content
        assert "bash" not in found[0].content  # Tool parts should be skipped

    def test_timestamp_uses_updated_over_created(self, temp_dir):
        """Test that time_updated is used over time_created when later."""
        db_path = temp_dir / "opencode.db"
        created_ms = 1704067200000  # 2024-01-01
        updated_ms = 1704153600000  # 2024-01-02

        sessions_data = [
            {
                "id": "ses_001",
                "title": "Test",
                "directory": "/test",
                "time_created": created_ms,
                "time_updated": updated_ms,
                "messages": [],
            }
        ]

        create_opencode_db(db_path, sessions_data)

        adapter = OpenCodeAdapter(db_path=db_path, legacy_dir=temp_dir / "nonexistent")
        found = adapter.find_sessions()

        assert len(found) == 1
        expected = datetime.fromtimestamp(updated_ms / 1000)
        assert found[0].timestamp == expected

    def test_user_messages_prefixed(self, temp_dir):
        """Test that user messages get » prefix and assistant messages get space."""
        db_path = temp_dir / "opencode.db"
        now_ms = int(datetime.now().timestamp() * 1000)

        sessions_data = [
            {
                "id": "ses_001",
                "title": "Test",
                "directory": "/test",
                "time_created": now_ms,
                "time_updated": now_ms,
                "messages": [
                    {
                        "id": "msg_001",
                        "role": "user",
                        "parts": [
                            {"id": "prt_001", "type": "text", "text": "User question"}
                        ],
                    },
                    {
                        "id": "msg_002",
                        "role": "assistant",
                        "parts": [
                            {
                                "id": "prt_002",
                                "type": "text",
                                "text": "Assistant reply",
                            }
                        ],
                    },
                ],
            }
        ]

        create_opencode_db(db_path, sessions_data)

        adapter = OpenCodeAdapter(db_path=db_path, legacy_dir=temp_dir / "nonexistent")
        found = adapter.find_sessions()

        assert len(found) == 1
        assert "» User question" in found[0].content
        assert "  Assistant reply" in found[0].content

    def test_incremental_new_sessions(self, temp_dir):
        """Test incremental loading detects new sessions."""
        db_path = temp_dir / "opencode.db"
        now_ms = int(datetime.now().timestamp() * 1000)

        sessions_data = [
            {
                "id": "ses_001",
                "title": "Session 1",
                "directory": "/project1",
                "time_created": now_ms,
                "time_updated": now_ms,
                "messages": [
                    {
                        "id": "msg_001",
                        "role": "user",
                        "parts": [{"id": "prt_001", "type": "text", "text": "Hello"}],
                    }
                ],
            },
            {
                "id": "ses_002",
                "title": "Session 2",
                "directory": "/project2",
                "time_created": now_ms,
                "time_updated": now_ms,
                "messages": [
                    {
                        "id": "msg_002",
                        "role": "user",
                        "parts": [{"id": "prt_002", "type": "text", "text": "World"}],
                    }
                ],
            },
        ]

        create_opencode_db(db_path, sessions_data)

        adapter = OpenCodeAdapter(db_path=db_path, legacy_dir=temp_dir / "nonexistent")

        callback_sessions = []

        def on_session(session):
            callback_sessions.append(session)

        new_or_modified, deleted_ids = adapter.find_sessions_incremental(
            known={}, on_session=on_session
        )

        assert len(callback_sessions) == 2
        assert len(new_or_modified) == 2
        ids = {s.id for s in new_or_modified}
        assert "ses_001" in ids
        assert "ses_002" in ids

    def test_incremental_no_changes(self, temp_dir):
        """Test incremental loading skips unchanged sessions."""
        db_path = temp_dir / "opencode.db"
        now_ms = 1704067200000  # Fixed timestamp

        sessions_data = [
            {
                "id": "ses_001",
                "title": "Session 1",
                "directory": "/project1",
                "time_created": now_ms,
                "time_updated": now_ms,
                "messages": [
                    {
                        "id": "msg_001",
                        "role": "user",
                        "parts": [{"id": "prt_001", "type": "text", "text": "Hello"}],
                    }
                ],
            },
        ]

        create_opencode_db(db_path, sessions_data)

        adapter = OpenCodeAdapter(db_path=db_path, legacy_dir=temp_dir / "nonexistent")

        # Known sessions with same mtime
        mtime = datetime.fromtimestamp(now_ms / 1000).timestamp()
        known = {"ses_001": (mtime, "opencode")}

        callback_sessions = []

        def on_session(session):
            callback_sessions.append(session)

        new_or_modified, deleted_ids = adapter.find_sessions_incremental(
            known=known, on_session=on_session
        )

        assert len(callback_sessions) == 0
        assert len(new_or_modified) == 0
        assert len(deleted_ids) == 0

    def test_incremental_detects_deletions(self, temp_dir):
        """Test incremental loading detects deleted sessions."""
        db_path = temp_dir / "opencode.db"

        # Empty database
        create_opencode_db(db_path, [])

        adapter = OpenCodeAdapter(db_path=db_path, legacy_dir=temp_dir / "nonexistent")

        # Known sessions that no longer exist
        known = {
            "ses_old_001": (1704067200.0, "opencode"),
            "ses_old_002": (1704067200.0, "opencode"),
            "ses_other": (1704067200.0, "claude"),  # Different agent, should not delete
        }

        new_or_modified, deleted_ids = adapter.find_sessions_incremental(known=known)

        assert len(new_or_modified) == 0
        assert set(deleted_ids) == {"ses_old_001", "ses_old_002"}

    def test_incremental_detects_updates(self, temp_dir):
        """Test incremental loading detects updated sessions."""
        db_path = temp_dir / "opencode.db"
        old_ms = 1704067200000
        new_ms = 1704153600000  # 1 day later

        sessions_data = [
            {
                "id": "ses_001",
                "title": "Updated session",
                "directory": "/project1",
                "time_created": old_ms,
                "time_updated": new_ms,
                "messages": [
                    {
                        "id": "msg_001",
                        "role": "user",
                        "parts": [{"id": "prt_001", "type": "text", "text": "Updated"}],
                    }
                ],
            },
        ]

        create_opencode_db(db_path, sessions_data)

        adapter = OpenCodeAdapter(db_path=db_path, legacy_dir=temp_dir / "nonexistent")

        # Known with old mtime
        old_mtime = datetime.fromtimestamp(old_ms / 1000).timestamp()
        known = {"ses_001": (old_mtime, "opencode")}

        new_or_modified, deleted_ids = adapter.find_sessions_incremental(known=known)

        assert len(new_or_modified) == 1
        assert new_or_modified[0].id == "ses_001"
        assert new_or_modified[0].title == "Updated session"

    def test_empty_database(self, temp_dir):
        """Test handling of empty database."""
        db_path = temp_dir / "opencode.db"
        create_opencode_db(db_path, [])

        adapter = OpenCodeAdapter(db_path=db_path, legacy_dir=temp_dir / "nonexistent")
        found = adapter.find_sessions()

        assert found == []

    def test_session_without_messages(self, temp_dir):
        """Test handling of sessions with no messages."""
        db_path = temp_dir / "opencode.db"
        now_ms = int(datetime.now().timestamp() * 1000)

        sessions_data = [
            {
                "id": "ses_empty",
                "title": "Empty session",
                "directory": "/test",
                "time_created": now_ms,
                "time_updated": now_ms,
                "messages": [],
            }
        ]

        create_opencode_db(db_path, sessions_data)

        adapter = OpenCodeAdapter(db_path=db_path, legacy_dir=temp_dir / "nonexistent")
        found = adapter.find_sessions()

        assert len(found) == 1
        assert found[0].title == "Empty session"
        assert found[0].content == ""
        assert found[0].message_count == 0

    def test_get_raw_stats_sqlite(self, temp_dir):
        """Test raw stats with SQLite database."""
        db_path = temp_dir / "opencode.db"
        create_opencode_db(db_path, [])

        adapter = OpenCodeAdapter(
            data_dir=temp_dir,
            db_path=db_path,
            legacy_dir=temp_dir / "nonexistent",
        )
        stats = adapter.get_raw_stats()

        assert stats.available is True
        assert stats.file_count >= 1
        assert stats.total_bytes > 0


# ── Legacy JSON format tests ─────────────────────────────────────────


class TestOpenCodeLegacy:
    """Tests for OpenCode legacy split-JSON format."""

    def test_find_sessions_legacy(self, temp_dir):
        """Test finding sessions from legacy JSON files."""
        legacy_dir = temp_dir / "storage"
        legacy_dir.mkdir()

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
        create_legacy_opencode_structure(legacy_dir, sessions)

        adapter = OpenCodeAdapter(
            legacy_dir=legacy_dir,
            db_path=temp_dir / "nonexistent.db",
        )
        found = adapter.find_sessions()

        assert len(found) == 2
        titles = {s.title for s in found}
        assert "Session 1" in titles
        assert "Session 2" in titles

    def test_parse_legacy_session_basic(self, temp_dir):
        """Test parsing a basic legacy OpenCode session."""
        legacy_dir = temp_dir / "storage"
        legacy_dir.mkdir()

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

        create_legacy_opencode_structure(legacy_dir, sessions)

        session_file = legacy_dir / "session" / "proj_abc" / "ses_ses_001.json"
        messages_by_session, parts_by_message = build_indexes(
            legacy_dir / "message", legacy_dir / "part"
        )

        adapter = OpenCodeAdapter(
            legacy_dir=legacy_dir,
            db_path=temp_dir / "nonexistent.db",
        )
        session = adapter._parse_legacy_session(
            session_file, messages_by_session, parts_by_message
        )

        assert session is not None
        assert session.agent == "opencode"
        assert session.id == "ses_001"
        assert session.title == "Fix database query"
        assert session.directory == "/home/user/project"
        assert "Help me optimize" in session.content
        assert "I'll help you optimize" in session.content

    def test_legacy_uses_file_mtime_without_timestamp(self, temp_dir):
        """Test that file mtime is used when timestamp is missing in legacy."""
        legacy_dir = temp_dir / "storage"
        session_dir = legacy_dir / "session" / "proj_abc"
        session_dir.mkdir(parents=True)

        session_file = session_dir / "ses_test.json"
        with open(session_file, "w") as f:
            json.dump({"id": "test", "title": "Test", "directory": "/test"}, f)

        messages_by_session, parts_by_message = build_indexes(
            legacy_dir / "message", legacy_dir / "part"
        )

        adapter = OpenCodeAdapter(
            legacy_dir=legacy_dir,
            db_path=temp_dir / "nonexistent.db",
        )
        session = adapter._parse_legacy_session(
            session_file, messages_by_session, parts_by_message
        )

        assert session is not None
        assert session.timestamp.year >= 2024

    def test_legacy_uses_updated_time_over_created(self, temp_dir):
        """Test that time.updated is used over time.created in legacy format."""
        legacy_dir = temp_dir / "storage"
        session_dir = legacy_dir / "session" / "proj_abc"
        session_dir.mkdir(parents=True)

        session_file = session_dir / "ses_test.json"
        created_ms = 1704067200000
        updated_ms = 1704153600000
        with open(session_file, "w") as f:
            json.dump(
                {
                    "id": "test",
                    "title": "Test",
                    "directory": "/test",
                    "time": {"created": created_ms, "updated": updated_ms},
                },
                f,
            )

        messages_by_session, parts_by_message = build_indexes(
            legacy_dir / "message", legacy_dir / "part"
        )

        adapter = OpenCodeAdapter(
            legacy_dir=legacy_dir,
            db_path=temp_dir / "nonexistent.db",
        )
        session = adapter._parse_legacy_session(
            session_file, messages_by_session, parts_by_message
        )

        assert session is not None
        expected_timestamp = datetime.fromtimestamp(updated_ms / 1000)
        assert session.timestamp == expected_timestamp

    def test_legacy_skips_non_text_parts(self, temp_dir):
        """Test that non-text parts are skipped in legacy format."""
        legacy_dir = temp_dir / "storage"
        message_dir = legacy_dir / "message"
        part_dir = legacy_dir / "part"

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

        adapter = OpenCodeAdapter(
            legacy_dir=legacy_dir,
            db_path=temp_dir / "nonexistent.db",
        )
        messages = adapter._get_legacy_messages(
            "ses_001", messages_by_session, parts_by_message
        )

        assert len(messages) == 1
        assert "Here's the file content" in messages[0]
        assert "tool_use" not in str(messages)

    def test_legacy_incremental_with_on_session_callback(self, temp_dir):
        """Test that on_session callback is called for each new legacy session."""
        legacy_dir = temp_dir / "storage"
        legacy_dir.mkdir()

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
        create_legacy_opencode_structure(legacy_dir, sessions)

        adapter = OpenCodeAdapter(
            legacy_dir=legacy_dir,
            db_path=temp_dir / "nonexistent.db",
        )

        callback_sessions = []

        def on_session(session):
            callback_sessions.append(session)

        new_or_modified, deleted_ids = adapter.find_sessions_incremental(
            known={}, on_session=on_session
        )

        assert len(callback_sessions) == 2
        callback_ids = {s.id for s in callback_sessions}
        assert "ses_001" in callback_ids
        assert "ses_002" in callback_ids
        assert len(new_or_modified) == 2

    def test_legacy_incremental_no_callback_when_no_changes(self, temp_dir):
        """Test that legacy on_session callback is not called when unchanged."""
        legacy_dir = temp_dir / "storage"
        legacy_dir.mkdir()

        sessions = [
            {
                "id": "ses_001",
                "title": "Session 1",
                "directory": "/project1",
                "project_hash": "proj_aaa",
                "created_ms": 1704067200000,
                "messages": [
                    {
                        "id": "msg_001",
                        "role": "user",
                        "parts": [{"id": "p1", "type": "text", "text": "Hello"}],
                    }
                ],
            },
        ]
        create_legacy_opencode_structure(legacy_dir, sessions)

        adapter = OpenCodeAdapter(
            legacy_dir=legacy_dir,
            db_path=temp_dir / "nonexistent.db",
        )

        mtime = datetime.fromtimestamp(1704067200000 / 1000).timestamp()
        known = {"ses_001": (mtime, "opencode")}

        callback_sessions = []

        def on_session(session):
            callback_sessions.append(session)

        new_or_modified, deleted_ids = adapter.find_sessions_incremental(
            known=known, on_session=on_session
        )

        assert len(callback_sessions) == 0
        assert len(new_or_modified) == 0


# ── Common tests ──────────────────────────────────────────────────────


class TestOpenCodeCommon:
    """Tests for shared behavior across both formats."""

    def test_get_resume_command(self, adapter):
        """Test resume command generation."""
        from fast_resume.adapters.base import Session

        session = Session(
            id="ses_abc123",
            agent="opencode",
            title="Test",
            directory="/home/user/project",
            timestamp=datetime.now(),
            content="",
        )

        cmd = adapter.get_resume_command(session)

        assert cmd == ["opencode", "/home/user/project", "--session", "ses_abc123"]

    def test_find_sessions_returns_empty_when_unavailable(self, adapter):
        """Test that find_sessions returns empty list when unavailable."""
        with patch.object(adapter, "is_available", return_value=False):
            sessions = adapter.find_sessions()
            assert sessions == []

    def test_sqlite_preferred_over_legacy(self, temp_dir):
        """Test that SQLite is preferred when both formats exist."""
        # Create SQLite database
        db_path = temp_dir / "opencode.db"
        now_ms = int(datetime.now().timestamp() * 1000)
        create_opencode_db(
            db_path,
            [
                {
                    "id": "ses_sqlite",
                    "title": "SQLite session",
                    "directory": "/sqlite",
                    "time_created": now_ms,
                    "time_updated": now_ms,
                    "messages": [
                        {
                            "id": "msg_001",
                            "role": "user",
                            "parts": [
                                {
                                    "id": "prt_001",
                                    "type": "text",
                                    "text": "From SQLite",
                                }
                            ],
                        }
                    ],
                }
            ],
        )

        # Create legacy JSON structure
        legacy_dir = temp_dir / "storage"
        legacy_dir.mkdir()
        create_legacy_opencode_structure(
            legacy_dir,
            [
                {
                    "id": "ses_legacy",
                    "title": "Legacy session",
                    "directory": "/legacy",
                    "project_hash": "proj_abc",
                    "messages": [
                        {
                            "id": "msg_002",
                            "role": "user",
                            "parts": [
                                {"id": "p1", "type": "text", "text": "From legacy"}
                            ],
                        }
                    ],
                }
            ],
        )

        adapter = OpenCodeAdapter(db_path=db_path, legacy_dir=legacy_dir)
        found = adapter.find_sessions()

        # Should only return SQLite sessions
        assert len(found) == 1
        assert found[0].id == "ses_sqlite"
        assert found[0].title == "SQLite session"

    def test_is_available_with_sqlite(self, temp_dir):
        """Test is_available returns True when SQLite exists."""
        db_path = temp_dir / "opencode.db"
        create_opencode_db(db_path, [])

        adapter = OpenCodeAdapter(db_path=db_path, legacy_dir=temp_dir / "nonexistent")
        assert adapter.is_available() is True

    def test_is_available_with_legacy(self, temp_dir):
        """Test is_available returns True when legacy dir exists."""
        legacy_dir = temp_dir / "storage"
        session_dir = legacy_dir / "session"
        session_dir.mkdir(parents=True)

        adapter = OpenCodeAdapter(
            db_path=temp_dir / "nonexistent.db", legacy_dir=legacy_dir
        )
        assert adapter.is_available() is True

    def test_is_available_with_nothing(self, temp_dir):
        """Test is_available returns False when nothing exists."""
        adapter = OpenCodeAdapter(
            db_path=temp_dir / "nonexistent.db",
            legacy_dir=temp_dir / "nonexistent",
        )
        assert adapter.is_available() is False

    def test_get_raw_stats_unavailable(self, temp_dir):
        """Test raw stats when unavailable."""
        adapter = OpenCodeAdapter(
            data_dir=temp_dir,
            db_path=temp_dir / "nonexistent.db",
            legacy_dir=temp_dir / "nonexistent",
        )
        stats = adapter.get_raw_stats()

        assert stats.available is False
        assert stats.file_count == 0
        assert stats.total_bytes == 0
