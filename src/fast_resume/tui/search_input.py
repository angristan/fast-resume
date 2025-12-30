"""Search input components: highlighter and suggester for keyword syntax."""

import re

from rich.highlighter import Highlighter
from rich.text import Text
from textual.suggester import Suggester

from ..config import AGENTS
from .query import KEYWORD_PATTERN, is_valid_filter_value


class KeywordHighlighter(Highlighter):
    """Highlighter for search keyword syntax (agent:, dir:, date:).

    Applies Rich styles directly to keyword prefixes and their values.
    Supports negation with - prefix or ! in value.
    Invalid values are shown in red with strikethrough.
    """

    def highlight(self, text: Text) -> None:
        """Apply highlighting to keyword syntax in the text."""
        plain = text.plain
        for match in KEYWORD_PATTERN.finditer(plain):
            neg_prefix = match.group(1)
            keyword = match.group(2)
            value = match.group(3)

            is_valid = is_valid_filter_value(keyword, value)

            # Style the negation prefix in red
            if neg_prefix:
                text.stylize("bold red", match.start(1), match.end(1))

            # Style the keyword prefix
            if is_valid:
                text.stylize("bold cyan", match.start(2), match.end(2))
            else:
                text.stylize("bold red", match.start(2), match.end(2))

            # Style the value
            if not is_valid:
                text.stylize("red strike", match.start(3), match.end(3))
            elif value.startswith("!"):
                text.stylize("bold red", match.start(3), match.start(3) + 1)
                text.stylize("green", match.start(3) + 1, match.end(3))
            else:
                text.stylize("green", match.start(3), match.end(3))


# Pattern to match partial keyword at end of input for autocomplete
_PARTIAL_KEYWORD_PATTERN = re.compile(
    r"(-?)(agent:|dir:|date:)([^\s]*)$"  # Keyword at end, possibly partial value
)

# Known values for each keyword type (agent values derived from AGENTS config)
_KEYWORD_VALUES = {
    "agent:": list(AGENTS.keys()),
    "date:": ["today", "yesterday", "week", "month"],
    # dir: has no predefined values (user-specific paths)
}


class KeywordSuggester(Suggester):
    """Suggester for keyword value autocomplete.

    Provides completions for:
    - agent: values (claude, codex, etc.)
    - date: values (today, yesterday, week, month)
    """

    def __init__(self) -> None:
        super().__init__(use_cache=True, case_sensitive=False)

    async def get_suggestion(self, value: str) -> str | None:
        """Get completion suggestion for the current input.

        Args:
            value: Current input text (casefolded if case_sensitive=False)

        Returns:
            Complete input with suggested value, or None if no suggestion.
        """
        # Find partial keyword at end of input
        match = _PARTIAL_KEYWORD_PATTERN.search(value)
        if not match:
            return None

        keyword = match.group(2)  # agent:, dir:, date:
        partial = match.group(3)  # Partial value typed so far

        # Get known values for this keyword
        known_values = _KEYWORD_VALUES.get(keyword)
        if not known_values:
            return None

        # Don't suggest if value is empty (user just typed "agent:")
        if not partial:
            return None

        # Handle ! prefix on partial value
        negated_value = partial.startswith("!")
        search_partial = partial[1:] if negated_value else partial

        # Find first matching value (but not exact match - already complete)
        for candidate in known_values:
            if candidate.lower().startswith(search_partial.lower()):
                # Skip if already complete (no suggestion needed)
                if candidate.lower() == search_partial.lower():
                    continue
                # Build the suggestion
                suggested_value = f"!{candidate}" if negated_value else candidate
                # Replace partial with full value
                suggestion = value[: match.start(3)] + suggested_value
                return suggestion

        return None
