"""Tests for Cursor session adapter."""

import json
import sqlite3
from datetime import datetime
from pathlib import Path

import pytest

from fast_resume.adapters.base import Session
from fast_resume.adapters.cursor import CursorAdapter
from fast_resume.index import TantivyIndex
from fast_resume.search import SessionSearch


def create_workspace_db(db_path: Path, composers: list[dict]) -> None:
    """Create a minimal Cursor workspace state.vscdb."""
    conn = sqlite3.connect(str(db_path))
    cursor = conn.cursor()
    cursor.execute("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT)")
    payload = json.dumps({"allComposers": composers})
    cursor.execute(
        "INSERT INTO ItemTable (key, value) VALUES (?, ?)",
        ("composer.composerData", payload),
    )
    conn.commit()
    conn.close()


def create_global_db(db_path: Path, rows: list[tuple[str, str]]) -> None:
    """Create a minimal Cursor global state.vscdb."""
    conn = sqlite3.connect(str(db_path))
    cursor = conn.cursor()
    cursor.execute("CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value TEXT)")
    cursor.executemany("INSERT INTO cursorDiskKV (key, value) VALUES (?, ?)", rows)
    conn.commit()
    conn.close()


@pytest.fixture
def cursor_fixture(temp_dir):
    """Create a realistic Cursor storage layout with one active session."""
    user_dir = temp_dir / "Cursor" / "User"
    workspace_storage = user_dir / "workspaceStorage"
    workspace = workspace_storage / "ws-1"
    workspace.mkdir(parents=True)

    project_dir = temp_dir / "my project"
    project_dir.mkdir(parents=True)
    with open(workspace / "workspace.json", "w") as f:
        json.dump({"folder": project_dir.as_uri()}, f)

    composers = [
        {
            "composerId": "cmp-active-1",
            "name": "Debug auth flow",
            "createdAt": 1704067200000,
            "lastUpdatedAt": 1704067300000,
            "isArchived": False,
        },
        {
            "composerId": "cmp-archived-1",
            "name": "Old chat",
            "createdAt": 1704067100000,
            "lastUpdatedAt": 1704067150000,
            "isArchived": True,
        },
    ]
    create_workspace_db(workspace / "state.vscdb", composers)

    global_storage = user_dir / "globalStorage"
    global_storage.mkdir(parents=True)
    create_global_db(
        global_storage / "state.vscdb",
        [
            (
                "composerData:cmp-active-1",
                json.dumps(
                    {
                        "fullConversationHeadersOnly": [
                            {"bubbleId": "u-1", "type": 1},
                            {"bubbleId": "a-1", "type": 2},
                        ]
                    }
                ),
            ),
            (
                "bubbleId:cmp-active-1:u-1",
                json.dumps({"bubbleId": "u-1", "text": "How do I fix auth?"}),
            ),
            (
                "bubbleId:cmp-active-1:a-1",
                json.dumps(
                    {
                        "bubbleId": "a-1",
                        "text": "Check token validation.",
                        "toolFormerData": {"name": "read_file"},
                    }
                ),
            ),
        ],
    )

    return {
        "user_dir": user_dir,
        "workspace_storage": workspace_storage,
        "global_db": global_storage / "state.vscdb",
        "project_dir": project_dir,
    }


class TestCursorAdapter:
    """Tests for CursorAdapter."""

    def test_name_and_attributes(self):
        adapter = CursorAdapter()
        assert adapter.name == "cursor"
        assert adapter.badge == "cursor"
        assert adapter.supports_yolo is False

    def test_find_sessions_parses_sqlite_data(self, cursor_fixture):
        adapter = CursorAdapter(
            user_dir=cursor_fixture["user_dir"],
            global_db_path=cursor_fixture["global_db"],
            workspace_storage_dir=cursor_fixture["workspace_storage"],
        )

        sessions = adapter.find_sessions()
        assert len(sessions) == 1

        session = sessions[0]
        assert session.id == "cursor:cmp-active-1"
        assert session.agent == "cursor"
        assert session.title == "Debug auth flow"
        assert session.directory == str(cursor_fixture["project_dir"])
        assert "How do I fix auth?" in session.content
        assert "[tool read_file]" in session.content
        assert "Check token validation." in session.content
        assert session.message_count == 2

    def test_incremental_detects_new_and_unchanged(self, cursor_fixture):
        adapter = CursorAdapter(
            user_dir=cursor_fixture["user_dir"],
            global_db_path=cursor_fixture["global_db"],
            workspace_storage_dir=cursor_fixture["workspace_storage"],
        )

        new_sessions, deleted = adapter.find_sessions_incremental({})
        assert len(new_sessions) == 1
        assert deleted == []

        known = {"cursor:cmp-active-1": (new_sessions[0].mtime, "cursor")}
        unchanged, deleted = adapter.find_sessions_incremental(known)
        assert unchanged == []
        assert deleted == []

    def test_incremental_detects_deleted_when_unavailable(self):
        adapter = CursorAdapter(
            user_dir=Path("/does/not/exist"),
            global_db_path=Path("/does/not/exist/global.vscdb"),
            workspace_storage_dir=Path("/does/not/exist/workspaceStorage"),
        )
        known = {"cursor:missing": (123.0, "cursor")}
        sessions, deleted = adapter.find_sessions_incremental(known)
        assert sessions == []
        assert deleted == ["cursor:missing"]

    def test_resume_command_opens_workspace(self):
        adapter = CursorAdapter()
        cmd = adapter.get_resume_command(
            Session(
                id="cursor:abc",
                agent="cursor",
                title="t",
                directory="/tmp/project",
                timestamp=datetime.now(),
                content="",
            )
        )
        assert cmd == ["cursor", "/tmp/project"]

    def test_find_sessions_works_without_global_db(self, temp_dir):
        user_dir = temp_dir / "Cursor" / "User"
        workspace_storage = user_dir / "workspaceStorage"
        workspace = workspace_storage / "ws-1"
        workspace.mkdir(parents=True)

        project_dir = temp_dir / "repo"
        project_dir.mkdir(parents=True)
        with open(workspace / "workspace.json", "w") as f:
            json.dump({"folder": project_dir.as_uri()}, f)

        create_workspace_db(
            workspace / "state.vscdb",
            [
                {
                    "composerId": "cmp-1",
                    "name": "Fallback title",
                    "createdAt": 1704067200000,
                    "lastUpdatedAt": 1704067300000,
                    "isArchived": False,
                }
            ],
        )

        adapter = CursorAdapter(
            user_dir=user_dir,
            global_db_path=user_dir / "globalStorage" / "state.vscdb",
            workspace_storage_dir=workspace_storage,
        )
        sessions = adapter.find_sessions()
        assert len(sessions) == 1
        assert sessions[0].content == "Fallback title"

    def test_parses_bubbles_with_control_characters(self, temp_dir):
        user_dir = temp_dir / "Cursor" / "User"
        workspace_storage = user_dir / "workspaceStorage"
        workspace = workspace_storage / "ws-1"
        workspace.mkdir(parents=True)

        project_dir = temp_dir / "repo"
        project_dir.mkdir(parents=True)
        with open(workspace / "workspace.json", "w") as f:
            json.dump({"folder": project_dir.as_uri()}, f)

        create_workspace_db(
            workspace / "state.vscdb",
            [
                {
                    "composerId": "cmp-control",
                    "name": "Control chars",
                    "createdAt": 1704067200000,
                    "lastUpdatedAt": 1704067300000,
                    "isArchived": False,
                }
            ],
        )

        global_storage = user_dir / "globalStorage"
        global_storage.mkdir(parents=True)
        raw_bubble = '{"bubbleId":"a-1","text":"hello' + chr(1) + 'world"}'
        create_global_db(
            global_storage / "state.vscdb",
            [
                (
                    "composerData:cmp-control",
                    json.dumps(
                        {
                            "fullConversationHeadersOnly": [
                                {"bubbleId": "a-1", "type": 2}
                            ]
                        }
                    ),
                ),
                (
                    "bubbleId:cmp-control:a-1",
                    raw_bubble,
                ),
            ],
        )

        adapter = CursorAdapter(
            user_dir=user_dir,
            global_db_path=global_storage / "state.vscdb",
            workspace_storage_dir=workspace_storage,
        )
        sessions = adapter.find_sessions()
        assert len(sessions) == 1
        assert "hello world" in sessions[0].content

    def test_get_raw_stats_counts_databases(self, cursor_fixture):
        adapter = CursorAdapter(
            user_dir=cursor_fixture["user_dir"],
            global_db_path=cursor_fixture["global_db"],
            workspace_storage_dir=cursor_fixture["workspace_storage"],
        )

        stats = adapter.get_raw_stats()
        assert stats.available is True
        assert stats.file_count >= 2  # global + at least one workspace DB
        assert stats.total_bytes > 0


class TestCursorSearchIntegration:
    """Cursor adapter integration with SessionSearch + Tantivy."""

    def test_search_finds_cursor_session(self, cursor_fixture, temp_dir):
        adapter = CursorAdapter(
            user_dir=cursor_fixture["user_dir"],
            global_db_path=cursor_fixture["global_db"],
            workspace_storage_dir=cursor_fixture["workspace_storage"],
        )

        search = SessionSearch()
        search.adapters = [adapter]
        search._index = TantivyIndex(index_path=temp_dir / "index")

        sessions = search.get_all_sessions()
        assert len(sessions) == 1
        assert sessions[0].agent == "cursor"

        results = search.search("token validation")
        assert len(results) == 1
        assert results[0].id == "cursor:cmp-active-1"

        filtered = search.search("", agent_filter="cursor")
        assert len(filtered) == 1
