"""Tests for update notification helpers."""

from pathlib import Path

from fast_resume.update import detect_install_source, update_command, update_instruction


def test_detects_homebrew_from_cellar_path():
    paths = [Path("/opt/homebrew/Cellar/fast-resume/1.17.3/bin/fr")]

    assert detect_install_source(paths) == "homebrew"
    assert update_command(paths) == "brew upgrade fast-resume"


def test_detects_homebrew_from_opt_path():
    paths = [Path("/opt/homebrew/opt/fast-resume/bin/fr")]

    assert detect_install_source(paths) == "homebrew"


def test_detects_uv_tool_install_path():
    paths = [Path("/Users/test/.local/share/uv/tools/fast-resume/bin/fr")]

    assert detect_install_source(paths) == "uv"
    assert update_command(paths) == "uv tool upgrade fast-resume"


def test_detects_pipx_install_path():
    paths = [Path("/Users/test/.local/pipx/venvs/fast-resume/bin/fr")]

    assert detect_install_source(paths) == "pipx"
    assert update_command(paths) == "pipx upgrade fast-resume"


def test_homebrew_wins_when_invoked_directly_with_uv_first_on_path():
    paths = [
        Path("/Users/test/.local/share/uv/tools/fast-resume/bin/fr"),
        Path("/opt/homebrew/Cellar/fast-resume/1.17.3/bin/fr"),
    ]

    assert detect_install_source(paths) == "homebrew"


def test_unknown_install_source_uses_neutral_instruction():
    paths = [Path("/tmp/fast-resume/fr")]

    assert update_command(paths) is None
    assert (
        update_instruction(paths)
        == "Update with the package manager used to install fast-resume."
    )


def test_update_instruction_can_use_rich_markup():
    paths = [Path("/opt/homebrew/Cellar/fast-resume/1.17.3/bin/fr")]

    assert update_instruction(paths, markup=True) == (
        "Run [bold]brew upgrade fast-resume[/bold] to update"
    )
