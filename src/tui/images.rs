use std::collections::HashMap;
use std::env;
use std::io::Cursor;

use image::{ImageReader, imageops::FilterType};
use ratatui::layout::Size;
use ratatui_image::{
    Resize,
    picker::{Picker, ProtocolType},
    protocol::Protocol,
};

use crate::config::AGENT_ORDER;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ImageProtocol {
    Auto,
    Kitty,
    Sixel,
    Iterm2,
}

#[derive(Default)]
pub(super) struct AgentImages {
    pub(super) row: HashMap<String, Protocol>,
    pub(super) preview: HashMap<String, Protocol>,
}

impl AgentImages {
    pub(super) fn load(protocol: ImageProtocol) -> Option<Self> {
        let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
        let protocol_type = match protocol {
            ImageProtocol::Auto => {
                let queried = picker.protocol_type();
                if queried == ProtocolType::Halfblocks {
                    detect_image_protocol(protocol)?
                } else {
                    queried
                }
            }
            ImageProtocol::Kitty => ProtocolType::Kitty,
            ImageProtocol::Sixel => ProtocolType::Sixel,
            ImageProtocol::Iterm2 => ProtocolType::Iterm2,
        };
        picker.set_protocol_type(protocol_type);

        let row = load_agent_protocols(&picker, Size::new(2, 1));
        let preview = load_agent_protocols(&picker, Size::new(8, 4));
        if preview.is_empty() {
            return None;
        }

        Some(Self { row, preview })
    }
}

fn detect_image_protocol(protocol: ImageProtocol) -> Option<ProtocolType> {
    match protocol {
        ImageProtocol::Kitty => return Some(ProtocolType::Kitty),
        ImageProtocol::Sixel => return Some(ProtocolType::Sixel),
        ImageProtocol::Iterm2 => return Some(ProtocolType::Iterm2),
        ImageProtocol::Auto => {}
    }

    if env_present("KITTY_WINDOW_ID")
        || env_present("GHOSTTY_BIN_DIR")
        || env_eq("TERM_PROGRAM", "ghostty")
    {
        return Some(ProtocolType::Kitty);
    }

    if env_present("ITERM_SESSION_ID")
        || env_contains("TERM_PROGRAM", "iTerm")
        || env_contains("TERM_PROGRAM", "WezTerm")
        || env_present("WEZTERM_EXECUTABLE")
    {
        return Some(ProtocolType::Iterm2);
    }

    if env_contains("TERM", "sixel") {
        return Some(ProtocolType::Sixel);
    }

    None
}

fn env_present(key: &str) -> bool {
    env::var(key).is_ok_and(|value| !value.is_empty())
}

fn env_eq(key: &str, expected: &str) -> bool {
    env::var(key).is_ok_and(|value| value.eq_ignore_ascii_case(expected))
}

fn env_contains(key: &str, needle: &str) -> bool {
    env::var(key).is_ok_and(|value| value.contains(needle))
}

fn load_agent_protocols(picker: &Picker, size: Size) -> HashMap<String, Protocol> {
    let mut protocols = HashMap::new();
    for agent in AGENT_ORDER {
        let Some(bytes) = agent_asset_bytes(agent) else {
            continue;
        };
        let Ok(reader) = ImageReader::new(Cursor::new(bytes)).with_guessed_format() else {
            continue;
        };
        let Ok(image) = reader.decode() else {
            continue;
        };
        if let Ok(protocol) =
            picker.new_protocol(image, size, Resize::Fit(Some(FilterType::Lanczos3)))
        {
            protocols.insert(agent.to_string(), protocol);
        }
    }
    protocols
}

fn agent_asset_bytes(agent: &str) -> Option<&'static [u8]> {
    match agent {
        "claude" => Some(include_bytes!("../../assets/agents/claude.png")),
        "codex" => Some(include_bytes!("../../assets/agents/codex.png")),
        "copilot-cli" => Some(include_bytes!("../../assets/agents/copilot-cli.png")),
        "copilot-vscode" => Some(include_bytes!("../../assets/agents/copilot-vscode.png")),
        "crush" => Some(include_bytes!("../../assets/agents/crush.png")),
        "opencode" => Some(include_bytes!("../../assets/agents/opencode.png")),
        "vibe" => Some(include_bytes!("../../assets/agents/vibe.png")),
        _ => None,
    }
}
