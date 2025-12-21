"""Agent adapters for different coding tools."""

from .base import AgentAdapter, Session
from .claude import ClaudeAdapter
from .codex import CodexAdapter
from .copilot import CopilotAdapter
from .copilot_vscode import CopilotVSCodeAdapter
from .crush import CrushAdapter
from .opencode import OpenCodeAdapter
from .vibe import VibeAdapter

__all__ = [
    "AgentAdapter",
    "Session",
    "ClaudeAdapter",
    "CodexAdapter",
    "CopilotAdapter",
    "CopilotVSCodeAdapter",
    "CrushAdapter",
    "OpenCodeAdapter",
    "VibeAdapter",
]
