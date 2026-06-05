"""Tests for early logging configuration."""

import importlib
import logging


def test_textual_image_logger_silenced_on_package_import():
    """Importing the package raises the textual_image logger to ERROR.

    textual_image probes the terminal for cell size at import time (in
    textual_image.widget) and logs a WARNING + traceback when the terminal can't
    answer (e.g. VTE terminals like Terminator). The package must silence it
    before any submodule imports textual_image, so the fix lives in
    fast_resume/__init__.py rather than in setup_logging() (which runs too late).
    """
    import fast_resume

    logging.getLogger("textual_image").setLevel(logging.NOTSET)
    importlib.reload(fast_resume)

    terminal_logger = logging.getLogger("textual_image._terminal")
    assert not terminal_logger.isEnabledFor(logging.WARNING)
