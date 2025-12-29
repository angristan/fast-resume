"""Query parser for keyword-based search syntax.

Supports syntax like: `agent:claude dir:my-project date:<1d api auth`

Keywords:
- agent: Filter by agent name
- dir: Filter by directory (substring match)
- date: Filter by date (today, yesterday, <1h, >1d, etc.)
"""

import re
from dataclasses import dataclass
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


@dataclass
class ParsedQuery:
    """Result of parsing a search query."""

    text: str  # Free-text search terms
    agent: str | None  # Extracted agent filter
    directory: str | None  # Extracted directory filter
    date: DateFilter | None  # Extracted date filter


# Pattern to match keyword:value pairs
# Handles: agent:value, dir:value, dir:"value with spaces", date:<1h
_KEYWORD_PATTERN = re.compile(
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


def _parse_date_value(value: str) -> DateFilter | None:
    """Parse a date filter value into a DateFilter.

    Supports:
    - today: sessions from today
    - yesterday: sessions from yesterday
    - <Nu: sessions newer than N units (e.g., <1h, <2d)
    - >Nu: sessions older than N units (e.g., >1h, >2d)
    - Nu: same as <Nu (default to "within")
    """
    now = datetime.now()
    value_lower = value.lower()

    # Handle named dates
    if value_lower == "today":
        cutoff = now.replace(hour=0, minute=0, second=0, microsecond=0)
        return DateFilter(op=DateOp.EXACT, value=value, cutoff=cutoff)
    elif value_lower == "yesterday":
        cutoff = (now - timedelta(days=1)).replace(
            hour=0, minute=0, second=0, microsecond=0
        )
        return DateFilter(op=DateOp.EXACT, value=value, cutoff=cutoff)
    elif value_lower == "week":
        cutoff = now - timedelta(days=7)
        return DateFilter(op=DateOp.LESS_THAN, value=value, cutoff=cutoff)
    elif value_lower == "month":
        cutoff = now - timedelta(days=30)
        return DateFilter(op=DateOp.LESS_THAN, value=value, cutoff=cutoff)

    # Handle relative time patterns
    match = _RELATIVE_TIME_PATTERN.match(value_lower)
    if match:
        op_str, num_str, unit = match.groups()
        num = int(num_str)
        seconds = num * _TIME_UNITS[unit]
        cutoff = now - timedelta(seconds=seconds)

        if op_str == ">":
            return DateFilter(op=DateOp.GREATER_THAN, value=value, cutoff=cutoff)
        else:  # < or no operator defaults to "within"
            return DateFilter(op=DateOp.LESS_THAN, value=value, cutoff=cutoff)

    return None


def parse_query(query: str) -> ParsedQuery:
    """Parse keyword syntax from query string.

    Args:
        query: Raw query string like "agent:claude dir:my-project date:<1d api auth"

    Returns:
        ParsedQuery with extracted filters and remaining free-text.

    Examples:
        >>> parse_query("agent:claude api auth")
        ParsedQuery(text="api auth", agent="claude", directory=None, date=None)

        >>> parse_query('dir:"my project" bug fix')
        ParsedQuery(text="bug fix", agent=None, directory="my project", date=None)

        >>> parse_query("date:<1h api")
        ParsedQuery(text="api", agent=None, directory=None, date=DateFilter(...))
    """
    agent: str | None = None
    directory: str | None = None
    date: DateFilter | None = None

    # Find all keyword matches and track their positions
    matches = list(_KEYWORD_PATTERN.finditer(query))

    # Extract keyword values (last one wins if duplicates)
    for match in matches:
        keyword = match.group(1)
        # Value is either quoted (group 2) or unquoted (group 3)
        value = match.group(2) or match.group(3)

        if keyword == "agent":
            agent = value
        elif keyword == "dir":
            directory = value
        elif keyword == "date":
            date = _parse_date_value(value)

    # Remove keyword:value pairs from query to get free-text
    text = _KEYWORD_PATTERN.sub("", query)
    # Clean up extra whitespace
    text = " ".join(text.split())

    return ParsedQuery(text=text, agent=agent, directory=directory, date=date)
