"""Tests for logging configuration."""

import logging

from fast_resume.logging_config import setup_logging


def test_setup_logging_silences_textual_image_warning():
    """textual_image's logger is raised to ERROR to suppress the cell-size warning.

    On terminals that cannot report cell size (e.g. VTE terminals), textual_image
    logs a WARNING with a traceback that surfaces on exit. We silence it since the
    library falls back gracefully.
    """
    # Start from a clean slate so we observe setup_logging's effect.
    logging.getLogger("textual_image").setLevel(logging.NOTSET)

    setup_logging()

    assert logging.getLogger("textual_image").level == logging.ERROR
    # A WARNING from a child logger is therefore suppressed.
    assert not logging.getLogger("textual_image._terminal").isEnabledFor(
        logging.WARNING
    )
