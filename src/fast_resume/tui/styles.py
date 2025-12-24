"""CSS styles for the TUI components."""

# CSS for the YoloModeModal
YOLO_MODAL_CSS = """
YoloModeModal {
    align: center middle;
    background: rgba(0, 0, 0, 0.6);
}

YoloModeModal > Vertical {
    width: 36;
    height: auto;
    background: $surface;
    border: thick $primary 80%;
    padding: 1 2;
}

YoloModeModal #title {
    text-align: center;
    text-style: bold;
    width: 100%;
}

YoloModeModal #buttons {
    width: 100%;
    height: auto;
    align: center middle;
    margin-top: 1;
}

YoloModeModal Button {
    margin: 0 1;
    min-width: 10;
}

YoloModeModal Button:focus {
    background: $warning;
}
"""

# CSS for the main FastResumeApp
APP_CSS = """
Screen {
    layout: vertical;
    width: 100%;
    background: $surface;
}

/* Title bar - branding + session count */
#title-bar {
    height: 1;
    width: 100%;
    padding: 0 1;
    background: $surface;
}

#app-title {
    width: 1fr;
    color: $text;
    text-style: bold;
}

#session-count {
    dock: right;
    color: $text-muted;
    width: auto;
}

/* Search row */
#search-row {
    height: 3;
    width: 100%;
    padding: 0 1;
}

#search-box {
    width: 100%;
    height: 3;
    border: solid $primary-background-lighten-2;
    background: $surface;
    padding: 0 1;
}

#search-box:focus-within {
    border: solid $accent;
}

#search-icon {
    width: 3;
    color: $text-muted;
    content-align: center middle;
}

#search-input {
    width: 1fr;
    border: none;
    background: transparent;
}

#search-input:focus {
    border: none;
}

/* Agent filter tabs - pill style */
#filter-container {
    height: 1;
    width: 100%;
    padding: 0 1;
    margin-bottom: 1;
}

.filter-btn {
    width: auto;
    height: 1;
    margin: 0 1 0 0;
    padding: 0 1;
    border: none;
    background: transparent;
    color: $text-muted;
}

.filter-btn:hover {
    color: $text;
}

.filter-btn:focus {
    text-style: none;
}

.filter-btn.-active {
    background: $accent 20%;
    color: $accent;
}

.filter-icon {
    width: 2;
    height: 1;
    margin-right: 1;
}

.filter-label {
    height: 1;
}

.filter-btn.-active .filter-label {
    text-style: bold;
}

/* Agent-specific filter colors */
#filter-claude {
    color: #E87B35;
}
#filter-claude.-active {
    background: #E87B35 20%;
    color: #E87B35;
}

#filter-codex {
    color: #00A67E;
}
#filter-codex.-active {
    background: #00A67E 20%;
    color: #00A67E;
}

#filter-copilot-cli {
    color: #9CA3AF;
}
#filter-copilot-cli.-active {
    background: #9CA3AF 20%;
    color: #9CA3AF;
}

#filter-copilot-vscode {
    color: #007ACC;
}
#filter-copilot-vscode.-active {
    background: #007ACC 20%;
    color: #007ACC;
}

#filter-crush {
    color: #FF5F87;
}
#filter-crush.-active {
    background: #FF5F87 20%;
    color: #FF5F87;
}

#filter-opencode {
    color: #6366F1;
}
#filter-opencode.-active {
    background: #6366F1 20%;
    color: #6366F1;
}

#filter-vibe {
    color: #FF6B35;
}
#filter-vibe.-active {
    background: #FF6B35 20%;
    color: #FF6B35;
}

/* Main content area */
#main-container {
    height: 1fr;
    width: 100%;
}

#results-container {
    height: 1fr;
    width: 100%;
    overflow-x: hidden;
}

#results-table {
    height: 100%;
    width: 100%;
    overflow-x: hidden;
}

DataTable {
    background: transparent;
    overflow-x: hidden;
}

DataTable > .datatable--header {
    text-style: bold;
    color: $text;
}

DataTable > .datatable--cursor {
    background: $accent 30%;
}

DataTable > .datatable--hover {
    background: $surface-lighten-1;
}

/* Preview pane - expanded */
#preview-container {
    height: 12;
    border-top: solid $accent 50%;
    background: $surface;
    padding: 0 1;
}

#preview-container.hidden {
    display: none;
}

#preview {
    height: 100%;
    overflow-y: auto;
}

/* Agent colors */
.agent-claude {
    color: #E87B35;
}

.agent-codex {
    color: #00A67E;
}

.agent-copilot {
    color: #9CA3AF;
}

.agent-opencode {
    color: #6366F1;
}

.agent-vibe {
    color: #FF6B35;
}

.agent-crush {
    color: #FF5F87;
}

/* Footer styling */
Footer {
    background: $primary-background;
}

Footer > .footer--key {
    background: $surface;
    color: $text;
}

Footer > .footer--description {
    color: $text-muted;
}

#query-time {
    width: auto;
    padding: 0 1;
    color: $text-muted;
}
"""
