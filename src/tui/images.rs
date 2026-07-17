use std::collections::HashMap;
use std::env;
use std::io::Cursor;

use image::{ImageReader, imageops::FilterType};
use ratatui::layout::Size;
use ratatui_image::{
    FontSize, Resize,
    picker::{Capability, Picker, ProtocolType},
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
        let mut picker = terminal_picker();
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

fn terminal_picker() -> Picker {
    let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    let has_reported_cell_size = picker
        .capabilities()
        .iter()
        .any(|capability| matches!(capability, Capability::CellSize(Some(_))));
    if has_reported_cell_size {
        return picker;
    }

    let Ok(window) = crossterm::terminal::window_size() else {
        return picker;
    };
    let Some(font_size) =
        cell_size_from_window(window.columns, window.rows, window.width, window.height)
    else {
        return picker;
    };

    let protocol_type = picker.protocol_type();
    #[allow(deprecated)]
    let mut picker = Picker::from_fontsize(font_size);
    picker.set_protocol_type(protocol_type);
    picker
}

fn cell_size_from_window(
    columns: u16,
    rows: u16,
    pixel_width: u16,
    pixel_height: u16,
) -> Option<FontSize> {
    if columns == 0 || rows == 0 || pixel_width == 0 || pixel_height == 0 {
        return None;
    }

    let cell_width = (u32::from(pixel_width) + u32::from(columns) / 2) / u32::from(columns);
    let cell_height = (u32::from(pixel_height) + u32::from(rows) / 2) / u32::from(rows);
    if cell_width == 0 || cell_height == 0 {
        return None;
    }

    Some(FontSize::new(cell_width as u16, cell_height as u16))
}

fn detect_image_protocol(protocol: ImageProtocol) -> Option<ProtocolType> {
    match protocol {
        ImageProtocol::Kitty => return Some(ProtocolType::Kitty),
        ImageProtocol::Sixel => return Some(ProtocolType::Sixel),
        ImageProtocol::Iterm2 => return Some(ProtocolType::Iterm2),
        ImageProtocol::Auto => {}
    }

    if env_present("KITTY_WINDOW_ID") || is_ghostty() {
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

fn is_ghostty() -> bool {
    env_present("GHOSTTY_BIN_DIR") || env_eq("TERM_PROGRAM", "ghostty")
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
        let image = compensate_logo_aspect(image);
        if let Ok(protocol) =
            picker.new_protocol(image, size, Resize::Fit(Some(FilterType::Lanczos3)))
        {
            protocols.insert(agent.to_string(), protocol);
        }
    }
    protocols
}

fn compensate_logo_aspect(image: image::DynamicImage) -> image::DynamicImage {
    let width = image.width();
    let height = image.height();
    let stretched_height = height.saturating_mul(11) / 10;
    if stretched_height <= height {
        return image;
    }

    let offset = (stretched_height - height) / 2;
    image
        .resize_exact(width, stretched_height, FilterType::Lanczos3)
        .crop_imm(0, offset, width, height)
}

fn agent_asset_bytes(agent: &str) -> Option<&'static [u8]> {
    match agent {
        "antigravity" => Some(include_bytes!("../../assets/agents/antigravity.png")),
        "claude" => Some(include_bytes!("../../assets/agents/claude.png")),
        "codex" => Some(include_bytes!("../../assets/agents/codex.png")),
        "copilot-cli" => Some(include_bytes!("../../assets/agents/copilot-cli.png")),
        "copilot-vscode" => Some(include_bytes!("../../assets/agents/copilot-vscode.png")),
        "crush" => Some(include_bytes!("../../assets/agents/crush.png")),
        "cursor" => Some(include_bytes!("../../assets/agents/cursor.png")),
        "gemini" => Some(include_bytes!("../../assets/agents/gemini.png")),
        "grok" => Some(include_bytes!("../../assets/agents/grok.png")),
        "opencode" => Some(include_bytes!("../../assets/agents/opencode.png")),
        "pi" => Some(include_bytes!("../../assets/agents/pi.png")),
        "vibe" => Some(include_bytes!("../../assets/agents/vibe.png")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible_size(image: &image::DynamicImage) -> (u32, u32) {
        let rgba = image.to_rgba8();
        let mut min_x = u32::MAX;
        let mut min_y = u32::MAX;
        let mut max_x = 0;
        let mut max_y = 0;
        for (x, y, pixel) in rgba.enumerate_pixels() {
            if pixel[3] <= 16 {
                continue;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        (max_x - min_x + 1, max_y - min_y + 1)
    }

    #[test]
    fn derives_rounded_cell_size_from_window_pixels() {
        let size = cell_size_from_window(120, 48, 1_080, 816).expect("cell size");

        assert_eq!(size.width, 9);
        assert_eq!(size.height, 17);
    }

    #[test]
    fn rejects_missing_window_pixel_metrics() {
        assert!(cell_size_from_window(120, 48, 0, 0).is_none());
        assert!(cell_size_from_window(0, 0, 1_080, 816).is_none());
    }

    #[test]
    fn logos_receive_the_same_vertical_compensation() {
        for agent in AGENT_ORDER {
            let image =
                ImageReader::new(Cursor::new(agent_asset_bytes(agent).expect("agent asset")))
                    .with_guessed_format()
                    .expect("image format")
                    .decode()
                    .expect("image");
            let before = visible_size(&image);
            let corrected = compensate_logo_aspect(image);
            let after = visible_size(&corrected);

            assert_eq!(corrected.width(), 64, "{agent}");
            assert_eq!(corrected.height(), 64, "{agent}");
            assert!(after.1 > before.1, "{agent}");
        }
    }
}
