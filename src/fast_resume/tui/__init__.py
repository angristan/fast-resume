"""TUI package for fast-resume."""

from .. import __version__
from .app import FastResumeApp
from .search_input import KeywordSuggester
from .modal import YoloModeModal
from .preview import SessionPreview
from .utils import (
    ASSETS_DIR,
    _icon_cache,
    copy_to_clipboard,
    format_directory,
    format_time_ago,
    get_age_color,
    get_agent_icon,
    highlight_matches,
)


def run_tui(
    query: str = "",
    agent_filter: str | None = None,
    yolo: bool = False,
    no_version_check: bool = False,
) -> tuple[list[str] | None, str | None]:
    """Run the TUI and return the resume command and directory if selected."""
    app = FastResumeApp(
        initial_query=query,
        agent_filter=agent_filter,
        yolo=yolo,
        no_version_check=no_version_check,
    )
    app.run()

    if not no_version_check and app._available_update:
        print(
            f"\nUpdate available: {__version__} → {app._available_update}\n"
            f"Run: uv tool upgrade fast-resume"
        )

    return app.get_resume_command(), app.get_resume_directory()


__all__ = [
    "run_tui",
    "FastResumeApp",
    "KeywordSuggester",
    "YoloModeModal",
    "SessionPreview",
    "ASSETS_DIR",
    "copy_to_clipboard",
    "format_directory",
    "format_time_ago",
    "get_age_color",
    "get_agent_icon",
    "highlight_matches",
    "_icon_cache",
]
