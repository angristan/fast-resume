"""fast-resume: Fuzzy finder for coding agent session history."""

import logging
from importlib.metadata import version

# textual_image probes the terminal for cell size at import time and logs a
# WARNING + traceback when the terminal can't answer (e.g. VTE terminals such as
# Terminator or GNOME Terminal). This must run before any submodule imports
# textual_image, so it lives here rather than in setup_logging(). The library
# falls back to default sizes, so the message is not actionable.
logging.getLogger("textual_image").setLevel(logging.ERROR)

__version__ = version("fast-resume")
