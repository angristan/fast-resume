"""Agent adapters for different coding tools."""

from .base import (
    AgentAdapter,
    ErrorCallback,
    ParseError,
    RawAdapterStats,
    Session,
    SessionCallback,
)
from .claude import ClaudeAdapter
from .codex import CodexAdapter
from .copilot import CopilotAdapter
from .copilot_vscode import CopilotVSCodeAdapter
from .crush import CrushAdapter
from .gemini import GeminiAdapter
from .kiro import KiroAdapter
from .opencode import OpenCodeAdapter
from .vibe import VibeAdapter

__all__ = [
    "AgentAdapter",
    "ErrorCallback",
    "ParseError",
    "RawAdapterStats",
    "Session",
    "SessionCallback",
    "ClaudeAdapter",
    "CodexAdapter",
    "CopilotAdapter",
    "CopilotVSCodeAdapter",
    "CrushAdapter",
    "GeminiAdapter",
    "KiroAdapter",
    "OpenCodeAdapter",
    "VibeAdapter",
]
