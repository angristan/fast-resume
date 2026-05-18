"""Tests for Gemini CLI session adapter."""

import json
from datetime import datetime
from unittest.mock import patch

import pytest

from fast_resume.adapters.base import Session
from fast_resume.adapters.gemini import GeminiAdapter


@pytest.fixture
def gemini_root(temp_dir):
    """A fake ~/.gemini layout with a projects.json and an empty tmp/."""
    (temp_dir / "tmp").mkdir()
    return temp_dir


@pytest.fixture
def adapter(gemini_root):
    return GeminiAdapter(sessions_dir=gemini_root)


def _make_projects(root, mapping):
    with open(root / "projects.json", "w") as f:
        json.dump({"projects": mapping}, f)


def _make_chats_dir(root, slug):
    chats = root / "tmp" / slug / "chats"
    chats.mkdir(parents=True)
    return chats


def _write_json_session(chats_dir, name, *, session_id, messages, **meta):
    payload = {
        "sessionId": session_id,
        "projectHash": "deadbeef",
        "startTime": meta.get("startTime", "2026-02-28T10:49:27.072Z"),
        "lastUpdated": meta.get("lastUpdated", "2026-02-28T10:50:47.266Z"),
        "messages": messages,
    }
    path = chats_dir / f"{name}.json"
    with open(path, "w") as f:
        json.dump(payload, f)
    return path


def _write_jsonl_session(chats_dir, name, *, session_id, lines, **meta):
    header = {
        "sessionId": session_id,
        "projectHash": "deadbeef",
        "startTime": meta.get("startTime", "2026-05-09T06:54:33.604Z"),
        "lastUpdated": meta.get("lastUpdated", "2026-05-09T06:54:33.604Z"),
        "kind": "main",
    }
    path = chats_dir / f"{name}.jsonl"
    with open(path, "w") as f:
        f.write(json.dumps(header) + "\n")
        for line in lines:
            f.write(json.dumps(line) + "\n")
    return path


class TestGeminiAdapter:
    def test_name_and_attributes(self, adapter):
        assert adapter.name == "gemini"
        assert adapter.color is not None
        assert adapter.badge == "gemini"
        assert adapter.supports_yolo is True

    def test_is_available_requires_tmp_dir(self, temp_dir):
        ad = GeminiAdapter(sessions_dir=temp_dir / "missing")
        assert ad.is_available() is False
        ad2 = GeminiAdapter(sessions_dir=temp_dir)
        # `temp_dir` itself has no `tmp/` yet -> unavailable.
        assert ad2.is_available() is False
        (temp_dir / "tmp").mkdir()
        assert GeminiAdapter(sessions_dir=temp_dir).is_available() is True

    def test_parse_json_session(self, gemini_root, adapter):
        _make_projects(gemini_root, {"/home/cody/git": "git"})
        chats = _make_chats_dir(gemini_root, "git")
        path = _write_json_session(
            chats,
            "session-2026-02-28T10-48-ac289373",
            session_id="ac289373-c1a3-47c3-b073-9580fd7208ef",
            messages=[
                {"type": "info", "content": "ignore me"},
                {
                    "type": "user",
                    "content": [{"text": "How do I deploy to staging?"}],
                },
                {"type": "gemini", "content": "Use the staging script."},
                {"type": "error", "content": "API rate limit"},
            ],
        )

        session = adapter._parse_session_file(path)

        assert session is not None
        assert session.id == "ac289373-c1a3-47c3-b073-9580fd7208ef"
        assert session.agent == "gemini"
        assert session.directory == "/home/cody/git"
        assert session.title.startswith("How do I deploy to staging?")
        assert "» How do I deploy to staging?" in session.content
        assert "  Use the staging script." in session.content
        # info + error rows must be excluded.
        assert "ignore me" not in session.content
        assert "API rate limit" not in session.content
        assert session.message_count == 2

    def test_parse_jsonl_session_dedupes_repeated_message_ids(
        self, gemini_root, adapter
    ):
        _make_projects(gemini_root, {"/home/cody": "cody"})
        chats = _make_chats_dir(gemini_root, "cody")
        path = _write_jsonl_session(
            chats,
            "session-2026-05-09T06-54-7c6dccb5",
            session_id="7c6dccb5-f136-49da-972b-b325489a3537",
            lines=[
                {"$set": {"lastUpdated": "2026-05-09T06:55:00Z"}},
                {
                    "id": "msg-1",
                    "type": "user",
                    "content": [{"text": "Update mise versions"}],
                },
                {"$set": {"lastUpdated": "2026-05-09T06:55:01Z"}},
                {
                    "id": "msg-2",
                    "type": "gemini",
                    "content": "Updated tools to latest.",
                },
                # Re-emitted same message id (Gemini does this when toolCalls
                # land later); must not show up twice in the indexed content.
                {
                    "id": "msg-2",
                    "type": "gemini",
                    "content": "Updated tools to latest.",
                    "toolCalls": [{"name": "replace"}],
                },
            ],
        )

        session = adapter._parse_session_file(path)

        assert session is not None
        assert session.id == "7c6dccb5-f136-49da-972b-b325489a3537"
        assert session.directory == "/home/cody"
        assert session.content.count("Updated tools to latest.") == 1
        assert session.message_count == 2
        # `$set` patches should land on the session metadata too.
        assert session.timestamp.year == 2026

    def test_parse_session_skips_when_no_user_message(self, gemini_root, adapter):
        _make_projects(gemini_root, {"/x": "x"})
        chats = _make_chats_dir(gemini_root, "x")
        path = _write_json_session(
            chats,
            "session-empty",
            session_id="empty-1",
            messages=[
                {"type": "info", "content": "starting up"},
                {"type": "error", "content": "boom"},
            ],
        )

        assert adapter._parse_session_file(path) is None

    def test_parse_session_directory_missing_when_slug_unmapped(
        self, gemini_root, adapter
    ):
        _make_projects(gemini_root, {})  # no slug mapping
        chats = _make_chats_dir(gemini_root, "rogue")
        path = _write_json_session(
            chats,
            "session-rogue",
            session_id="rogue-1",
            messages=[{"type": "user", "content": "hello"}],
        )

        session = adapter._parse_session_file(path)

        assert session is not None
        assert session.directory == ""

    def test_parse_session_handles_string_content(self, gemini_root, adapter):
        _make_projects(gemini_root, {"/y": "y"})
        chats = _make_chats_dir(gemini_root, "y")
        path = _write_json_session(
            chats,
            "session-string",
            session_id="str-1",
            messages=[
                {"type": "user", "content": "plain string prompt"},
                {"type": "gemini", "content": "plain string reply"},
            ],
        )

        session = adapter._parse_session_file(path)

        assert session is not None
        assert "» plain string prompt" in session.content
        assert "  plain string reply" in session.content

    def test_find_sessions_walks_all_project_slugs(self, gemini_root, adapter):
        _make_projects(
            gemini_root,
            {"/home/cody/git": "git", "/home/cody/git/kiro-agents": "kiro-agents"},
        )
        chats_git = _make_chats_dir(gemini_root, "git")
        chats_kiro = _make_chats_dir(gemini_root, "kiro-agents")
        _write_json_session(
            chats_git,
            "session-a",
            session_id="a",
            messages=[{"type": "user", "content": "first"}],
        )
        _write_jsonl_session(
            chats_kiro,
            "session-b",
            session_id="b",
            lines=[{"id": "1", "type": "user", "content": "second"}],
        )

        sessions = adapter.find_sessions()

        ids = sorted(s.id for s in sessions)
        assert ids == ["a", "b"]
        by_id = {s.id: s for s in sessions}
        assert by_id["a"].directory == "/home/cody/git"
        assert by_id["b"].directory == "/home/cody/git/kiro-agents"

    def test_find_sessions_returns_empty_when_unavailable(self, adapter):
        with patch.object(adapter, "is_available", return_value=False):
            assert adapter.find_sessions() == []

    def test_scan_session_files_prefers_newer_file_per_id(self, gemini_root, adapter):
        """When two files (legacy `.json` + newer `.jsonl`) share a
        `sessionId`, `_scan_session_files` keeps whichever has the newer
        mtime."""
        import os
        import time

        _make_projects(gemini_root, {"/z": "z"})
        chats = _make_chats_dir(gemini_root, "z")
        session_id = "dup-session-id"
        older = _write_json_session(
            chats,
            "session-legacy",
            session_id=session_id,
            messages=[{"type": "user", "content": "x"}],
        )
        newer = _write_jsonl_session(
            chats,
            "session-current",
            session_id=session_id,
            lines=[{"id": "1", "type": "user", "content": "x"}],
        )
        now = time.time()
        os.utime(older, (now - 100, now - 100))
        os.utime(newer, (now, now))

        files = adapter._scan_session_files()

        # Scanner keys by the parsed sessionId (so it matches the index's
        # stored ids), and the newer file wins on collision.
        assert session_id in files
        path, _ = files[session_id]
        assert path == newer

    def test_find_sessions_does_not_double_yield_pair(self, gemini_root, adapter):
        """A legacy `.json` and newer `.jsonl` with the same stem must not
        produce two `Session` objects from `find_sessions`."""
        _make_projects(gemini_root, {"/x": "x"})
        chats = _make_chats_dir(gemini_root, "x")
        stem = "session-pair-2026-05-09-deadbeef"
        _write_json_session(
            chats,
            stem,
            session_id="paired",
            messages=[{"type": "user", "content": "hi"}],
        )
        _write_jsonl_session(
            chats,
            stem,
            session_id="paired",
            lines=[{"id": "1", "type": "user", "content": "hi"}],
        )

        sessions = adapter.find_sessions()

        assert len(sessions) == 1

    def test_incremental_scan_matches_parsed_session_id(self, gemini_root, adapter):
        """The scan key MUST equal the parsed `sessionId`.

        If `_session_id_quick` returns the filename stem but the parsed
        `Session.id` is the file's `sessionId` UUID, the incremental flow
        in `BaseSessionAdapter.find_sessions_incremental` joins the wrong
        keys and re-parses every session on every run. This regression test
        guards that contract.
        """
        _make_projects(gemini_root, {"/home/cody": "cody"})
        chats = _make_chats_dir(gemini_root, "cody")
        full_uuid = "7c6dccb5-f136-49da-972b-b325489a3537"
        stem = "session-2026-05-09T06-54-7c6dccb5"
        _write_jsonl_session(
            chats,
            stem,
            session_id=full_uuid,
            lines=[{"id": "1", "type": "user", "content": "hello"}],
        )

        files = adapter._scan_session_files()
        # The scan must key by the parsed sessionId, not the filename stem,
        # so that `known.get(scan_key)` lines up with what was stored.
        assert full_uuid in files
        assert stem not in files

    def test_incremental_skips_known_session(self, gemini_root, adapter):
        """A second scan that matches `known` should report zero changes.

        Uses a `sessionId` value that DOES NOT appear in the filename stem,
        so the test fails if `_session_id_quick` is ever regressed back to
        returning the stem (which would defeat the incremental join).
        """
        _make_projects(gemini_root, {"/home/cody": "cody"})
        chats = _make_chats_dir(gemini_root, "cody")
        full_uuid = "ffffffff-1111-2222-3333-444444444444"  # not in stem
        _write_jsonl_session(
            chats,
            "session-stable",
            session_id=full_uuid,
            lines=[{"id": "1", "type": "user", "content": "hi"}],
        )

        # First pass to find the session and its mtime.
        first_pass, deleted = adapter.find_sessions_incremental({})
        assert len(first_pass) == 1
        assert first_pass[0].id == full_uuid
        assert deleted == []
        known = {first_pass[0].id: (first_pass[0].mtime, "gemini")}

        # Second pass with matching `known` should yield no changes.
        second_pass, deleted = adapter.find_sessions_incremental(known)
        assert second_pass == []
        assert deleted == []

    def test_session_id_quick_skips_set_patches_before_header(
        self, gemini_root, adapter
    ):
        """If Gemini ever emits a `$set` patch before the session header,
        `_session_id_quick` must scan past it to find `sessionId`."""
        _make_projects(gemini_root, {"/home/cody": "cody"})
        chats = _make_chats_dir(gemini_root, "cody")
        target_uuid = "deadbeef-cafe-0000-1111-feedfacefeed"
        path = chats / "session-out-of-order.jsonl"
        with open(path, "w") as f:
            # Simulate a malformed/out-of-order JSONL: a $set patch on line 1
            # before the session header lands.
            f.write(
                json.dumps({"$set": {"lastUpdated": "2026-05-09T07:00:00Z"}}) + "\n"
            )
            f.write(
                json.dumps(
                    {
                        "sessionId": target_uuid,
                        "projectHash": "abc",
                        "startTime": "2026-05-09T06:54:33Z",
                        "lastUpdated": "2026-05-09T06:54:33Z",
                        "kind": "main",
                    }
                )
                + "\n"
            )
            f.write(json.dumps({"id": "1", "type": "user", "content": "hello"}) + "\n")

        assert adapter._session_id_quick(path) == target_uuid

    def test_get_resume_command(self, adapter):
        session = Session(
            id="gem-1",
            agent="gemini",
            title="t",
            directory="/d",
            timestamp=datetime.now(),
            content="",
        )

        assert adapter.get_resume_command(session) == [
            "gemini",
            "--resume",
            "gem-1",
        ]

    def test_get_resume_command_yolo(self, adapter):
        session = Session(
            id="gem-1",
            agent="gemini",
            title="t",
            directory="/d",
            timestamp=datetime.now(),
            content="",
        )

        assert adapter.get_resume_command(session, yolo=True) == [
            "gemini",
            "--yolo",
            "--resume",
            "gem-1",
        ]
