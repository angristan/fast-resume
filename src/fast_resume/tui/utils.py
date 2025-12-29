"""Utility functions for the TUI."""

import math
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

import humanize
from datetime import datetime
from rich.columns import Columns
from rich.console import RenderableType
from rich.text import Text
from textual_image.renderable import Image as ImageRenderable

from ..config import AGENTS

# Asset paths for agent icons
ASSETS_DIR = Path(__file__).parent.parent / "assets"

# Cache for agent icon renderables (textual_image has incomplete type stubs)
_icon_cache: dict[str, Any] = {}


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


def get_age_color(age_hours: float) -> str:
    """Return a hex color based on session age using exponential decay gradient.

    Colors transition: Green (0h) → Yellow (12h) → Orange (3d) → Dim gray (30d+)
    """
    decay_rate = 0.005  # Controls how fast colors fade
    t = 1 - math.exp(-decay_rate * age_hours)  # 0 at 0h, approaches 1 asymptotically

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
        g = 100
        b = int(50 + s * 50)  # 50 → 100

    return f"#{r:02x}{g:02x}{b:02x}"


def copy_to_clipboard(text: str) -> bool:
    """Copy text to system clipboard.

    Returns True on success, False on failure.
    """
    try:
        if sys.platform == "darwin":
            subprocess.run(["pbcopy"], input=text.encode(), check=True)
        elif sys.platform == "win32":
            subprocess.run(["clip"], input=text.encode(), check=True)
        else:
            # Linux - try xclip or xsel
            try:
                subprocess.run(
                    ["xclip", "-selection", "clipboard"],
                    input=text.encode(),
                    check=True,
                )
            except FileNotFoundError:
                subprocess.run(
                    ["xsel", "--clipboard", "--input"],
                    input=text.encode(),
                    check=True,
                )
        return True
    except Exception:
        return False
