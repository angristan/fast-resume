use std::collections::HashMap;
use std::path::PathBuf;

use once_cell::sync::Lazy;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const INDEX_SCHEMA_VERSION: u32 = 23;

pub const AGENT_ORDER: [&str; 7] = [
    "claude",
    "codex",
    "copilot-cli",
    "crush",
    "opencode",
    "vibe",
    "copilot-vscode",
];

#[derive(Debug, Clone, Copy)]
pub struct AgentConfig {
    pub name: &'static str,
    pub badge: &'static str,
    pub color: ratatui::style::Color,
}

pub static AGENTS: Lazy<HashMap<&'static str, AgentConfig>> = Lazy::new(|| {
    HashMap::from([
        (
            "claude",
            AgentConfig {
                name: "claude",
                badge: "claude",
                color: ratatui::style::Color::Rgb(232, 123, 53),
            },
        ),
        (
            "codex",
            AgentConfig {
                name: "codex",
                badge: "codex",
                color: ratatui::style::Color::Rgb(0, 166, 126),
            },
        ),
        (
            "opencode",
            AgentConfig {
                name: "opencode",
                badge: "opencode",
                color: ratatui::style::Color::Rgb(207, 206, 205),
            },
        ),
        (
            "vibe",
            AgentConfig {
                name: "vibe",
                badge: "vibe",
                color: ratatui::style::Color::Rgb(255, 107, 53),
            },
        ),
        (
            "crush",
            AgentConfig {
                name: "crush",
                badge: "crush",
                color: ratatui::style::Color::Rgb(107, 81, 255),
            },
        ),
        (
            "copilot-cli",
            AgentConfig {
                name: "copilot-cli",
                badge: "copilot",
                color: ratatui::style::Color::Rgb(156, 163, 175),
            },
        ),
        (
            "copilot-vscode",
            AgentConfig {
                name: "copilot-vscode",
                badge: "vscode",
                color: ratatui::style::Color::Rgb(0, 122, 204),
            },
        ),
    ])
});

pub fn is_agent(value: &str) -> bool {
    AGENT_ORDER.contains(&value)
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn cache_dir() -> PathBuf {
    home_dir().join(".cache").join("fast-resume")
}

pub fn index_dir() -> PathBuf {
    cache_dir().join("tantivy_index")
}

pub fn claude_dir() -> PathBuf {
    home_dir().join(".claude").join("projects")
}

pub fn codex_dir() -> PathBuf {
    home_dir().join(".codex").join("sessions")
}

pub fn codex_session_index_file() -> PathBuf {
    home_dir().join(".codex").join("session_index.jsonl")
}

pub fn opencode_dir() -> PathBuf {
    home_dir().join(".local").join("share").join("opencode")
}

pub fn opencode_db() -> PathBuf {
    opencode_dir().join("opencode.db")
}

pub fn opencode_legacy_dir() -> PathBuf {
    opencode_dir().join("storage")
}

pub fn vibe_dir() -> PathBuf {
    home_dir().join(".vibe").join("logs").join("session")
}

pub fn crush_projects_file() -> PathBuf {
    home_dir()
        .join(".local")
        .join("share")
        .join("crush")
        .join("projects.json")
}

pub fn copilot_dir() -> PathBuf {
    home_dir().join(".copilot").join("session-state")
}

pub fn vscode_storage_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        home_dir()
            .join("Library")
            .join("Application Support")
            .join("Code")
    } else if cfg!(target_os = "windows") {
        home_dir().join("AppData").join("Roaming").join("Code")
    } else {
        home_dir().join(".config").join("Code")
    }
}

pub fn vscode_empty_window_chat_dir() -> PathBuf {
    vscode_storage_dir()
        .join("User")
        .join("globalStorage")
        .join("emptyWindowChatSessions")
}

pub fn vscode_workspace_storage_dir() -> PathBuf {
    vscode_storage_dir().join("User").join("workspaceStorage")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_order_is_alphabetical_by_visible_badge() {
        let badges: Vec<_> = AGENT_ORDER
            .iter()
            .map(|agent| AGENTS.get(agent).expect("known agent").badge)
            .collect();
        let mut sorted = badges.clone();
        sorted.sort_unstable();

        assert_eq!(badges, sorted);
    }
}
