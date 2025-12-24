"""Session preview widget for the TUI."""

from rich.markup import escape as escape_markup
from rich.text import Text
from textual.widgets import Static

from ..adapters.base import Session
from .utils import highlight_matches


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
