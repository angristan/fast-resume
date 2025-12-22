"""Textual TUI for fast-resume."""

import math
import os
import time
from datetime import datetime
from pathlib import Path

import humanize
from rich.columns import Columns
from rich.console import RenderableType
from rich.markup import escape as escape_markup
from rich.text import Text
from textual import on, work
from textual.events import Click
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.reactive import reactive
from textual.widgets import DataTable, Footer, Input, Static, Label
from textual_image.renderable import Image as ImageRenderable
from textual_image.widget import Image as ImageWidget

from .adapters.base import ParseError, Session
from .config import AGENTS, LOG_FILE
from .search import SessionSearch

# Asset paths for agent icons
ASSETS_DIR = Path(__file__).parent / "assets"

# Cache for agent icon renderables
_icon_cache: dict[str, ImageRenderable | None] = {}


def get_agent_icon(agent: str) -> RenderableType:
    """Get the icon + name renderable for an agent."""
    agent_config = AGENTS.get(agent, {"color": "white", "badge": agent})

    if agent not in _icon_cache:
        icon_path = ASSETS_DIR / f"{agent}.png"
        if icon_path.exists():
            try:
                _icon_cache[agent] = ImageRenderable(icon_path, width=2, height=1)
            except Exception:
                _icon_cache[agent] = None
        else:
            _icon_cache[agent] = None

    icon = _icon_cache[agent]
    name = Text(agent_config["badge"])
    name.stylize(agent_config["color"])

    if icon is not None:
        # Combine icon and name horizontally
        return Columns([icon, name], padding=(0, 1), expand=False)

    # Fallback to just colored name with a dot
    badge = Text(f"● {agent_config['badge']}")
    badge.stylize(agent_config["color"], 0, 1)  # Color just the dot
    return badge


def format_time_ago(dt: datetime) -> str:
    """Format a datetime as a human-readable time ago string."""
    return humanize.naturaltime(dt)


def format_directory(path: str) -> str:
    """Format directory path, replacing home with ~."""
    if not path:
        return "n/a"
    home = os.path.expanduser("~")
    if path.startswith(home):
        return "~" + path[len(home) :]
    return path


def highlight_matches(
    text: str, query: str, max_len: int | None = None, style: str = "bold reverse"
) -> Text:
    """Highlight matching portions of text based on query terms.

    Returns a Rich Text object with matches highlighted.
    """
    if max_len and len(text) > max_len:
        text = text[: max_len - 3] + "..."

    if not query:
        return Text(text)

    result = Text(text)
    query_lower = query.lower()
    text_lower = text.lower()

    # Split query into terms and highlight each
    terms = query_lower.split()
    for term in terms:
        if not term:
            continue
        start = 0
        while True:
            idx = text_lower.find(term, start)
            if idx == -1:
                break
            result.stylize(style, idx, idx + len(term))
            start = idx + 1

    return result


class SessionPreview(Static):
    """Preview pane showing session content."""

    # Highlight style for matches in preview
    MATCH_STYLE = "bold reverse"
    # Max lines to show for a single assistant message
    MAX_ASSISTANT_LINES = 4

    def __init__(self) -> None:
        super().__init__("", id="preview")

    def update_preview(self, session: Session | None, query: str = "") -> None:
        """Update the preview with session content, highlighting matches."""
        if session is None:
            self.update("")
            return

        content = session.content
        preview_text = ""

        # If there's a query, try to show the part containing the match
        if query:
            query_lower = query.lower()
            content_lower = content.lower()
            terms = query_lower.split()

            # Find the first matching term
            best_pos = -1
            for term in terms:
                if term:
                    pos = content_lower.find(term)
                    if pos != -1 and (best_pos == -1 or pos < best_pos):
                        best_pos = pos

            if best_pos != -1:
                # Show context around the match (start 100 chars before, up to 1500 chars)
                start = max(0, best_pos - 100)
                end = min(len(content), start + 1500)
                preview_text = content[start:end]
                if start > 0:
                    preview_text = "..." + preview_text
                if end < len(content):
                    preview_text = preview_text + "..."

        # Fall back to regular preview if no match found
        if not preview_text:
            preview_text = session.preview

        # Build rich text with role-based styling
        result = Text()

        # Split by double newlines to get individual messages
        messages = preview_text.split("\n\n")

        for i, msg in enumerate(messages):
            msg = escape_markup(msg.strip())
            if not msg:
                continue

            # Add separator between messages (not before first)
            if i > 0:
                result.append("\n")

            # Detect if this is a user message
            is_user = msg.startswith("» ")

            # Detect code blocks (``` markers)
            in_code = False
            lines = msg.split("\n")

            # Truncate assistant messages
            if not is_user and len(lines) > self.MAX_ASSISTANT_LINES:
                lines = lines[: self.MAX_ASSISTANT_LINES]
                lines.append("...")

            for line in lines:
                if line.startswith("```"):
                    in_code = not in_code
                    result.append(line + "\n", style="dim italic")
                elif line.startswith("» "):
                    # User prompt
                    result.append("» ", style="bold cyan")
                    content_part = line[2:]
                    if len(content_part) > 200:
                        content_part = content_part[:200].rsplit(" ", 1)[0] + " ..."
                    highlighted = highlight_matches(
                        content_part, query, style=self.MATCH_STYLE
                    )
                    result.append_text(highlighted)
                    result.append("\n")
                elif line == "...":
                    result.append("  ...\n", style="dim")
                elif in_code:
                    # Inside code block
                    result.append("  " + line + "\n", style="dim")
                elif line.startswith("  "):
                    # Assistant response
                    highlighted = highlight_matches(line, query, style=self.MATCH_STYLE)
                    result.append_text(highlighted)
                    result.append("\n")
                else:
                    # Other content (possibly from truncated context)
                    if line.startswith("..."):
                        result.append(line + "\n", style="dim")
                    else:
                        highlighted = highlight_matches(
                            line, query, style=self.MATCH_STYLE
                        )
                        result.append_text(highlighted)
                        result.append("\n")

        self.update(result)


class FastResumeApp(App):
    """Main TUI application for fast-resume."""

    ENABLE_COMMAND_PALETTE = True
    TITLE = "fast-resume"
    SUB_TITLE = "Session manager"

    CSS = """
    Screen {
        layout: vertical;
        width: 100%;
        background: $surface;
    }

    /* Title bar - branding + session count */
    #title-bar {
        height: 1;
        width: 100%;
        padding: 0 1;
        background: $surface;
    }

    #app-title {
        width: 1fr;
        color: $text;
        text-style: bold;
    }

    #session-count {
        dock: right;
        color: $text-muted;
        width: auto;
    }

    /* Search row */
    #search-row {
        height: 3;
        width: 100%;
        padding: 0 1;
    }

    #search-box {
        width: 100%;
        height: 3;
        border: solid $primary-background-lighten-2;
        background: $surface;
        padding: 0 1;
    }

    #search-box:focus-within {
        border: solid $accent;
    }

    #search-icon {
        width: 3;
        color: $text-muted;
        content-align: center middle;
    }

    #search-input {
        width: 1fr;
        border: none;
        background: transparent;
    }

    #search-input:focus {
        border: none;
    }

    /* Agent filter tabs - pill style */
    #filter-container {
        height: 1;
        width: 100%;
        padding: 0 1;
        margin-bottom: 1;
    }

    .filter-btn {
        width: auto;
        height: 1;
        margin: 0 1 0 0;
        padding: 0 1;
        border: none;
        background: transparent;
        color: $text-muted;
    }

    .filter-btn:hover {
        color: $text;
    }

    .filter-btn:focus {
        text-style: none;
    }

    .filter-btn.-active {
        background: $accent 20%;
        color: $accent;
    }

    .filter-icon {
        width: 2;
        height: 1;
        margin-right: 1;
    }

    .filter-label {
        height: 1;
    }

    .filter-btn.-active .filter-label {
        text-style: bold;
    }

    /* Agent-specific filter colors */
    #filter-claude {
        color: #E87B35;
    }
    #filter-claude.-active {
        background: #E87B35 20%;
        color: #E87B35;
    }

    #filter-codex {
        color: #00A67E;
    }
    #filter-codex.-active {
        background: #00A67E 20%;
        color: #00A67E;
    }

    #filter-copilot-cli {
        color: #9CA3AF;
    }
    #filter-copilot-cli.-active {
        background: #9CA3AF 20%;
        color: #9CA3AF;
    }

    #filter-copilot-vscode {
        color: #007ACC;
    }
    #filter-copilot-vscode.-active {
        background: #007ACC 20%;
        color: #007ACC;
    }

    #filter-crush {
        color: #FF5F87;
    }
    #filter-crush.-active {
        background: #FF5F87 20%;
        color: #FF5F87;
    }

    #filter-opencode {
        color: #6366F1;
    }
    #filter-opencode.-active {
        background: #6366F1 20%;
        color: #6366F1;
    }

    #filter-vibe {
        color: #FF6B35;
    }
    #filter-vibe.-active {
        background: #FF6B35 20%;
        color: #FF6B35;
    }

    /* Main content area */
    #main-container {
        height: 1fr;
        width: 100%;
    }

    #results-container {
        height: 1fr;
        width: 100%;
        overflow-x: hidden;
    }

    #results-table {
        height: 100%;
        width: 100%;
        overflow-x: hidden;
    }

    DataTable {
        background: transparent;
        overflow-x: hidden;
    }

    DataTable > .datatable--header {
        text-style: bold;
        color: $text;
    }

    DataTable > .datatable--cursor {
        background: $accent 30%;
    }

    DataTable > .datatable--hover {
        background: $surface-lighten-1;
    }

    /* Preview pane - expanded */
    #preview-container {
        height: 12;
        border-top: solid $accent 50%;
        background: $surface;
        padding: 0 1;
    }

    #preview-container.hidden {
        display: none;
    }

    #preview {
        height: 100%;
        overflow-y: auto;
    }

    /* Agent colors */
    .agent-claude {
        color: #E87B35;
    }

    .agent-codex {
        color: #00A67E;
    }

    .agent-copilot {
        color: #9CA3AF;
    }

    .agent-opencode {
        color: #6366F1;
    }

    .agent-vibe {
        color: #FF6B35;
    }

    .agent-crush {
        color: #FF5F87;
    }

    /* Footer styling */
    Footer {
        background: $primary-background;
    }

    Footer > .footer--key {
        background: $surface;
        color: $text;
    }

    Footer > .footer--description {
        color: $text-muted;
    }

    #query-time {
        width: auto;
        padding: 0 1;
        color: $text-muted;
    }
    """

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
        Binding("1", "filter_all", "All", show=False),
        Binding("2", "filter_claude", "Claude", show=False),
        Binding("3", "filter_codex", "Codex", show=False),
        Binding("4", "filter_copilot_cli", "Copilot", show=False),
        Binding("5", "filter_copilot_vscode", "VS Code", show=False),
        Binding("6", "filter_crush", "Crush", show=False),
        Binding("7", "filter_opencode", "OpenCode", show=False),
        Binding("8", "filter_vibe", "Vibe", show=False),
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

    def compose(self) -> ComposeResult:
        """Create child widgets."""
        with Vertical():
            # Title bar: app name + session count
            with Horizontal(id="title-bar"):
                yield Label("fast-resume", id="app-title")
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

    def _update_column_widths(self) -> None:
        """Update column widths based on terminal size."""
        table = self.query_one("#results-table", DataTable)
        width = self.size.width
        padding = 8  # column gaps + scrollbar

        # Responsive column widths based on terminal width
        # Agent column: icon (2) + space (1) + name (up to 11 for "claude-code") = 14
        if width >= 120:
            # Wide: show everything
            agent_w = 15
            dir_w = 30
            msgs_w = 6
            date_w = 18
        elif width >= 90:
            # Medium: slightly smaller
            agent_w = 15
            dir_w = 22
            msgs_w = 5
            date_w = 15
        elif width >= 60:
            # Narrow: compact
            agent_w = 15
            dir_w = 16
            msgs_w = 5
            date_w = 12
        else:
            # Very narrow: minimal
            agent_w = 13
            dir_w = 0  # hide directory
            msgs_w = 4
            date_w = 10

        title_w = max(15, width - agent_w - dir_w - msgs_w - date_w - padding)

        # Disable auto_width and set explicit widths
        for col in table.columns.values():
            col.auto_width = False

        table.columns[self._col_agent].width = agent_w
        table.columns[self._col_title].width = title_w
        table.columns[self._col_dir].width = dir_w
        table.columns[self._col_msgs].width = msgs_w
        table.columns[self._col_date].width = date_w

        # Store for truncation
        self._title_width = title_w
        self._dir_width = dir_w

        # Force refresh
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
            now = datetime.now()
            age = now - session.timestamp
            age_hours = age.total_seconds() / 3600

            # Continuous gradient using exponential decay
            # Green (0h) → Yellow (12h) → Orange (3d) → Dim (30d+)
            decay_rate = 0.005  # Controls how fast colors fade
            t = 1 - math.exp(
                -decay_rate * age_hours
            )  # 0 at 0h, approaches 1 asymptotically

            # Interpolate through color stops: green → yellow → orange → gray
            if t < 0.3:
                # Muted green to yellow
                s = t / 0.3
                r = int(100 + s * 100)  # 100 → 200
                g = int(200 - s * 20)  # 200 → 180
                b = int(50 - s * 50)  # 50 → 0
            elif t < 0.6:
                # Yellow to muted orange
                s = (t - 0.3) / 0.3
                r = 200
                g = int(180 - s * 80)  # 180 → 100
                b = int(0 + s * 50)  # 0 → 50
            else:
                # Muted orange to dim gray
                s = (t - 0.6) / 0.4
                r = int(200 - s * 100)  # 200 → 100
                g = int(100)  # 100 → 100
                b = int(50 + s * 50)  # 50 → 100

            time_text.stylize(f"#{r:02x}{g:02x}{b:02x}")

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

    def action_copy_path(self) -> None:
        """Copy the full resume command (cd + agent resume) to clipboard."""
        if self.selected_session:
            import shlex
            import subprocess
            import sys

            # Build full resume command: cd <dir> && <resume command>
            # Use yolo mode if CLI flag set OR session was started in yolo mode
            use_yolo = self.yolo or self.selected_session.yolo
            resume_cmd = self.search_engine.get_resume_command(
                self.selected_session, yolo=use_yolo
            )
            if not resume_cmd:
                self.notify(
                    "No resume command available", severity="warning", timeout=2
                )
                return

            directory = self.selected_session.directory
            cmd_str = shlex.join(resume_cmd)
            full_cmd = f"cd {shlex.quote(directory)} && {cmd_str}"

            try:
                if sys.platform == "darwin":
                    subprocess.run(["pbcopy"], input=full_cmd.encode(), check=True)
                elif sys.platform == "win32":
                    subprocess.run(["clip"], input=full_cmd.encode(), check=True)
                else:
                    # Linux - try xclip or xsel
                    try:
                        subprocess.run(
                            ["xclip", "-selection", "clipboard"],
                            input=full_cmd.encode(),
                            check=True,
                        )
                    except FileNotFoundError:
                        subprocess.run(
                            ["xsel", "--clipboard", "--input"],
                            input=full_cmd.encode(),
                            check=True,
                        )
                self.notify(f"Copied: {full_cmd}", timeout=3)
            except Exception:
                # Fallback: show command in notification if clipboard fails
                self.notify(
                    f"{full_cmd}",
                    title="Clipboard unavailable",
                    timeout=5,
                )

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
        if self.selected_session:
            # Crush doesn't support CLI resume - show a toast instead
            if self.selected_session.agent == "crush":
                self.notify(
                    f"Crush doesn't support CLI resume. Open crush in: [bold]{self.selected_session.directory}[/bold] and use ctrl+s to find your session",
                    title="Cannot resume",
                    severity="warning",
                    timeout=5,
                )
                return

            # Use yolo mode if CLI flag set OR session was started in yolo mode
            use_yolo = self.yolo or self.selected_session.yolo
            self._resume_command = self.search_engine.get_resume_command(
                self.selected_session, yolo=use_yolo
            )
            self._resume_directory = self.selected_session.directory
            self.exit()

    def _set_filter(self, agent: str | None) -> None:
        """Set the agent filter and refresh results."""
        self.active_filter = agent
        self._update_filter_buttons()
        self._do_search(self._current_query)

    def action_filter_all(self) -> None:
        """Show all sessions."""
        self._set_filter(None)

    def action_filter_claude(self) -> None:
        """Filter to Claude sessions only."""
        self._set_filter("claude")

    def action_filter_codex(self) -> None:
        """Filter to Codex sessions only."""
        self._set_filter("codex")

    def action_filter_copilot_cli(self) -> None:
        """Filter to Copilot CLI sessions only."""
        self._set_filter("copilot-cli")

    def action_filter_copilot_vscode(self) -> None:
        """Filter to VS Code Copilot sessions only."""
        self._set_filter("copilot-vscode")

    def action_filter_crush(self) -> None:
        """Filter to Crush sessions only."""
        self._set_filter("crush")

    def action_filter_opencode(self) -> None:
        """Filter to OpenCode sessions only."""
        self._set_filter("opencode")

    def action_filter_vibe(self) -> None:
        """Filter to Vibe sessions only."""
        self._set_filter("vibe")

    def action_cycle_filter(self) -> None:
        """Cycle to the next agent filter."""
        try:
            current_index = self.FILTER_KEYS.index(self.active_filter)
            next_index = (current_index + 1) % len(self.FILTER_KEYS)
        except ValueError:
            next_index = 0
        self._set_filter(self.FILTER_KEYS[next_index])

    @on(Click, ".filter-btn")
    def on_filter_click(self, event: Click) -> None:
        """Handle click on filter buttons."""
        # Walk up to find the filter-btn container (click might be on child widget)
        widget = event.widget
        while widget and "filter-btn" not in widget.classes:
            widget = widget.parent
        btn_id = widget.id if widget else None
        if btn_id == "filter-all":
            self._set_filter(None)
        elif btn_id == "filter-claude":
            self._set_filter("claude")
        elif btn_id == "filter-codex":
            self._set_filter("codex")
        elif btn_id == "filter-copilot-cli":
            self._set_filter("copilot-cli")
        elif btn_id == "filter-copilot-vscode":
            self._set_filter("copilot-vscode")
        elif btn_id == "filter-crush":
            self._set_filter("crush")
        elif btn_id == "filter-opencode":
            self._set_filter("opencode")
        elif btn_id == "filter-vibe":
            self._set_filter("vibe")

    def get_resume_command(self) -> list[str] | None:
        """Get the resume command to execute after exit."""
        return self._resume_command

    def get_resume_directory(self) -> str | None:
        """Get the directory to change to before running the resume command."""
        return self._resume_directory


def run_tui(
    query: str = "", agent_filter: str | None = None, yolo: bool = False
) -> tuple[list[str] | None, str | None]:
    """Run the TUI and return the resume command and directory if selected."""
    app = FastResumeApp(initial_query=query, agent_filter=agent_filter, yolo=yolo)
    app.run()
    return app.get_resume_command(), app.get_resume_directory()
