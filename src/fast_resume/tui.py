"""Textual TUI for fast-resume."""

import os
from datetime import datetime

from rich.markup import escape as escape_markup
from rich.text import Text
from textual import on, work
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.reactive import reactive
from textual.widgets import DataTable, Footer, Input, Static, Label, Button

from .adapters.base import Session
from .config import AGENTS
from .search import SessionSearch


def format_time_ago(dt: datetime) -> str:
    """Format a datetime as a human-readable time ago string."""
    now = datetime.now()
    diff = now - dt

    seconds = diff.total_seconds()
    if seconds < 60:
        return "just now"
    elif seconds < 3600:
        mins = int(seconds / 60)
        return f"{mins}m ago"
    elif seconds < 86400:
        hours = int(seconds / 3600)
        return f"{hours}h ago"
    elif seconds < 604800:
        days = int(seconds / 86400)
        return f"{days}d ago"
    else:
        return dt.strftime("%Y-%m-%d")


def format_directory(path: str) -> str:
    """Format directory path, replacing home with ~."""
    home = os.path.expanduser("~")
    if path.startswith(home):
        return "~" + path[len(home) :]
    return path


def highlight_matches(text: str, query: str, max_len: int | None = None) -> Text:
    """Highlight matching portions of text based on query terms.

    Returns a Rich Text object with matches highlighted.
    """
    if max_len and len(text) > max_len:
        text = text[:max_len] + "..."

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
            # Modern highlight style - bold with accent background
            result.stylize("bold reverse", idx, idx + len(term))
            start = idx + 1

    return result


class SessionPreview(Static):
    """Preview pane showing session content."""

    def __init__(self) -> None:
        super().__init__("", id="preview")
        self.border_title = "Preview"

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
                # Show context around the match (start 50 chars before, up to 500 chars)
                start = max(0, best_pos - 50)
                end = min(len(content), start + 500)
                preview_text = content[start:end]
                if start > 0:
                    preview_text = "..." + preview_text
                if end < len(content):
                    preview_text = preview_text + "..."

        # Fall back to regular preview if no match found
        if not preview_text:
            preview_text = session.preview

        preview_text = escape_markup(preview_text)
        highlighted = highlight_matches(preview_text, query)
        self.update(highlighted)


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

    /* Top bar - search + count */
    #top-bar {
        height: 1;
        width: 100%;
        padding: 0 1;
    }

    #search-input {
        width: 1fr;
        border: none;
        background: $surface;
    }

    #search-input:focus {
        border: none;
    }

    #session-count {
        dock: right;
        color: $text-muted;
        width: auto;
    }

    /* Agent filter tabs - compact */
    #filter-container {
        height: 1;
        width: 100%;
        padding: 0 1;
    }

    .filter-btn {
        min-width: 8;
        height: 1;
        margin: 0 1 0 0;
        border: none;
        background: transparent;
        text-style: none;
        color: $text-muted;
    }

    .filter-btn:hover {
        color: $text;
        text-style: bold;
    }

    .filter-btn:focus {
        text-style: none;
    }

    .filter-btn.-active {
        color: $accent;
        text-style: bold;
    }

    /* Main content area */
    #main-container {
        height: 1fr;
        width: 100%;
    }

    #results-container {
        height: 1fr;
        width: 100%;
    }

    #results-table {
        height: 100%;
        width: 100%;
    }

    DataTable {
        background: transparent;
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

    /* Preview pane - compact */
    #preview-container {
        height: 8;
        border-top: solid $surface-lighten-2;
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

    .agent-opencode {
        color: #6366F1;
    }

    .agent-vibe {
        color: #FF6B35;
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
    """

    BINDINGS = [
        Binding("q", "quit", "Quit"),
        Binding("escape", "quit", "Quit"),
        Binding("ctrl+c", "quit", "Quit", show=False),
        Binding("/", "focus_search", "Search"),
        Binding("tab", "toggle_preview", "Preview"),
        Binding("j", "cursor_down", "Down", show=False),
        Binding("k", "cursor_up", "Up", show=False),
        Binding("down", "cursor_down", "Down", show=False),
        Binding("up", "cursor_up", "Up", show=False),
        Binding("enter", "resume_session", "Resume"),
        Binding("1", "filter_all", "All", show=False),
        Binding("2", "filter_claude", "Claude", show=False),
        Binding("3", "filter_codex", "Codex", show=False),
        Binding("4", "filter_opencode", "OpenCode", show=False),
        Binding("5", "filter_vibe", "Vibe", show=False),
        Binding("ctrl+p", "command_palette", "Commands"),
    ]

    show_preview: reactive[bool] = reactive(True)
    selected_session: reactive[Session | None] = reactive(None)
    active_filter: reactive[str | None] = reactive(None)
    is_loading: reactive[bool] = reactive(True)

    def __init__(self, initial_query: str = "", agent_filter: str | None = None):
        super().__init__()
        self.search_engine = SessionSearch()
        self.initial_query = initial_query
        self.agent_filter = agent_filter
        self.sessions: list[Session] = []
        self._resume_command: list[str] | None = None
        self._resume_directory: str | None = None
        self._current_query: str = ""
        self._filter_buttons: dict[str | None, Static] = {}

    def compose(self) -> ComposeResult:
        """Create child widgets."""
        with Vertical():
            # Top bar: search + session count
            with Horizontal(id="top-bar"):
                yield Input(
                    placeholder="Search...",
                    id="search-input",
                    value=self.initial_query,
                )
                yield Label("", id="session-count")

            # Agent filter buttons - compact inline
            with Horizontal(id="filter-container"):
                for filter_key, filter_label in [
                    (None, "All"),
                    ("claude", "Claude"),
                    ("codex", "Codex"),
                    ("opencode", "OpenCode"),
                    ("vibe", "Vibe"),
                ]:
                    btn_id = f"filter-{filter_key or 'all'}"
                    btn = Button(filter_label, id=btn_id, classes="filter-btn")
                    self._filter_buttons[filter_key] = btn
                    yield btn

            # Main content area
            with Vertical(id="main-container"):
                with Vertical(id="results-container"):
                    yield DataTable(id="results-table", cursor_type="row")
                with Vertical(id="preview-container"):
                    yield SessionPreview()
        yield Footer()

    def on_mount(self) -> None:
        """Set up the app when mounted."""
        table = self.query_one("#results-table", DataTable)
        self._col_agent, self._col_title, self._col_dir, self._col_date = table.add_columns(
            "Agent", "Title", "Directory", "Date"
        )

        # Set fixed column widths
        self._agent_width = 12
        self._dir_width = 22
        self._date_width = 10

        # Delay column width calculation until layout is ready
        self.call_after_refresh(self._update_column_widths)

        # Set initial filter state from agent_filter parameter
        self.active_filter = self.agent_filter
        self._update_filter_buttons()

        # Focus search input
        self.query_one("#search-input", Input).focus()

        # Try fast sync load first (cache hit), fall back to async
        self._initial_load()

    def _initial_load(self) -> None:
        """Load sessions - sync if cached, async otherwise."""
        # Try to get cached sessions directly (fast path)
        cached = self.search_engine._load_from_cache()
        if cached is not None:
            # Cache hit - load synchronously, no flicker
            self.search_engine._sessions = cached
            self.sessions = self.search_engine.search(
                self.initial_query, agent_filter=self.active_filter, limit=100
            )
            self.is_loading = False
            self._update_table()
            self._update_session_count()
        else:
            # Cache miss - show loading and fetch async
            self._update_table()
            self._update_session_count()
            self._do_search(self.initial_query)

    def _update_filter_buttons(self) -> None:
        """Update filter button active states."""
        for filter_key, btn in self._filter_buttons.items():
            if filter_key == self.active_filter:
                btn.add_class("-active")
            else:
                btn.remove_class("-active")

    def _update_session_count(self) -> None:
        """Update the session count display."""
        count_label = self.query_one("#session-count", Label)
        if self.is_loading:
            count_label.update("...")
        else:
            count_label.update(f"{len(self.sessions)}")

    def on_resize(self) -> None:
        """Handle terminal resize."""
        if hasattr(self, '_col_agent'):
            self._update_column_widths()

    def _update_column_widths(self) -> None:
        """Update column widths based on terminal size."""
        table = self.query_one("#results-table", DataTable)

        # Use the full screen width for calculation
        width = self.size.width

        # Account for column separators (3 gaps × 2 spaces) and scrollbar gutter
        fixed_cols = self._agent_width + self._dir_width + self._date_width
        padding = 8  # column gaps + potential scrollbar
        title_width = max(20, width - fixed_cols - padding)

        # Disable auto_width and set explicit widths
        for col in table.columns.values():
            col.auto_width = False

        table.columns[self._col_agent].width = self._agent_width
        table.columns[self._col_title].width = title_width
        table.columns[self._col_dir].width = self._dir_width
        table.columns[self._col_date].width = self._date_width

        # Store for truncation
        self._title_width = title_width

        # Force refresh
        table.refresh()

    @work(exclusive=True, thread=True)
    def _do_search(self, query: str) -> None:
        """Perform search and update results in background thread."""
        self._current_query = query
        sessions = self.search_engine.search(
            query, agent_filter=self.active_filter, limit=100
        )
        # Update UI from worker thread via call_from_thread
        self.call_from_thread(self._update_results, sessions)

    def _update_results(self, sessions: list[Session]) -> None:
        """Update the UI with search results (called from main thread)."""
        self.sessions = sessions
        self.is_loading = False
        self._update_table()
        self._update_session_count()

    def _update_table(self) -> None:
        """Update the results table with current sessions."""
        table = self.query_one("#results-table", DataTable)
        table.clear()

        if self.is_loading and not self.sessions:
            # Show loading row
            table.add_row("", Text("Loading sessions...", style="italic dim"), "", "")
            return

        for session in self.sessions:
            # Create colored agent badge
            agent_config = AGENTS.get(session.agent, {"color": "white", "badge": session.agent})
            badge = Text(f"[{agent_config['badge']}]")
            badge.stylize(agent_config["color"])

            # Title - truncate and highlight matches
            max_title = getattr(self, '_title_width', 60) - 3
            title = highlight_matches(session.title, self._current_query, max_len=max_title)

            # Format directory - right aligned with padding, also highlight
            directory = format_directory(session.directory)
            if len(directory) > 20:
                directory = "..." + directory[-17:]
            dir_text = highlight_matches(directory, self._current_query)

            # Format time - right aligned with padding
            time_ago = format_time_ago(session.timestamp)
            time_ago = time_ago.rjust(8)

            table.add_row(badge, title, dir_text, time_ago)

        # Select first row if available
        if self.sessions:
            table.move_cursor(row=0)
            self._update_selected_session()

    def _update_selected_session(self) -> None:
        """Update the selected session based on cursor position."""
        table = self.query_one("#results-table", DataTable)
        if table.cursor_row is not None and table.cursor_row < len(self.sessions):
            self.selected_session = self.sessions[table.cursor_row]
            preview = self.query_one(SessionPreview)
            preview.update_preview(self.selected_session, self._current_query)

    @on(Input.Changed, "#search-input")
    def on_search_changed(self, event: Input.Changed) -> None:
        """Handle search input changes."""
        self.is_loading = True
        self._update_session_count()
        self._do_search(event.value)

    @on(Input.Submitted, "#search-input")
    def on_search_submitted(self, event: Input.Submitted) -> None:
        """Handle search submission - resume selected session."""
        self.action_resume_session()

    @on(DataTable.RowHighlighted)
    def on_row_highlighted(self, event: DataTable.RowHighlighted) -> None:
        """Handle cursor movement in results table."""
        self._update_selected_session()

    @on(DataTable.RowSelected)
    def on_row_selected(self, event: DataTable.RowSelected) -> None:
        """Handle row selection (Enter on table) - resume session."""
        self.action_resume_session()

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

    def action_resume_session(self) -> None:
        """Resume the selected session."""
        if self.selected_session:
            self._resume_command = self.search_engine.get_resume_command(
                self.selected_session
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

    def action_filter_opencode(self) -> None:
        """Filter to OpenCode sessions only."""
        self._set_filter("opencode")

    def action_filter_vibe(self) -> None:
        """Filter to Vibe sessions only."""
        self._set_filter("vibe")

    @on(Button.Pressed, ".filter-btn")
    def on_filter_click(self, event: Button.Pressed) -> None:
        """Handle click on filter buttons."""
        btn_id = event.button.id
        if btn_id == "filter-all":
            self._set_filter(None)
        elif btn_id == "filter-claude":
            self._set_filter("claude")
        elif btn_id == "filter-codex":
            self._set_filter("codex")
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


def run_tui(query: str = "", agent_filter: str | None = None) -> tuple[list[str] | None, str | None]:
    """Run the TUI and return the resume command and directory if selected."""
    app = FastResumeApp(initial_query=query, agent_filter=agent_filter)
    app.run()
    return app.get_resume_command(), app.get_resume_directory()
