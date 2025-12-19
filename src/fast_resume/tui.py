"""Textual TUI for fast-resume."""

import os
from datetime import datetime

from rich.markup import escape as escape_markup
from rich.text import Text
from textual import on
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.reactive import reactive
from textual.widgets import DataTable, Footer, Input, Static

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
            # Use reverse video (black on yellow) for high visibility
            result.stylize("bold black on yellow", idx, idx + len(term))
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

    ENABLE_COMMAND_PALETTE = False
    theme = "gruvbox"

    CSS = """
    Screen {
        layout: vertical;
        width: 100%;
    }

    #search-container {
        height: 3;
        width: 100%;
    }

    #search-input {
        width: 1fr;
    }

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

    #preview-container {
        height: 10;
        border: solid $primary;
        padding: 0 1;
    }

    #preview-container.hidden {
        display: none;
    }

    #preview {
        height: 100%;
        overflow-y: auto;
    }

    DataTable > .datatable--cursor {
        background: $accent;
    }

    .agent-badge {
        padding: 0 1;
    }

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

    Footer {
        background: $surface;
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
    ]

    show_preview: reactive[bool] = reactive(True)
    selected_session: reactive[Session | None] = reactive(None)

    def __init__(self, initial_query: str = "", agent_filter: str | None = None):
        super().__init__()
        self.search_engine = SessionSearch()
        self.initial_query = initial_query
        self.agent_filter = agent_filter
        self.sessions: list[Session] = []
        self._resume_command: list[str] | None = None
        self._resume_directory: str | None = None
        self._current_query: str = ""

    def compose(self) -> ComposeResult:
        """Create child widgets."""
        with Vertical():
            with Horizontal(id="search-container"):
                yield Input(
                    placeholder="Search sessions...",
                    id="search-input",
                    value=self.initial_query,
                )
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

        # Focus search input
        self.query_one("#search-input", Input).focus()

        # Initial search
        self._do_search(self.initial_query)

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

    def _do_search(self, query: str) -> None:
        """Perform search and update results."""
        self._current_query = query
        self.sessions = self.search_engine.search(
            query, agent_filter=self.agent_filter, limit=100
        )
        self._update_table()

    def _update_table(self) -> None:
        """Update the results table with current sessions."""
        table = self.query_one("#results-table", DataTable)
        table.clear()

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
