"""Logging configuration for fast-resume."""

import logging
from pathlib import Path

from .config import CACHE_DIR, LOG_FILE

# Module logger for parse errors
parse_logger = logging.getLogger("fast_resume.parse_errors")


def setup_logging() -> None:
    """Set up logging with file handler for parse errors.

    Logs are written to ~/.cache/fast-resume/parse-errors.log
    """
    # Ensure cache directory exists
    CACHE_DIR.mkdir(parents=True, exist_ok=True)

    # Configure parse error logger
    parse_logger.setLevel(logging.WARNING)

    # Avoid duplicate handlers if called multiple times
    if not parse_logger.handlers:
        # File handler - append mode, rotates on size
        handler = logging.FileHandler(LOG_FILE, mode="a", encoding="utf-8")
        handler.setLevel(logging.WARNING)

        # Format: timestamp - level - message
        formatter = logging.Formatter(
            "%(asctime)s - %(levelname)s - %(message)s",
            datefmt="%Y-%m-%d %H:%M:%S",
        )
        handler.setFormatter(formatter)
        parse_logger.addHandler(handler)

        # Don't propagate to root logger (avoid console output)
        parse_logger.propagate = False


def log_parse_error(
    agent: str, file_path: str | Path, error_type: str, message: str
) -> None:
    """Log a parse error to the log file.

    Args:
        agent: Which adapter encountered the error (e.g., "claude", "codex")
        file_path: Path to the problematic file
        error_type: Exception type name (e.g., "JSONDecodeError")
        message: Human-readable error message
    """
    parse_logger.warning("[%s] %s in %s: %s", agent, error_type, file_path, message)


def get_log_file_path() -> Path:
    """Return the path to the log file."""
    return LOG_FILE
