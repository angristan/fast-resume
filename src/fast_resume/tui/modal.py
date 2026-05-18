"""Modal dialogs for the TUI."""

from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.screen import ModalScreen
from textual.widgets import Button, Checkbox, Label

from .styles import YOLO_MODAL_CSS


class YoloModeModal(ModalScreen[bool | None]):
    """Modal asking whether to launch the selected session and in what mode.

    Dismisses with:
        None  -> cancel (stay in TUI, no resume).
        False -> launch without yolo.
        True  -> launch with yolo (auto-approve).
    """

    BINDINGS = [
        Binding("escape", "cancel", "Cancel", show=False),
        Binding("enter", "launch", "Launch", show=False),
        Binding("space", "toggle_yolo", "Toggle yolo", show=False),
        Binding("y", "set_yolo_on", "Yolo on", show=False),
        Binding("n", "set_yolo_off", "Yolo off", show=False),
        Binding("left", "focus_cancel", "Focus cancel", show=False),
        Binding("right", "focus_launch", "Focus launch", show=False),
        # Screen bindings take precedence over the App's priority Tab
        # binding (which is used for search-input suggestion accept), so
        # define our own to cycle Cancel ↔ Launch instead of being eaten.
        Binding("tab", "focus_next", "Next", show=False),
        Binding("shift+tab", "focus_previous", "Previous", show=False),
    ]

    CSS = YOLO_MODAL_CSS

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label("Launch session", id="title")
            with Horizontal(id="yolo-row"):
                # Skip the checkbox in the Tab cycle so users can't end up
                # focused on it and have Enter toggle (Textual's Checkbox
                # binds enter→toggle by default). Modal-level bindings
                # (space/y/n) still let users flip it. `can_focus` is set
                # in on_mount because it's not a constructor kwarg.
                yield Checkbox("Yolo mode (auto-approve)", id="yolo-checkbox")
            with Horizontal(id="buttons"):
                yield Button("Cancel", id="cancel-btn")
                yield Button("Launch", id="launch-btn", variant="primary")
            yield Label(
                "Space/y/n: toggle yolo · Enter: launch · Esc: cancel",
                id="hint",
            )

    def on_mount(self) -> None:
        # Skip the checkbox in Tab cycling; modal-level bindings handle toggle.
        self.query_one("#yolo-checkbox", Checkbox).can_focus = False
        # Focus Launch by default so plain Enter is the obvious action.
        self.query_one("#launch-btn", Button).focus()

    def _yolo_value(self) -> bool:
        return self.query_one("#yolo-checkbox", Checkbox).value

    def action_cancel(self) -> None:
        self.dismiss(None)

    def action_launch(self) -> None:
        self.dismiss(self._yolo_value())

    def action_toggle_yolo(self) -> None:
        self.query_one("#yolo-checkbox", Checkbox).toggle()

    def action_set_yolo_on(self) -> None:
        self.query_one("#yolo-checkbox", Checkbox).value = True

    def action_set_yolo_off(self) -> None:
        self.query_one("#yolo-checkbox", Checkbox).value = False

    def action_focus_cancel(self) -> None:
        self.query_one("#cancel-btn", Button).focus()

    def action_focus_launch(self) -> None:
        self.query_one("#launch-btn", Button).focus()

    @on(Button.Pressed, "#cancel-btn")
    def on_cancel_pressed(self) -> None:
        self.dismiss(None)

    @on(Button.Pressed, "#launch-btn")
    def on_launch_pressed(self) -> None:
        self.dismiss(self._yolo_value())
