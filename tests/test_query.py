"""Tests for query parser."""

from datetime import datetime, timedelta


from fast_resume.query import DateOp, parse_query


class TestParseQuery:
    """Tests for parse_query function."""

    def test_no_keywords(self):
        """Plain text query without keywords."""
        result = parse_query("api auth bug")
        assert result.text == "api auth bug"
        assert result.agent is None
        assert result.directory is None
        assert result.date is None

    def test_agent_keyword(self):
        """Query with agent keyword."""
        result = parse_query("agent:claude api auth")
        assert result.text == "api auth"
        assert result.agent == "claude"
        assert result.directory is None
        assert result.date is None

    def test_dir_keyword(self):
        """Query with dir keyword."""
        result = parse_query("dir:my-project bug fix")
        assert result.text == "bug fix"
        assert result.agent is None
        assert result.directory == "my-project"
        assert result.date is None

    def test_both_keywords(self):
        """Query with both agent and dir keywords."""
        result = parse_query("agent:claude dir:my-project auth")
        assert result.text == "auth"
        assert result.agent == "claude"
        assert result.directory == "my-project"
        assert result.date is None

    def test_keywords_at_end(self):
        """Keywords at the end of query."""
        result = parse_query("auth bug agent:codex")
        assert result.text == "auth bug"
        assert result.agent == "codex"

    def test_keywords_in_middle(self):
        """Keywords in the middle of query."""
        result = parse_query("api agent:claude auth")
        assert result.text == "api auth"
        assert result.agent == "claude"

    def test_empty_query(self):
        """Empty query string."""
        result = parse_query("")
        assert result.text == ""
        assert result.agent is None
        assert result.directory is None
        assert result.date is None

    def test_only_keywords(self):
        """Query with only keywords, no free text."""
        result = parse_query("agent:claude dir:project")
        assert result.text == ""
        assert result.agent == "claude"
        assert result.directory == "project"

    def test_quoted_value(self):
        """Keyword with quoted value containing spaces."""
        result = parse_query('dir:"my project" bug')
        assert result.text == "bug"
        assert result.directory == "my project"

    def test_duplicate_keyword_last_wins(self):
        """When same keyword appears twice, last value wins."""
        result = parse_query("agent:claude agent:codex api")
        assert result.text == "api"
        assert result.agent == "codex"

    def test_whitespace_handling(self):
        """Extra whitespace is cleaned up."""
        result = parse_query("  agent:claude   api   auth  ")
        assert result.text == "api auth"
        assert result.agent == "claude"

    def test_case_preserved(self):
        """Keyword values preserve case."""
        result = parse_query("dir:MyProject")
        assert result.directory == "MyProject"

    def test_hyphenated_values(self):
        """Keywords with hyphenated values."""
        result = parse_query("agent:copilot-cli api")
        assert result.text == "api"
        assert result.agent == "copilot-cli"

    def test_path_with_slashes(self):
        """Dir keyword with path containing slashes."""
        result = parse_query("dir:/home/user/project api")
        assert result.text == "api"
        assert result.directory == "/home/user/project"

    def test_colon_in_free_text(self):
        """Colon in free text that's not a keyword."""
        result = parse_query("error: something failed agent:claude")
        # "error:" is not a known keyword, so it stays in text
        assert result.text == "error: something failed"
        assert result.agent == "claude"


class TestDateFilter:
    """Tests for date filter parsing."""

    def test_date_today(self):
        """date:today filters to sessions from today."""
        result = parse_query("date:today api")
        assert result.text == "api"
        assert result.date is not None
        assert result.date.op == DateOp.EXACT
        assert result.date.value == "today"
        # Cutoff should be start of today
        now = datetime.now()
        expected = now.replace(hour=0, minute=0, second=0, microsecond=0)
        assert result.date.cutoff == expected

    def test_date_yesterday(self):
        """date:yesterday filters to sessions from yesterday."""
        result = parse_query("date:yesterday")
        assert result.date is not None
        assert result.date.op == DateOp.EXACT
        assert result.date.value == "yesterday"

    def test_date_less_than_hours(self):
        """date:<1h filters to sessions within the last hour."""
        result = parse_query("date:<1h api")
        assert result.text == "api"
        assert result.date is not None
        assert result.date.op == DateOp.LESS_THAN
        # Cutoff should be approximately 1 hour ago
        now = datetime.now()
        expected = now - timedelta(hours=1)
        assert abs((result.date.cutoff - expected).total_seconds()) < 2

    def test_date_less_than_days(self):
        """date:<2d filters to sessions within the last 2 days."""
        result = parse_query("date:<2d")
        assert result.date is not None
        assert result.date.op == DateOp.LESS_THAN
        now = datetime.now()
        expected = now - timedelta(days=2)
        assert abs((result.date.cutoff - expected).total_seconds()) < 2

    def test_date_greater_than(self):
        """date:>1d filters to sessions older than 1 day."""
        result = parse_query("date:>1d")
        assert result.date is not None
        assert result.date.op == DateOp.GREATER_THAN
        now = datetime.now()
        expected = now - timedelta(days=1)
        assert abs((result.date.cutoff - expected).total_seconds()) < 2

    def test_date_without_operator(self):
        """date:1h (no operator) defaults to less than."""
        result = parse_query("date:1h")
        assert result.date is not None
        assert result.date.op == DateOp.LESS_THAN

    def test_date_minutes(self):
        """date:<30m filters to sessions within the last 30 minutes."""
        result = parse_query("date:<30m")
        assert result.date is not None
        now = datetime.now()
        expected = now - timedelta(minutes=30)
        assert abs((result.date.cutoff - expected).total_seconds()) < 2

    def test_date_weeks(self):
        """date:<2w filters to sessions within the last 2 weeks."""
        result = parse_query("date:<2w")
        assert result.date is not None
        now = datetime.now()
        expected = now - timedelta(weeks=2)
        assert abs((result.date.cutoff - expected).total_seconds()) < 2

    def test_date_months(self):
        """date:<1mo filters to sessions within the last month."""
        result = parse_query("date:<1mo")
        assert result.date is not None
        now = datetime.now()
        expected = now - timedelta(days=30)
        assert abs((result.date.cutoff - expected).total_seconds()) < 2

    def test_date_week_shortcut(self):
        """date:week filters to sessions within the last week."""
        result = parse_query("date:week")
        assert result.date is not None
        assert result.date.op == DateOp.LESS_THAN

    def test_date_month_shortcut(self):
        """date:month filters to sessions within the last month."""
        result = parse_query("date:month")
        assert result.date is not None
        assert result.date.op == DateOp.LESS_THAN

    def test_date_invalid(self):
        """Invalid date value results in None date filter."""
        result = parse_query("date:invalid")
        assert result.date is None

    def test_date_combined_with_other_filters(self):
        """Date can be combined with agent and dir filters."""
        result = parse_query("agent:claude date:<1d dir:project api")
        assert result.text == "api"
        assert result.agent == "claude"
        assert result.directory == "project"
        assert result.date is not None
        assert result.date.op == DateOp.LESS_THAN
