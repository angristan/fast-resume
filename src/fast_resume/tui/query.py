"""Query parsing utilities for search keyword syntax."""

import re

from ..config import AGENTS

# Pattern to match keyword:value syntax in search queries (with optional - prefix)
KEYWORD_PATTERN = re.compile(r"(-?)(agent:|dir:|date:)(\S+)")

# Valid date keywords and pattern
VALID_DATE_KEYWORDS = {"today", "yesterday", "week", "month"}
DATE_PATTERN = re.compile(r"^([<>])?(\d+)(m|h|d|w|mo|y)$")

# Pattern to match agent: keyword specifically (for extraction/replacement)
AGENT_KEYWORD_PATTERN = re.compile(r"-?agent:(\S+)")


def is_valid_filter_value(keyword: str, value: str) -> bool:
    """Check if a filter value is valid for the given keyword."""
    check_value = value.lstrip("!")
    values = [v.strip().lstrip("!") for v in check_value.split(",") if v.strip()]

    if keyword == "agent:":
        return all(v.lower() in AGENTS for v in values)
    elif keyword == "date:":
        for v in values:
            v_lower = v.lower()
            if v_lower not in VALID_DATE_KEYWORDS and not DATE_PATTERN.match(v_lower):
                return False
        return True
    elif keyword == "dir:":
        return True  # Any value is valid for directory
    return True


def extract_agent_from_query(query: str) -> str | None:
    """Extract agent value from query string if present.

    Returns the first non-negated agent value, or None if no agent keyword.
    For mixed filters like agent:claude,!codex, returns 'claude'.
    """
    match = AGENT_KEYWORD_PATTERN.search(query)
    if not match:
        return None

    # Check if the whole keyword is negated with - prefix
    full_match = match.group(0)
    if full_match.startswith("-"):
        return None  # Negated filter, don't sync to buttons

    value = match.group(1)
    # Handle ! prefix on value
    if value.startswith("!"):
        return None  # Negated, don't sync

    # Get first non-negated value from comma-separated list
    for v in value.split(","):
        v = v.strip()
        if v and not v.startswith("!"):
            return v

    return None


def update_agent_in_query(query: str, agent: str | None) -> str:
    """Update or remove agent keyword in query string.

    Args:
        query: Current query string
        agent: Agent to set, or None to remove agent keyword

    Returns:
        Updated query string with agent keyword added/updated/removed.
    """
    # Remove existing agent keyword(s)
    query_without_agent = AGENT_KEYWORD_PATTERN.sub("", query).strip()
    # Clean up extra whitespace
    query_without_agent = " ".join(query_without_agent.split())

    if agent is None:
        return query_without_agent

    # Append agent keyword at the end
    if query_without_agent:
        return f"{query_without_agent} agent:{agent}"
    return f"agent:{agent}"
