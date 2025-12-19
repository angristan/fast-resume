"""CLI entry point for fast-resume."""

import os
import sys

import click
from rich.console import Console
from rich.table import Table

from .config import AGENTS
from .search import SessionSearch
from .tui import run_tui


@click.command()
@click.argument("query", required=False, default="")
@click.option(
    "-a",
    "--agent",
    type=click.Choice(["claude", "codex", "opencode", "vibe"]),
    help="Filter by agent",
)
@click.option("-d", "--directory", help="Filter by directory (substring match)")
@click.option("--no-tui", is_flag=True, help="Output list to stdout instead of TUI")
@click.option("--list", "list_only", is_flag=True, help="Just list sessions, don't resume")
@click.option("--rebuild", is_flag=True, help="Force rebuild the session cache")
@click.version_option()
def main(
    query: str,
    agent: str | None,
    directory: str | None,
    no_tui: bool,
    list_only: bool,
    rebuild: bool,
) -> None:
    """Fast fuzzy finder for coding agent session history.

    Search across Claude Code, Codex CLI, OpenCode, and Vibe sessions.
    Select a session to resume it with the appropriate agent.

    Examples:

        fr                    # Open TUI with all sessions

        fr auth middleware    # Search for "auth middleware"

        fr -a claude          # Only show Claude Code sessions

        fr --no-tui           # List sessions in terminal
    """
    if rebuild:
        # Force rebuild cache
        search = SessionSearch()
        search.get_all_sessions(force_refresh=True)
        click.echo("Cache rebuilt.")
        if not (no_tui or list_only or query):
            return

    if no_tui or list_only:
        _list_sessions(query, agent, directory)
    else:
        resume_cmd, resume_dir = run_tui(query=query, agent_filter=agent)
        if resume_cmd:
            # Change to session directory before running command
            if resume_dir:
                os.chdir(resume_dir)
            # Execute the resume command
            os.execvp(resume_cmd[0], resume_cmd)


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
            directory_display = "~" + directory_display[len(home):]
        if len(directory_display) > 35:
            directory_display = "..." + directory_display[-32:]

        table.add_row(
            f"[{agent_style}]{session.agent}[/{agent_style}]",
            title,
            directory_display,
            session.id[:20] + "..." if len(session.id) > 20 else session.id,
        )

    console.print(table)
    console.print(f"\n[dim]Showing {min(len(sessions), 50)} of {len(sessions)} sessions[/dim]")


if __name__ == "__main__":
    main()
