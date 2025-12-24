"""Main TUI application for fast-resume."""

import logging
import os
import shlex
import time
from collections.abc import Callable
from datetime import datetime

from rich.text import Text
from textual import on, work
from textual.events import Click
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.reactive import reactive
from textual.widgets import DataTable, Footer, Input, Static, Label
from textual_image.widget import Image as ImageWidget

from .. import __version__
from ..adapters.base import ParseError, Session
from ..config import AGENTS, LOG_FILE
from ..search import SessionSearch
from .modal import YoloModeModal
from .preview import SessionPreview
from .styles import APP_CSS
from .utils import (
    ASSETS_DIR,
    copy_to_clipboard,
    format_directory,
    format_time_ago,
    get_age_color,
    get_agent_icon,
    highlight_matches,
)

logger = logging.getLogger(__name__)


class FastResumeApp(App):
    """Main TUI application for fast-resume."""

    ENABLE_COMMAND_PALETTE = True
    TITLE = "fast-resume"
    SUB_TITLE = "Session manager"

    CSS = APP_CSS

    FILTER_KEYS: list[str | None] = [
        None,
        "claude",
        "codex",
        "copilot-cli",
        "copilot-vscode",
        "crush",
        "opencode",
        "vibe",
    ]

    # Map button IDs to filter values
    _FILTER_ID_MAP: dict[str, str | None] = {
        "filter-all": None,
        "filter-claude": "claude",
        "filter-codex": "codex",
        "filter-copilot-cli": "copilot-cli",
        "filter-copilot-vscode": "copilot-vscode",
        "filter-crush": "crush",
        "filter-opencode": "opencode",
        "filter-vibe": "vibe",
    }

    BINDINGS = [
        Binding("escape", "quit", "Quit", priority=True),
        Binding("q", "quit", "Quit", show=False),
        Binding("ctrl+c", "quit", "Quit", show=False),
        Binding("/", "focus_search", "Search", priority=True),
        Binding("enter", "resume_session", "Resume"),
        Binding("c", "copy_path", "Copy resume command", priority=True),
        Binding("ctrl+grave_accent", "toggle_preview", "Preview", priority=True),
        Binding("tab", "cycle_filter", "Next Filter", priority=True),
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
    ):
        super().__init__()
        self.search_engine = SessionSearch()
        self.initial_query = initial_query
        self.agent_filter = agent_filter
        self.yolo = yolo
        self.sessions: list[Session] = []
        self._displayed_sessions: list[Session] = []
        self._resume_command: list[str] | None = None
        self._resume_directory: str | None = None
        self._current_query: str = ""
        self._filter_buttons: dict[str | None, Static] = {}
        self._total_loaded: int = 0
        self._search_timer: object | None = None
        self._available_update: str | None = None

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
                        placeholder="Search by title or messages...",
                        id="search-input",
                        value=self.initial_query,
                    )
                    yield Label("", id="query-time")

            # Agent filter buttons - pill style with icons
            with Horizontal(id="filter-container"):
                for filter_key in self.FILTER_KEYS:
                    filter_label = AGENTS[filter_key]["badge"] if filter_key else "All"
                    btn_id = f"filter-{filter_key or 'all'}"
                    with Horizontal(id=btn_id, classes="filter-btn") as btn_container:
                        if filter_key:
                            icon_path = ASSETS_DIR / f"{filter_key}.png"
                            if icon_path.exists():
                                yield ImageWidget(icon_path, classes="filter-icon")
                            yield Label(
                                filter_label, classes=f"filter-label agent-{filter_key}"
                            )
                        else:
                            yield Label(filter_label, classes="filter-label")
                    self._filter_buttons[filter_key] = btn_container

            # Main content area
            with Vertical(id="main-container"):
                with Vertical(id="results-container"):
                    yield DataTable(
                        id="results-table",
                        cursor_type="row",
                        cursor_background_priority="renderable",
                        cursor_foreground_priority="renderable",
                    )
                with Vertical(id="preview-container"):
                    yield SessionPreview()
        yield Footer()

    def on_mount(self) -> None:
        """Set up the app when mounted."""
        table = self.query_one("#results-table", DataTable)
        (
            self._col_agent,
            self._col_title,
            self._col_dir,
            self._col_msgs,
            self._col_date,
        ) = table.add_columns("Agent", "Title", "Directory", "Turns", "Date")

        # Initialize column widths immediately based on current size
        self._update_column_widths()

        # Also update after layout is fully ready (in case size changes)
        self.call_after_refresh(self._update_column_widths)

        # Set initial filter state from agent_filter parameter
        self.active_filter = self.agent_filter
        self._update_filter_buttons()

        # Focus search input
        self.query_one("#search-input", Input).focus()

        # Start spinner animation
        self._spinner_timer = self.set_interval(0.08, self._update_spinner)

        # Try fast sync load first (index hit), fall back to async
        self._initial_load()

        # Check for updates asynchronously
        self._check_for_updates()

    def _initial_load(self) -> None:
        """Load sessions - sync if index is current, async with streaming otherwise."""
        # Try to get sessions directly from index (fast path)
        sessions = self.search_engine._load_from_index()
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
            self._update_table()
        else:
            # Index needs update - show loading and fetch with streaming
            self._update_table()
            self._update_session_count()
            self._do_streaming_load()

    def _update_filter_buttons(self) -> None:
        """Update filter button active states."""
        for filter_key, btn in self._filter_buttons.items():
            if filter_key == self.active_filter:
                btn.add_class("-active")
            else:
                btn.remove_class("-active")

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
            total = self._total_loaded
            if shown < total:
                count_label.update(f"{shown}/{total} sessions")
            else:
                count_label.update(f"{total} sessions")
            # Update query time in search box
            if self.query_time_ms is not None:
                time_label.update(f"{self.query_time_ms:.1f}ms")
            else:
                time_label.update("")

    def on_resize(self) -> None:
        """Handle terminal resize."""
        if hasattr(self, "_col_agent"):
            self._update_column_widths()
            # Re-render rows with new truncation widths
            if self.sessions:
                self._update_table()

    # Column width breakpoints: (min_width, agent, dir, msgs, date)
    _COL_WIDTHS = [
        (120, 12, 30, 6, 18),  # Wide
        (90, 12, 22, 5, 15),  # Medium
        (60, 12, 16, 5, 12),  # Narrow
        (0, 11, 0, 4, 10),  # Very narrow (hide directory)
    ]

    def _update_column_widths(self) -> None:
        """Update column widths based on terminal size."""
        table = self.query_one("#results-table", DataTable)
        width = self.size.width

        # Find appropriate breakpoint
        agent_w, dir_w, msgs_w, date_w = next(
            (a, d, m, t) for min_w, a, d, m, t in self._COL_WIDTHS if width >= min_w
        )
        title_w = max(15, width - agent_w - dir_w - msgs_w - date_w - 8)

        for col in table.columns.values():
            col.auto_width = False
        table.columns[self._col_agent].width = agent_w
        table.columns[self._col_title].width = title_w
        table.columns[self._col_dir].width = dir_w
        table.columns[self._col_msgs].width = msgs_w
        table.columns[self._col_date].width = date_w

        self._title_width, self._dir_width = title_w, dir_w
        table.refresh()

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
        self, sessions: list[Session], total: int, elapsed_ms: float | None = None
    ) -> None:
        """Update UI with streaming results (keeps loading state)."""
        self.sessions = sessions
        self._total_loaded = total
        if elapsed_ms is not None:
            self.query_time_ms = elapsed_ms
        self._update_table()
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
        self._update_table()
        self._update_session_count()

    @work(thread=True)
    def _check_for_updates(self) -> None:
        """Check PyPI for newer version and notify if available."""
        import json
        import urllib.request

        try:
            url = "https://pypi.org/pypi/fast-resume/json"
            with urllib.request.urlopen(url, timeout=3) as response:
                data = json.load(response)
                latest = data["info"]["version"]

            logger.debug(f"Update check: current={__version__}, latest={latest}")

            if latest != __version__:
                logger.info(f"Update available: {__version__} → {latest}")
                self._available_update = latest
                self.call_from_thread(
                    self.notify,
                    f"{__version__} → {latest}\nRun [bold]uv tool upgrade fast-resume[/bold] to update",
                    title="Update available",
                    timeout=5,
                )
        except Exception as e:
            logger.debug(f"Update check failed: {e}")

    def _update_table(self) -> None:
        """Update the results table with current sessions."""
        table = self.query_one("#results-table", DataTable)
        table.clear()

        if not self.sessions:
            # Show empty state message
            table.add_row(
                "",
                Text("No sessions found", style="dim italic"),
                "",
                "",
                "",
            )
            self._displayed_sessions = []
            return

        # Store for selection tracking
        self._displayed_sessions = self.sessions

        for session in self._displayed_sessions:
            # Get agent icon (image or text fallback)
            icon = get_agent_icon(session.agent)

            # Title - truncate and highlight matches
            max_title = getattr(self, "_title_width", 60)
            title = highlight_matches(
                session.title, self._current_query, max_len=max_title
            )

            # Format directory - truncate based on column width
            dir_w = getattr(self, "_dir_width", 22)
            directory = format_directory(session.directory)
            if dir_w > 0 and len(directory) > dir_w:
                directory = "..." + directory[-(dir_w - 3) :]
            dir_text = (
                highlight_matches(directory, self._current_query)
                if dir_w > 0
                else Text("")
            )

            # Format message count
            msgs_text = str(session.message_count) if session.message_count > 0 else "-"

            # Format time with age-based gradient coloring
            time_ago = format_time_ago(session.timestamp)
            time_text = Text(time_ago.rjust(8))
            age_hours = (datetime.now() - session.timestamp).total_seconds() / 3600
            time_text.stylize(get_age_color(age_hours))

            table.add_row(icon, title, dir_text, msgs_text, time_text)

        # Select first row if available
        if self._displayed_sessions:
            table.move_cursor(row=0)
            self._update_selected_session()

    def _update_selected_session(self) -> None:
        """Update the selected session based on cursor position."""
        table = self.query_one("#results-table", DataTable)
        displayed = getattr(self, "_displayed_sessions", self.sessions)
        if table.cursor_row is not None and table.cursor_row < len(displayed):
            self.selected_session = displayed[table.cursor_row]
            preview = self.query_one(SessionPreview)
            preview.update_preview(self.selected_session, self._current_query)

    @on(Input.Changed, "#search-input")
    def on_search_changed(self, event: Input.Changed) -> None:
        """Handle search input changes with debouncing."""
        # Cancel previous timer if still pending
        if self._search_timer:
            self._search_timer.stop()
        self.is_loading = True
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

    @on(DataTable.RowHighlighted)
    def on_row_highlighted(self, event: DataTable.RowHighlighted) -> None:
        """Handle cursor movement in results table."""
        self._update_selected_session()

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
        table = self.query_one("#results-table", DataTable)
        table.action_cursor_down()
        self._update_selected_session()

    def action_cursor_up(self) -> None:
        """Move cursor up in results."""
        table = self.query_one("#results-table", DataTable)
        table.action_cursor_up()
        self._update_selected_session()

    def action_page_down(self) -> None:
        """Move cursor down by a page."""
        table = self.query_one("#results-table", DataTable)
        # Move down by ~10 rows (approximate page)
        for _ in range(10):
            table.action_cursor_down()
        self._update_selected_session()

    def action_page_up(self) -> None:
        """Move cursor up by a page."""
        table = self.query_one("#results-table", DataTable)
        # Move up by ~10 rows (approximate page)
        for _ in range(10):
            table.action_cursor_up()
        self._update_selected_session()

    def _resolve_yolo_mode(
        self,
        action: Callable[[bool], None],
        modal_callback: Callable[[bool | None], None],
    ) -> None:
        """Resolve yolo mode and call the action with the result.

        Determines whether to use yolo mode based on CLI flag, session state,
        or user selection via modal. Then calls `action(yolo_value)`.
        """
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
        self._resume_command = self.search_engine.get_resume_command(
            self.selected_session, yolo=yolo
        )
        self._resume_directory = self.selected_session.directory
        self.exit()

    def _on_yolo_modal_result(self, result: bool | None) -> None:
        """Handle result from yolo mode modal."""
        if result is not None:
            self._do_resume(yolo=result)

    def _set_filter(self, agent: str | None) -> None:
        """Set the agent filter and refresh results."""
        self.active_filter = agent
        self._update_filter_buttons()
        self._do_search(self._current_query)

    def action_cycle_filter(self) -> None:
        """Cycle to the next agent filter."""
        # If a modal is open, delegate tab to the modal
        if len(self.screen_stack) > 1:
            top_screen = self.screen_stack[-1]
            if isinstance(top_screen, YoloModeModal):
                top_screen.action_toggle_focus()
            return
        try:
            current_index = self.FILTER_KEYS.index(self.active_filter)
            next_index = (current_index + 1) % len(self.FILTER_KEYS)
        except ValueError:
            next_index = 0
        self._set_filter(self.FILTER_KEYS[next_index])

    def action_quit(self) -> None:
        """Quit the app, or dismiss modal if one is open."""
        if len(self.screen_stack) > 1:
            top_screen = self.screen_stack[-1]
            if isinstance(top_screen, YoloModeModal):
                top_screen.dismiss(None)
            return
        self.exit()

    @on(Click, ".filter-btn")
    def on_filter_click(self, event: Click) -> None:
        """Handle click on filter buttons."""
        # Walk up to find the filter-btn container (click might be on child widget)
        widget = event.widget
        while widget and "filter-btn" not in widget.classes:
            widget = widget.parent
        if widget and widget.id in self._FILTER_ID_MAP:
            self._set_filter(self._FILTER_ID_MAP[widget.id])

    def get_resume_command(self) -> list[str] | None:
        """Get the resume command to execute after exit."""
        return self._resume_command

    def get_resume_directory(self) -> str | None:
        """Get the directory to change to before running the resume command."""
        return self._resume_directory
