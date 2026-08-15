use ratatui::style::Color;

use crate::config::AgentConfig;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ThemeMode {
    Auto,
    Dark,
    Light,
}

impl ThemeMode {
    pub(super) fn resolve(self) -> Theme {
        let detected_luma = if self == Self::Auto {
            terminal_light::luma().ok()
        } else {
            None
        };
        self.resolve_with_luma(detected_luma)
    }

    fn resolve_with_luma(self, detected_luma: Option<f32>) -> Theme {
        match self {
            Self::Light => Theme::light(),
            Self::Dark => Theme::dark(),
            Self::Auto if detected_luma.is_some_and(|luma| luma > 0.6) => Theme::light(),
            Self::Auto => Theme::dark(),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) struct Theme {
    pub(super) is_light: bool,
    pub(super) foreground: Color,
    pub(super) accent: Color,
    pub(super) warning: Color,
    pub(super) error: Color,
    pub(super) success: Color,
    pub(super) info: Color,
    pub(super) muted: Color,
    pub(super) secondary: Color,
    pub(super) panel_border: Color,
    pub(super) filter_selected_bg: Color,
    pub(super) selected_fg: Color,
    pub(super) selected_bg: Color,
    pub(super) key_fg: Color,
    pub(super) key_bg: Color,
    pub(super) user_accent: Color,
    pub(super) user_text: Color,
    pub(super) assistant_text: Color,
    pub(super) code_comment: Color,
    pub(super) code_string: Color,
    pub(super) code_literal: Color,
    pub(super) code_keyword: Color,
    pub(super) code_text: Color,
    pub(super) code_punctuation: Color,
    pub(super) match_fg: Color,
    pub(super) match_bg: Color,
    pub(super) age_new: Color,
    pub(super) age_recent: Color,
    pub(super) age_middle: Color,
    pub(super) age_old: Color,
}

impl Theme {
    pub(super) const fn dark() -> Self {
        Self {
            is_light: false,
            foreground: Color::White,
            accent: Color::Rgb(224, 150, 70),
            warning: Color::Rgb(240, 180, 80),
            error: Color::Red,
            success: Color::Green,
            info: Color::Cyan,
            muted: Color::DarkGray,
            secondary: Color::Gray,
            panel_border: Color::Rgb(70, 80, 95),
            filter_selected_bg: Color::Rgb(42, 46, 54),
            selected_fg: Color::White,
            selected_bg: Color::Rgb(68, 52, 34),
            key_fg: Color::Black,
            key_bg: Color::Gray,
            user_accent: Color::Rgb(120, 210, 255),
            user_text: Color::Rgb(180, 225, 245),
            assistant_text: Color::Rgb(220, 225, 230),
            code_comment: Color::Rgb(100, 160, 120),
            code_string: Color::Rgb(150, 220, 150),
            code_literal: Color::Rgb(210, 160, 255),
            code_keyword: Color::Rgb(120, 210, 255),
            code_text: Color::Rgb(220, 225, 230),
            code_punctuation: Color::DarkGray,
            match_fg: Color::Black,
            match_bg: Color::Rgb(250, 220, 110),
            age_new: Color::Rgb(100, 200, 50),
            age_recent: Color::Rgb(200, 180, 0),
            age_middle: Color::Rgb(200, 100, 50),
            age_old: Color::Rgb(100, 100, 100),
        }
    }

    pub(super) const fn light() -> Self {
        Self {
            is_light: true,
            foreground: Color::Reset,
            accent: Color::Rgb(160, 82, 0),
            warning: Color::Rgb(150, 82, 0),
            error: Color::Rgb(180, 35, 45),
            success: Color::Rgb(0, 110, 70),
            info: Color::Rgb(0, 92, 135),
            muted: Color::Rgb(100, 105, 115),
            secondary: Color::Rgb(60, 65, 75),
            panel_border: Color::Rgb(145, 150, 160),
            filter_selected_bg: Color::Rgb(225, 228, 232),
            selected_fg: Color::Rgb(35, 30, 25),
            selected_bg: Color::Rgb(255, 224, 190),
            key_fg: Color::Black,
            key_bg: Color::Rgb(200, 205, 212),
            user_accent: Color::Rgb(0, 92, 135),
            user_text: Color::Rgb(0, 70, 105),
            assistant_text: Color::Reset,
            code_comment: Color::Rgb(30, 105, 50),
            code_string: Color::Rgb(15, 105, 55),
            code_literal: Color::Rgb(125, 55, 165),
            code_keyword: Color::Rgb(0, 85, 135),
            code_text: Color::Reset,
            code_punctuation: Color::Rgb(95, 100, 110),
            match_fg: Color::Black,
            match_bg: Color::Rgb(245, 205, 70),
            age_new: Color::Rgb(0, 115, 45),
            age_recent: Color::Rgb(125, 100, 0),
            age_middle: Color::Rgb(165, 85, 0),
            age_old: Color::Rgb(100, 80, 125),
        }
    }

    pub(super) fn agent_color(self, agent: &AgentConfig) -> Color {
        if self.is_light {
            agent.light_color
        } else {
            agent.color
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::AGENTS;

    use super::*;

    #[test]
    fn automatic_mode_uses_terminal_luma_and_falls_back_to_dark() {
        assert_eq!(
            ThemeMode::Auto.resolve_with_luma(Some(0.61)),
            Theme::light()
        );
        assert_eq!(ThemeMode::Auto.resolve_with_luma(Some(0.60)), Theme::dark());
        assert_eq!(ThemeMode::Auto.resolve_with_luma(None), Theme::dark());
    }

    #[test]
    fn explicit_mode_ignores_detected_luma() {
        assert_eq!(
            ThemeMode::Light.resolve_with_luma(Some(0.0)),
            Theme::light()
        );
        assert_eq!(ThemeMode::Dark.resolve_with_luma(Some(1.0)), Theme::dark());
    }

    #[test]
    fn light_theme_replaces_white_agent_badges() {
        let cursor = &AGENTS["cursor"];

        assert_eq!(Theme::dark().agent_color(cursor), Color::Rgb(255, 255, 255));
        assert_eq!(Theme::light().agent_color(cursor), Color::Rgb(30, 30, 30));
    }
}
