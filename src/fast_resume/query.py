"""Query parser for keyword-based search syntax.

Supports syntax like: `agent:claude,codex dir:my-project date:<1d api auth`

Keywords:
- agent: Filter by agent name (supports multiple: agent:claude,codex)
- dir: Filter by directory (substring match, supports multiple: dir:proj1,proj2)
- date: Filter by date (today, yesterday, <1h, >1d, etc.)

Negation:
- Use ! prefix on value: agent:!claude (exclude claude)
- Use - prefix on keyword: -agent:claude (exclude claude)
"""

import re
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from enum import Enum


class DateOp(Enum):
    """Date filter comparison operator."""

    EXACT = "exact"  # today, yesterday
    LESS_THAN = "<"  # <1h (within the last hour)
    GREATER_THAN = ">"  # >1d (older than 1 day)


@dataclass
class DateFilter:
    """Parsed date filter."""

    op: DateOp
    value: str  # Original value for display
    cutoff: datetime  # The cutoff datetime for comparison
    negated: bool = False  # True if filter should exclude matches


@dataclass
class Filter:
    """A filter with multiple possible values and negation support.

    Supports mixed include/exclude: agent:claude,!codex means
    "match claude but not codex".
    """

    include: list[str] = field(default_factory=list)  # Values to match (OR logic)
    exclude: list[str] = field(default_factory=list)  # Values to exclude (AND logic)

    @property
    def values(self) -> list[str]:
        """All values (for backward compat and display)."""
        return self.include + self.exclude

    @property
    def negated(self) -> bool:
        """True if filter is exclude-only (for backward compat)."""
        return len(self.include) == 0 and len(self.exclude) > 0

    def matches(self, value: str, substring: bool = False) -> bool:
        """Check if value matches this filter.

        Args:
            value: The value to check
            substring: If True, check if any filter value is a substring

        Returns:
            True if the value matches include list (or no include list)
            AND doesn't match exclude list.
        """
        if not self.include and not self.exclude:
            return True

        def check(filter_val: str) -> bool:
            if substring:
                return filter_val.lower() in value.lower()
            return value == filter_val

        # Check excludes first - if any match, reject
        if any(check(v) for v in self.exclude):
            return False

        # If no includes, accept (exclude-only filter)
        if not self.include:
            return True

        # Check includes - at least one must match
        return any(check(v) for v in self.include)


@dataclass
class ParsedQuery:
    """Result of parsing a search query."""

    text: str  # Free-text search terms
    agent: Filter | None  # Extracted agent filter
    directory: Filter | None  # Extracted directory filter
    date: DateFilter | None  # Extracted date filter


# Pattern to match keyword:value pairs with optional - prefix for negation
# Handles: agent:value, -agent:value, dir:"value with spaces", date:<1h
_KEYWORD_PATTERN = re.compile(
    r"(-?)"  # optional negation prefix
    r"(agent|dir|date):"  # keyword prefix
    r'(?:"([^"]+)"|(\S+))'  # quoted value or unquoted value
)

# Pattern to parse relative time like <1h, >2d, <30m
_RELATIVE_TIME_PATTERN = re.compile(
    r"^([<>])?(\d+)(m|h|d|w|mo|y)$"  # operator, number, unit
)

# Time unit multipliers (in seconds)
_TIME_UNITS = {
    "m": 60,  # minutes
    "h": 3600,  # hours
    "d": 86400,  # days
    "w": 604800,  # weeks
    "mo": 2592000,  # months (30 days)
    "y": 31536000,  # years (365 days)
}


def _parse_date_value(value: str, negated: bool = False) -> DateFilter | None:
    """Parse a date filter value into a DateFilter.

    Supports:
    - today: sessions from today
    - yesterday: sessions from yesterday
    - <Nu: sessions newer than N units (e.g., <1h, <2d)
    - >Nu: sessions older than N units (e.g., >1h, >2d)
    - Nu: same as <Nu (default to "within")
    """
    now = datetime.now()

    # Handle ! prefix for negation in value
    if value.startswith("!"):
        value = value[1:]
        negated = True

    value_lower = value.lower()

    # Handle named dates
    if value_lower == "today":
        cutoff = now.replace(hour=0, minute=0, second=0, microsecond=0)
        return DateFilter(op=DateOp.EXACT, value=value, cutoff=cutoff, negated=negated)
    elif value_lower == "yesterday":
        cutoff = (now - timedelta(days=1)).replace(
            hour=0, minute=0, second=0, microsecond=0
        )
        return DateFilter(op=DateOp.EXACT, value=value, cutoff=cutoff, negated=negated)
    elif value_lower == "week":
        cutoff = now - timedelta(days=7)
        return DateFilter(
            op=DateOp.LESS_THAN, value=value, cutoff=cutoff, negated=negated
        )
    elif value_lower == "month":
        cutoff = now - timedelta(days=30)
        return DateFilter(
            op=DateOp.LESS_THAN, value=value, cutoff=cutoff, negated=negated
        )

    # Handle relative time patterns
    match = _RELATIVE_TIME_PATTERN.match(value_lower)
    if match:
        op_str, num_str, unit = match.groups()
        num = int(num_str)
        seconds = num * _TIME_UNITS[unit]
        cutoff = now - timedelta(seconds=seconds)

        if op_str == ">":
            return DateFilter(
                op=DateOp.GREATER_THAN, value=value, cutoff=cutoff, negated=negated
            )
        else:  # < or no operator defaults to "within"
            return DateFilter(
                op=DateOp.LESS_THAN, value=value, cutoff=cutoff, negated=negated
            )

    return None


def _parse_filter_value(value: str, negated: bool) -> Filter:
    """Parse a filter value, handling ! prefix and comma-separated values.

    Args:
        value: The filter value (e.g., "claude", "claude,codex", "claude,!codex")
        negated: True if the keyword had - prefix (e.g., -agent:claude)

    Returns:
        Filter with values sorted into include/exclude lists.
    """
    include: list[str] = []
    exclude: list[str] = []

    # Split comma-separated values
    raw_values = [v.strip() for v in value.split(",") if v.strip()]

    for val in raw_values:
        # Check for ! prefix on individual value
        if val.startswith("!"):
            exclude.append(val[1:])
        elif negated:
            # Whole filter negated with - prefix
            exclude.append(val)
        else:
            include.append(val)

    return Filter(include=include, exclude=exclude)


def parse_query(query: str) -> ParsedQuery:
    """Parse keyword syntax from query string.

    Args:
        query: Raw query string like "agent:claude,codex dir:my-project date:<1d api"

    Returns:
        ParsedQuery with extracted filters and remaining free-text.

    Examples:
        >>> parse_query("agent:claude api auth")
        ParsedQuery(text="api auth", agent=Filter(include=["claude"]), ...)

        >>> parse_query("agent:claude,codex api")  # Multiple values (OR)
        ParsedQuery(text="api", agent=Filter(include=["claude", "codex"]), ...)

        >>> parse_query("-agent:claude api")  # Negation (exclude all)
        ParsedQuery(text="api", agent=Filter(exclude=["claude"]), ...)

        >>> parse_query("agent:!claude api")  # Negation (exclude specific)
        ParsedQuery(text="api", agent=Filter(exclude=["claude"]), ...)

        >>> parse_query("agent:claude,!codex api")  # Mixed include/exclude
        ParsedQuery(text="api", agent=Filter(include=["claude"], exclude=["codex"]), ...)
    """
    agent: Filter | None = None
    directory: Filter | None = None
    date: DateFilter | None = None

    # Find all keyword matches and track their positions
    matches = list(_KEYWORD_PATTERN.finditer(query))

    # Extract keyword values (last one wins if duplicates)
    for match in matches:
        neg_prefix = match.group(1) == "-"
        keyword = match.group(2)
        # Value is either quoted (group 3) or unquoted (group 4)
        value = match.group(3) or match.group(4)

        if keyword == "agent":
            agent = _parse_filter_value(value, neg_prefix)
        elif keyword == "dir":
            directory = _parse_filter_value(value, neg_prefix)
        elif keyword == "date":
            date = _parse_date_value(value, neg_prefix)

    # Remove keyword:value pairs from query to get free-text
    text = _KEYWORD_PATTERN.sub("", query)
    # Clean up extra whitespace
    text = " ".join(text.split())

    return ParsedQuery(text=text, agent=agent, directory=directory, date=date)
