"""Shared fixtures for tests."""

import warnings

import pytest
from pathlib import Path
import tempfile
import shutil

# Suppress PIL warning about palette images with transparency (from agent icons)
warnings.filterwarnings("ignore", message="Palette images with Transparency")


@pytest.fixture
def temp_dir():
    """Create a temporary directory for test data."""
    dirpath = tempfile.mkdtemp()
    yield Path(dirpath)
    # Use ignore_errors=True because TantivyIndex may still be flushing
    # data to disk in background threads when teardown runs
    shutil.rmtree(dirpath, ignore_errors=True)
