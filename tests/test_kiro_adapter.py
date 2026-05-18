"""Tests for Kiro CLI session adapter."""

import json
from datetime import datetime
from unittest.mock import patch

import pytest

from fast_resume.adapters.base import Session
from fast_resume.adapters.kiro import KiroAdapter


@pytest.fixture
def adapter(temp_dir):
    return KiroAdapter(sessions_dir=temp_dir)


def _write_session(
    sessions_dir,
    session_id="abc-123",
    cwd="/home/user/project",
    title="Write 'p2' to /tmp/p2.txt",
    updated_at="2026-04-28T02:49:27.063836082Z",
    prompts=("Write 'p2' to /tmp/p2.txt",),
    assistant_texts=("",),
    extra_events=(),
):
    meta = {
        "session_id": session_id,
        "cwd": cwd,
        "created_at": "2026-04-28T02:49:18.496286644Z",
        "updated_at": updated_at,
        "title": title,
    }
    meta_path = sessions_dir / f"{session_id}.json"
    with open(meta_path, "w") as f:
        json.dump(meta, f)

    events_path = sessions_dir / f"{session_id}.jsonl"
    with open(events_path, "w") as f:
        for prompt in prompts:
            f.write(
                json.dumps(
                    {
                        "version": "v1",
                        "kind": "Prompt",
                        "data": {
                            "message_id": "m1",
                            "content": [{"kind": "text", "data": prompt}],
                            "meta": {"timestamp": 1777344563},
                        },
                    }
                )
                + "\n"
            )
        for txt in assistant_texts:
            f.write(
                json.dumps(
                    {
                        "version": "v1",
                        "kind": "AssistantMessage",
                        "data": {
                            "message_id": "m2",
                            "content": [
                                {"kind": "text", "data": txt},
                                {"kind": "toolUse", "data": {"name": "write"}},
                            ],
                        },
                    }
                )
                + "\n"
            )
        for line in extra_events:
            f.write(json.dumps(line) + "\n")

    return meta_path, events_path


class TestKiroAdapter:
    def test_name_and_attributes(self, adapter):
        assert adapter.name == "kiro"
        assert adapter.color is not None
        assert adapter.badge == "kiro"
        assert adapter.supports_yolo is True

    def test_parse_session_basic(self, temp_dir, adapter):
        meta_path, _ = _write_session(
            temp_dir,
            session_id="basic-1",
            title="My Kiro task",
            prompts=("Implement OAuth", "Add tests"),
            assistant_texts=("Sure, I'll start with OAuth.",),
        )

        session = adapter._parse_session_file(meta_path)

        assert session is not None
        assert session.id == "basic-1"
        assert session.agent == "kiro"
        assert session.directory == "/home/user/project"
        assert session.title == "My Kiro task"
        assert "Implement OAuth" in session.content
        assert "» Implement OAuth" in session.content
        assert "  Sure, I'll start with OAuth." in session.content
        assert session.message_count == 3  # 2 user + 1 assistant

    def test_parse_session_falls_back_to_first_prompt_for_title(
        self, temp_dir, adapter
    ):
        meta_path, _ = _write_session(
            temp_dir,
            session_id="notitle",
            title="",
            prompts=("Fix the failing build pipeline immediately",),
            assistant_texts=(),
        )

        session = adapter._parse_session_file(meta_path)

        assert session is not None
        assert "Fix the failing build pipeline" in session.title

    def test_parse_session_truncates_long_fallback_title(self, temp_dir, adapter):
        long_prompt = "A" * 200
        meta_path, _ = _write_session(
            temp_dir,
            session_id="long",
            title="",
            prompts=(long_prompt,),
            assistant_texts=(),
        )

        session = adapter._parse_session_file(meta_path)

        assert session is not None
        assert session.title.endswith("...")
        assert len(session.title) <= 83

    def test_parse_session_default_title_when_no_prompt(self, temp_dir, adapter):
        meta_path, _ = _write_session(
            temp_dir,
            session_id="empty",
            title="",
            prompts=(),
            assistant_texts=("hello",),
        )

        session = adapter._parse_session_file(meta_path)

        assert session is not None
        assert session.title == "Kiro session"

    def test_parse_session_uses_meta_timestamp(self, temp_dir, adapter):
        meta_path, _ = _write_session(
            temp_dir,
            session_id="ts",
            updated_at="2025-09-04T12:00:00Z",
        )

        session = adapter._parse_session_file(meta_path)

        assert session is not None
        assert session.timestamp.year == 2025
        assert session.timestamp.tzinfo is None

    def test_parse_session_skips_tool_results(self, temp_dir, adapter):
        meta_path, _ = _write_session(
            temp_dir,
            session_id="tools",
            prompts=("do thing",),
            assistant_texts=("ok",),
            extra_events=(
                {
                    "version": "v1",
                    "kind": "ToolResults",
                    "data": {
                        "message_id": "tr",
                        "content": [
                            {
                                "kind": "toolResult",
                                "data": {
                                    "toolUseId": "x",
                                    "content": [
                                        {
                                            "kind": "text",
                                            "data": "SHOULD_NOT_BE_INDEXED",
                                        }
                                    ],
                                    "status": "success",
                                },
                            }
                        ],
                    },
                },
            ),
        )

        session = adapter._parse_session_file(meta_path)

        assert session is not None
        assert "SHOULD_NOT_BE_INDEXED" not in session.content

    def test_parse_session_handles_missing_events_file(self, temp_dir, adapter):
        meta = {
            "session_id": "no-events",
            "cwd": "/x",
            "title": "Stub",
            "updated_at": "2026-04-28T02:49:27Z",
        }
        meta_path = temp_dir / "no-events.json"
        with open(meta_path, "w") as f:
            json.dump(meta, f)

        session = adapter._parse_session_file(meta_path)

        assert session is not None
        assert session.title == "Stub"
        assert session.content == ""

    def test_parse_session_skips_malformed_jsonl_lines(self, temp_dir, adapter):
        meta_path, events_path = _write_session(
            temp_dir,
            session_id="bad",
            prompts=("hello",),
        )
        with open(events_path, "a") as f:
            f.write("not valid json\n")
            f.write(
                json.dumps(
                    {
                        "version": "v1",
                        "kind": "AssistantMessage",
                        "data": {
                            "message_id": "m3",
                            "content": [{"kind": "text", "data": "still readable"}],
                        },
                    }
                )
                + "\n"
            )

        session = adapter._parse_session_file(meta_path)

        assert session is not None
        assert "still readable" in session.content

    def test_parse_session_invalid_meta_json_returns_none(self, temp_dir, adapter):
        meta_path = temp_dir / "broken.json"
        with open(meta_path, "w") as f:
            f.write("{not valid json")

        assert adapter._parse_session_file(meta_path) is None

    def test_find_sessions(self, temp_dir, adapter):
        _write_session(temp_dir, session_id="s1", prompts=("a",))
        _write_session(temp_dir, session_id="s2", prompts=("b",))
        _write_session(temp_dir, session_id="s3", prompts=("c",))

        sessions = adapter.find_sessions()

        ids = sorted(s.id for s in sessions)
        assert ids == ["s1", "s2", "s3"]

    def test_find_sessions_returns_empty_when_unavailable(self, adapter):
        with patch.object(adapter, "is_available", return_value=False):
            assert adapter.find_sessions() == []

    def test_scan_session_files_uses_latest_mtime(self, temp_dir, adapter):
        import os
        import time

        meta_path, events_path = _write_session(
            temp_dir,
            session_id="scan",
        )
        # Force events file to be newer than meta file.
        future = time.time() + 100
        os.utime(meta_path, (future - 50, future - 50))
        os.utime(events_path, (future, future))

        files = adapter._scan_session_files()

        assert "scan" in files
        _, mtime = files["scan"]
        assert mtime == pytest.approx(future, abs=1)

    def test_get_resume_command(self, adapter):
        session = Session(
            id="kiro-xyz",
            agent="kiro",
            title="t",
            directory="/x",
            timestamp=datetime.now(),
            content="",
        )

        assert adapter.get_resume_command(session) == [
            "kiro-cli",
            "chat",
            "--resume-id",
            "kiro-xyz",
        ]

    def test_get_resume_command_with_yolo(self, adapter):
        session = Session(
            id="kiro-xyz",
            agent="kiro",
            title="t",
            directory="/x",
            timestamp=datetime.now(),
            content="",
        )

        assert adapter.get_resume_command(session, yolo=True) == [
            "kiro-cli",
            "chat",
            "--trust-all-tools",
            "--resume-id",
            "kiro-xyz",
        ]
