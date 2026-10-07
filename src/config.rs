use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use once_cell::sync::Lazy;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const INDEX_SCHEMA_VERSION: u32 = 24;

pub const AGENT_ORDER: [&str; 13] = [
    "antigravity",
    "claude",
    "codex",
    "copilot-cli",
    "crush",
    "cursor",
    "grok",
    "kimi",
    "opencode",
    "pi",
    "pi-go",
    "vibe",
    "copilot-vscode",
];

#[derive(Debug, Clone, Copy)]
pub struct AgentConfig {
    pub name: &'static str,
    pub badge: &'static str,
    pub color: ratatui::style::Color,
    pub light_color: ratatui::style::Color,
}

pub static AGENTS: Lazy<HashMap<&'static str, AgentConfig>> = Lazy::new(|| {
    HashMap::from([
        (
            "antigravity",
            AgentConfig {
                name: "antigravity",
                badge: "agy",
                color: ratatui::style::Color::Rgb(66, 133, 244),
                light_color: ratatui::style::Color::Rgb(45, 100, 200),
            },
        ),
        (
            "claude",
            AgentConfig {
                name: "claude",
                badge: "claude",
                color: ratatui::style::Color::Rgb(232, 123, 53),
                light_color: ratatui::style::Color::Rgb(180, 80, 20),
            },
        ),
        (
            "codex",
            AgentConfig {
                name: "codex",
                badge: "codex",
                color: ratatui::style::Color::Rgb(0, 166, 126),
                light_color: ratatui::style::Color::Rgb(0, 115, 85),
            },
        ),
        (
            "cursor",
            AgentConfig {
                name: "cursor",
                badge: "cursor",
                color: ratatui::style::Color::Rgb(255, 255, 255),
                light_color: ratatui::style::Color::Rgb(30, 30, 30),
            },
        ),
        (
            "grok",
            AgentConfig {
                name: "grok",
                badge: "grok",
                color: ratatui::style::Color::Rgb(255, 255, 255),
                light_color: ratatui::style::Color::Rgb(30, 30, 30),
            },
        ),
        (
            "kimi",
            AgentConfig {
                name: "kimi",
                badge: "kimi",
                color: ratatui::style::Color::Rgb(56, 189, 248),
                light_color: ratatui::style::Color::Rgb(0, 105, 165),
            },
        ),
        (
            "opencode",
            AgentConfig {
                name: "opencode",
                badge: "opencode",
                color: ratatui::style::Color::Rgb(207, 206, 205),
                light_color: ratatui::style::Color::Rgb(65, 65, 65),
            },
        ),
        (
            "pi",
            AgentConfig {
                name: "pi",
                badge: "pi",
                color: ratatui::style::Color::Rgb(151, 118, 255),
                light_color: ratatui::style::Color::Rgb(100, 70, 195),
            },
        ),
        (
            "pi-go",
            AgentConfig {
                name: "pi-go",
                badge: "pi-go",
                color: ratatui::style::Color::Rgb(255, 140, 0),
            },
        ),
        (
            "vibe",
            AgentConfig {
                name: "vibe",
                badge: "vibe",
                color: ratatui::style::Color::Rgb(255, 107, 53),
                light_color: ratatui::style::Color::Rgb(190, 65, 25),
            },
        ),
        (
            "crush",
            AgentConfig {
                name: "crush",
                badge: "crush",
                color: ratatui::style::Color::Rgb(107, 81, 255),
                light_color: ratatui::style::Color::Rgb(85, 60, 190),
            },
        ),
        (
            "copilot-cli",
            AgentConfig {
                name: "copilot-cli",
                badge: "copilot",
                color: ratatui::style::Color::Rgb(156, 163, 175),
                light_color: ratatui::style::Color::Rgb(75, 80, 90),
            },
        ),
        (
            "copilot-vscode",
            AgentConfig {
                name: "copilot-vscode",
                badge: "vscode",
                color: ratatui::style::Color::Rgb(0, 122, 204),
                light_color: ratatui::style::Color::Rgb(0, 90, 160),
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
    cache_dir_from(
        env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
        &home_dir(),
    )
}

fn cache_dir_from(xdg_cache_home: Option<PathBuf>, home: &Path) -> PathBuf {
    xdg_cache_home
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".cache"))
        .join("fast-resume")
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

pub fn antigravity_dir() -> PathBuf {
    home_dir().join(".gemini").join("antigravity-cli")
}

pub fn cursor_chats_dir() -> PathBuf {
    home_dir().join(".cursor").join("chats")
}

pub fn grok_sessions_dir() -> PathBuf {
    env_path("GROK_HOME")
        .unwrap_or_else(|| home_dir().join(".grok"))
        .join("sessions")
}

pub fn kimi_sessions_dir() -> PathBuf {
    env_path("KIMI_CODE_HOME")
        .unwrap_or_else(|| home_dir().join(".kimi-code"))
        .join("sessions")
}

pub fn opencode_dir() -> PathBuf {
    home_dir().join(".local").join("share").join("opencode")
}

pub fn pi_sessions_dir() -> PathBuf {
    if let Some(path) = env_path("PI_CODING_AGENT_SESSION_DIR") {
        return path;
    }

    let agent_dir = pi_agent_dir();
    let settings_path = agent_dir.join("settings.json");
    if let Ok(settings_bytes) = fs::read(settings_path)
        && let Ok(settings) = serde_json::from_slice::<Value>(&settings_bytes)
        && let Some(session_dir) = settings
            .get("sessionDir")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
    {
        return expand_tilde(session_dir);
    }

    agent_dir.join("sessions")
}

fn pi_agent_dir() -> PathBuf {
    env_path("PI_CODING_AGENT_DIR").unwrap_or_else(|| home_dir().join(".pi").join("agent"))
}

pub fn pi_go_sessions_dir() -> PathBuf {
    home_dir().join(".pi-go").join("sessions")
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| expand_tilde(&value))
}

fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    PathBuf::from(path)
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

    #[test]
    fn expands_pi_tilde_paths() {
        assert_eq!(expand_tilde("~"), home_dir());
        assert_eq!(
            expand_tilde("~/pi-sessions"),
            home_dir().join("pi-sessions")
        );
    }
}
