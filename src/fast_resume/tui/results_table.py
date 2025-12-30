"""Results table widget for displaying sessions."""

from datetime import datetime

from rich.text import Text
from textual.message import Message
from textual.widgets import DataTable

from ..adapters.base import Session
from .utils import (
    format_directory,
    format_time_ago,
    get_age_color,
    get_agent_icon,
    highlight_matches,
)


# Column width breakpoints: (min_width, agent, dir, msgs, date)
_COL_WIDTHS = [
    (120, 12, 30, 6, 18),  # Wide
    (90, 12, 22, 5, 15),  # Medium
    (60, 12, 16, 5, 12),  # Narrow
    (0, 11, 0, 4, 10),  # Very narrow (hide directory)
]


class ResultsTable(DataTable):
    """A data table for displaying session results.

    Handles responsive column sizing and session rendering.
    Emits ResultsTable.Selected when a row is highlighted.
    """

    class Selected(Message):
        """Posted when a session is selected."""

        def __init__(self, session: Session | None) -> None:
            self.session = session
            super().__init__()

    def __init__(self, id: str | None = None) -> None:
        super().__init__(
            id=id,
            cursor_type="row",
            cursor_background_priority="renderable",
            cursor_foreground_priority="renderable",
        )
        self._displayed_sessions: list[Session] = []
        self._title_width: int = 60
        self._dir_width: int = 22
        self._current_query: str = ""

    def on_mount(self) -> None:
        """Set up table columns on mount."""
        (
            self._col_agent,
            self._col_title,
            self._col_dir,
            self._col_msgs,
            self._col_date,
        ) = self.add_columns("Agent", "Title", "Directory", "Turns", "Date")
        self._update_responsive_widths()

    def on_resize(self) -> None:
        """Handle resize events."""
        if hasattr(self, "_col_agent"):
            self._update_responsive_widths()
            if self._displayed_sessions:
                self._render_sessions()

    def _update_responsive_widths(self) -> None:
        """Update column widths based on container size."""
        width = self.size.width
        if width == 0:
            # Not yet laid out, use reasonable defaults
            width = 120

        # Find appropriate breakpoint
        agent_w, dir_w, msgs_w, date_w = next(
            (a, d, m, t) for min_w, a, d, m, t in _COL_WIDTHS if width >= min_w
        )
        title_w = max(15, width - agent_w - dir_w - msgs_w - date_w - 8)

        for col in self.columns.values():
            col.auto_width = False
        self.columns[self._col_agent].width = agent_w
        self.columns[self._col_title].width = title_w
        self.columns[self._col_dir].width = dir_w
        self.columns[self._col_msgs].width = msgs_w
        self.columns[self._col_date].width = date_w

        self._title_width, self._dir_width = title_w, dir_w
        self.refresh()

    def update_sessions(
        self, sessions: list[Session], query: str = ""
    ) -> Session | None:
        """Update the table with new sessions.

        Args:
            sessions: List of sessions to display.
            query: Current search query for highlighting matches.

        Returns:
            The selected session (first one), or None if no sessions.
        """
        self._displayed_sessions = sessions
        self._current_query = query
        self._render_sessions()

        if sessions:
            self.move_cursor(row=0)
            return sessions[0]
        return None

    def _render_sessions(self) -> None:
        """Render sessions to the table."""
        self.clear()

        if not self._displayed_sessions:
            self.add_row(
                "",
                Text("No sessions found", style="dim italic"),
                "",
                "",
                "",
            )
            return

        for session in self._displayed_sessions:
            # Get agent icon (image or text fallback)
            icon = get_agent_icon(session.agent)

            # Title - truncate and highlight matches
            title = highlight_matches(
                session.title, self._current_query, max_len=self._title_width
            )

            # Format directory - truncate based on column width
            dir_w = self._dir_width
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

            self.add_row(icon, title, dir_text, msgs_text, time_text)

    def get_selected_session(self) -> Session | None:
        """Get the currently selected session."""
        if self.cursor_row is not None and self.cursor_row < len(
            self._displayed_sessions
        ):
            return self._displayed_sessions[self.cursor_row]
        return None

    def on_data_table_row_highlighted(self, event: DataTable.RowHighlighted) -> None:
        """Handle row highlight and emit Selected message."""
        session = self.get_selected_session()
        self.post_message(self.Selected(session))

    @property
    def displayed_sessions(self) -> list[Session]:
        """Get the list of displayed sessions."""
        return self._displayed_sessions
