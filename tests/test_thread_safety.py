"""Thread safety tests for SessionSearch.

Tests concurrent access patterns that occur when the TUI runs background
loading while the main thread performs searches.
"""

import threading
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime
from pathlib import Path
from unittest.mock import MagicMock


from fast_resume.adapters.base import Session
from fast_resume.search import SessionSearch


def make_session(id: str, agent: str = "claude", title: str = "Test") -> Session:
    """Create a test session."""
    return Session(
        id=id,
        agent=agent,
        title=title,
        directory="/tmp/test",
        timestamp=datetime.now(),
        preview="test preview",
        content="test content for searching",
        message_count=1,
        mtime=time.time(),
        yolo=False,
    )


class TestSessionSearchThreadSafety:
    """Test concurrent access to SessionSearch."""

    def test_concurrent_reads_from_sessions_by_id(self, tmp_path: Path):
        """Verify multiple readers can safely access _sessions_by_id."""
        search = SessionSearch()
        search._index = MagicMock()
        search._index.index_path = tmp_path / "index"

        # Pre-populate with sessions
        with search._lock:
            for i in range(100):
                session = make_session(f"session-{i}")
                search._sessions_by_id[session.id] = session

        errors: list[Exception] = []
        results: list[int] = []

        def reader():
            try:
                for _ in range(50):
                    with search._lock:
                        count = len(list(search._sessions_by_id.values()))
                    results.append(count)
                    time.sleep(0.001)
            except Exception as e:
                errors.append(e)

        # Run multiple readers concurrently
        threads = [threading.Thread(target=reader) for _ in range(5)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert not errors, f"Errors during concurrent reads: {errors}"
        assert all(r == 100 for r in results), "All reads should see 100 sessions"

    def test_concurrent_read_write_sessions_by_id(self, tmp_path: Path):
        """Verify readers and writers can safely access _sessions_by_id concurrently."""
        search = SessionSearch()
        search._index = MagicMock()
        search._index.index_path = tmp_path / "index"

        errors: list[Exception] = []
        write_count = 0
        read_counts: list[int] = []

        def writer():
            nonlocal write_count
            try:
                for i in range(100):
                    session = make_session(f"session-{i}")
                    with search._lock:
                        search._sessions_by_id[session.id] = session
                    write_count += 1
                    time.sleep(0.001)
            except Exception as e:
                errors.append(e)

        def reader():
            try:
                for _ in range(100):
                    with search._lock:
                        count = len(search._sessions_by_id)
                    read_counts.append(count)
                    time.sleep(0.001)
            except Exception as e:
                errors.append(e)

        # Run writer and readers concurrently
        threads = [
            threading.Thread(target=writer),
            threading.Thread(target=reader),
            threading.Thread(target=reader),
        ]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert not errors, f"Errors during concurrent access: {errors}"
        assert write_count == 100, "Writer should complete all writes"
        # Read counts should be monotonically non-decreasing (within each reader)
        # and final state should have all sessions
        assert len(search._sessions_by_id) == 100

    def test_concurrent_pop_and_read(self, tmp_path: Path):
        """Verify pop operations are safe during concurrent reads."""
        search = SessionSearch()
        search._index = MagicMock()
        search._index.index_path = tmp_path / "index"

        # Pre-populate
        with search._lock:
            for i in range(100):
                session = make_session(f"session-{i}")
                search._sessions_by_id[session.id] = session

        errors: list[Exception] = []

        def deleter():
            try:
                for i in range(50):
                    with search._lock:
                        search._sessions_by_id.pop(f"session-{i}", None)
                    time.sleep(0.001)
            except Exception as e:
                errors.append(e)

        def reader():
            try:
                for _ in range(100):
                    with search._lock:
                        # This should never raise even if items are being deleted
                        _ = list(search._sessions_by_id.values())
                    time.sleep(0.001)
            except Exception as e:
                errors.append(e)

        threads = [
            threading.Thread(target=deleter),
            threading.Thread(target=reader),
            threading.Thread(target=reader),
        ]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert not errors, f"Errors during concurrent pop/read: {errors}"
        assert len(search._sessions_by_id) == 50, "50 sessions should remain"

    def test_streaming_flag_thread_safety(self, tmp_path: Path):
        """Verify _streaming_in_progress flag is safely accessed."""
        search = SessionSearch()
        search._index = MagicMock()
        search._index.index_path = tmp_path / "index"

        errors: list[Exception] = []
        flag_values: list[bool] = []

        def toggler():
            try:
                for _ in range(100):
                    with search._lock:
                        search._streaming_in_progress = True
                    time.sleep(0.001)
                    with search._lock:
                        search._streaming_in_progress = False
                    time.sleep(0.001)
            except Exception as e:
                errors.append(e)

        def reader():
            try:
                for _ in range(200):
                    with search._lock:
                        flag_values.append(search._streaming_in_progress)
                    time.sleep(0.001)
            except Exception as e:
                errors.append(e)

        threads = [
            threading.Thread(target=toggler),
            threading.Thread(target=reader),
        ]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert not errors, f"Errors during flag access: {errors}"
        # Flag should only ever be True or False
        assert all(isinstance(v, bool) for v in flag_values)

    def test_search_during_simulated_streaming(self, tmp_path: Path):
        """Simulate search queries during streaming updates."""
        search = SessionSearch()

        # Mock the index to avoid actual file operations
        mock_index = MagicMock()
        mock_index.index_path = tmp_path / "index"
        mock_index.get_known_sessions.return_value = {}
        mock_index.get_all_sessions.return_value = []
        mock_index.search.return_value = []
        mock_index.get_session_count.return_value = 0
        search._index = mock_index

        errors: list[Exception] = []
        search_results: list[int] = []

        def simulate_streaming():
            """Simulate what get_sessions_streaming does."""
            try:
                with search._lock:
                    search._streaming_in_progress = True

                for i in range(50):
                    session = make_session(f"stream-session-{i}")
                    with search._lock:
                        search._sessions_by_id[session.id] = session
                    time.sleep(0.002)

                with search._lock:
                    search._streaming_in_progress = False
            except Exception as e:
                errors.append(e)

        def do_searches():
            """Perform searches during streaming."""
            try:
                for _ in range(30):
                    # This mimics what search() does - read with lock
                    with search._lock:
                        sessions = list(search._sessions_by_id.values())
                    search_results.append(len(sessions))
                    time.sleep(0.003)
            except Exception as e:
                errors.append(e)

        threads = [
            threading.Thread(target=simulate_streaming),
            threading.Thread(target=do_searches),
            threading.Thread(target=do_searches),
        ]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert not errors, f"Errors during search/streaming: {errors}"
        # Final state should have all streamed sessions
        assert len(search._sessions_by_id) == 50


class TestSessionSearchLockBehavior:
    """Test that locks are properly reentrant and don't deadlock."""

    def test_reentrant_lock_allows_nested_acquisition(self, tmp_path: Path):
        """Verify RLock allows same thread to acquire lock multiple times."""
        search = SessionSearch()
        search._index = MagicMock()
        search._index.index_path = tmp_path / "index"

        # This should not deadlock because we use RLock
        with search._lock:
            with search._lock:
                with search._lock:
                    search._sessions_by_id["test"] = make_session("test")

        assert "test" in search._sessions_by_id

    def test_lock_released_on_exception(self, tmp_path: Path):
        """Verify lock is released even when exception occurs."""
        search = SessionSearch()
        search._index = MagicMock()
        search._index.index_path = tmp_path / "index"

        try:
            with search._lock:
                raise ValueError("Test exception")
        except ValueError:
            pass

        # Lock should be released - this should not block
        acquired = search._lock.acquire(blocking=False)
        assert acquired, "Lock should be available after exception"
        search._lock.release()


class TestThreadPoolIntegration:
    """Test thread safety with ThreadPoolExecutor (like the real code uses)."""

    def test_executor_based_concurrent_access(self, tmp_path: Path):
        """Test concurrent access pattern similar to get_sessions_streaming."""
        search = SessionSearch()
        search._index = MagicMock()
        search._index.index_path = tmp_path / "index"

        errors: list[Exception] = []

        def adapter_work(adapter_id: int) -> list[Session]:
            """Simulate adapter returning sessions."""
            sessions = []
            for i in range(10):
                sessions.append(make_session(f"adapter-{adapter_id}-session-{i}"))
            return sessions

        def update_sessions(sessions: list[Session]):
            """Update shared state with sessions."""
            with search._lock:
                for session in sessions:
                    search._sessions_by_id[session.id] = session

        # Simulate multiple adapters running in parallel
        with ThreadPoolExecutor(max_workers=4) as executor:
            futures = {executor.submit(adapter_work, i): i for i in range(4)}
            for future in as_completed(futures):
                try:
                    sessions = future.result()
                    update_sessions(sessions)
                except Exception as e:
                    errors.append(e)

        assert not errors, f"Errors during executor work: {errors}"
        assert len(search._sessions_by_id) == 40, "Should have 4 adapters * 10 sessions"
