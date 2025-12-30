"""CLI entry point for fast-resume."""

import os

import click
import humanize
from rich.console import Console
from rich.table import Table

from .config import AGENTS, INDEX_DIR
from .index import TantivyIndex
from .logging_config import setup_logging
from .search import SessionSearch
from .tui import run_tui


@click.command()
@click.argument("query", required=False, default="")
@click.option(
    "-a",
    "--agent",
    type=click.Choice(
        [
            "claude",
            "codex",
            "copilot-cli",
            "copilot-vscode",
            "crush",
            "opencode",
            "vibe",
        ]
    ),
    help="Filter by agent",
)
@click.option("-d", "--directory", help="Filter by directory (substring match)")
@click.option("--no-tui", is_flag=True, help="Output list to stdout instead of TUI")
@click.option(
    "--list", "list_only", is_flag=True, help="Just list sessions, don't resume"
)
@click.option("--rebuild", is_flag=True, help="Force rebuild the session index")
@click.option("--stats", is_flag=True, help="Show index statistics")
@click.option(
    "--yolo",
    is_flag=True,
    help="Resume sessions with auto-approve/skip-permissions flags",
)
@click.option(
    "--no-version-check",
    is_flag=True,
    help="Disable checking for new versions",
)
@click.version_option()
def main(
    query: str,
    agent: str | None,
    directory: str | None,
    no_tui: bool,
    list_only: bool,
    rebuild: bool,
    stats: bool,
    yolo: bool,
    no_version_check: bool,
) -> None:
    """Fast fuzzy finder for coding agent session history.

    Search across Claude Code, Codex CLI, Copilot CLI, Crush, OpenCode, and Vibe sessions.
    Select a session to resume it with the appropriate agent.

    Supports keyword search syntax:

        agent:NAME       Filter by agent (e.g., agent:claude)

        agent:A,B        Multiple values with OR (e.g., agent:claude,codex)

        -agent:NAME      Exclude agent (or agent:!NAME)

        dir:PATH         Filter by directory substring

        date:VALUE       Filter by date/time (today, <1h, >1d, etc.)

    Examples:

        fr agent:claude,codex api       # Claude OR Codex sessions

        fr -agent:vibe                  # Exclude Vibe sessions

        fr date:<1d -agent:claude       # Last 24h, not Claude

        fr dir:project date:today       # Today's sessions in project
    """
    # Initialize logging for parse errors
    setup_logging()

    if stats:
        _show_stats()
        return

    if rebuild:
        # Force rebuild index
        search = SessionSearch()
        search.get_all_sessions(force_refresh=True)
        click.echo("Index rebuilt.")
        if not (no_tui or list_only or query):
            return

    if no_tui or list_only:
        _list_sessions(query, agent, directory)
    else:
        resume_cmd, resume_dir = run_tui(
            query=query,
            agent_filter=agent,
            yolo=yolo,
            no_version_check=no_version_check,
        )
        if resume_cmd:
            # Change to session directory before running command
            if resume_dir:
                os.chdir(resume_dir)
            # Execute the resume command
            os.execvp(resume_cmd[0], resume_cmd)


def _show_stats() -> None:
    """Display index statistics."""
    console = Console()
    index = TantivyIndex()
    stats = index.get_stats()

    if stats.total_sessions == 0:
        console.print(
            "[dim]No sessions indexed yet. Run [bold]fr[/bold] to index sessions.[/dim]"
        )
        return

    # Header
    console.print("\n[bold]Index Statistics[/bold]\n")

    # Overview table
    overview = Table(show_header=False, box=None, padding=(0, 2))
    overview.add_column("Label", style="dim")
    overview.add_column("Value")

    overview.add_row("Total sessions", f"[bold]{stats.total_sessions}[/bold]")
    overview.add_row("Total messages", f"{stats.total_messages:,}")
    overview.add_row("Avg messages/session", f"{stats.avg_messages_per_session:.1f}")
    overview.add_row("Index size", humanize.naturalsize(stats.index_size_bytes))
    overview.add_row("Index location", str(INDEX_DIR))

    if stats.oldest_session and stats.newest_session:
        date_range = (
            f"{stats.oldest_session:%Y-%m-%d} to {stats.newest_session:%Y-%m-%d}"
        )
        overview.add_row("Date range", date_range)

    console.print(overview)

    # Data by agent (raw + indexed)
    console.print("\n[bold]Data by Agent[/bold]\n")
    search = SessionSearch()
    agent_table = Table(show_header=True, header_style="bold")
    agent_table.add_column("Agent", no_wrap=True)
    agent_table.add_column("Files", justify="right")
    agent_table.add_column("Disk", justify="right")
    agent_table.add_column("Sessions", justify="right")
    agent_table.add_column("Messages", justify="right")
    agent_table.add_column("Content", justify="right")
    agent_table.add_column("Data Directory")

    messages_by_agent = stats.messages_by_agent or {}
    content_chars_by_agent = stats.content_chars_by_agent or {}

    # Collect data for all agents and sort by indexed session count
    agent_data = []
    for adapter in search.adapters:
        raw_stats = adapter.get_raw_stats()
        sessions = stats.sessions_by_agent.get(adapter.name, 0)
        agent_data.append((adapter.name, raw_stats, sessions))

    # Sort by session count descending
    agent_data.sort(key=lambda x: -x[2])

    for agent_name, raw_stats, sessions in agent_data:
        agent_config = AGENTS.get(agent_name, {"color": "white"})
        color = agent_config["color"]
        messages = messages_by_agent.get(agent_name, 0)
        content_size = content_chars_by_agent.get(agent_name, 0)

        if raw_stats.available:
            # Shorten home directory in path
            data_dir = raw_stats.data_dir
            home = os.path.expanduser("~")
            if data_dir.startswith(home):
                data_dir = "~" + data_dir[len(home) :]

            agent_table.add_row(
                f"[{color}]{agent_name}[/{color}]",
                str(raw_stats.file_count),
                humanize.naturalsize(raw_stats.total_bytes),
                str(sessions) if sessions > 0 else "[dim]0[/dim]",
                f"{messages:,}" if messages > 0 else "[dim]0[/dim]",
                humanize.naturalsize(content_size)
                if content_size > 0
                else "[dim]-[/dim]",
                f"[dim]{data_dir}[/dim]",
            )
        else:
            agent_table.add_row(
                f"[{color}]{agent_name}[/{color}]",
                "[dim]-[/dim]",
                "[dim]-[/dim]",
                "[dim]-[/dim]",
                "[dim]-[/dim]",
                "[dim]-[/dim]",
                "[dim]not found[/dim]",
            )

    console.print(agent_table)

    # Activity by day of week
    if stats.sessions_by_weekday:
        console.print("\n[bold]Activity by Day[/bold]\n")
        day_table = Table(show_header=False, box=None, padding=(0, 1))
        day_table.add_column("Day", style="dim", width=4)
        day_table.add_column("Bar")
        day_table.add_column("Count", justify="right", width=4)

        max_day = (
            max(stats.sessions_by_weekday.values()) if stats.sessions_by_weekday else 1
        )
        for day in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]:
            count = stats.sessions_by_weekday.get(day, 0)
            bar_width = int((count / max_day) * 20) if max_day else 0
            bar = "[green]" + "█" * bar_width + "[/green]"
            day_table.add_row(day, bar, str(count))

        console.print(day_table)

    # Activity by hour
    if stats.sessions_by_hour:
        console.print("\n[bold]Activity by Hour[/bold]\n")
        # Show as a compact sparkline-style display
        max_hour = max(stats.sessions_by_hour.values()) if stats.sessions_by_hour else 1
        blocks = " ▁▂▃▄▅▆▇█"

        hour_line = ""
        for h in range(24):
            count = stats.sessions_by_hour.get(h, 0)
            idx = int((count / max_hour) * 8) if max_hour else 0
            hour_line += blocks[idx]

        console.print(f"  [dim]0h[/dim] [yellow]{hour_line}[/yellow] [dim]23h[/dim]")

        # Find peak hours
        sorted_hours = sorted(stats.sessions_by_hour.items(), key=lambda x: -x[1])
        if sorted_hours:
            top_hours = sorted_hours[:3]
            peak_str = ", ".join(f"{h}:00 ({c})" for h, c in top_hours)
            console.print(f"  [dim]Peak hours: {peak_str}[/dim]")

    # Top directories
    if stats.top_directories:
        console.print("\n[bold]Top Directories[/bold]\n")
        dir_table = Table(show_header=True, header_style="bold")
        dir_table.add_column("Directory")
        dir_table.add_column("Sessions", justify="right")
        dir_table.add_column("Messages", justify="right")

        home = os.path.expanduser("~")
        for directory, sessions, messages in stats.top_directories[:10]:
            display_dir = directory
            if display_dir.startswith(home):
                display_dir = "~" + display_dir[len(home) :]
            dir_table.add_row(display_dir, str(sessions), f"{messages:,}")

        console.print(dir_table)

    console.print()


def _list_sessions(query: str, agent: str | None, directory: str | None) -> None:
    """List sessions in terminal without TUI."""
    console = Console()
    search = SessionSearch()

    sessions = search.search(query, agent_filter=agent, directory_filter=directory)

    if not sessions:
        console.print("[dim]No sessions found.[/dim]")
        return

    table = Table(show_header=True, header_style="bold")
    table.add_column("Agent", style="bold")
    table.add_column("Title")
    table.add_column("Directory", style="dim")
    table.add_column("ID", style="dim")

    for session in sessions[:50]:  # Limit output
        agent_config = AGENTS.get(session.agent, {"color": "white"})
        agent_style = agent_config["color"]

        # Truncate fields
        title = session.title[:50] + "..." if len(session.title) > 50 else session.title
        directory_display = session.directory
        home = os.path.expanduser("~")
        if directory_display.startswith(home):
            directory_display = "~" + directory_display[len(home) :]
        if len(directory_display) > 35:
            directory_display = "..." + directory_display[-32:]

        table.add_row(
            f"[{agent_style}]{session.agent}[/{agent_style}]",
            title,
            directory_display,
            session.id[:20] + "..." if len(session.id) > 20 else session.id,
        )

    console.print(table)
    console.print(
        f"\n[dim]Showing {min(len(sessions), 50)} of {len(sessions)} sessions[/dim]"
    )


if __name__ == "__main__":
    main()
