"""Filter bar widget for agent selection."""

from textual.containers import Horizontal
from textual.events import Click
from textual.message import Message
from textual.reactive import reactive
from textual.widgets import Label
from textual_image.widget import Image as ImageWidget

from ..config import AGENTS
from .utils import ASSETS_DIR

# Filter keys in display order (None = "All")
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
    f"filter-{key or 'all'}": key for key in FILTER_KEYS
}


class FilterBar(Horizontal):
    """A horizontal bar of filter buttons for agent selection.

    Emits FilterBar.Changed when filter selection changes.
    """

    class Changed(Message):
        """Posted when the active filter changes."""

        def __init__(self, filter_key: str | None) -> None:
            self.filter_key = filter_key
            super().__init__()

    active_filter: reactive[str | None] = reactive(None)

    def __init__(
        self,
        initial_filter: str | None = None,
        id: str | None = None,
    ) -> None:
        super().__init__(id=id)
        self._initial_filter = initial_filter
        self._filter_buttons: dict[str | None, Horizontal] = {}

    def compose(self):
        """Create filter buttons."""
        for filter_key in FILTER_KEYS:
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

    def on_mount(self) -> None:
        """Initialize active filter state."""
        self.active_filter = self._initial_filter
        self._update_button_styles()

    def watch_active_filter(self, value: str | None) -> None:
        """Update button styles when active filter changes."""
        self._update_button_styles()

    def _update_button_styles(self) -> None:
        """Update filter button active states."""
        for filter_key, btn in self._filter_buttons.items():
            if filter_key == self.active_filter:
                btn.add_class("-active")
            else:
                btn.remove_class("-active")

    def set_active(self, filter_key: str | None, notify: bool = False) -> None:
        """Set the active filter programmatically.

        Args:
            filter_key: The filter to activate, or None for "All".
            notify: If True, emit a Changed message.
        """
        if filter_key != self.active_filter:
            self.active_filter = filter_key
            if notify:
                self.post_message(self.Changed(filter_key))

    def on_click(self, event: Click) -> None:
        """Handle click on filter buttons."""
        # Walk up to find the filter-btn container (click might be on child widget)
        widget = event.widget
        while widget and widget is not self:
            if hasattr(widget, "classes") and "filter-btn" in widget.classes:
                if widget.id in _FILTER_ID_MAP:
                    new_filter = _FILTER_ID_MAP[widget.id]
                    if new_filter != self.active_filter:
                        self.active_filter = new_filter
                        self.post_message(self.Changed(new_filter))
                return
            widget = widget.parent
