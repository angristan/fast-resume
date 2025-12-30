"""Main TUI application for fast-resume."""

import logging
import os
import shlex
import time
from collections.abc import Callable

from textual import on, work
from textual.app import App, ComposeResult
from textual.css.query import NoMatches
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.reactive import reactive
from textual.timer import Timer
from textual.widgets import Footer, Input, Label

from .. import __version__
from ..adapters.base import ParseError, Session
from ..config import LOG_FILE
from ..search import SessionSearch
from .filter_bar import FILTER_KEYS, FilterBar
from .modal import YoloModeModal
from .preview import SessionPreview
from .query import extract_agent_from_query, update_agent_in_query
from .results_table import ResultsTable
from .search_input import KeywordHighlighter, KeywordSuggester
from .styles import APP_CSS
from .utils import copy_to_clipboard

logger = logging.getLogger(__name__)


class FastResumeApp(App):
    """Main TUI application for fast-resume."""

    ENABLE_COMMAND_PALETTE = True
    TITLE = "fast-resume"
    SUB_TITLE = "Session manager"

    CSS = APP_CSS

    BINDINGS = [
        Binding("escape", "quit", "Quit", priority=True),
        Binding("q", "quit", "Quit", show=False),
        Binding("ctrl+c", "quit", "Quit", show=False),
        Binding("/", "focus_search", "Search", priority=True),
        Binding("enter", "resume_session", "Resume"),
        Binding("c", "copy_path", "Copy resume command", priority=True),
        Binding("ctrl+grave_accent", "toggle_preview", "Preview", priority=True),
        Binding("tab", "accept_suggestion", "Accept", show=False, priority=True),
        Binding("j", "cursor_down", "Down", show=False),
        Binding("k", "cursor_up", "Up", show=False),
        Binding("down", "cursor_down", "Down", show=False),
        Binding("up", "cursor_up", "Up", show=False),
        Binding("pagedown", "page_down", "Page Down", show=False),
        Binding("pageup", "page_up", "Page Up", show=False),
        Binding("plus", "increase_preview", "+Preview", show=False),
        Binding("equals", "increase_preview", "+Preview", show=False),
        Binding("minus", "decrease_preview", "-Preview", show=False),
        Binding("ctrl+p", "command_palette", "Commands"),
    ]

    show_preview: reactive[bool] = reactive(True)
    selected_session: reactive[Session | None] = reactive(None)
    active_filter: reactive[str | None] = reactive(None)
    is_loading: reactive[bool] = reactive(True)
    preview_height: reactive[int] = reactive(12)
    search_query: reactive[str] = reactive("", init=False)
    query_time_ms: reactive[float | None] = reactive(None)
    _spinner_frame: int = 0
    _spinner_chars: str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"

    def __init__(
        self,
        initial_query: str = "",
        agent_filter: str | None = None,
        yolo: bool = False,
        no_version_check: bool = False,
    ):
        super().__init__()
        self.search_engine = SessionSearch()
        self.initial_query = initial_query
        self.agent_filter = agent_filter
        self.yolo = yolo
        self.no_version_check = no_version_check
        self.sessions: list[Session] = []
        self._resume_command: list[str] | None = None
        self._resume_directory: str | None = None
        self._current_query: str = ""
        self._total_loaded: int = 0
        self._search_timer: Timer | None = None
        self._available_update: str | None = None
        self._syncing_filter: bool = False  # Prevent infinite loops during sync

    def compose(self) -> ComposeResult:
        """Create child widgets."""
        with Vertical():
            # Title bar: app name + version + session count
            with Horizontal(id="title-bar"):
                yield Label(f"fast-resume v{__version__}", id="app-title")
                yield Label("", id="session-count")

            # Search row with boxed input
            with Horizontal(id="search-row"):
                with Horizontal(id="search-box"):
                    yield Label("🔍", id="search-icon")
                    yield Input(
                        placeholder="Search titles & messages. Try agent:claude or date:today",
                        id="search-input",
                        value=self.initial_query,
                        highlighter=KeywordHighlighter(),
                        suggester=KeywordSuggester(),
                    )
                    yield Label("", id="query-time")

            # Agent filter buttons
            yield FilterBar(initial_filter=self.agent_filter, id="filter-container")

            # Main content area
            with Vertical(id="main-container"):
                with Vertical(id="results-container"):
                    yield ResultsTable(id="results-table")
                with Vertical(id="preview-container"):
                    yield SessionPreview()
        yield Footer()

    def on_mount(self) -> None:
        """Set up the app when mounted."""
        # Set initial filter state from agent_filter parameter
        self.active_filter = self.agent_filter

        # Focus search input
        self.query_one("#search-input", Input).focus()

        # Start spinner animation
        self._spinner_timer = self.set_interval(0.08, self._update_spinner)

        # Try fast sync load first (index hit), fall back to async
        self._initial_load()

        # Check for updates asynchronously (unless disabled)
        if not self.no_version_check:
            self._check_for_updates()

    # -------------------------------------------------------------------------
    # Loading logic
    # -------------------------------------------------------------------------

    def _initial_load(self) -> None:
        """Load sessions - sync if index is current, async with streaming otherwise."""
        # Try to get sessions directly from index (fast path)
        sessions = self.search_engine._load_from_index()
        table = self.query_one(ResultsTable)
        if sessions is not None:
            # Index is current - load synchronously, no flicker
            self.search_engine._sessions = sessions
            self._total_loaded = len(sessions)
            start_time = time.perf_counter()
            self.sessions = self.search_engine.search(
                self.initial_query, agent_filter=self.active_filter, limit=100
            )
            self.query_time_ms = (time.perf_counter() - start_time) * 1000
            self._finish_loading()
            self.selected_session = table.update_sessions(
                self.sessions, self._current_query
            )
        else:
            # Index needs update - show loading and fetch with streaming
            table.update_sessions([], self._current_query)
            self._update_session_count()
            self._do_streaming_load()

    def _update_spinner(self) -> None:
        """Advance spinner animation in search icon."""
        search_icon = self.query_one("#search-icon", Label)
        if self.is_loading:
            self._spinner_frame = (self._spinner_frame + 1) % len(self._spinner_chars)
            search_icon.update(self._spinner_chars[self._spinner_frame])
        else:
            search_icon.update("🔍")

    def _update_session_count(self) -> None:
        """Update the session count display."""
        count_label = self.query_one("#session-count", Label)
        time_label = self.query_one("#query-time", Label)
        if self.is_loading:
            count_label.update(f"{self._total_loaded} sessions loaded")
            time_label.update("")
        else:
            shown = len(self.sessions)
            # Get total for current filter (or all if no filter)
            total = self.search_engine.get_session_count(self.active_filter)
            if shown < total:
                count_label.update(f"{shown}/{total} sessions")
            else:
                count_label.update(f"{total} sessions")
            # Update query time in search box
            if self.query_time_ms is not None:
                time_label.update(f"{self.query_time_ms:.1f}ms")
            else:
                time_label.update("")

    @work(exclusive=True, thread=True)
    def _do_streaming_load(self) -> None:
        """Load sessions with progressive updates as each adapter completes."""
        # Collect parse errors (thread-safe list)
        parse_errors: list[ParseError] = []

        def on_progress():
            # Use Tantivy search with initial_query
            query = self.initial_query
            start_time = time.perf_counter()
            sessions = self.search_engine.search(
                query, agent_filter=self.active_filter, limit=100
            )
            elapsed_ms = (time.perf_counter() - start_time) * 1000
            total = self.search_engine.get_session_count()
            self.call_from_thread(
                self._update_results_streaming, sessions, total, elapsed_ms
            )

        def on_error(error: ParseError):
            parse_errors.append(error)

        _, new, updated, deleted = self.search_engine.get_sessions_streaming(
            on_progress, on_error=on_error
        )
        # Mark loading complete and show toast if there were changes
        self.call_from_thread(
            self._finish_loading, new, updated, deleted, len(parse_errors)
        )

    def _update_results_streaming(
        self, sessions: list, total: int, elapsed_ms: float | None = None
    ) -> None:
        """Update UI with streaming results (keeps loading state)."""
        self.sessions = sessions
        self._total_loaded = total
        if elapsed_ms is not None:
            self.query_time_ms = elapsed_ms
        try:
            table = self.query_one(ResultsTable)
        except NoMatches:
            return  # Widget not mounted yet
        self.selected_session = table.update_sessions(sessions, self._current_query)
        self._update_session_count()

    def _finish_loading(
        self, new: int = 0, updated: int = 0, deleted: int = 0, errors: int = 0
    ) -> None:
        """Mark loading as complete and show toast if there were changes."""
        self.is_loading = False
        if hasattr(self, "_spinner_timer"):
            self._spinner_timer.stop()
        self._update_spinner()
        self._update_session_count()

        # Show toast if there were changes
        if new or updated or deleted:
            parts = []
            # Put "session(s)" on the first item only
            if new:
                parts.append(f"{new} new session{'s' if new != 1 else ''}")
            if updated:
                if not parts:  # First item
                    parts.append(
                        f"{updated} session{'s' if updated != 1 else ''} updated"
                    )
                else:
                    parts.append(f"{updated} updated")
            if deleted:
                if not parts:  # First item
                    parts.append(
                        f"{deleted} session{'s' if deleted != 1 else ''} deleted"
                    )
                else:
                    parts.append(f"{deleted} deleted")
            self.notify(", ".join(parts), title="Index updated")

        # Show warning toast for parse errors
        if errors:
            home = os.path.expanduser("~")
            log_path = str(LOG_FILE)
            if log_path.startswith(home):
                log_path = "~" + log_path[len(home) :]
            self.notify(
                f"{errors} session{'s' if errors != 1 else ''} failed to parse. "
                f"See {log_path}",
                severity="warning",
                timeout=5,
            )

    @work(thread=True)
    def _check_for_updates(self) -> None:
        """Check PyPI for newer version and notify if available."""
        import json
        import urllib.request

        from .. import __version__

        try:
            url = "https://pypi.org/pypi/fast-resume/json"
            with urllib.request.urlopen(url, timeout=3) as response:
                data = json.load(response)
                latest = data["info"]["version"]

            if latest != __version__:
                self._available_update = latest
                self.call_from_thread(
                    self.notify,
                    f"{__version__} → {latest}\nRun [bold]uv tool upgrade fast-resume[/bold] to update",
                    title="Update available",
                    timeout=5,
                )
        except Exception:
            pass  # Silently ignore update check failures

    # -------------------------------------------------------------------------
    # Search logic
    # -------------------------------------------------------------------------

    @work(exclusive=True, thread=True)
    def _do_search(self, query: str) -> None:
        """Perform search and update results in background thread."""
        self._current_query = query
        start_time = time.perf_counter()
        sessions = self.search_engine.search(
            query, agent_filter=self.active_filter, limit=100
        )
        elapsed_ms = (time.perf_counter() - start_time) * 1000
        # Update UI from worker thread via call_from_thread
        self.call_from_thread(self._update_results, sessions, elapsed_ms)

    def _update_results(
        self, sessions: list[Session], elapsed_ms: float | None = None
    ) -> None:
        """Update the UI with search results (called from main thread)."""
        self.sessions = sessions
        if elapsed_ms is not None:
            self.query_time_ms = elapsed_ms
        # Only stop loading spinner if streaming indexing is also done
        if not self.search_engine._streaming_in_progress:
            self.is_loading = False
        try:
            table = self.query_one(ResultsTable)
        except NoMatches:
            return  # Widget not mounted yet
        self.selected_session = table.update_sessions(sessions, self._current_query)
        self._update_session_count()

    def _update_selected_session(self) -> None:
        """Update the selected session based on cursor position."""
        try:
            table = self.query_one(ResultsTable)
        except NoMatches:
            return  # Widget not mounted yet
        session = table.get_selected_session()
        if session:
            self.selected_session = session
            preview = self.query_one(SessionPreview)
            preview.update_preview(session, self._current_query)

    @on(ResultsTable.Selected)
    def on_results_table_selected(self, event: ResultsTable.Selected) -> None:
        """Handle session selection in results table."""
        if event.session:
            self.selected_session = event.session
            preview = self.query_one(SessionPreview)
            preview.update_preview(event.session, self._current_query)

    @on(Input.Changed, "#search-input")
    def on_search_changed(self, event: Input.Changed) -> None:
        """Handle search input changes with debouncing."""
        # Cancel previous timer if still pending
        if self._search_timer:
            self._search_timer.stop()
        self.is_loading = True

        # Sync filter buttons with agent keyword in query (if not already syncing)
        if not self._syncing_filter:
            agent_in_query = extract_agent_from_query(event.value)
            # Only sync if the extracted agent is different from current filter
            if agent_in_query != self.active_filter:
                # Check if this is a valid agent
                if agent_in_query is None or agent_in_query in FILTER_KEYS:
                    self._syncing_filter = True
                    self.active_filter = agent_in_query
                    self.query_one(FilterBar).set_active(agent_in_query)
                    self._syncing_filter = False

        # Debounce: wait 50ms before triggering search
        value = event.value
        self._search_timer = self.set_timer(
            0.05, lambda: setattr(self, "search_query", value)
        )

    def watch_search_query(self, query: str) -> None:
        """React to search query changes."""
        self._do_search(query)

    @on(Input.Submitted, "#search-input")
    def on_search_submitted(self, event: Input.Submitted) -> None:
        """Handle search submission - resume selected session."""
        self.action_resume_session()

    # -------------------------------------------------------------------------
    # Resume logic
    # -------------------------------------------------------------------------

    def _resolve_yolo_mode(
        self,
        action: Callable[[bool], None],
        modal_callback: Callable[[bool | None], None],
    ) -> None:
        """Resolve yolo mode and call the action with the result.

        Determines whether to use yolo mode based on CLI flag, session state,
        or user selection via modal. Then calls `action(yolo_value)`.
        """
        assert self.selected_session is not None
        adapter = self.search_engine.get_adapter_for_session(self.selected_session)

        # If CLI --yolo flag is set, always use yolo
        if self.yolo:
            action(True)
            return

        # If session has stored yolo mode, use it directly
        if self.selected_session.yolo:
            action(True)
            return

        # If adapter supports yolo but session doesn't have stored value, show modal
        if adapter and adapter.supports_yolo:
            self.push_screen(YoloModeModal(), modal_callback)
            return

        # Otherwise proceed without yolo
        action(False)

    def action_copy_path(self) -> None:
        """Copy the full resume command (cd + agent resume) to clipboard."""
        if not self.selected_session:
            return
        self._resolve_yolo_mode(self._do_copy_command, self._on_copy_yolo_modal_result)

    def _do_copy_command(self, yolo: bool) -> None:
        """Execute the copy command with specified yolo mode."""
        assert self.selected_session is not None
        resume_cmd = self.search_engine.get_resume_command(
            self.selected_session, yolo=yolo
        )
        if not resume_cmd:
            self.notify("No resume command available", severity="warning", timeout=2)
            return

        directory = self.selected_session.directory
        cmd_str = shlex.join(resume_cmd)
        full_cmd = f"cd {shlex.quote(directory)} && {cmd_str}"

        if copy_to_clipboard(full_cmd):
            self.notify(f"Copied: {full_cmd}", timeout=3)
        else:
            self.notify(full_cmd, title="Clipboard unavailable", timeout=5)

    def _on_copy_yolo_modal_result(self, result: bool | None) -> None:
        """Handle result from yolo mode modal for copy action."""
        if result is not None:
            self._do_copy_command(yolo=result)

    def action_resume_session(self) -> None:
        """Resume the selected session."""
        if not self.selected_session:
            return

        # Crush doesn't support CLI resume - show a toast instead
        if self.selected_session.agent == "crush":
            self.notify(
                f"Crush doesn't support CLI resume. Open crush in: [bold]{self.selected_session.directory}[/bold] and use ctrl+s to find your session",
                title="Cannot resume",
                severity="warning",
                timeout=5,
            )
            return

        self._resolve_yolo_mode(self._do_resume, self._on_yolo_modal_result)

    def _do_resume(self, yolo: bool) -> None:
        """Execute the resume with specified yolo mode."""
        assert self.selected_session is not None
        self._resume_command = self.search_engine.get_resume_command(
            self.selected_session, yolo=yolo
        )
        self._resume_directory = self.selected_session.directory
        self.exit()

    def _on_yolo_modal_result(self, result: bool | None) -> None:
        """Handle result from yolo mode modal."""
        if result is not None:
            self._do_resume(yolo=result)

    # -------------------------------------------------------------------------
    # UI actions
    # -------------------------------------------------------------------------

    def action_focus_search(self) -> None:
        """Focus the search input."""
        self.query_one("#search-input", Input).focus()

    def action_toggle_preview(self) -> None:
        """Toggle the preview pane."""
        self.show_preview = not self.show_preview
        preview_container = self.query_one("#preview-container")
        if self.show_preview:
            preview_container.remove_class("hidden")
        else:
            preview_container.add_class("hidden")

    def action_cursor_down(self) -> None:
        """Move cursor down in results."""
        table = self.query_one(ResultsTable)
        table.action_cursor_down()

    def action_cursor_up(self) -> None:
        """Move cursor up in results."""
        table = self.query_one(ResultsTable)
        table.action_cursor_up()

    def action_page_down(self) -> None:
        """Move cursor down by a page."""
        table = self.query_one(ResultsTable)
        # Move down by ~10 rows (approximate page)
        for _ in range(10):
            table.action_cursor_down()

    def action_page_up(self) -> None:
        """Move cursor up by a page."""
        table = self.query_one(ResultsTable)
        # Move up by ~10 rows (approximate page)
        for _ in range(10):
            table.action_cursor_up()

    def action_increase_preview(self) -> None:
        """Increase preview pane height."""
        if self.preview_height < 30:
            self.preview_height += 3
            self._apply_preview_height()

    def action_decrease_preview(self) -> None:
        """Decrease preview pane height."""
        if self.preview_height > 6:
            self.preview_height -= 3
            self._apply_preview_height()

    def _apply_preview_height(self) -> None:
        """Apply the current preview height to the container."""
        preview_container = self.query_one("#preview-container")
        preview_container.styles.height = self.preview_height

    def _set_filter(self, agent: str | None) -> None:
        """Set the agent filter and refresh results, syncing query string."""
        self.active_filter = agent
        self.query_one(FilterBar).set_active(agent)

        # Update search input to reflect the new filter (if not already syncing)
        if not self._syncing_filter:
            self._syncing_filter = True
            search_input = self.query_one("#search-input", Input)
            new_query = update_agent_in_query(search_input.value, agent)
            if new_query != search_input.value:
                search_input.value = new_query
                self._current_query = new_query
            self._syncing_filter = False

        self._do_search(self._current_query)

    def action_accept_suggestion(self) -> None:
        """Accept autocomplete suggestion in search input."""
        search_input = self.query_one("#search-input", Input)
        if search_input._suggestion:
            search_input.action_cursor_right()

    def action_cycle_filter(self) -> None:
        """Cycle to the next agent filter."""
        try:
            current_index = FILTER_KEYS.index(self.active_filter)
            next_index = (current_index + 1) % len(FILTER_KEYS)
        except ValueError:
            next_index = 0
        self._set_filter(FILTER_KEYS[next_index])

    async def action_quit(self) -> None:
        """Quit the app, or dismiss modal if one is open."""
        if len(self.screen_stack) > 1:
            top_screen = self.screen_stack[-1]
            if isinstance(top_screen, YoloModeModal):
                top_screen.dismiss(None)
            return
        self.exit()

    @on(FilterBar.Changed)
    def on_filter_bar_changed(self, event: FilterBar.Changed) -> None:
        """Handle filter bar selection change."""
        self._set_filter(event.filter_key)

    def get_resume_command(self) -> list[str] | None:
        """Get the resume command to execute after exit."""
        return self._resume_command

    def get_resume_directory(self) -> str | None:
        """Get the directory to change to before running the resume command."""
        return self._resume_directory

    @property
    def _displayed_sessions(self) -> list[Session]:
        """Get currently displayed sessions (for backward compatibility)."""
        try:
            return self.query_one(ResultsTable).displayed_sessions
        except Exception:
            return []
