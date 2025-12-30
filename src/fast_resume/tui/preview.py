"""Session preview widget for the TUI."""

import re
from io import StringIO
from pathlib import Path
from typing import Any

from rich.columns import Columns
from rich.console import Console, Group, RenderableType
from rich.markup import escape as escape_markup
from rich.syntax import Syntax
from rich.text import Text
from textual.widgets import Static
from textual_image.renderable import Image as ImageRenderable

from ..adapters.base import Session
from ..config import AGENTS
from .utils import highlight_matches

# Asset paths for agent icons
ASSETS_DIR = Path(__file__).parent.parent / "assets"

# Cache for agent icon renderables
_preview_icon_cache: dict[str, Any] = {}


def _get_agent_icon(agent: str) -> RenderableType | None:
    """Get the icon renderable for an agent."""
    if agent not in _preview_icon_cache:
        icon_path = ASSETS_DIR / f"{agent}.png"
        if icon_path.exists():
            try:
                _preview_icon_cache[agent] = ImageRenderable(
                    icon_path, width=2, height=1
                )
            except Exception:
                _preview_icon_cache[agent] = None
        else:
            _preview_icon_cache[agent] = None
    return _preview_icon_cache[agent]


class SessionPreview(Static):
    """Preview pane showing session content."""

    # Highlight style for matches in preview
    MATCH_STYLE = "bold reverse"
    # Max lines to show for a single assistant message
    MAX_ASSISTANT_LINES = 4

    # Pattern to match code blocks with optional language
    CODE_BLOCK_PATTERN = re.compile(r"```(\w*)")

    def __init__(self) -> None:
        super().__init__("", id="preview")

    def update_preview(self, session: Session | None, query: str = "") -> None:
        """Update the preview with session content, highlighting matches."""
        if session is None:
            self.update("")
            return

        result = self._build_preview_renderable(session, query)
        self.update(result)

    def _build_preview_renderable(
        self, session: Session, query: str = ""
    ) -> RenderableType:
        """Build the preview renderable with icons. Returns a Group of renderables."""
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

        # Get agent config and icon
        agent_config = AGENTS.get(
            session.agent, {"color": "white", "badge": session.agent}
        )
        agent_icon = _get_agent_icon(session.agent)

        # Build list of renderables
        renderables: list[RenderableType] = []

        # Split by double newlines to get individual messages
        messages = preview_text.split("\n\n")

        for i, msg in enumerate(messages):
            msg = msg.rstrip()
            if not msg.strip():
                continue

            # Detect if this is a user message
            is_user = msg.startswith("» ")

            if is_user:
                # User message - render as text
                text = Text()
                self._render_message(text, msg, query, is_user, agent_config)
                renderables.append(text)
            else:
                # Assistant message - add icon + text
                if agent_icon is not None:
                    # Create icon with text on same line using Columns
                    text = Text()
                    self._render_message_content(text, msg, query, agent_config)
                    renderables.append(Columns([agent_icon, text], padding=(0, 1)))
                else:
                    # Fallback to badge
                    text = Text()
                    self._render_message(text, msg, query, is_user, agent_config)
                    renderables.append(text)

        return Group(*renderables)

    def build_preview_text(self, session: Session, query: str = "") -> Text:
        """Build the preview text for a session. Returns a Rich Text object."""
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

        # Get agent config for styling
        agent_config = AGENTS.get(
            session.agent, {"color": "white", "badge": session.agent}
        )

        # Split by double newlines to get individual messages
        messages = preview_text.split("\n\n")

        for i, msg in enumerate(messages):
            msg = (
                msg.rstrip()
            )  # Only strip trailing whitespace, preserve leading indent
            if not msg.strip():  # Skip if empty after stripping
                continue

            # Detect if this is a user message
            is_user = msg.startswith("» ")

            # Process message with code block handling
            self._render_message(result, msg, query, is_user, agent_config)

        return result

    def _render_message(
        self,
        result: Text,
        msg: str,
        query: str,
        is_user: bool,
        agent_config: dict[str, str],
    ) -> None:
        """Render a single message with code block syntax highlighting."""
        lines = msg.split("\n")

        # Truncate assistant messages
        if not is_user and len(lines) > self.MAX_ASSISTANT_LINES:
            lines = lines[: self.MAX_ASSISTANT_LINES]
            lines.append("...")

        # Add role prefix on first line
        first_line = True

        i = 0
        while i < len(lines):
            line = lines[i]

            # Check for code block start
            match = self.CODE_BLOCK_PATTERN.match(line)
            if match:
                language = match.group(1) or ""
                # Collect code block content
                code_lines = []
                i += 1
                while i < len(lines) and not lines[i].startswith("```"):
                    code_lines.append(lines[i])
                    i += 1

                # Render code block with syntax highlighting
                if code_lines:
                    code = "\n".join(code_lines)
                    self._render_code_with_highlighting(result, code, language)

                # Skip closing ```
                if i < len(lines) and lines[i].startswith("```"):
                    i += 1
                continue

            # Handle user prompt
            if line.startswith("» "):
                result.append("» ", style="bold cyan")
                content_part = escape_markup(line[2:])
                if len(content_part) > 200:
                    content_part = content_part[:200].rsplit(" ", 1)[0] + " ..."
                highlighted = highlight_matches(
                    content_part, query, style=self.MATCH_STYLE
                )
                result.append_text(highlighted)
                result.append("\n")
                first_line = False
            elif line == "...":
                result.append("   ⋯\n", style="dim")
            elif line.startswith("..."):
                # Truncation indicator from context
                result.append(escape_markup(line) + "\n", style="dim")
            elif line.startswith("  "):
                # Assistant response (indented) - add agent prefix on first line
                if first_line:
                    agent_color = agent_config["color"]
                    result.append("● ", style=agent_color)
                    result.append(agent_config["badge"], style=f"bold {agent_color}")
                    result.append(" ")
                    # Remove leading indent for first line since we added prefix
                    content = line.lstrip()
                    first_line = False
                else:
                    content = line
                highlighted = highlight_matches(
                    escape_markup(content), query, style=self.MATCH_STYLE
                )
                result.append_text(highlighted)
                result.append("\n")
            else:
                # Other content
                highlighted = highlight_matches(
                    escape_markup(line), query, style=self.MATCH_STYLE
                )
                result.append_text(highlighted)
                result.append("\n")

            i += 1

    def _render_message_content(
        self,
        result: Text,
        msg: str,
        query: str,
        agent_config: dict[str, str],
    ) -> None:
        """Render assistant message content without badge (icon shown separately)."""
        lines = msg.split("\n")

        # Truncate assistant messages
        if len(lines) > self.MAX_ASSISTANT_LINES:
            lines = lines[: self.MAX_ASSISTANT_LINES]
            lines.append("...")

        i = 0
        while i < len(lines):
            line = lines[i]

            # Check for code block start
            match = self.CODE_BLOCK_PATTERN.match(line)
            if match:
                language = match.group(1) or ""
                code_lines = []
                i += 1
                while i < len(lines) and not lines[i].startswith("```"):
                    code_lines.append(lines[i])
                    i += 1

                if code_lines:
                    code = "\n".join(code_lines)
                    self._render_code_with_highlighting(result, code, language)

                if i < len(lines) and lines[i].startswith("```"):
                    i += 1
                continue

            if line == "...":
                result.append("⋯\n", style="dim")
            elif line.startswith("..."):
                result.append(escape_markup(line) + "\n", style="dim")
            elif line.startswith("  "):
                # Assistant response - strip leading indent
                content = line.lstrip()
                highlighted = highlight_matches(
                    escape_markup(content), query, style=self.MATCH_STYLE
                )
                result.append_text(highlighted)
                result.append("\n")
            else:
                highlighted = highlight_matches(
                    escape_markup(line), query, style=self.MATCH_STYLE
                )
                result.append_text(highlighted)
                result.append("\n")

            i += 1

    def _render_code_with_highlighting(
        self, result: Text, code: str, language: str
    ) -> None:
        """Render code with syntax highlighting."""
        # Map common language aliases
        lang_map = {
            "js": "javascript",
            "ts": "typescript",
            "py": "python",
            "rb": "ruby",
            "sh": "bash",
            "yml": "yaml",
            "": "text",
        }
        language = lang_map.get(language, language) or "text"

        try:
            syntax = Syntax(
                code,
                language,
                theme="ansi_dark",
                line_numbers=False,
                word_wrap=True,
                background_color="default",
            )
            # Rich Syntax objects can be converted to Text
            string_io = StringIO()
            console = Console(file=string_io, force_terminal=True, width=200)
            console.print(syntax, end="")
            highlighted_code = string_io.getvalue()

            # Add with indentation
            for line in highlighted_code.rstrip().split("\n"):
                result.append("  ")
                result.append_text(Text.from_ansi(line))
                result.append("\n")
        except Exception:
            # Fall back to plain dim text
            for line in code.split("\n"):
                result.append("  " + escape_markup(line) + "\n", style="dim")
